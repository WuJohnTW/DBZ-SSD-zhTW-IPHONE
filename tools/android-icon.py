"""Writes the Android launcher icon: Super Saiyan Goku's head over the installer's speed lines, the same two
pictures the Windows icon is cut from (launcher/icon/make_icon.py). Earlier builds used the four-star ball of
assets/icon/sfera-4-stelle-54.png, written as vector drawables.

    python tools/android-icon.py GOKU_PNG BG_PNG [PREVIEW_PNG]

GOKU_PNG is the transparent picture (`mode7_goku_ssj_t0_0_d.png`), BG_PNG the installer's background
(`bg_vertical_v1.png`); neither is in the repository. The script only cuts, scales and saves: nothing is drawn.

Layers of the adaptive icon (108 dp; a launcher shows the middle 72 dp at most, and keeps what matters inside the middle
66 dp), as 432 px pictures (108 dp at xxxhdpi, scaled down by Android for the other densities):
  foreground  Goku cut at CUT, so his head (HEAD, the Windows icon's cut) takes the middle 66 dp; transparent around him
  background  a centred square of the speed lines
No monochrome layer: a themed icon (Android 13+) needs a one-colour shape, and there is none of Goku's head that was
not drawn for it; without the layer Android shows the icon as it is.
"""
import os
import sys

from PIL import Image

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RES = os.path.join(ROOT, 'android', 'app', 'src', 'main', 'res')
SIZE = 432
# the Windows icon's cut of the head: left, top, side
HEAD = (424, 330, 252)
# the head takes 66 of the 108 dp
SIDE = round(HEAD[2] * 108 / 66)
CUT = (HEAD[0] + HEAD[2] // 2 - SIDE // 2, HEAD[1] + HEAD[2] // 2 - SIDE // 2, SIDE)
OLD = ('ic_launcher_foreground.xml', 'ic_launcher_background.xml', 'ic_launcher_monochrome.xml')


def main():
    goku = Image.open(sys.argv[1]).convert('RGBA')
    bg = Image.open(sys.argv[2]).convert('RGB')
    x, y, side = CUT
    foreground = goku.crop((x, y, x + side, y + side)).resize((SIZE, SIZE), Image.LANCZOS)
    # guard: a faint edge of the cut-out (alpha under 20) is cleared, as the installer's art does (the mask only)
    foreground.putalpha(foreground.getchannel('A').point(lambda a: 0 if a < 20 else a))
    bw, bh = bg.size
    bs = min(bw, bh)
    left, top = (bw - bs) // 2, (bh - bs) // 2
    background = bg.crop((left, top, left + bs, top + bs)).resize((SIZE, SIZE), Image.LANCZOS)

    out = os.path.join(RES, 'drawable-xxxhdpi')
    os.makedirs(out, exist_ok=True)
    foreground.save(os.path.join(out, 'ic_launcher_foreground.png'), optimize=True)
    background.save(os.path.join(out, 'ic_launcher_background.png'), optimize=True)
    # the vector layers of the four-star ball would be the same resources
    for name in OLD:
        path = os.path.join(RES, 'drawable', name)
        if os.path.exists(path):
            os.remove(path)
    adaptive = ('<?xml version="1.0" encoding="utf-8"?>\n'
                '<!-- Written by tools/android-icon.py: run it, do not edit this. -->\n'
                '<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">\n'
                '    <background android:drawable="@drawable/ic_launcher_background" />\n'
                '    <foreground android:drawable="@drawable/ic_launcher_foreground" />\n'
                '</adaptive-icon>\n')
    os.makedirs(os.path.join(RES, 'mipmap-anydpi-v26'), exist_ok=True)
    for name in ('ic_launcher', 'ic_launcher_round'):
        with open(os.path.join(RES, 'mipmap-anydpi-v26', name + '.xml'), 'w', newline='\n') as f:
            f.write(adaptive)
    print(f'cut {CUT}; layers written to {RES}')

    if len(sys.argv) > 3:
        # how launchers show it: the middle 72 dp, as a circle, a rounded square and a square, on a grey ground
        full = Image.alpha_composite(background.convert('RGBA'), foreground)
        inner = SIZE * 72 // 108
        edge = (SIZE - inner) // 2
        shown = full.crop((edge, edge, edge + inner, edge + inner))
        from PIL import ImageDraw
        sheet = Image.new('RGBA', (SIZE + 3 * inner + 5 * 16, SIZE + 32), (70, 70, 80, 255))
        sheet.alpha_composite(full, (16, 16))
        for i, radius in enumerate((inner // 2, inner // 5, 0)):
            mask = Image.new('L', (inner, inner), 0)
            ImageDraw.Draw(mask).rounded_rectangle((0, 0, inner - 1, inner - 1), radius=radius, fill=255)
            cell = shown.copy()
            cell.putalpha(mask)
            sheet.alpha_composite(cell, (SIZE + 32 + i * (inner + 16), 16 + (SIZE - inner) // 2))
        sheet.convert('RGB').save(sys.argv[3])


if __name__ == '__main__':
    main()
