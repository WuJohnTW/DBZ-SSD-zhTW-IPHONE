"""fetch_assets.sh: every folder of art-override/ the launcher installs is copied into assets/, ending-goku/ too."""
import shutil
import subprocess
from pathlib import Path

_script = Path(__file__).parent.parent / "fetch_assets.sh"


def _src(tmp_path):
    """A <src> as fetch_assets.sh reads it, with four pictures in art-override/ending-goku/."""
    src = tmp_path / "src"
    win = src / "win" / "dbz-ssd-recompiled-windows-v2"
    (win / "licenses").mkdir(parents=True)
    for name in ("ssd-recompiled.exe", "symbols.toml", "LICENSE.txt", "NOTICE.md", "PATCH_NOTES.txt"):
        (win / name).write_bytes(name.encode())
    (win / "licenses" / "OFL.txt").write_bytes(b"ofl")
    (src / "zh-tw.tsv").write_bytes(b"zh")
    (src / "voice").mkdir()
    (src / "voice" / "ssj-transform.wav").write_bytes(b"RIFF")
    (src / "ui-zh-tw.tsv").write_bytes(b"ui")
    (src / "zh-hk.tsv").write_bytes(b"hk")
    (src / "ui-zh-hk.tsv").write_bytes(b"ui hk")
    (src / "fonts").mkdir()
    (src / "fonts" / "LXGWWenKaiTC-Regular.ttf").write_bytes(b"font")
    (src / "fonts" / "OFL.txt").write_bytes(b"ofl")
    art = src / "art-override"
    (art / "ending-goku").mkdir(parents=True)
    (art / "battle-super-vegeta-000.png").write_bytes(b"sv")
    for n in ("023", "024", "025"):
        (art / "ending-goku" / f"battle-goku-{n}.png").write_bytes(n.encode())
    (art / "ending-goku" / "gallery-party-goku-u00-056.png").write_bytes(b"056")
    return src


def _run(tmp_path, src, *flags):
    """fetch_assets.sh copied beside its own assets/ in tmp_path (it writes into the assets/ beside it), run on src."""
    here = tmp_path / "launcher"
    here.mkdir(exist_ok=True)
    shutil.copy(_script, here / "fetch_assets.sh")
    out = subprocess.run(["zsh", str(here / "fetch_assets.sh"), str(src), *flags], capture_output=True, text=True)
    assert out.returncode == 0, out.stderr
    return here / "assets", out.stdout


def test_the_ending_goku_pictures_are_copied_into_their_folder(tmp_path):
    src = _src(tmp_path)
    assets, listing = _run(tmp_path, src)
    got = sorted(p.name for p in (assets / "art-override" / "ending-goku").glob("*.png"))
    assert got == ["battle-goku-023.png", "battle-goku-024.png", "battle-goku-025.png", "gallery-party-goku-u00-056.png"]
    assert not (assets / "art-override" / "battle-goku-025.png").exists(), "not beside the folder: everywhere then"
    assert "./art-override/ending-goku/battle-goku-025.png" in listing, "listed in ASSETS.sha256"
    # a picture taken out of <src> is taken out of assets/ too, as in the other folders
    (src / "art-override" / "ending-goku" / "battle-goku-023.png").unlink()
    assets, _ = _run(tmp_path, src)
    assert not (assets / "art-override" / "ending-goku" / "battle-goku-023.png").exists()
    assert len(list((assets / "art-override" / "ending-goku").glob("*.png"))) == 3


def test_the_lightning_pictures_are_copied_into_their_folder_and_a_src_without_them_is_no_error(tmp_path):
    src = _src(tmp_path)
    assets, _ = _run(tmp_path, src)
    assert (assets / "art-override" / "ssj-lightning").is_dir(), "the folder, empty without pictures"
    assert not list((assets / "art-override" / "ssj-lightning").iterdir())
    (src / "art-override" / "ssj-lightning").mkdir()
    for n in (1, 2, 3):
        (src / "art-override" / "ssj-lightning" / f"frame-{n}.png").write_bytes(str(n).encode())
    assets, listing = _run(tmp_path, src)
    got = sorted(p.name for p in (assets / "art-override" / "ssj-lightning").glob("*.png"))
    assert got == ["frame-1.png", "frame-2.png", "frame-3.png"]
    assert not (assets / "art-override" / "frame-1.png").exists(), "not beside the folder"
    assert "./art-override/ssj-lightning/frame-2.png" in listing, "listed in ASSETS.sha256"


def test_the_poses_of_super_vegetas_keyed_pictures_are_copied_beside_them_and_never_their_pixels(tmp_path):
    # The game draws super-vegeta/key-<key>.png in a release only with its pose key-<key>.pose beside it: both are copied
    # and listed. The pixels key-<key>.txt (the original sprite's) stay in <src>: never copied, never listed, and an older
    # fetch's are removed. Another file, a name that is not 16 hexadecimal digits, and the same name in another folder are not
    src = _src(tmp_path)
    sv = src / "art-override" / "super-vegeta"
    sv.mkdir()
    (sv / "battle-vegeta-803.png").write_bytes(b"803")
    (sv / "key-9fb61f64d7d15be9.png").write_bytes(b"art")
    (sv / "key-9fb61f64d7d15be9.pose").write_bytes(b"pose")
    (sv / "key-9fb61f64d7d15be9.txt").write_bytes(b"0110\n")
    (sv / "key-14B06CA30F359B50.png").write_bytes(b"upper art")
    (sv / "key-14B06CA30F359B50.pose").write_bytes(b"upper")
    (sv / "key-9fb61f64d7d15be.pose").write_bytes(b"15 digits")
    (sv / "notes.txt").write_bytes(b"notes")
    (src / "art-override" / "ending-goku" / "key-9fb61f64d7d15be9.pose").write_bytes(b"other folder")
    # pixels an older fetch copied
    old = tmp_path / "launcher" / "assets" / "art-override" / "super-vegeta"
    old.mkdir(parents=True)
    (old / "key-9b36beaeedd4fd2b.txt").write_bytes(b"old pixels")
    assets, listing = _run(tmp_path, src)
    got = sorted(p.name for p in (assets / "art-override" / "super-vegeta").iterdir())
    assert got == [
        "battle-vegeta-803.png",
        "key-14B06CA30F359B50.png",
        "key-14B06CA30F359B50.pose",
        "key-9fb61f64d7d15be9.png",
        "key-9fb61f64d7d15be9.pose",
    ]
    assert (assets / "art-override" / "super-vegeta" / "key-9fb61f64d7d15be9.pose").read_bytes() == b"pose"
    assert not list(assets.rglob("key-*.txt")), "never the pixels"
    assert not list((assets / "art-override" / "ending-goku").glob("*.pose"))
    assert "./art-override/super-vegeta/key-9fb61f64d7d15be9.pose" in listing, "listed in ASSETS.sha256"
    assert ".txt" not in "".join(line for line in listing.splitlines() if "key-" in line), "no pixels listed"
    # a picture and its pose taken out of <src> are taken out of assets/ too
    (sv / "key-9fb61f64d7d15be9.png").unlink()
    (sv / "key-9fb61f64d7d15be9.pose").unlink()
    assets, _ = _run(tmp_path, src)
    assert not (assets / "art-override" / "super-vegeta" / "key-9fb61f64d7d15be9.png").exists()
    assert not (assets / "art-override" / "super-vegeta" / "key-9fb61f64d7d15be9.pose").exists()


def _refused(tmp_path, src):
    """fetch_assets.sh run as `_run` does, expected to stop: its error output, and whether it left assets/ as it was."""
    here = tmp_path / "launcher"
    here.mkdir(exist_ok=True)
    shutil.copy(_script, here / "fetch_assets.sh")
    out = subprocess.run(["zsh", str(here / "fetch_assets.sh"), str(src)], capture_output=True, text=True)
    assert out.returncode != 0, out.stdout
    return out.stderr, not (here / "assets").exists()


def test_a_keyed_picture_of_super_vegeta_without_its_pose_stops_the_fetch(tmp_path):
    # A release's game leaves key-<key>.png out without key-<key>.pose: the fetch stops, naming it, before anything is
    # copied, even with the pixels key-<key>.txt there (they are never copied: `xtask keyed-poses` writes the pose)
    src = _src(tmp_path)
    sv = src / "art-override" / "super-vegeta"
    sv.mkdir()
    (sv / "key-9fb61f64d7d15be9.png").write_bytes(b"art")
    (sv / "key-9fb61f64d7d15be9.txt").write_bytes(b"0110\n")
    (sv / "key-9b36beaeedd4fd2b.png").write_bytes(b"art")
    (sv / "key-9b36beaeedd4fd2b.pose").write_bytes(b"pose")
    err, untouched = _refused(tmp_path, src)
    assert "key-9fb61f64d7d15be9.png without key-9fb61f64d7d15be9.pose" in err, err
    assert "key-9b36beaeedd4fd2b" not in err, "a pair is no error"
    assert untouched, "nothing copied"


def test_a_keyed_pose_of_super_vegeta_without_its_picture_stops_the_fetch(tmp_path):
    # key-<key>.pose without key-<key>.png draws nothing: the fetch stops, naming it, before anything is copied
    src = _src(tmp_path)
    sv = src / "art-override" / "super-vegeta"
    sv.mkdir()
    (sv / "key-14B06CA30F359B50.pose").write_bytes(b"pose")
    err, untouched = _refused(tmp_path, src)
    assert "key-14B06CA30F359B50.pose without key-14B06CA30F359B50.png" in err, err
    assert untouched, "nothing copied"


def test_the_voice_is_required_and_copied(tmp_path):
    src = _src(tmp_path)
    (src / "voice" / "ssj-transform.wav").unlink()
    here = tmp_path / "launcher"
    here.mkdir()
    shutil.copy(_script, here / "fetch_assets.sh")
    out = subprocess.run(["zsh", str(here / "fetch_assets.sh"), str(src)], capture_output=True, text=True)
    assert out.returncode != 0 and "ssj-transform.wav" in out.stderr, "no voice, no assets"
    (src / "voice" / "ssj-transform.wav").write_bytes(b"RIFF....WAVE")
    assets, listing = _run(tmp_path, src)
    assert (assets / "voice" / "ssj-transform.wav").read_bytes() == b"RIFF....WAVE"
    assert "./voice/ssj-transform.wav" in listing, "listed in ASSETS.sha256"


def test_both_languages_tables_are_copied(tmp_path):
    src = _src(tmp_path)
    assets, listing = _run(tmp_path, src)
    for name, text in (("zh-tw.tsv", b"zh"), ("ui-zh-tw.tsv", b"ui"), ("zh-hk.tsv", b"hk"), ("ui-zh-hk.tsv", b"ui hk")):
        assert (assets / name).read_bytes() == text, name
        assert f"./{name}" in listing, f"{name} listed in ASSETS.sha256"
