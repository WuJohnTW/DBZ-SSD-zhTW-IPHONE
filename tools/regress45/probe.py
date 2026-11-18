#!/usr/bin/env python3
"""probe.py -- where is the game at each frame? Plays a script without drawing (fast, ~400 frames/s) and prints a line
each time one of the watched bytes changes, to find the frames for a timeline and for a run's --from/--to.

Usage: probe.py --bin XTASK --data SSD_DATA_DIR --script S --upto N [--hooks] [--sram F] [--state-in F]
                [--wram F:ADDR=HEX]... [--peek names_or_hex]
(the xtask and the data folder of a run.py work folder: WORK/target/release/xtask, WORK/data)
Default bytes: game_mode ($24: 0 map, 1 town, 2/4 battle, 6 field, 7 story, $87 title/intro, $11 ending),
menu_screen ($0D5F), message_state ($071B), scene_id ($01D8), effect_step ($073C), item_count ($01B2).
"""
import argparse
import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument('--bin', required=True)
p.add_argument('--data', required=True)
p.add_argument('--script', required=True)
p.add_argument('--upto', type=int, required=True)
p.add_argument('--hooks', action='store_true', help='shots --app-hooks (as the game runs)')
p.add_argument('--sram')
p.add_argument('--state-in')
p.add_argument('--wram', action='append', default=[])
p.add_argument('--peek', default='game_mode,menu_screen,message_state,scene_id,effect_step,item_count')
a = p.parse_args()
cmd = [a.bin, 'shots', a.script, '--every', '1', '--upto', str(a.upto), '--no-png', '--peek', a.peek]
for w in a.wram:
    cmd += ['--wram', w]
if a.sram:
    cmd += ['--sram', a.sram]
if a.state_in:
    cmd += ['--state-in', a.state_in]
if a.hooks:
    cmd.append('--app-hooks')
env = dict(os.environ, SSD_DATA_DIR=a.data)
proc = subprocess.Popen(cmd, cwd=REPO, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
last = None
for line in proc.stdout:
    if not line.startswith('frame'):
        print(line.rstrip())
        continue
    f = line.split()
    values = ' '.join(x for x in f[2:] if '=' in x)   # (not the picture's crc)
    if values != last:
        print(f'frame {f[1]:>6}  {values}')
        last = values
sys.exit(proc.wait())
