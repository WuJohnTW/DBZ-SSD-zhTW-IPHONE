#!/usr/bin/env python3
"""sweep.py -- English the zh-tw overlay hides only when it saw the frames before (a state loaded, the language
switched): the overlay drawn every frame against the overlay started cold at sampled frames.

    python3 tools/regress45/sweep.py --inputs DIR                       # every replay of DIR/secrets/replays
    python3 tools/regress45/sweep.py --inputs DIR secret-nail-porunga   # these replays only
    python3 tools/regress45/sweep.py --inputs DIR --every 10 --from 9000 --to 10300 secret-nail-porunga

It uses the work folder of run.py (run that once first): its xtask (WORK/target/release/xtask, or `--xtask`) and
its data folder (the cached ROM). The replays are copied to WORK/sweep_replays and rebased for that xtask's core, as
run.py does with its own.

Per replay, three `xtask shots` runs (HD 2x, the app's hooks): the zh-tw table drawn every frame FROM..TO (warm: the
overlay sees every frame), the zh-tw table drawn every EVERY-th frame (cold: `shots` runs the overlay on the frames it
draws only, so it starts afresh at each), and the empty table (the original) at the same frames. At each sampled
frame a pixel counts where the cold picture is the original's and the warm one is not, and it is bright (its channels
summed over `--bright`: the cells of the window's paper and their letters, not the dark screen). A frame with more
than `--min` such pixels is a hit: printed with the box of those pixels, and the first `--png` hits of the replay
saved as WORK/sweep/<replay>/hit_<frame>.png (warm | cold | original, 3x, the box and a margin).

A hit is not always a bug: on the frames between two pages (the engine names none) the overlay keeps the page it
saw last, which a cold overlay has not seen; look at the pictures.

Each replay prints the frames sampled and those compared (drawn by all three runs). Exit 0 when no replay has a
hit, 1 when one has, 2 on a harness error (also when the three runs drew different sampled frames).
"""
import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np
from PIL import Image

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
ZH = REPO / 'translations/zh-tw/zh-tw.tsv'
EMPTY = HERE / 'empty.tsv'
FONT = REPO / 'translations/zh-tw/fonts/LXGWWenKaiTC-Regular.ttf'
PACK = Path('pc039/hd/A8CF4F2A/1')


def parse_args():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    env = os.environ.get
    p.add_argument('names', nargs='*', help='replays (their names without .ssdreplay); default all')
    p.add_argument('--inputs', type=Path, default=env('DBZ_REGRESS_INPUTS'), help='folder of the inputs (run.py)')
    p.add_argument('--pack', type=Path, default=env('DBZ_HD_PACK'), help='the installed HD pack folder')
    p.add_argument('--replays', type=Path, default=env('DBZ_REPLAYS'), help='folder of the secret-*.ssdreplay')
    p.add_argument('--work', type=Path, default=Path(env('DBZ_REGRESS_WORK') or REPO / '.work' / 'regress45'),
                   help="run.py's work folder (default: .work/regress45 in the worktree)")
    p.add_argument('--xtask', type=Path, help='this xtask instead of WORK/target/release/xtask')
    p.add_argument('--from', dest='lo', type=int, default=30, help='first frame (default 30)')
    p.add_argument('--to', dest='hi', type=int, default=16000, help='last frame (default 16000; a replay ends earlier)')
    p.add_argument('--every', type=int, default=30, help='the cold overlay draws every EVERY-th frame (default 30)')
    p.add_argument('--min', type=int, default=50, help='pixels for a hit (default 50)')
    p.add_argument('--bright', type=int, default=600, help='a pixel counts when its channels sum over this')
    p.add_argument('--png', type=int, default=4, help='pictures saved per replay (default 4)')
    p.add_argument('-j', type=int, default=4, help='replays at a time (each is three shots runs; default 4)')
    a = p.parse_args()
    if a.inputs:
        a.pack = a.pack or a.inputs / PACK
        a.replays = a.replays or a.inputs / 'secrets' / 'replays'
    if not a.pack or not a.replays:
        p.error('give --inputs, or --pack and --replays')
    if a.every < 1 or a.lo > a.hi:
        p.error('--every is at least 1 and --from not past --to')
    return a


def file_hash(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()[:16]


def rebased(names, a, xtask, env):
    """The replays copied to WORK/sweep_replays and rebased for `xtask` (again only when the input or xtask changed)."""
    rdir = a.work / 'sweep_replays'
    rdir.mkdir(exist_ok=True)
    fresh = []
    for n in names:
        src, dst, stamp = a.replays / f'{n}.ssdreplay', rdir / f'{n}.ssdreplay', rdir / f'{n}.from'
        key = f'{file_hash(src)} {file_hash(xtask)}'
        if not dst.exists() or not stamp.exists() or stamp.read_text() != key:
            shutil.copyfile(src, dst)
            fresh.append((dst, stamp, key))
    if fresh:
        print(f'xtask rebase-replays ({len(fresh)} replays)', flush=True)
        subprocess.run([str(xtask), 'rebase-replays'] + [str(d) for d, _, _ in fresh], cwd=REPO, env=env,
                       check=True, stdout=subprocess.DEVNULL)
        for _, stamp, key in fresh:
            stamp.write_text(key)
    return {n: rdir / f'{n}.ssdreplay' for n in names}


def pictures(xtask, env, replay, pack, table, frames, keep):
    """The pictures of `frames` (those in `keep`) drawn by one `xtask shots`, read from an anonymous pipe."""
    rfd, wfd = os.pipe()
    cmd = [str(xtask), 'shots', str(replay), '--frames', ','.join(map(str, frames)), '--no-png', '--hd-scale', '2',
           '--pack', str(pack), '--translation', str(table), '--translation-font', str(FONT), '--app-hooks',
           '--raw-out', f'/dev/fd/{wfd}']
    proc = subprocess.Popen(cmd, cwd=REPO, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            pass_fds=(wfd,))
    os.close(wfd)
    out = {}
    with os.fdopen(rfd, 'rb', buffering=1 << 22) as f:
        while True:
            head = f.readline()
            if not head:
                break
            _, n, w, h = head.split()
            n, w, h = int(n), int(w), int(h)
            data = f.read(w * h * 4)
            if n in keep:
                out[n] = np.frombuffer(data, np.uint8).reshape(h, w, 4)[:, :, :3].copy()
    if proc.wait() != 0:
        raise RuntimeError(f'xtask shots exited {proc.returncode}: {replay.name}')
    return out


def sweep(name, replay, a, xtask, env):
    sampled = [f for f in range(a.lo, a.hi + 1) if f % a.every == 0]
    keep = set(sampled)
    got = {}

    def run(key, table, frames):
        got[key] = pictures(xtask, env, replay, a.pack, table, frames, keep)

    threads = [threading.Thread(target=run, args=args) for args in
               (('warm', ZH, list(range(a.lo, a.hi + 1))), ('cold', ZH, sampled), ('orig', EMPTY, sampled))]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    if set(got) != {'warm', 'cold', 'orig'}:
        raise RuntimeError(f'{name}: a shots run failed')
    # (the sampled frames each run drew: a run that stopped early would leave frames out of the comparison and look
    # like no hit)
    drawn = {k: sorted(got[k]) for k in ('warm', 'cold', 'orig')}
    if not drawn['cold'] or drawn['warm'] != drawn['cold'] or drawn['cold'] != drawn['orig']:
        counts = ', '.join(f'{k} {len(v)}' for k, v in drawn.items())
        raise RuntimeError(f'{name}: the shots runs drew different sampled frames ({counts} of {len(sampled)})')
    compared = drawn['cold']
    out = a.work / 'sweep' / name
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    hits = []
    for n in compared:
        warm, cold, orig = (got[k][n] for k in ('warm', 'cold', 'orig'))
        m = (np.all(cold == orig, axis=2) & np.any(warm != orig, axis=2)
             & (cold.astype(np.int32).sum(axis=2) > a.bright))
        count = int(m.sum())
        if count <= a.min:
            continue
        ys, xs = np.nonzero(m)
        box = (int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max()))
        hits.append((n, count, box))
        if len(hits) <= a.png:
            x0, y0, x1, y1 = box
            h, w = warm.shape[:2]
            x0, y0, x1, y1 = max(x0 - 40, 0), max(y0 - 40, 0), min(x1 + 41, w), min(y1 + 41, h)
            row = np.concatenate([p[y0:y1, x0:x1] for p in (warm, cold, orig)], axis=1)
            Image.fromarray(row).resize((row.shape[1] * 3, row.shape[0] * 3), Image.NEAREST).save(
                out / f'hit_{n:06d}.png')
    summary = f'sampled {len(sampled)} compared {len(compared)} hits {len(hits)}'
    lines = [f'{n} px={c} box={b}' for n, c, b in hits] + [summary]
    (out / 'hits.txt').write_text('\n'.join(lines) + '\n')
    return name, hits, summary


def main():
    a = parse_args()
    xtask = a.xtask or a.work / 'target' / 'release' / 'xtask'
    if not xtask.exists() or not (a.work / 'data' / 'cache' / 'rom' / 'canonical.sfc').exists():
        sys.exit(f'no {xtask} or no ROM in {a.work / "data"}: run run.py once first (or give --xtask, --work)')
    names = a.names or sorted(p.stem for p in a.replays.glob('*.ssdreplay'))
    missing = [n for n in names if not (a.replays / f'{n}.ssdreplay').exists()]
    if missing:
        sys.exit(f'no such replay in {a.replays}: {", ".join(missing)}')
    env = dict(os.environ, SSD_DATA_DIR=str(a.work / 'data'))
    replays = rebased(names, a, xtask, env)
    total = 0
    with ThreadPoolExecutor(a.j) as ex:
        for name, hits, summary in ex.map(lambda n: sweep(n, replays[n], a, xtask, env), names):
            total += len(hits)
            print(f'{name}: {summary}' + ''.join(f'\n  {n} px={c} box={b}' for n, c, b in hits), flush=True)
    print(f'TOTAL hits {total} (pictures in {a.work / "sweep"})')
    return 1 if total else 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (RuntimeError, subprocess.CalledProcessError, OSError) as e:
        print(f'ERROR {e}', file=sys.stderr)
        sys.exit(2)
