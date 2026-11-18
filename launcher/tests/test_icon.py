"""Exe icon: build.rs embeds icon/app_icon.ico (Super Saiyan Goku's head, 256..16) via winresource,
and the icon stays out of assets/ so it is neither unpacked nor part of BUNDLE_ID."""
import re
import struct
from pathlib import Path

LAUNCHER = Path(__file__).resolve().parent.parent
SIZES = {256, 128, 64, 48, 32, 16}


def _build_rs():
    return (LAUNCHER / "build.rs").read_text(encoding="utf-8")


def test_build_rs_sets_icon_from_icon_dir():
    src = _build_rs()
    path = re.search(r'const ICON: &str = "([^"]+)";', src).group(1)
    assert not path.startswith("assets/")
    assert (LAUNCHER / path).is_file()
    body = src[src.index("fn version_info"):]
    assert "res.set_icon(" in body and body.index("res.set_icon(") < body.index("res.compile()")


def test_icon_sizes_and_formats():
    data = (LAUNCHER / "icon" / "app_icon.ico").read_bytes()
    reserved, kind, count = struct.unpack_from("<HHH", data, 0)
    assert (reserved, kind) == (0, 1)
    got = {}
    for i in range(count):
        w, h, _, _, planes, bpp, size, off = struct.unpack_from("<BBBBHHII", data, 6 + 16 * i)
        w, h = w or 256, h or 256
        assert w == h and (planes, bpp) == (1, 32)
        img = data[off:off + size]
        assert len(img) == size
        if w == 256:
            assert img[:8] == b"\x89PNG\r\n\x1a\n"
            assert struct.unpack(">II", img[16:24]) == (256, 256)
        else:
            hdr, bw, bh, _, bbpp = struct.unpack_from("<IiiHH", img, 0)
            assert (hdr, bw, bh, bbpp) == (40, w, 2 * w, 32)
        got[w] = img
    assert set(got) == SIZES
    # pixel art: the 16 px image has a transparent corner and an opaque centre
    px16 = got[16][40:40 + 16 * 16 * 4]       # bottom-up BGRA
    assert px16[3] == 0 and px16[(8 * 16 + 7) * 4 + 3] == 255
