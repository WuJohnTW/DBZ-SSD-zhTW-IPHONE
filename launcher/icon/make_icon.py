"""App icon from the installer art: Super Saiyan Goku's head (cut from the transparent picture) over a cut
of the installer's speed-line background, in a rounded square. Only cuts, scales, composites and masks: nothing is drawn.

The sizes of SMALL (32 and 16) take a tighter cut, the face and the fringe only: at those sizes the whole head is a
yellow blur.

usage: make_icon.py GOKU_PNG BG_PNG OUT_ICO OUT_PREVIEW_PNG X0 Y0 SIDE TIGHT_X0 TIGHT_Y0 TIGHT_SIDE"""
import io
import struct
import sys

from PIL import Image, ImageChops, ImageDraw

goku_path, bg_path, out_ico, out_preview = sys.argv[1:5]
x0, y0, side = (int(v) for v in sys.argv[5:8])
tx0, ty0, tside = (int(v) for v in sys.argv[8:11])
SIZES = [256, 128, 64, 48, 32, 16]
SMALL = {32, 16}
BIG = 1024

goku = Image.open(goku_path).convert("RGBA")
bg = Image.open(bg_path).convert("RGBA")
# the background: a centred square of the installer's speed lines
bw, bh = bg.size
bs = min(bw, bh)
bg = bg.crop(((bw - bs) // 2, (bh - bs) // 2, (bw - bs) // 2 + bs, (bh - bs) // 2 + bs)).resize((BIG, BIG), Image.LANCZOS)
# rounded square (a mask only), drawn 4x and scaled down for a smooth edge
mask = Image.new("L", (BIG * 2, BIG * 2), 0)
ImageDraw.Draw(mask).rounded_rectangle((0, 0, BIG * 2 - 1, BIG * 2 - 1), radius=int(BIG * 2 * 0.18), fill=255)
mask = mask.resize((BIG, BIG), Image.LANCZOS)


def composed(cx: int, cy: int, cside: int) -> Image.Image:
    """The square (cx, cy, cside) of Goku's picture over the background, in the rounded square."""
    head = goku.crop((cx, cy, cx + cside, cy + cside)).resize((BIG, BIG), Image.LANCZOS)
    out = Image.alpha_composite(bg, head)
    out.putalpha(ImageChops.multiply(out.getchannel("A"), mask))
    return out


master = composed(x0, y0, side)
tight = composed(tx0, ty0, tside)


def dib(im: Image.Image) -> bytes:
    w = im.width
    px = im.tobytes("raw", "BGRA")
    rows = [px[y * w * 4:(y + 1) * w * 4] for y in range(w)]
    xor = b"".join(reversed(rows))
    stride = ((w + 31) // 32) * 4
    alpha = im.getchannel("A").tobytes()
    and_rows = []
    for y in range(w):
        bits = bytearray(stride)
        for x in range(w):
            if alpha[y * w + x] == 0:
                bits[x // 8] |= 0x80 >> (x % 8)
        and_rows.append(bytes(bits))
    and_mask = b"".join(reversed(and_rows))
    header = struct.pack("<IiiHHIIiiII", 40, w, 2 * w, 1, 32, 0, len(xor) + len(and_mask), 0, 0, 0, 0)
    return header + xor + and_mask


def sized(s: int) -> Image.Image:
    """The master at s x s; the faint edge of the rounded corner (alpha under 32) is cleared, so a corner pixel of the
    small sizes is fully transparent (the mask only: no colour is changed)."""
    im = (tight if s in SMALL else master).resize((s, s), Image.LANCZOS)
    im.putalpha(im.getchannel("A").point(lambda a: 0 if a < 32 else a))
    return im


images = []
for s in SIZES:
    im = sized(s)
    if s == 256:
        buf = io.BytesIO()
        im.save(buf, "PNG", optimize=True)
        images.append((s, buf.getvalue()))
    else:
        images.append((s, dib(im)))

offset = 6 + 16 * len(images)
out = struct.pack("<HHH", 0, 1, len(images))
for s, data in images:
    out += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
for _, data in images:
    out += data
with open(out_ico, "wb") as f:
    f.write(out)

# preview: every size at 3x (nearest) on a light and a dark ground
pad = 12
cells = [sized(s).resize((s * 3, s * 3), Image.NEAREST) for s in SIZES]
width = sum(c.width for c in cells) + pad * (len(cells) + 1)
height = cells[0].height + 2 * pad
sheet = Image.new("RGBA", (width, height * 2), (240, 240, 240, 255))
sheet.paste((32, 32, 40, 255), (0, height, width, height * 2))
for row in range(2):
    x = pad
    for c in cells:
        sheet.alpha_composite(c, (x, row * height + pad))
        x += c.width + pad
sheet.convert("RGB").save(out_preview)
print("wrote", out_ico, len(out), "bytes;", out_preview)
