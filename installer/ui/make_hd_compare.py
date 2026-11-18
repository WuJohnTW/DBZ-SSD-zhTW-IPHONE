#!/usr/bin/env python3
"""Builds the HD art pack page's comparison pictures (installer/ui/hd/plain_<dpi>.bmp and art_<dpi>.bmp) from two real
screenshots of the same frame.

    make_hd_compare.py <plain.png> <art.png>

The two PNGs are one frame of the game drawn twice by `xtask hd-art shots` (the same frame and game state, the same
wide 16:9 view at 4x: once without the HD art pack, as the game draws it when there is no pack, and once with
upstream v0.3.9's pack):

    SSD_DATA_DIR=<a data folder with cache/rom/canonical.sfc> xtask make-replays
    SSD_DATA_DIR=... xtask hd-art shots --pack <the installed v0.3.9 pack folder> --out <dir> \\
        f8700=.work/replays/story-1.ssdreplay:8700
    -> <dir>/f8700-plain.png, <dir>/f8700-art.png

Frame 8700 of the story-1 reference replay is the first battle: Goku facing the enemy in the sky, before his attack.
Goku stands the same in both shots (the same pose, at x 380..579, y 360..551 of the 1488x956 shots); the pack draws
its own picture for each pose, and for this one it keeps the original's stance. The crop (CROP) is Goku whole with
the sky and clouds around him (the enemy, which the pack draws from a different angle, is left out), the same crop
of both shots. Each picture is that crop, whole, with a label (「原版」 or 「HD」) in its corner in the installer's
title font; nothing is drawn or generated but the label.

One BMP of each per display scaling (96, 120, 144, 192 DPI = 100 %, 125 %, 150 %, 200 %), each resampled (Lanczos)
from the 4x shots at its own size so none is stretched by Windows; at 192 DPI it is the shots' own pixels. Needs
Pillow."""
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
FONT = HERE / "LXGWWenKaiTC-installer.ttf"
OUT = HERE / "hd"
# the crop of the 1488x956 shots (left, top, right, bottom): Goku whole, in the middle; its size at 192 DPI
CROP = (300, 356, 660, 556)
SIZE_192 = (360, 200)
DPIS = (96, 120, 144, 192)
CREAM = (255, 244, 220)
LABEL = (196, 64, 20)
LABEL_PX = 13     # the labels' size in px at 96 DPI


def build(shot: Image.Image, label: str, dpi: int) -> Image.Image:
    w = round(SIZE_192[0] * dpi / 192)
    h = round(SIZE_192[1] * dpi / 192)
    crop = shot.crop(CROP)
    pic = crop if crop.size == (w, h) else crop.resize((w, h), Image.LANCZOS)
    draw = ImageDraw.Draw(pic)
    font = ImageFont.truetype(str(FONT), round(LABEL_PX * dpi / 96))
    pad = round(3 * dpi / 96)
    l, t, r, b = draw.textbbox((0, 0), label, font=font)
    draw.rectangle((pad, pad, pad + (r - l) + 2 * pad, pad + (b - t) + 2 * pad), fill=CREAM)
    draw.text((2 * pad - l, 2 * pad - t), label, font=font, fill=LABEL)
    return pic


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    plain = Image.open(sys.argv[1]).convert("RGB")
    art = Image.open(sys.argv[2]).convert("RGB")
    if plain.size != art.size:
        raise SystemExit(f"兩張截圖大小不同：{plain.size}／{art.size}")
    OUT.mkdir(exist_ok=True)
    for dpi in DPIS:
        for name, shot, label in (("plain", plain, "原版"), ("art", art, "HD")):
            pic = build(shot, label, dpi)
            path = OUT / f"{name}_{dpi}.bmp"
            pic.save(path, "BMP")
            print(f"{path.name}: {pic.size[0]}x{pic.size[1]}, {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()
