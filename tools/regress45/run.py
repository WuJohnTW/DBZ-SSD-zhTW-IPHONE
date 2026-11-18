#!/usr/bin/env python3
"""run.py -- the zh-tw overlay regression suite: every scene of scenarios.py, PASS / FAIL / XFAIL / XPASS.

    python3 tools/regress45/run.py --inputs DIR            # build, prepare, run all, print the results
    python3 tools/regress45/run.py --inputs DIR -k porunga # only the scenarios whose name has porunga
    python3 tools/regress45/run.py --list                  # the scenarios, nothing run

Inputs (never in the repository): the English ROM, the installed HD pack folder, the three saves and the secret
replays. `--inputs DIR` (or DBZ_REGRESS_INPUTS) takes them from DIR/en.sfc, DIR/pc039/hd/A8CF4F2A/1, DIR/saves and
DIR/secrets/replays; `--rom`, `--pack`, `--saves`, `--replays` (or DBZ_ROM, DBZ_HD_PACK, DBZ_SAVES, DBZ_REPLAYS) name
each one instead.

What it does (README.md has the details):
1. builds `xtask` from a copy of this worktree with overlay_log.patch applied (the overlay's per-frame log the
   detector reads; the product sources are not touched), in WORK/src and WORK/target -- or takes `--xtask BIN`;
2. prepares WORK: the cached ROM (SSD_DATA_DIR), the English text (`xtask text-dump`), the replays rebased for this
   core (`xtask rebase-replays`), the save states and edited tables the scenarios need;
3. runs scan.py once per run of scenarios.py (several scenarios share a run), `-j` at a time;
4. judges every scenario from its run's events and probes and prints one line each and the totals.

Exit 0 when every scenario is PASS or XFAIL (a known failure still failing), 1 when one is FAIL, XPASS (a known
failure now passing: take its `known` mark off) or ERROR.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))
import scenarios as S  # noqa: E402

PATCH = HERE / 'overlay_log.patch'
MARK = 'SSD_OVERLAY_LOG'
TRANSLATE = Path('crates/ssd-hd/src/translate.rs')
ZH = REPO / 'translations/zh-tw/zh-tw.tsv'


def parse_args():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    env = os.environ.get
    p.add_argument('--inputs', type=Path, default=env('DBZ_REGRESS_INPUTS'), help='folder of the inputs')
    p.add_argument('--rom', type=Path, default=env('DBZ_ROM'), help='the English ROM (.sfc)')
    p.add_argument('--pack', type=Path, default=env('DBZ_HD_PACK'), help='the installed HD pack folder')
    p.add_argument('--saves', type=Path, default=env('DBZ_SAVES'), help='folder of grav.sav, nail-porunga.sav, ...')
    p.add_argument('--replays', type=Path, default=env('DBZ_REPLAYS'), help='folder of the secret-*.ssdreplay')
    p.add_argument('--work', type=Path, default=Path(env('DBZ_REGRESS_WORK') or REPO / '.work' / 'regress45'),
                   help='work folder (default: .work/regress45 in the worktree, ignored by git)')
    p.add_argument('--xtask', type=Path, help='use this xtask (built with overlay_log.patch) instead of building one')
    p.add_argument('--legacy-env', action='store_true',
                   help='--xtask is an older build (SHOTS_APP_HOOKS / SHOTS_RAW instead of the options): '
                        'to see the fixed scenarios fail on the builds before their fixes')
    p.add_argument('-k', dest='only', action='append', default=[], help='only scenarios whose name has this')
    p.add_argument('-j', dest='jobs', type=int, default=4, help='runs at a time (each is 2 to 4 shots processes)')
    p.add_argument('--reuse', action='store_true',
                   help='keep a run whose output is there and was made from the same parameters and xtask')
    p.add_argument('--list', action='store_true', help='list the scenarios and stop')
    p.add_argument('--build-only', action='store_true', help='build xtask and prepare the work folder, run nothing')
    p.add_argument('--keep', action='store_true', help='keep the runs\' outputs (default: only summaries and PNGs)')
    return p.parse_args()


def log(msg):
    print(f'[{time.strftime("%H:%M:%S")}] {msg}', flush=True)


# ---------------------------------------------------------------- inputs and the work folder
def resolve_inputs(a):
    base = a.inputs
    got = {
        'rom': a.rom or (base / 'en.sfc' if base else None),
        'pack': a.pack or (base / 'pc039/hd/A8CF4F2A/1' if base else None),
        'saves': a.saves or (base / 'saves' if base else None),
        'replays': a.replays or (base / 'secrets/replays' if base else None),
    }
    missing = [f'{k} ({v})' for k, v in got.items() if not v or not Path(v).exists()]
    if missing:
        sys.exit('missing inputs: ' + ', '.join(missing) + '\n(give --inputs DIR or --rom/--pack/--saves/--replays; '
                 'see README.md)')
    return {k: Path(v).resolve() for k, v in got.items()}


def sh(cmd, **kw):
    r = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if r.returncode != 0:
        raise RuntimeError(f"{' '.join(map(str, cmd))} failed ({r.returncode}):\n{r.stdout[-2000:]}{r.stderr[-3000:]}")
    return r.stdout


def build_xtask(work):
    """xtask from a copy of the worktree (as it is, committed or not) with overlay_log.patch applied."""
    src, target = work / 'src', work / 'target'
    src.mkdir(parents=True, exist_ok=True)
    log(f'copying the worktree to {src}')
    sh(['rsync', '-a', '--delete', '--exclude=/.git', '--exclude=/target', '--exclude=/.work',
        f'--exclude=/{TRANSLATE}', "--filter=:- .gitignore", f'{REPO}/', f'{src}/'])
    text = (REPO / TRANSLATE).read_text()
    if MARK not in text:
        tmp = work / 'translate.rs.patched'
        tmp.write_text(text)
        r = subprocess.run(['patch', '--forward', '--silent', str(tmp), str(PATCH)], capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError(f'{PATCH.name} no longer applies to {TRANSLATE} (the end of Translator::draw changed):'
                               f'\n{r.stdout}{r.stderr}\nsee README.md, "overlay_log.patch"')
        text = tmp.read_text()
    dest = src / TRANSLATE
    if not dest.exists() or dest.read_text() != text:
        dest.write_text(text)
    cargo = shutil.which('cargo') or str(Path.home() / '.cargo/bin/cargo')
    log('cargo build --release -p xtask (the first build takes a few minutes)')
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    sh([cargo, 'build', '--release', '-p', 'xtask'], cwd=src, env=env)
    return target / 'release' / 'xtask'


def file_hash(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()[:16]


def xtask_env(work):
    return dict(os.environ, SSD_DATA_DIR=str(work / 'data'))


def prepare(work, xtask, inputs, runs):
    data = work / 'data' / 'cache' / 'rom'
    data.mkdir(parents=True, exist_ok=True)
    rom = data / 'canonical.sfc'
    if not rom.exists() or file_hash(rom) != file_hash(inputs['rom']):
        shutil.copyfile(inputs['rom'], rom)
    en = work / 'en_text.tsv'
    if not en.exists():
        log('xtask text-dump (the English pages)')
        sh([str(xtask), 'text-dump', str(en)], cwd=REPO, env=xtask_env(work))
    # replays: copied and rebased to this build's core (the inputs stay as they are)
    names = sorted({r['script'][7:] for r in runs.values() if r['script'].startswith('replay:')} |
                   {st['script'][7:] for st in S.STATES.values() if st['script'].startswith('replay:')})
    rdir = work / 'replays'
    rdir.mkdir(exist_ok=True)
    fresh = []
    for n in names:
        src = inputs['replays'] / f'{n}.ssdreplay'
        dst = rdir / f'{n}.ssdreplay'
        stamp = rdir / f'{n}.from'
        key = f'{file_hash(src)} {file_hash(xtask)}'
        if not dst.exists() or not stamp.exists() or stamp.read_text() != key:
            shutil.copyfile(src, dst)
            fresh.append((dst, stamp, key))
    if fresh:
        log(f'xtask rebase-replays ({len(fresh)} replays)')
        sh([str(xtask), 'rebase-replays'] + [str(d) for d, _, _ in fresh], cwd=REPO, env=xtask_env(work))
        for _, stamp, key in fresh:
            stamp.write_text(key)
    # save states
    sdir = work / 'states'
    sdir.mkdir(exist_ok=True)
    for name, st in S.STATES.items():
        if not any(r.get('state') == name for r in runs.values()):
            continue
        out = sdir / f'{name}.ssdstate'
        stamp = sdir / f'{name}.from'
        key = json.dumps(st, sort_keys=True) + ' ' + file_hash(xtask)
        if out.exists() and stamp.exists() and stamp.read_text() == key:
            continue
        log(f'save state {name}')
        cmd = [str(xtask), 'shots', script_path(st['script'], work, inputs), '--upto', str(st['upto']), '--every',
               '100000', '--no-png', '--state-out', str(out)]
        if st.get('hooks'):
            cmd.append('--app-hooks')
        if st.get('sram'):
            cmd += ['--sram', str(inputs['saves'] / f"{st['sram']}.sav")]
        for w in st.get('wram', []):
            cmd += ['--wram', w]
        sh(cmd, cwd=REPO, env=xtask_env(work))
        stamp.write_text(key)
    # edited tables of the controls
    tdir = work / 'tables'
    tdir.mkdir(exist_ok=True)
    zh_lines = ZH.read_text(encoding='utf-8').splitlines(keepends=True)
    for r in runs.values():
        for cname, edit in r.get('controls', {}).items():
            out = []
            drop = set(edit.get('drop', []))
            sets = edit.get('set', {})
            seen = set()
            for line in zh_lines:
                key = line.split('\t', 1)[0]
                if key in drop:
                    seen.add(key)
                    continue
                if key in sets:
                    f = line.rstrip('\n').split('\t')
                    f[1] = sets[key]
                    line = '\t'.join(f) + '\n'
                    seen.add(key)
                out.append(line)
            lost = (drop | set(sets)) - seen
            if lost:
                raise RuntimeError(f'control {cname}: {sorted(lost)} not in zh-tw.tsv')
            (tdir / f'{cname}.tsv').write_text(''.join(out), encoding='utf-8')
    return en


def script_path(script, work, inputs):
    if script.startswith('replay:'):
        return str(work / 'replays' / f'{script[7:]}.ssdreplay')
    if script.startswith('scenes/'):
        return str(HERE / script)
    return script        # a builtin script name (battle, menus, ...)


# ---------------------------------------------------------------- the runs
def run_args(name, r, work, inputs, xtask, en, legacy=False):
    out = work / 'out' / name
    cmd = [sys.executable, '-B', str(HERE / 'scan.py'), '--bin', str(xtask), '--data', str(work / 'data'),
           '--pack', str(inputs['pack']), '--en-text', str(en), '--script', script_path(r['script'], work, inputs),
           '--from', str(r['lo']), '--to', str(r['hi']), '--out', str(out), '--max-png', '40', '--per-event', '4']
    if r.get('hooks', True):
        cmd.append('--hooks')
    if legacy:
        cmd.append('--legacy-env')
    if r.get('sram'):
        cmd += ['--sram', str(inputs['saves'] / f"{r['sram']}.sav")]
    if r.get('state'):
        cmd += ['--state-in', str(work / 'states' / f"{r['state']}.ssdstate")]
    for w in r.get('wram', []):
        cmd += ['--wram', w]
    for cname in r.get('controls', {}):
        cmd += ['--control', f"{cname}={work / 'tables' / f'{cname}.tsv'}"]
    probes = [c['spec'] for sc in S.SCENARIOS if sc['run'] == name for c in sc['checks'] if c['type'] == 'probe']
    if probes:
        cmd += ['--probes', str(out / 'probes.json')]
    return out, cmd, probes


def do_run(name, r, work, inputs, xtask, en, reuse, keep, legacy):
    out, cmd, probes = run_args(name, r, work, inputs, xtask, en, legacy)
    key = hashlib.sha256((json.dumps([cmd[3:], probes], sort_keys=True) + file_hash(xtask) +
                          file_hash(ZH)).encode()).hexdigest()
    done = out / 'done.json'
    if reuse and done.exists():
        d = json.loads(done.read_text())
        if d.get('key') == key and d.get('rc') in (0, 1):
            return name, d['rc'], 0.0, True
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    if probes:
        (out / 'probes.json').write_text(json.dumps(probes, indent=1))
    t = time.time()
    with open(out / 'scan.log', 'w') as f:
        rc = subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT, cwd=REPO).returncode
    if rc in (0, 1):
        read_hits(out)                 # (hits.json: every hit by frame, from the per-frame report)
    if not keep:                       # the per-frame report is big; summaries, events, probes and PNGs stay
        (out / 'report.tsv').unlink(missing_ok=True)
    done.write_text(json.dumps({'key': key, 'rc': rc}))
    return name, rc, time.time() - t, False


# ---------------------------------------------------------------- judging
def read_hits(out):
    """(frame, rule, key) of every rule hit of a run, from report.tsv or, without it, from events.tsv."""
    hits = []
    rep = out / 'report.tsv'
    ev = out / 'events.tsv'
    cache = out / 'hits.json'
    if cache.exists():
        return [tuple(h) for h in json.loads(cache.read_text())]
    if rep.exists():
        for line in rep.read_text().splitlines()[1:]:
            frame, rule, _, note = (line.split('\t') + [''])[:4]
            key = ''
            if rule == 'WRONGPAGE':
                key = note.split(' over ')[0]
            elif rule in ('MASKED', 'UNDRAWN', 'NOTR'):
                key = note
            elif rule == 'INPAPER':
                key = note.split('@')[-1]
            hits.append((int(frame), rule, key))
        hits = sorted(set(hits))
    elif ev.exists():      # (events only give first and last: each frame between counts)
        for line in ev.read_text().splitlines()[1:]:
            rule, key, first, last = line.split('\t')[:4]
            hits += [(f, rule, key) for f in range(int(first), int(last) + 1)]
    cache.write_text(json.dumps(hits))
    return hits


def matches(check, hit):
    frame, rule, key = hit
    if rule not in check['rules']:
        return False
    if check.get('frames') and not any(lo <= frame <= hi for lo, hi in check['frames']):
        return False
    if check.get('key') and not re.fullmatch(check['key'], key):
        return False
    return True


def ranges_of(frames):
    frames = sorted(set(frames))
    out = []
    for f in frames:
        if out and out[-1][1] == f - 1:
            out[-1][1] = f
        else:
            out.append([f, f])
    return ','.join(f'{a}' if a == b else f'{a}-{b}' for a, b in out)


def judge(sc, out, claimed):
    """(ok, detail) of a scenario from its run's output."""
    problems = []
    hits = read_hits(out)
    probes = {}
    pt = out / 'probes.tsv'
    if pt.exists():
        for line in pt.read_text().splitlines()[1:]:
            f = line.split('\t')
            probes[f[0]] = f
    for c in sc['checks']:
        if c['type'] == 'rules':
            bad = [h for h in hits if matches(c, h) and (sc['kind'] == 'known' or h not in claimed)
                   and not any(matches(a, h) for a in c.get('allow', []))]
            if bad:
                by = {}
                for fr, rule, key in bad:
                    by.setdefault(f'{rule} {key}'.strip(), []).append(fr)
                problems.append('; '.join(f'{k} {ranges_of(v)} ({len(v)})' for k, v in by.items()))
        else:
            row = probes.get(c['spec']['id'])
            if row is None:
                problems.append(f"probe {c['spec']['id']}: no result")
            elif int(row[2]) == 0:
                problems.append(f"probe {c['spec']['id']}: no frame checked ({row[3]} skipped)")
            elif int(row[4]):
                problems.append(f"probe {row[0]} ({row[1]}): {row[4]} bad frames {row[8][:80]} worst {row[7]} {row[9]}"
                                .rstrip())
    return not problems, ' | '.join(problems)


def main():
    a = parse_args()
    scen = [sc for sc in S.SCENARIOS if not a.only or any(k in sc['name'] for k in a.only)]
    if a.list:
        for sc in scen:
            print(f"{sc['name']:44} {sc['kind']:6} run={sc['run']:22} {sc['what']}")
        print(f'{len(scen)} scenarios, {len({sc["run"] for sc in scen})} runs')
        return 0
    if not scen:
        sys.exit('no scenario matches')
    inputs = resolve_inputs(a)
    work = a.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    xtask = a.xtask.resolve() if a.xtask else build_xtask(work)
    runs = {name: S.RUNS[name] for name in dict.fromkeys(sc['run'] for sc in scen)}
    en = prepare(work, xtask, inputs, runs)
    if a.build_only:
        log(f'xtask {xtask}; work folder {work}')
        return 0
    log(f'{len(scen)} scenarios on {len(runs)} runs, {a.jobs} at a time')
    rcs = {}
    with ThreadPoolExecutor(a.jobs) as pool:
        futs = [pool.submit(do_run, n, r, work, inputs, xtask, en, a.reuse, a.keep, a.legacy_env) for n, r in runs.items()]
        for f in futs:
            name, rc, secs, reused = f.result()
            rcs[name] = rc
            log(f'run {name}: ' + ('reused' if reused else f'rc={rc} {secs:.0f} s'))
    # the frames the known failures of a run claim: a guard or fixed scenario on that run does not count them
    claimed = {}
    for name in runs:
        hits = read_hits(work / 'out' / name) if rcs[name] in (0, 1) else []
        mine = set()
        for sc in S.SCENARIOS:
            if sc['run'] == name and sc['kind'] == 'known':
                for c in sc['checks']:
                    if c['type'] == 'rules':
                        mine |= {h for h in hits if matches(c, h)}
        claimed[name] = mine
    counts = {'PASS': 0, 'FAIL': 0, 'XFAIL': 0, 'XPASS': 0, 'ERROR': 0}
    lines = []
    for sc in scen:
        out = work / 'out' / sc['run']
        if rcs[sc['run']] not in (0, 1):
            verdict, detail = 'ERROR', f"run {sc['run']} rc={rcs[sc['run']]}: see {out / 'scan.log'}"
        else:
            ok, detail = judge(sc, out, claimed[sc['run']])
            verdict = ('XPASS' if ok else 'XFAIL') if sc['kind'] == 'known' else ('PASS' if ok else 'FAIL')
        counts[verdict] += 1
        lines.append(f"{verdict:5}  {sc['name']:44} [{sc['kind']}] {detail}".rstrip())
    print()
    print('\n'.join(lines))
    print()
    total = ' '.join(f'{k} {v}' for k, v in counts.items())
    print(f'TOTAL {len(scen)}: {total}  ({time.time() - t0:.0f} s; outputs in {work / "out"})')
    if counts['XPASS']:
        print('XPASS: a known failure passes now -- check the picture and take its kind="known" off in scenarios.py')
    (work / 'results.txt').write_text('\n'.join(lines) + f'\nTOTAL {len(scen)}: {total}\n')
    return 1 if counts['FAIL'] or counts['XPASS'] or counts['ERROR'] else 0


if __name__ == '__main__':
    sys.exit(main())
