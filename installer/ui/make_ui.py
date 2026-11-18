#!/usr/bin/env python3
"""Builds the installer's interface pictures and title font into installer/ui/1x and 2x.

    make_ui.py --art-src DIR

- The interface pieces (sky gradient, cream panel with its double frame and corner studs, the portrait frame, the ▶
  cursor, the button bar) are cut from pieces.html, which is the design's CSS, rendered by headless Chrome at 1x and
  2x on a transparent page. They are saved as 32-bit BMPs with premultiplied alpha (the installer draws them with
  AlphaBlend), whole: the installer cuts the nine slices itself (FRAME_SLICE / PORTRAIT_SLICE pixels at 1x).
- The Goku pictures are the installer art only: 1x = installer/wizard.bmp and header.bmp as they are; 2x = the same
  art's x4 renders (wizard_x4.png, header_x4.png; where the art comes from: README.md, credits) scaled down by half. No
  picture is drawn or generated here.
- The title font: LXGW WenKai TC (translations/zh-tw/fonts, OFL 1.1, no Reserved Font Name) cut down to the characters
  of the installer's strings (dbz-zhtw.nsi and NSIS's Traditional Chinese Modern UI strings); OFL.txt is copied beside.
Needs Chrome, Pillow and fontTools (pyftsubset)."""
import argparse
import os
import re
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
FONT = REPO / "translations" / "zh-tw" / "fonts" / "LXGWWenKaiTC-Regular.ttf"
NSIS_SHARE = Path.home() / "Tools" / "nsis" / "3.13" / "share" / "nsis"

# (name, left, top, width, height) at 1x in pieces.html, the box including what sticks out
PIECES = [
    ("sky", 0, 0, 8, 400),
    ("frame", 34, 14, 132, 112),        # #frame 40,20 120x100; studs 6px out, shadow 3px below
    ("portrait", 214, 14, 112, 112),    # #portrait 220,20 100x100; rings 6px out
    ("cursor", 40, 160, 24, 24),
    ("cursor_s", 70, 160, 16, 16),
    ("barline", 100, 160, 8, 3),
    ("barfill", 120, 160, 8, 8),
]
FRAME_SLICE = 18      # px at 1x from the outer edge: studs (to 8px inside) and the inner outline (to 9px inside)
PORTRAIT_SLICE = 9    # px at 1x: red ring 3 + cream ring 3 + black border 3


def render(scale: int, out: Path):
    with tempfile.TemporaryDirectory() as profile:
        subprocess.run([CHROME, "--headless=new", "--no-sandbox", f"--user-data-dir={profile}", "--disable-gpu",
                        "--hide-scrollbars", "--allow-file-access-from-files", "--default-background-color=00000000",
                        f"--force-device-scale-factor={scale}", "--window-size=400,600", f"--screenshot={out}",
                        (HERE / "pieces.html").as_uri()], check=True, capture_output=True)


def write_bmp32(img: Image.Image, path: Path):
    """32-bit BI_RGB bottom-up BMP with premultiplied alpha (what AlphaBlend with AC_SRC_ALPHA expects)."""
    img = img.convert("RGBA")
    w, h = img.size
    rows = []
    px = img.load()
    for y in range(h - 1, -1, -1):
        row = bytearray()
        for x in range(w):
            r, g, b, a = px[x, y]
            row += bytes((b * a // 255, g * a // 255, r * a // 255, a))
        rows.append(bytes(row))
    data = b"".join(rows)
    header = struct.pack("<2sIHHI", b"BM", 54 + len(data), 0, 0, 54)
    info = struct.pack("<IiiHHIIiiII", 40, w, h, 1, 32, 0, len(data), 2835, 2835, 0, 0)
    path.write_bytes(header + info + data)


def write_bmp24(img: Image.Image, path: Path):
    img.convert("RGB").save(path, format="BMP")


def subset_font(out_dir: Path):
    # the installer's own strings (quoted text outside comments), not the comments
    lines = (REPO / "installer" / "dbz-zhtw.nsi").read_text(encoding="utf-8-sig").splitlines()
    text = "".join("".join(re.findall(r'"([^"]*)"', ln)) for ln in lines if not ln.lstrip().startswith(";"))
    found = 0
    for f in ("Contrib/Language files/TradChinese.nlf", "Contrib/Modern UI 2/Language files/TradChinese.nsh",
              "Contrib/Language files/TradChinese.nsh"):
        p = NSIS_SHARE / f
        if p.exists():
            text += p.read_text(encoding="utf-8", errors="ignore")
            found += 1
    if not found:
        # without them the titles Modern UI writes (歡迎…、正在安裝、已中止安裝…) would fall back to another font
        raise SystemExit(f"找不到 NSIS 的繁中語言檔（{NSIS_SHARE}／Contrib/…/TradChinese.*），字型子集會缺字，停止")
    chars = sorted(set(c for c in text if ord(c) >= 0x20) | set("▶"))
    (out_dir / "subset-chars.txt").write_text("".join(chars), encoding="utf-8")
    pyftsubset = shutil.which("pyftsubset")
    if not pyftsubset:
        raise SystemExit("PATH 上找不到 pyftsubset（pip install fonttools），停止")
    subprocess.run([pyftsubset, str(FONT), f"--text-file={out_dir / 'subset-chars.txt'}",
                    f"--output-file={out_dir / 'LXGWWenKaiTC-installer.ttf'}", "--name-IDs=*", "--name-languages=*"],
                   check=True)
    (out_dir / "subset-chars.txt").unlink()
    shutil.copyfile(FONT.parent / "OFL.txt", out_dir / "OFL.txt")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--art-src", type=Path, required=True,
                    help="the folder of the installer art's x4 renders (wizard_x4.png, header_x4.png); only read")
    args = ap.parse_args()
    for scale in (1, 2):
        out = HERE / f"{scale}x"
        out.mkdir(exist_ok=True)
        shot = out / "_pieces.png"
        render(scale, shot)
        sheet = Image.open(shot).convert("RGBA")
        for name, x, y, w, h in PIECES:
            write_bmp32(sheet.crop((x * scale, y * scale, (x + w) * scale, (y + h) * scale)), out / f"{name}.bmp")
        shot.unlink()
        if scale == 1:
            shutil.copyfile(REPO / "installer" / "wizard.bmp", out / "wizard.bmp")
            shutil.copyfile(REPO / "installer" / "header.bmp", out / "header.bmp")
        else:
            for name, size in (("wizard", (328, 628)), ("header", (300, 114))):
                src = Image.open(args.art_src / f"{name}_x4.png")
                write_bmp24(src.resize(size, Image.LANCZOS), out / f"{name}.bmp")
    subset_font(HERE)
    print("ok")


if __name__ == "__main__":
    main()
