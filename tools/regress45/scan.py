#!/usr/bin/env python3
"""scan.py -- English-residue and misplaced-translation detector for the zh-tw overlay (tools/regress45).

Plays one scene at the same time in several `xtask shots` processes (HD, every frame FROM..TO drawn): with the zh-tw
table, with an empty table (the original English, the control), and with any number of edited tables (`--control
NAME=TSV`: the zh-tw table less a page, to see what that page does to the picture). Pictures go through pipes
(`shots --raw-out /dev/fd/N`, no FIFO in the out folder) into memory and are compared frame by frame; nothing but the
flagged frames is written to disk.
The overlay's own log (SSD_OVERLAY_LOG, from tools/regress45/overlay_log.patch: the page the engine names, the pages
drawn and their paper, the letter cells) comes with each frame of the zh-tw run.

Two kinds of checks:
- the rules below, on every frame (events.tsv, summary.txt), and
- pixel probes (`--probes probes.json`, probes.tsv): boxes of the picture compared between the runs over a range of
  frames (see PROBES below).

Rules (each flagged frame lists the rules that hit; see HARNESS.md):
  REVERT     a letter cell shows the original English again although the overlay covered it before and the original
             cell has not changed since (help box back to English, `Le` `3` left beside another window)
  INPAPER    a letter cell shows the original English inside the paper of a page the overlay draws (English remnants
             in a translated window)
  WRONGPAGE  a page is drawn whose English does not match the letters the original shows under its paper (another
             page's Chinese in a window, Chinese over another window's letters)
  ZHONLY     the translated picture changes where the original picture did not change at all (overlay flicker,
             another page drawn into a still window)
  MASKED     the original changes under the paper of a page that is not being written, the translated picture does
             not (Chinese kept over a window drawn on top)
  UNDRAWN    the engine names a page the table has, its English is on screen, the overlay draws nothing
Info only (not a problem by itself): NOTR = the engine names a page the table lacks and its English is on screen.

PRESENCE (info only, never changes the exit code): the overlay log's `pres=` field is the
presence (`translate.rs` `presence`) of the pages a frame may draw -- IN (its English shows, nothing else on its cells),
COV (its English and something else), EMPTY (only blank cells of its window), OUT (nothing of it), NONE (no cell to
compare). The pages measured are the page the engine names, the pages the overlay keeps (shown, behind) and the last
six pages named. The new judgement draws each of them that is IN (rule 4 comes with it: a page whose next page is not
on the screen yet stays; so does a page written at once that reaches the screen after the engine stopped naming it),
one page per place (the first of them in that order: pages of the same English, help:33 to 39, are all IN there); a
NONE page, or one not measured, follows the old judgement. Frames where the old judgement (a page with paper)
and the new one differ: presence.tsv (each frame), pres_events.tsv (runs of frames: class, key, presence), pres_keys.tsv
(per page: its presence over the frames named and where it was found), PNGs pres/ (the first and last frame of each
event; the box: where presence put the page's first cell). Classes: OLD_ONLY (the old draws, the new does not),
NEW_ONLY (the new draws, the old does not).

PROBES (a JSON list; boxes are [x0, y0, x1, y1] pixels of the 2x picture, 744x478 wide; frames [first, last]):
  {"id": .., "kind": "equal",  "a": "zh", "b": "orig"|"<control>", "box": .., "frames": .., "tol": 0.0}
        bad frame: more than `tol` of the box's pixels differ between the two pictures
  {"id": .., "kind": "differ", "a": .., "b": .., "box": .., "frames": .., "tol": 0.02}
        bad frame: `tol` of the box's pixels or less differ (the two should not look the same)
  {"id": .., "kind": "stable", "a": "zh", "ref": FRAME, "box": .., "frames": .., "tol": 0.0}
        bad frame: the original's box is as it was at frame REF but `a`'s box is not (`a` changed by itself)
  {"id": .., "kind": "blink", "a": "zh", "color": [r, g, b], "box": .., "frames": .., "k": 8}
        one verdict for the range, bad when `a`'s count of `color` pixels in the box varies by less than `k` (no
        blinking mark) -- or the original's does (no mark to compare with: a wrong box or range)
  {"id": .., "kind": "ink", "a": "zh", "box": .., "frames": .., "dark": 200, "min": 40}
        bad frame: fewer than `min` pixels of the box are dark (every channel below `dark`): nothing written there;
        with `"max": N` instead, more than N are dark: something written where nothing should be
  {"id": .., "kind": "mark", "a": "zh", "color": [r, g, b], "box": .., "obox": .., "frames": .., "min": 4,
   "paper": [r, g, b]}
        bad frame: the original's `obox` shows the mark (`min` pixels of `color` or more) and `a`'s `box` does not, or
        the original's does not and `a`'s does -- or, with `paper`, has any pixel other than `paper` there (something
        drawn where the original shows no mark: the `▼` after the translation, `box` around where it goes)

Usage: scan.py --script S --from N --to M --out DIR --bin XTASK --data DIR --pack DIR --en-text TSV [--hooks]
               [--sram F] [--state-in F] [--wram F:ADDR=HEX]... [--table TSV] [--control NAME=TSV]... [--probes JSON]
               [--scale 2] [--max-png 300] [--per-event 12] [--keep-logs] [--legacy-env]
Exit 0 when no rule hit and no probe found a bad frame, 1 when some did, 2 on a harness error.
"""
import argparse
import json
import gzip
import os
import queue
import re
import subprocess
import sys
import threading
import time
from collections import defaultdict
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
FONT = REPO / 'translations/zh-tw/fonts/LXGWWenKaiTC-Regular.ttf'
ZH = REPO / 'translations/zh-tw/zh-tw.tsv'
EMPTY = HERE / 'empty.tsv'
RULES = ['REVERT', 'INPAPER', 'WRONGPAGE', 'ZHONLY', 'MASKED', 'UNDRAWN']
COLOUR = {'REVERT': (255, 0, 0), 'INPAPER': (255, 128, 0), 'WRONGPAGE': (255, 0, 255), 'ZHONLY': (0, 200, 255),
          'MASKED': (0, 255, 0), 'UNDRAWN': (255, 255, 0)}


def parse_args():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument('--script', required=True, help='timeline file, builtin script name, or .ssdreplay')
    p.add_argument('--from', dest='lo', type=int, required=True, help='first frame drawn and checked')
    p.add_argument('--to', dest='hi', type=int, required=True, help='last frame drawn and checked')
    p.add_argument('--out', required=True, type=Path)
    p.add_argument('--hooks', action='store_true', help='shots --app-hooks: the app hooks (cursor rows etc.)')
    p.add_argument('--sram', help='power on with this 8 KiB battery save')
    p.add_argument('--state-in', help='start from this save state')
    p.add_argument('--wram', action='append', default=[], help='FRAME:ADDR=HEX host write (repeatable)')
    p.add_argument('--mods', action='append', default=[], help='data pack (repeatable), as shots --mods')
    p.add_argument('--peek', default='', help='extra shots --peek list, printed in the logs')
    p.add_argument('--table', type=Path, default=ZH, help='translation table (default: the worktree zh-tw.tsv)')
    p.add_argument('--control', action='append', default=[], help='NAME=TSV: one more run with this table')
    p.add_argument('--probes', type=Path, help='pixel probes (JSON list, see PROBES)')
    p.add_argument('--bin', type=Path, required=True, help='xtask built with tools/regress45/overlay_log.patch')
    p.add_argument('--data', type=Path, required=True, help='SSD_DATA_DIR (the cached English ROM)')
    p.add_argument('--pack', type=Path, required=True, help='the installed HD pack folder')
    p.add_argument('--en-text', type=Path, required=True, help='`xtask text-dump` of the English ROM')
    p.add_argument('--legacy-env', action='store_true',
                   help='an older xtask: SHOTS_APP_HOOKS / SHOTS_RAW environment instead of --app-hooks / --raw-out')
    p.add_argument('--scale', type=int, default=2)
    p.add_argument('--max-png', type=int, default=300, help='cap on PNGs written for the whole run')
    p.add_argument('--per-event', type=int, default=12, help='PNGs per event (first, last and evenly between)')
    p.add_argument('--radius', type=int, default=2, help='ZHONLY: cells around an original change that may change')
    p.add_argument('--keep-logs', action='store_true', help='keep the overlay logs (gzip) next to the report')
    p.add_argument('--max-pres-png', type=int, default=200, help='cap on PRESENCE pictures for the whole run')
    return p.parse_args()


# ---------------------------------------------------------------- English text of the pages (xtask text-dump)
SLOT = re.compile(r'\{(name|num|phrase):\d+\}')
TOKEN = re.compile(r'\{[^}]*\}|<[0-9A-F]{2}>')


def load_english(path):
    pages = {}
    for line in path.read_text(encoding='utf-8').splitlines()[1:]:
        f = line.split('\t')
        if len(f) < 5:
            continue
        lines = []
        for part in f[4].split('\\n'):
            part = SLOT.sub('\x00', part)          # a slot: any letters
            part = TOKEN.sub('', part)
            lines.append(''.join(ch for ch in part if ch.isascii() and ch.isalnum() or ch == '\x00'))
        pages[f[0]] = lines
    return pages


def alnum(text):
    return ''.join(ch for ch in text if ch.isascii() and ch.isalnum())


def load_number_tails(path):
    """key -> [(n, tail)]: each `{num:n}` slot of the page and the letters its line has after it (up to the next slot
    or the line's end)."""
    pages = {}
    for line in path.read_text(encoding='utf-8').splitlines()[1:]:
        f = line.split('\t')
        if len(f) < 5:
            continue
        tails = []
        for part in f[4].split('\\n'):
            pieces = re.split(r'(\{(?:name|num|phrase):\d+\})', part)    # text, slot, text, slot, ..., text
            for i in range(1, len(pieces), 2):
                m = re.fullmatch(r'\{num:(\d+)\}', pieces[i])
                if m:
                    tails.append((int(m.group(1)), alnum(TOKEN.sub('', pieces[i + 1]))))
        if tails:
            pages[f[0]] = tails
    return pages


def number_tails(tails, numbers):
    """[(number, tail)] of the page's number slots the game wrote a number in. `numbers` are the overlay log's `nums=`
    (the page's `{num:1}` to `{num:3}`), None when the log has none (an xtask built before it)."""
    if not numbers:
        return []
    out = []
    for n, tail in tails or ():
        value = alnum(numbers[n - 1]) if 0 < n <= len(numbers) else ''
        if value:
            out.append((value, tail))
    return out


def typed_after_number(row, numbers):
    """Whether `row` is a slot's number and then the start (maybe none of it) of what its line has after the slot:
    the line being typed after the game wrote the number (the gravity training's `10`, `10x`, `10xG`, ...)."""
    return any(row.startswith(value) and tail.startswith(row[len(value):]) for value, tail in numbers)


def lcs(a, b):
    if not a or not b:
        return 0
    prev = [0] * (len(b) + 1)
    for ca in a:
        cur = [0]
        for j, cb in enumerate(b):
            cur.append(prev[j] + 1 if ca == cb else max(prev[j + 1], cur[j]))
        prev = cur
    return prev[-1]


def row_fits(row, lines, numbers=()):
    """Whether the letters `row` (one row of a window, left to right) can be a piece of one of the page's lines.
    `numbers`: [`number_tails`] of the page, when the game's numbers in its slots are known."""
    for line in lines:
        lit = line.replace('\x00', '')
        slots = line.count('\x00')
        common = lcs(row, lit)
        if common / len(row) >= 0.6:
            return True
        # a name fills each slot (names are at most about ten letters): the letters the line does not have may be
        # the names', while the line's own letters are there in order (or not written yet: the engine writes the
        # name first, `{name:1} attacks`)
        if slots and len(row) - common <= 12 * slots and (common >= 0.6 * min(len(row), len(lit)) or common == 0):
            return True
    return typed_after_number(row, numbers)


# ---------------------------------------------------------------- the overlay log
def parse_ovl(line):
    """`OVL wx=.. named=KEY tr=.. st=.. ... | shown=KEY paper=x0,y0,x1,y1 place=.. | behind=... | letters=x:y:c;...`"""
    parts = line[4:].split(' | ')
    head = parts[0].split()
    o = {'wx': int(head[0][3:]), 'named': None}
    if head[1] != 'named=-':
        kv = dict(x.split('=', 1) for x in head[2:])
        o['named'] = {'key': head[1][6:], 'tr': kv['tr'] == '1', 'st': int(kv['st']), 'prog': float(kv['prog']),
                      'base': kv['base'], 'lines': int(kv['lines']),
                      'nums': kv['nums'].split(',') if 'nums' in kv else None}

    def page(text, prefix):
        f = text.split()
        key = f[0][len(prefix):]
        paper = f[1][6:]
        rect = tuple(int(v) for v in paper.split(',')) if paper != '-' else None
        return {'key': key, 'paper': rect} if key != '-' else None

    o['shown'] = page(parts[1], 'shown=')
    o['behind'] = page(parts[2], 'behind=')
    letters = []
    for item in parts[3][8:].split(';'):
        if item:
            x, y, c = item.split(':')
            letters.append((int(x), int(y), c))
    o['letters'] = letters
    o['pres'] = parse_pres(parts[4][5:]) if len(parts) > 4 and parts[4].startswith('pres=') else None
    return o


PRES_FIELDS = ['state', 'own', 'blank', 'other', 'skip', 'letters', 'dx', 'dy', 'pitch', 'matched', 'x', 'y']


def parse_pres(text):
    """`KEY=STATE,own,blank,other,skipped,letters,dx,dy,pitch,matched,x,y,misses;...` (overlay_log.patch)."""
    pres = {}
    for item in text.split(';'):
        if not item:
            continue
        key, _, rest = item.partition('=')
        f = rest.split(',')
        d = {'state': f[0]}
        for name, v in zip(PRES_FIELDS[1:], f[1:12]):
            d[name] = int(v)
        d['misses'] = f[12] if len(f) > 12 else ''
        pres[key] = d
    return pres


def misses_text(misses):
    """`line.cell.EE.GOT+...` as `line.cell 'e'>'g'` (GOT: a code point, C card, B blank of another ground, N none)."""
    out = []
    for m in misses.split('+'):
        if not m:
            continue
        line, cell, ee, got = m.split('.')
        g = {'C': 'card', 'B': 'blank', 'N': 'none'}.get(got) or repr(chr(int(got, 16)))
        out.append(f"{line}.{cell} {chr(int(ee, 16))!r}>{g}")
    return ' '.join(out)


class PresenceCheck:
    """The presence judgement replayed from the `pres=` field, against the old one (see PRESENCE)."""

    def __init__(self):
        self.keys = defaultdict(lambda: [defaultdict(int), defaultdict(int)])   # key -> [state counts, places]

    def frame(self, o):
        if not o or o.get('pres') is None:
            return None
        pres = o['pres']
        old = {p['key'] for p in (o['shown'], o['behind']) if p and p['paper']}
        named = o['named']
        n = named['key'] if named and named['tr'] else None
        if n and n in pres:
            d = pres[n]
            self.keys[n][0][d['state']] += 1
            self.keys[n][1][(d['dx'], d['dy'], d['pitch'], d['matched'])] += 1

        new, places = set(), set()
        for key, d in pres.items():       # (the page named first, then shown, behind, the pages named last)
            if d['state'] == 'IN' or (d['state'] == 'NONE' and key in old):
                if (d['x'], d['y']) not in places:
                    places.add((d['x'], d['y']))
                    new.add(key)
        new |= {key for key in old if key not in pres}
        return ([('OLD_ONLY', k) for k in sorted(old - new)] + [('NEW_ONLY', k) for k in sorted(new - old)],
                old, new)


def save_pres_png(path, frame, zh, orig, o, cls, key, s):
    h, w = zh.shape[:2]
    im = Image.new('RGB', (2 * w + 8, h + 52), (40, 40, 40))
    im.paste(Image.fromarray(np.ascontiguousarray(zh)), (0, 52))
    im.paste(Image.fromarray(np.ascontiguousarray(orig)), (w + 8, 52))
    d = ImageDraw.Draw(im)
    named = o['named']['key'] if o.get('named') else '-'
    shown = o['shown']['key'] if o.get('shown') else '-'
    behind = o['behind']['key'] if o.get('behind') else '-'
    p = o['pres'].get(key)
    d.text((4, 2), f"frame {frame}  {cls} {key}  left: zh-tw  right: original  named={named} shown={shown} "
                   f"behind={behind}", fill=(255, 255, 255))
    if p:
        d.text((4, 18), f"presence {p['state']} own {p['own']} blank {p['blank']} other {p['other']} skip "
                        f"{p['skip']} of {p['letters']}  dx {p['dx']} dy {p['dy']} pitch {p['pitch']} "
                        f"matched {p['matched']}", fill=(255, 255, 0))
        d.text((4, 34), misses_text(p['misses'])[:200], fill=(255, 200, 0))
        for dx in (0, w + 8):
            x, y = p['x'] * s + dx, p['y'] * s + 52
            d.rectangle((x, y, x + 8 * s - 1, y + 8 * s - 1), outline=(0, 255, 255))
    for page in (o.get('shown'), o.get('behind')):
        if page and page['key'] == key and page['paper']:
            x0, y0, x1, y1 = page['paper']
            for dx in (0, w + 8):
                d.rectangle((x0 * s + dx, y0 * s + 52, x1 * s + dx - 1, y1 * s + 52 - 1), outline=(255, 0, 0))
    im.save(path, compress_level=3)


class Run:
    """One `xtask shots` process: pictures from its pipe, its log lines by frame.

    The pictures come through an anonymous pipe handed to that one child as /dev/fd/N, not a named FIFO in the out
    folder: anything else that opened a FIFO there (`grep -r` over the out folder opens every file, FIFOs included)
    became a second reader, took bytes of the stream and left this reader in the middle of a picture."""

    def __init__(self, name, args, table, ovl):
        self.name = name
        frames = ','.join(str(f) for f in range(args.lo, args.hi + 1))
        cmd = [str(args.bin), 'shots', args.script, '--frames', frames, '--no-png', '--hd-scale', str(args.scale),
               '--pack', str(args.pack), '--translation', str(table), '--translation-font', str(FONT)]
        for w in args.wram:
            cmd += ['--wram', w]
        for m in args.mods:
            cmd += ['--mods', m]
        if args.sram:
            cmd += ['--sram', args.sram]
        if args.state_in:
            cmd += ['--state-in', args.state_in]
        if args.peek:
            cmd += ['--peek', args.peek]
        env = dict(os.environ, SSD_DATA_DIR=str(args.data))
        for k in ('SSD_OVERLAY_LOG', 'SHOTS_APP_HOOKS', 'SHOTS_RAW'):
            env.pop(k, None)
        if ovl:
            env['SSD_OVERLAY_LOG'] = '1'
        rfd, wfd = os.pipe()            # (not inheritable: only this child gets the write end, by pass_fds)
        fifo = f'/dev/fd/{wfd}'
        if args.legacy_env:
            env['SHOTS_RAW'] = str(fifo)
            if args.hooks:
                env['SHOTS_APP_HOOKS'] = '1'
        else:
            cmd += ['--raw-out', str(fifo)]
            if args.hooks:
                cmd += ['--app-hooks']
        self.proc = subprocess.Popen(cmd, cwd=REPO, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                     text=True, bufsize=1, pass_fds=(wfd,))
        os.close(wfd)                   # (the child's copy is the only writer: EOF when it exits)
        self.lines = queue.Queue()
        self.text = []          # every line but OVL (for the log file)
        self.ovl_gz = None
        threading.Thread(target=self._read, daemon=True).start()
        self.fifo = os.fdopen(rfd, 'rb', buffering=1 << 22)
        self.pending = None

    def _read(self):
        ovl = None
        for line in self.proc.stdout:
            line = line.rstrip('\n')
            if line.startswith('OVL '):
                ovl = line
                if self.ovl_gz:
                    self.ovl_gz.write(line + '\n')
                continue
            self.text.append(line)
            m = re.match(r'frame\s+(\d+) hd', line)
            if m:
                self.lines.put((int(m.group(1)), line, ovl))
                ovl = None
        self.lines.put(None)

    def _read_exact(self, n):
        buf = bytearray()
        while len(buf) < n:
            chunk = self.fifo.read(n - len(buf))
            if not chunk:
                if self.proc.poll() is not None:
                    return None
                time.sleep(0.05)      # (end of the pipe: the child is exiting)
                continue
            buf += chunk
        return bytes(buf)

    def next(self):
        """(frame, picture HxWx3, frame log line, overlay line) of the next frame drawn; None at the end. The
        picture comes first (shots prints the frame's log line only once the FIFO took the picture); the log lines
        of frames not drawn in between are skipped (the caller sees the gap)."""
        head = b''
        while not head.endswith(b'\n'):
            ch = self._read_exact(1)
            if ch is None:
                return None
            head += ch
        _, f, w, h = head.decode().split()
        frame = int(f)
        data = self._read_exact(int(w) * int(h) * 4)
        if data is None:
            return None
        pic = np.frombuffer(data, np.uint8).reshape(int(h), int(w), 4)[:, :, :3]
        while True:
            item = self.lines.get()
            if item is None:
                raise RuntimeError(f'{self.name}: no log line for frame {frame}')
            if item[0] == frame:
                return frame, pic, item[1], item[2]
            if item[0] > frame:
                raise RuntimeError(f'{self.name}: log of frame {item[0]} before the picture of frame {frame}')


# ---------------------------------------------------------------- per-frame checks
def blocks(mask, s):
    """Any-true per 8x8 (1x) cell of a per-pixel mask at scale s."""
    b = 8 * s
    h, w = mask.shape
    hp, wp = -(-h // b) * b, -(-w // b) * b
    pad = np.zeros((hp, wp), bool)
    pad[:h, :w] = mask
    return pad.reshape(hp // b, b, wp // b, b).any(axis=(1, 3))


def dilate(m, r):
    out = m.copy()
    for dy in range(-r, r + 1):
        for dx in range(-r, r + 1):
            sh = np.zeros_like(m)
            ys = slice(max(dy, 0), m.shape[0] + min(dy, 0))
            yd = slice(max(-dy, 0), m.shape[0] + min(-dy, 0))
            xs = slice(max(dx, 0), m.shape[1] + min(dx, 0))
            xd = slice(max(-dx, 0), m.shape[1] + min(-dx, 0))
            sh[ys, xs] = m[yd, xd]
            out |= sh
    return out


def cell_box(x, y, s, shape):
    h, w = shape[:2]
    return max(0, y * s), min(h, (y + 8) * s), max(0, x * s), min(w, (x + 8) * s)


def inside(x, y, rect):
    return rect and rect[0] <= x + 4 < rect[2] and rect[1] <= y + 4 < rect[3]


def rows_in(letters, rect):
    rows = defaultdict(list)
    for x, y, c in letters:
        if inside(x, y, rect):
            rows[y].append((x, c))
    return {y: ''.join(c for _, c in sorted(v)) for y, v in rows.items()}


class Checker:
    def __init__(self, args, english, tails=None):
        self.s = args.scale
        self.r = args.radius
        self.english = english
        self.tails = tails or {}    # key -> [(n, tail)] of the page's number slots (load_number_tails)
        self.covered = {}   # (x, y, c) -> hash of the original's cell when the overlay last covered it
        self.prev = None    # (frame, zh, orig)

    def check(self, frame, zh, orig, o):
        s = self.s
        hits = defaultdict(list)       # rule -> [(x0, y0, x1, y1) picture pixels, note]
        letters = o['letters'] if o else []
        pages = [p for p in ((o or {}).get('shown'), (o or {}).get('behind')) if p and p['paper']]
        named = (o or {}).get('named')
        if zh.shape != orig.shape:
            raise RuntimeError(f'frame {frame}: zh picture {zh.shape} and original {orig.shape} differ in size')
        diff = (zh != orig).any(axis=2)
        # letter cells: English visible (the same pixels as the original) or covered
        for x, y, c in letters:
            y0, y1, x0, x1 = cell_box(x, y, s, zh.shape)
            if y1 <= y0 or x1 <= x0:
                continue
            key = (x, y, c)
            ob = orig[y0:y1, x0:x1]
            h = hash(ob.tobytes())
            if diff[y0:y1, x0:x1].any():
                self.covered[key] = h
                continue
            if self.covered.get(key) == h:
                hits['REVERT'].append(((x0, y0, x1, y1), c))
            else:
                self.covered.pop(key, None)
            for p in pages:
                if inside(x, y, p['paper']):
                    hits['INPAPER'].append(((x0, y0, x1, y1), f"{c}@{p['key']}"))
                    break
        # a page drawn over letters that are not its English (letters the overlay covered: Chinese over them)
        hidden = []
        for x, y, c in letters:
            y0, y1, x0, x1 = cell_box(x, y, s, zh.shape)
            if y1 > y0 and x1 > x0 and diff[y0:y1, x0:x1].any():
                hidden.append((x, y, c))
        for p in pages:
            lines = self.english.get(p['key'])
            if not lines:
                continue
            # the numbers the game wrote in the slots of the page the engine names (`10` of the gravity training's
            # `{num:1}x Gravity`): the number and then the line typed after it are the page's own letters
            numbers = (number_tails(self.tails.get(p['key']), named['nums'])
                       if named and named['key'] == p['key'] else [])
            for y, row in rows_in(hidden, p['paper']).items():
                if len(row) >= 3 and not row_fits(row, lines, numbers):
                    x0, y0, x1, y1 = p['paper']
                    hits['WRONGPAGE'].append(((x0 * s, y * s, x1 * s, (y + 8) * s), f"{p['key']} over '{row}'"))
        # the engine names a translated page whose English is on screen and nothing is drawn
        if named and not pages:
            lines = self.english.get(named['key'])
            if lines:
                rows = rows_in(letters, (0, 0, 10 ** 6, 10 ** 6))
                seen = [y for y, row in rows.items() if len(row) >= 4 and row_fits(row, lines)]
                if seen:
                    rule = 'UNDRAWN' if named['tr'] else 'NOTR'
                    hits[rule].append(((0, min(seen) * s, zh.shape[1], (max(seen) + 8) * s), named['key']))
        # changes from the frame before
        if self.prev and self.prev[0] == frame - 1 and self.prev[1].shape == zh.shape:
            _, pz, po = self.prev
            zch = blocks((zh != pz).any(axis=2), s)
            och = blocks((orig != po).any(axis=2), s)
            cov = blocks(diff, s)
            calm = ~dilate(och, self.r)
            for p in pages:      # a page whose window the original changes in may change all over (writing)
                x0, y0, x1, y1 = p['paper']
                by0, by1, bx0, bx1 = max(0, y0 // 8 - 1), y1 // 8 + 2, max(0, x0 // 8 - 1), x1 // 8 + 2
                if och[by0:by1, bx0:bx1].any():
                    calm[by0:by1, bx0:bx1] = False
            # (a row of the original that changes may move the translation's glyphs along it: the intro's lines)
            band = och.any(axis=1)
            band = band | np.roll(band, 1) | np.roll(band, -1)
            calm[band, :] = False
            for by, bx in zip(*np.nonzero(zch & ~och & calm)):
                b = 8 * s
                hits['ZHONLY'].append(((bx * b, by * b, (bx + 1) * b, (by + 1) * b), ''))
            writing = None
            if named and named['st'] in (1, 5):
                writing = named['key']
            masked = []
            fits = {}
            for p in pages:      # the rows of the original under a page's paper that are the page's English
                lines = self.english.get(p['key'])
                fits[p['key']] = [y for y, row in rows_in(letters, p['paper']).items()
                                  if lines and len(row) >= 2 and row_fits(row, lines)]
            for by, bx in zip(*np.nonzero(och & ~zch & cov)):
                cx, cy = bx * 8 + 4, by * 8 + 4
                for p in pages:
                    x0, y0, x1, y1 = p['paper']
                    if x0 <= cx < x1 and y0 <= cy < y1 and p['key'] != writing:
                        # (the page's own English coming back on its rows, the window over it gone, is no problem)
                        if any(y - 8 < by * 8 + 8 and by * 8 < y + 16 for y in fits[p['key']]):
                            break
                        b = 8 * s
                        masked.append(((bx * b, by * b, (bx + 1) * b, (by + 1) * b), p['key']))
                        break
            if len(masked) >= 2:
                hits['MASKED'] = masked
        self.prev = (frame, zh, orig)
        return hits


def merge_boxes(items):
    if not items:
        return None
    return (min(b[0] for b, _ in items), min(b[1] for b, _ in items), max(b[2] for b, _ in items),
            max(b[3] for b, _ in items))


def save_png(path, frame, zh, orig, hits, o):
    h, w = zh.shape[:2]
    im = Image.new('RGB', (2 * w + 8, h + 36), (40, 40, 40))
    im.paste(Image.fromarray(np.ascontiguousarray(zh)), (0, 36))
    im.paste(Image.fromarray(np.ascontiguousarray(orig)), (w + 8, 36))
    d = ImageDraw.Draw(im)
    named = o['named']['key'] if o and o.get('named') else '-'
    shown = o['shown']['key'] if o and o.get('shown') else '-'
    behind = o['behind']['key'] if o and o.get('behind') else '-'
    d.text((4, 2), f"frame {frame}  left: zh-tw  right: empty table (original)  named={named} shown={shown} "
                   f"behind={behind}", fill=(255, 255, 255))
    d.text((4, 18), '  '.join(f"{r}:{len(v)}" for r, v in hits.items()), fill=(255, 255, 0))
    for rule, items in hits.items():
        for (x0, y0, x1, y1), _ in items:
            for dx in (0, w + 8):
                d.rectangle((x0 + dx, y0 + 36, x1 + dx - 1, y1 + 36 - 1), outline=COLOUR.get(rule, (200, 200, 200)))
    im.save(path, optimize=False, compress_level=3)


# ---------------------------------------------------------------- pixel probes
def crop(pic, box):
    x0, y0, x1, y1 = box
    h, w = pic.shape[:2]
    return pic[max(0, y0):min(h, y1), max(0, x0):min(w, x1)]


def ranges(frames):
    out = []
    for f in frames:
        if out and out[-1][1] == f - 1:
            out[-1][1] = f
        else:
            out.append([f, f])
    return ','.join(f'{a}' if a == b else f'{a}-{b}' for a, b in out)


class Probes:
    """The pixel probes of `--probes`: per probe the frames checked, the bad ones, the worst value; a picture of the
    first bad frame (or, for `blink`, of the frame the original's mark showed most)."""

    def __init__(self, specs, out):
        self.specs = specs
        self.out = out
        self.state = {p['id']: {'checked': 0, 'skipped': 0, 'bad': [], 'worst': None, 'ref': None, 'counts': [],
                                'pic': None, 'best': -1} for p in specs}

    def frame(self, frame, pics):
        for p in self.specs:
            st = self.state[p['id']]
            kind = p['kind']
            box = p['box']
            a = pics.get(p.get('a', 'zh'))
            if a is None:
                raise RuntimeError(f"probe {p['id']}: no run `{p.get('a')}`")
            if kind == 'stable' and frame == p['ref']:
                st['ref'] = (crop(pics['orig'], box).copy(), crop(a, box).copy())
            lo, hi = p['frames']
            if not lo <= frame <= hi:
                continue
            if kind in ('equal', 'differ'):
                b = pics.get(p['b'])
                if b is None:
                    raise RuntimeError(f"probe {p['id']}: no run `{p['b']}`")
                ca, cb = crop(a, box), crop(b, box)
                if ca.shape != cb.shape or ca.size == 0:
                    st['skipped'] += 1
                    continue
                value = float((ca != cb).any(axis=2).mean())
                tol = p.get('tol', 0.0 if kind == 'equal' else 0.02)
                bad = value > tol if kind == 'equal' else value <= tol
                worse = max if kind == 'equal' else min
            elif kind == 'stable':
                if st['ref'] is None:
                    raise RuntimeError(f"probe {p['id']}: reference frame {p['ref']} not drawn before {frame}")
                ro, ra = st['ref']
                co, ca = crop(pics['orig'], box), crop(a, box)
                if co.shape != ro.shape or ca.shape != ra.shape or not np.array_equal(co, ro):
                    st['skipped'] += 1          # (the original changed there: nothing to say)
                    continue
                value = float((ca != ra).any(axis=2).mean())
                bad = value > p.get('tol', 0.0)
                worse = max
            elif kind == 'blink':
                colour = np.array(p['color'], np.uint8)
                ca, co = crop(a, box), crop(pics['orig'], box)
                na = int((ca == colour).all(axis=2).sum())
                no = int((co == colour).all(axis=2).sum())
                st['counts'].append((frame, na, no))
                if no > st['best']:
                    st['best'] = no
                    st['pic'] = (frame, {k: v.copy() for k, v in pics.items()})
                st['checked'] += 1
                continue
            elif kind == 'mark':
                colour = np.array(p['color'], np.uint8)
                ca, co = crop(a, box), crop(pics['orig'], p['obox'])
                value = int((ca == colour).all(axis=2).sum())
                shows = int((co == colour).all(axis=2).sum()) >= p.get('min', 4)
                if shows:
                    bad = value < p.get('min', 4)
                elif 'paper' in p:
                    bad = not (ca == np.array(p['paper'], np.uint8)).all()
                else:
                    bad = value >= p.get('min', 4)
                worse = max
            elif kind == 'ink':
                ca = crop(a, box)
                value = int((ca < p.get('dark', 200)).all(axis=2).sum())
                if 'max' in p:
                    bad = value > p['max']
                    worse = max
                else:
                    bad = value < p.get('min', 40)
                    worse = min
            else:
                raise RuntimeError(f"probe {p['id']}: unknown kind {kind}")
            st['worst'] = value if st['worst'] is None else worse(st['worst'], value)
            st['checked'] += 1
            if bad:
                if not st['bad']:
                    self.picture(p, frame, pics)
                st['bad'].append(frame)

    def picture(self, p, frame, pics):
        first = p.get('a', 'zh')
        names = list(dict.fromkeys([first] + [n for n in (p.get('b'), 'orig') if n]))
        h, w = pics['zh'].shape[:2]
        im = Image.new('RGB', (len(names) * (w + 8), h + 20), (40, 40, 40))
        d = ImageDraw.Draw(im)
        d.text((4, 2), f"probe {p['id']} ({p['kind']}) frame {frame}: " + ' | '.join(names), fill=(255, 255, 255))
        for i, n in enumerate(names):
            im.paste(Image.fromarray(np.ascontiguousarray(pics[n])), (i * (w + 8), 20))
            x0, y0, x1, y1 = p['box']
            d.rectangle((x0 + i * (w + 8), y0 + 20, x1 - 1 + i * (w + 8), y1 - 1 + 20), outline=(255, 0, 255))
        im.save(self.out / f"probe_{p['id']}_f{frame:06d}.png", compress_level=3)

    def finish(self):
        rows = []
        for p in self.specs:
            st = self.state[p['id']]
            note = ''
            if p['kind'] == 'blink':
                if st['counts']:
                    ra = max(c[1] for c in st['counts']) - min(c[1] for c in st['counts'])
                    ro = max(c[2] for c in st['counts']) - min(c[2] for c in st['counts'])
                    st['worst'] = ra
                    # (an original with no blinking mark there is a wrong box or range: bad too, not a pass)
                    if ro < p.get('k', 8) or ra < p.get('k', 8):
                        st['bad'] = [st['pic'][0]]
                        self.picture(p, st['pic'][0], st['pic'][1])
                    note = f"orig varies {ro}, {p.get('a', 'zh')} varies {ra}"
                else:
                    note = 'no frame'
            worst = st['worst']
            rows.append((p['id'], p['kind'], st['checked'], st['skipped'], len(st['bad']),
                         st['bad'][0] if st['bad'] else '-', st['bad'][-1] if st['bad'] else '-',
                         '-' if worst is None else round(worst, 4), ranges(st['bad'])[:300], note))
        with open(self.out / 'probes.tsv', 'w') as f:
            f.write('id\tkind\tchecked\tskipped\tbad\tfirst_bad\tlast_bad\tworst\tbad_frames\tnote\n')
            for r in rows:
                f.write('\t'.join(map(str, r)) + '\n')
        return rows


def main():
    args = parse_args()
    out = args.out
    (out / 'frames').mkdir(parents=True, exist_ok=True)
    controls = []
    for c in args.control:
        name, _, table = c.partition('=')
        if not table or name in ('zh', 'orig'):
            print(f'bad --control {c}', file=sys.stderr)
            sys.exit(2)
        controls.append((name, Path(table)))
    english = load_english(args.en_text)
    probes = Probes(json.loads(args.probes.read_text()) if args.probes else [], out)
    started = time.time()
    zh_run = Run('zh', args, args.table, ovl=True)
    if args.keep_logs:
        zh_run.ovl_gz = gzip.open(out / 'overlay_zh.log.gz', 'wt')
    orig_run = Run('orig', args, EMPTY, ovl=False)
    ctl_runs = [(name, Run(name, args, table, ovl=False)) for name, table in controls]
    checker = Checker(args, english, load_number_tails(args.en_text))
    pres_check = PresenceCheck()
    (out / 'pres').mkdir(exist_ok=True)
    pres_report = open(out / 'presence.tsv', 'w')
    pres_report.write('frame\tclass\tkey\tstate\town\tblank\tother\tskip\tletters\tdx\tdy\tpitch\tmatched\tmisses\n')
    pres_events = []        # [class, key, state, first, last, frames, misses of the first frame]
    pres_open = {}          # (class, key) -> event
    pres_written = set()
    pres_frames = 0
    pres_last = {}          # (class, key) -> (frame, zh, orig, o) of the event's last frame so far
    report = open(out / 'report.tsv', 'w')
    report.write('frame\trule\tbox_x0,y0,x1,y1\tnote\n')
    flagged = {}            # frame -> {rule: count}
    events = []             # [rule, key, first, last, frames, box, example, last picture or None]
    open_ev = {}            # (rule, key) -> event
    named_keys = defaultdict(lambda: [0, False, 0, 0])   # key -> [frames, tr, first, last]
    written = set()

    def write(frame, pic):
        if frame in written or len(written) >= args.max_png:
            return
        zh_, orig_, hits_, o_ = pic
        rules = '-'.join(r for r in RULES if r in hits_)
        save_png(out / 'frames' / f'f{frame:06d}_{rules}.png', frame, zh_, orig_, hits_, o_)
        written.add(frame)

    def close(ev):
        if ev[7] is not None:       # the event's last frame
            write(ev[3], ev[7])
            ev[7] = None

    n = 0
    while True:
        a = zh_run.next()
        b = orig_run.next()
        cs = [(name, r.next()) for name, r in ctl_runs]
        if a is None or b is None or any(c is None for _, c in cs):
            break
        frame, zh, _, ovl = a
        if b[0] != frame or any(c[0] != frame for _, c in cs):
            raise RuntimeError(f'frames out of step: zh {frame}, orig {b[0]}, ' +
                               ', '.join(f'{name} {c[0]}' for name, c in cs))
        orig = b[1]
        n += 1
        probes.frame(frame, dict([('zh', zh), ('orig', orig)] + [(name, c[1]) for name, c in cs]))
        o = parse_ovl(ovl) if ovl else None
        if o and o['named']:
            k = named_keys[o['named']['key']]
            k[0] += 1
            k[1] = o['named']['tr']
            k[2] = k[2] or frame
            k[3] = frame
        hits = checker.check(frame, zh, orig, o)
        judged = pres_check.frame(o)
        if judged:
            differ = judged[0]
            pres_frames += bool(differ)
            for cls, key in differ:
                p = o['pres'].get(key)
                state = p['state'] if p else '-'
                cols = [str(p[f]) for f in PRES_FIELDS[1:10]] if p else ['-'] * 9
                pres_report.write('\t'.join([str(frame), cls, key, state] + cols +
                                            [misses_text(p['misses']) if p else '']) + '\n')
                ev = pres_open.get((cls, key))
                if ev and ev[4] == frame - 1 and ev[2] == state:
                    ev[4] = frame
                    ev[5] += 1
                else:
                    if ev:
                        last = pres_last.pop((cls, key), None)
                        if last and last[0] not in pres_written and len(pres_written) < args.max_pres_png:
                            save_pres_png(out / 'pres' / f'p{last[0]:06d}_{cls}_last.png', *last, cls, key, args.scale)
                            pres_written.add(last[0])
                    ev = [cls, key, state, frame, frame, 1, misses_text(p['misses']) if p else '']
                    pres_events.append(ev)
                    pres_open[(cls, key)] = ev
                    if frame not in pres_written and len(pres_written) < args.max_pres_png:
                        save_pres_png(out / 'pres' / f'p{frame:06d}_{cls}.png', frame, zh, orig, o, cls, key, args.scale)
                        pres_written.add(frame)
                pres_last[(cls, key)] = (frame, zh, orig, o)
        for (rule, key), ev in list(open_ev.items()):
            if ev[3] < frame - 1:
                close(ev)
                del open_ev[(rule, key)]
        real = {r: v for r, v in hits.items() if v and r != 'NOTR'}
        pic = (zh, orig, real, o) if real else None
        for rule, items in hits.items():
            if not items:
                continue
            notes = sorted({note.split(' over ')[0] if rule == 'WRONGPAGE' else note for _, note in items})
            key = notes[0] if rule in ('WRONGPAGE', 'MASKED', 'UNDRAWN', 'NOTR') else ''
            if rule == 'INPAPER':
                key = notes[0].split('@')[-1]
            for box, note in items:
                report.write(f"{frame}\t{rule}\t{','.join(map(str, box))}\t{note}\n")
            ev = open_ev.get((rule, key))
            box = merge_boxes(items)
            if ev:
                ev[3] = frame
                ev[4].append(frame)
                ev[5] = merge_boxes([(ev[5], ''), (box, '')])
            else:
                ev = [rule, key, frame, frame, [frame], box, ' '.join(n_ for _, n_ in items[:6]), None]
                events.append(ev)
                open_ev[(rule, key)] = ev
            if rule == 'NOTR':
                continue
            # pictures: the event's first `per_event - 1` frames as they come, its last one when it ends
            if len(ev[4]) < args.per_event:
                write(frame, pic)
            else:
                ev[7] = pic
        if real:
            flagged[frame] = {r: len(v) for r, v in real.items()}
    for ev in events:
        close(ev)
    for (cls, key), last in pres_last.items():
        ev = pres_open.get((cls, key))
        if ev and ev[3] != last[0] and last[0] not in pres_written and len(pres_written) < args.max_pres_png:
            save_pres_png(out / 'pres' / f'p{last[0]:06d}_{cls}_last.png', *last, cls, key, args.scale)
            pres_written.add(last[0])
    pres_report.close()
    with open(out / 'pres_events.tsv', 'w') as f:
        f.write('class\tkey\tstate\tfirst\tlast\tframes\tmisses\n')
        for ev in pres_events:
            f.write('\t'.join(map(str, ev)) + '\n')
    with open(out / 'pres_keys.tsv', 'w') as f:
        f.write('key\tframes_named\tstates\tplaces (dx,dy,pitch,matched: frames)\n')
        for key, (states, places) in sorted(pres_check.keys.items()):
            top = sorted(places.items(), key=lambda kv: -kv[1])[:4]
            f.write(f"{key}\t{sum(states.values())}\t" + ' '.join(f'{k}:{v}' for k, v in sorted(states.items())) +
                    '\t' + ' '.join(f"{a},{b},{c},{m}:{v}" for (a, b, c, m), v in top) + '\n')
    runs = [('zh', zh_run), ('orig', orig_run)] + ctl_runs
    for _, run in runs:
        run.proc.wait()
    probe_rows = probes.finish()
    report.close()
    elapsed = time.time() - started
    with open(out / 'events.tsv', 'w') as f:
        f.write('rule\tkey\tfirst\tlast\tframes\tbox\texample\n')
        for ev in events:
            f.write(f"{ev[0]}\t{ev[1]}\t{ev[2]}\t{ev[3]}\t{len(ev[4])}\t{','.join(map(str, ev[5]))}\t{ev[6]}\n")
    with open(out / 'keys.tsv', 'w') as f:
        f.write('key\ttranslated\tframes_named\tfirst\tlast\n')
        for key, (cnt, tr, first, last) in sorted(named_keys.items(), key=lambda kv: kv[1][2]):
            f.write(f"{key}\t{int(tr)}\t{cnt}\t{first}\t{last}\n")
    for name, run in runs:
        (out / f'shots_{name}.log').write_text('\n'.join(l for l in run.text if not l.startswith('frame')) + '\n')
    if zh_run.ovl_gz:
        zh_run.ovl_gz.close()
    rc = [run.proc.returncode for _, run in runs]
    per_rule = defaultdict(int)
    for v in flagged.values():
        for r in v:
            per_rule[r] += 1
    lines = [f"scene {args.script} frames {args.lo}..{args.hi} hooks={int(args.hooks)} table={args.table}",
             f"frames compared {n} of {args.hi - args.lo + 1} in {elapsed:.0f} s; shots rc " +
             ' '.join(f'{name}={r}' for (name, _), r in zip(runs, rc)),
             f"flagged frames {len(flagged)}: " + ', '.join(f'{r} {per_rule[r]}' for r in RULES if per_rule[r]),
             'events (rule key first..last frames):']
    for ev in events:
        if ev[0] != 'NOTR':
            lines.append(f"  {ev[0]:9} {ev[1]:18} {ev[2]}..{ev[3]} ({len(ev[4])})  {ev[6][:100]}")
    notr = [ev for ev in events if ev[0] == 'NOTR']
    if notr:
        lines.append('info NOTR (pages the table lacks, English on screen): ' +
                     ', '.join(sorted({ev[1] for ev in notr})))
    lines.append(f"presence (info): frames where the old and the new judgement differ {pres_frames}; events "
                 f"(class key presence first..last frames):")
    for ev in pres_events[:150]:
        lines.append(f"  {ev[0]:8} {ev[1]:18} {ev[2]:5} {ev[3]}..{ev[4]} ({ev[5]})  {ev[6][:80]}")
    if len(pres_events) > 150:
        lines.append(f"  ... {len(pres_events) - 150} more in pres_events.tsv")
    if probe_rows:
        lines.append('probes (id kind checked bad first..last worst):')
        for r in probe_rows:
            lines.append(f"  {r[0]:28} {r[1]:7} {r[2]:5} bad {r[4]:5} {r[5]}..{r[6]} worst {r[7]} {r[9]}")
    (out / 'summary.txt').write_text('\n'.join(lines) + '\n')
    print('\n'.join(lines))
    if n != args.hi - args.lo + 1 or any(rc):
        print('HARNESS ERROR: not every frame compared or shots failed; see shots_*.log', file=sys.stderr)
        sys.exit(2)
    sys.exit(1 if flagged or any(r[4] for r in probe_rows) else 0)


if __name__ == '__main__':
    # A crash (an exception: Python exits 1) is a harness error, not "frames flagged" (1): exit 2, so run.py
    # judges the run's scenarios ERROR and --reuse does not keep its partial hits (a FIFO read under load once
    # ended the `battle` run at frame ~2700 and its scenarios came out XFAIL).
    try:
        main()
    except Exception:
        import traceback
        traceback.print_exc()
        print('HARNESS ERROR: scan.py crashed; see the traceback above', file=sys.stderr)
        sys.exit(2)
