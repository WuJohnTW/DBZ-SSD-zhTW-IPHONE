#!/usr/bin/env python3
"""Checks that the uninstaller's list of names (un.ProgramItems / un.DataItems in installer/dbz-zhtw.nsi) covers every
file and folder name the launcher (launcher/src) and the game (crates/ssd-app) write beside the exe or into save\\.

    check_data_list.py

Names are taken from the source (every .rs file under those folders, recursively, outside test modules):
- string constants that look like a file or folder name (`const X: &str = "…"`, and the items of `&[&str]` arrays);
- `.join(…)` chains of literals and constants (also constants of other crates, such as `ssd_mod::set::FOLDER`), with
  what they start from: `launcher_root()` is the launcher's own folder (%LOCALAPPDATA%\\dbz-ssd-zhtw); in the game,
  `data_dir()` is save\\, `saves_dir()` save\\saves\\, `states_dir()` save\\states\\.
A constant's bare name is covered when a name on the list has that name. A path from a `.join` chain must match at its
own level: the same path on the list, a path inside a folder on the list (deleted whole), or a folder above names on the
list. Every other name must be in NOT_LISTED with the reason it is left out (a path from a chain: by its whole path).

Known blind spots: names put together in ways this cannot read. Each is matched by hand in BLIND_SPOTS to the list item
that covers it, and checked to still be on the list:
- `.write-test-{pid}` (main.rs:136, `format!`) → the un.ItemPrefix `.write-test-` in save\\;
- `<table>.bak` (an OsString with ".bak" pushed, main.rs ~493 and ~590) → each `.bak` named on the list;
- `hd-pack.zip.oldN` (hd_offer.rs backup_path, `format!`) → the `hd-pack.zip.old2`–`old99` loop of un.DataItems;
- `<folder>.removing` (main.rs:251-261, remove_by_rename): only for %LOCALAPPDATA%\\dbz-ssd-zhtw\\app\\<bundle> (gone with
  the launcher's folder) and for upstream's %LOCALAPPDATA%\\dbz-ssd-recompiled and its saves\\ states\\ mods\\ (not
  deleted): never beside the exe or in save\\, so not on the list.
Exit 1 (and the names) when a name is in neither the list nor NOT_LISTED, or a blind spot's list item is gone.
No Wine needed; run_wine_tests.py runs this first."""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
NSI = REPO / "installer" / "dbz-zhtw.nsi"

# Names the list leaves out on purpose, each with why. Bare names (constants) match by name; paths (from .join chains)
# match by the whole path, or by a key ending in "/" for everything under that folder.
NOT_LISTED = {
    # the launcher's own folder %LOCALAPPDATA%\dbz-ssd-zhtw is on the list as a whole (when this copy owns it)
    "ssd-recompiled.exe": "解開的遊戲本體，在 %LOCALAPPDATA%\\dbz-ssd-zhtw\\app\\<bundle>\\ 裡",
    ".unpacked": "解開完成的標記，在 %LOCALAPPDATA%\\dbz-ssd-zhtw\\app\\<bundle>\\ 裡",
    ".credits-shown": "在 %LOCALAPPDATA%\\dbz-ssd-zhtw 裡（main.rs:1745）",
    ".hd-pack-warned": "在 %LOCALAPPDATA%\\dbz-ssd-zhtw 裡（main.rs:1362）",
    "hd-pack-offer.txt": "在 %LOCALAPPDATA%\\dbz-ssd-zhtw 裡（main.rs:1490）",
    "dbz102.ips": "app.join(IPS_FILE)（main.rs:1169、:1197）：解開的遊戲本體資料夾 %LOCALAPPDATA%\\dbz-ssd-zhtw\\app\\<bundle>\\ 裡的 IPS",
    "update-settings.txt": "在 %LOCALAPPDATA%\\dbz-ssd-zhtw 裡（main.rs:1420）",
    "update-offer.txt": "在 %LOCALAPPDATA%\\dbz-ssd-zhtw 裡（update.rs）",
    "update-checked.txt": "在 %LOCALAPPDATA%\\dbz-ssd-zhtw 裡（update.rs）",
    ".removing": "改名刪除時加的字尾，見檔頭「已知盲點」",
    # bare names of things inside a folder on the list (deleted whole)
    "super-vegeta": "save\\art-override\\ 裡",
    "key-": "save\\art-override\\ 裡圖檔名的開頭",
    "goku": "save\\art-override\\ 裡",
    "ending-goku": "save\\art-override\\ 裡",
    "variants": "save\\art-override\\ 裡",
    "ssj-lightning": "save\\art-override\\ 裡",
    "mode7": "save\\art-override\\ 裡",
    "palette.txt": "save\\art-override\\mode7\\ 裡",
    "replace": "save\\art-override\\mode7\\ 裡",
    "LICENSE-ART.txt": "save\\art-override\\ 裡（main.rs:881）",
    "ssj-transform.wav": "save\\voice\\ 裡",
    "README.txt": "save\\voice\\ 裡",
    "rom": "save\\cache\\rom\\ 裡",
    "canonical.sfc": "save\\cache\\rom\\ 裡",
    "ssd.sav": "save\\saves\\ 裡",
    # not the program's to delete
    "hd-pack": "exe 旁的 hd-pack\\ 資料夾：玩家自己放的美術包來源，不刪",
    "hd-pack/hd-pack.toml": "exe 旁 hd-pack\\ 資料夾裡的檔，不刪",
    "hd-pack.toml": "hd-pack\\ 裡的檔（以常數出現時），不刪",
    "save/screenshots": "save\\screenshots\\：找不到「圖片」資料夾時遊戲放的玩家截圖，不刪",
    "dbz-ssd-recompiled": "%LOCALAPPDATA%\\dbz-ssd-recompiled：上游原版遊戲的資料夾，不刪（存檔已由啟動器搬進 save\\）",
    "language-settled": "Android 版的語言標記（crates/ssd-app/src/language_pick.rs SETTLED_MARK）：只在 Android 的資料夾，Windows 資料夾不會有",
    # constants that only look like names
    "0EF46E0B": "不是檔名：HD 美術包的摘要值（main.rs HD_PACK_DIGEST）",
    "2": "不是檔名：HD 美術包的修訂號（main.rs HD_PACK_REVISION）",
    "github.com": "不是檔名：允許的下載網域（update_manifest.rs）",
    "objects.githubusercontent.com": "不是檔名：允許的下載網域（update_manifest.rs）",
    "release-assets.githubusercontent.com": "不是檔名：允許的下載網域（update_manifest.rs）",
    "ssd-settings": "不是檔名：遊戲設定檔第一行的標頭字（main.rs SETTINGS_MAGIC）",
    "access.language": "不是檔名：遊戲設定檔裡語言那一列的 key（main.rs LANGUAGE_KEY）",
    "zh-tw": "不是檔名：之前的啟動器玩的語言代碼（main.rs EARLIER_LANGUAGE）",
}

FILENAME = re.compile(r"^\.?[A-Za-z0-9_][A-Za-z0-9_.\-]*(/[A-Za-z0-9_.\-]+)*$")


def strip_tests(text: str) -> str:
    """The source before its `#[cfg(test)]` module."""
    i = text.find("#[cfg(test)]")
    return text if i < 0 else text[:i]


def rust_files(root: Path):
    return sorted(p for p in root.rglob("*.rs") if p.is_file())


def constants(text: str) -> dict:
    """{CONST_NAME: value or [values]} of the string constants."""
    out = {}
    for m in re.finditer(r'const ([A-Z_0-9]+): &str = "([^"]*)"', text):
        out[m.group(1)] = m.group(2)
    for m in re.finditer(r'const ([A-Z_0-9]+): (?:&\[&str\]|\[&str; \d+\]) = &?\[([^\]]*)\]', text):
        out[m.group(1)] = re.findall(r'"([^"]*)"', m.group(2))
    return out


def all_constants() -> dict:
    """Constants of the launcher and of every crate, by NAME and by <file stem>::NAME (set::FOLDER, pack::ART_OVERRIDE)."""
    consts = {}
    for root in [REPO / "launcher" / "src"] + sorted((REPO / "crates").glob("*/src")):
        for f in rust_files(root):
            for name, v in constants(strip_tests(f.read_text(encoding="utf-8"))).items():
                consts[f"{f.stem}::{name}"] = v
                consts.setdefault(name, v)
    return consts


def resolve(ident: str, consts: dict):
    parts = ident.split("::")
    for key in ("::".join(parts[-2:]), parts[-1]):
        v = consts.get(key)
        if isinstance(v, str):
            return v
    return None


def join_chains(text: str, consts: dict):
    """(base, "a/b/c") for each `<base>.join(…).join(…)` of literals and known constants."""
    arg = r'\.join\((?:"([^"]+)"|([A-Za-z_][A-Za-z_0-9:]*))\)'
    # Path::new("cache").join("rom")… starts with its own literal
    text = re.sub(r'Path::new\("([^"]+)"\)', lambda m: f'PATHNEW.join("{m.group(1)}")', text)
    for m in re.finditer(r'([A-Za-z_][A-Za-z_0-9:]*(?:\(\))?|\))((?:' + arg + r')+)', text):
        parts = []
        for a in re.finditer(arg, m.group(2)):
            if a.group(1) is not None:
                parts.append(a.group(1))
            else:
                v = resolve(a.group(2), consts)
                if v is None:
                    parts = None
                    break
                parts.append(v)
        if parts:
            yield m.group(1), "/".join(parts)


def written_names(consts: dict):
    """({bare name: where}, {path: where}) of the names written."""
    bare, paths = {}, {}
    for f in rust_files(REPO / "launcher" / "src"):
        text = strip_tests(f.read_text(encoding="utf-8"))
        for name, v in constants(text).items():
            for value in (v if isinstance(v, list) else [v]):
                if FILENAME.match(value):
                    bare.setdefault(value, f"launcher/src/{f.relative_to(REPO / 'launcher' / 'src')} {name}")
        for base, path in join_chains(text, {**consts, **constants(text)}):   # this file's own constants first
            if base.endswith("launcher_root()"):
                path = "dbz-ssd-zhtw/" + path
            elif base.endswith("local_app_data()"):
                pass
            if FILENAME.match(path):
                paths.setdefault(path, f"launcher/src/{f.name} .join（{base}）")
    game_bases = {"data_dir()": "save/", "saves_dir()": "save/saves/", "states_dir()": "save/states/"}
    for f in rust_files(REPO / "crates" / "ssd-app" / "src"):
        text = strip_tests(f.read_text(encoding="utf-8"))
        for base, path in join_chains(text, {**consts, **constants(text)}):
            short = base.split("::")[-1]
            if short in game_bases and FILENAME.match(path):
                paths.setdefault(game_bases[short] + path,
                                 f"crates/ssd-app/src/{f.relative_to(REPO / 'crates' / 'ssd-app' / 'src')} {short}")
    return bare, paths


def listed():
    """(files, folders, body) on the uninstaller's list, as paths relative to the install folder or %LOCALAPPDATA%."""
    text = NSI.read_text(encoding="utf-8-sig")
    defines = dict(re.findall(r'^!define ([A-Z_]+) "([^"]*)"', text, re.M))
    body = ""
    for fn in ("un.ProgramItems", "un.DataItems"):
        m = re.search(rf"^Function {re.escape(fn)}\n(.*?)^FunctionEnd", text, re.S | re.M)
        assert m, fn
        body += m.group(1)

    def norm(p):
        p = re.sub(r"\$\{([A-Z_]+)\}", lambda d: defines.get(d.group(1), d.group(0)), p)
        p = p.replace("$INSTDIR\\", "").replace("$LOCALAPPDATA\\", "").replace("$0\\", "save\\")
        if p == "$0":
            p = "save"
        return p.replace("\\", "/")

    files, folders = set(), set()
    for m in re.finditer(r'Push "([^"]+)"[^\n]*\n\s*Call un\.Item(File|Dir)', body):
        (folders if m.group(2) == "Dir" else files).add(norm(m.group(1)))
    for m in re.finditer(r'Push "([^"]+)"[^\n]*\n\s*Push "([^"]+)"[^\n]*\n\s*Call un\.ItemPrefix', body):
        files.add(norm(m.group(1)) + "/" + m.group(2) + "*")
    return files, folders, body


def path_covered(path: str, files, folders) -> bool:
    """A path at its own level: on the list, inside a listed folder, or a folder above listed names."""
    for c in (path, "save/" + path):
        if c in files or c in folders:
            return True
        if any(c.startswith(d + "/") for d in folders):
            return True
        if any(p.startswith(c + "/") for p in files | folders):
            return True
        if any(f.endswith("*") and c.startswith(f[:-1]) for f in files):
            return True
    return False


def bare_covered(name: str, files, folders) -> bool:
    if path_covered(name, files, folders):
        return True
    return any(f.rsplit("/", 1)[-1] == name for f in files)


def path_not_listed(path: str) -> bool:
    if path in NOT_LISTED:
        return True
    return any(k.endswith("/") and path.startswith(k) for k in NOT_LISTED)


def blind_spots(files, body):
    """[(blind spot, ok, what is expected on the list)]"""
    out = [(".write-test-{pid}（main.rs:136）", "save/.write-test-*" in files, "save\\ 的 un.ItemPrefix .write-test-")]
    for t in ("zh-tw.tsv", "ui-zh-tw.tsv", "fonts/LXGWWenKaiTC-Regular.ttf", "fonts/OFL.txt", "zh-hk.tsv", "ui-zh-hk.tsv"):
        out.append((f"{t}.bak（main.rs SYNCED／RETIRED 換掉時改名）", f"save/{t}.bak" in files, f"save\\{t}.bak"))
    loop = re.search(r'StrCpy \$1 2\n.*?Push "\$INSTDIR\\hd-pack\.zip\.old\$1".*?\$\{If\} \$1 > 99', body, re.S)
    out.append(("hd-pack.zip.oldN（hd_offer.rs backup_path）", bool(loop) and "hd-pack.zip.old" in files,
                "hd-pack.zip.old 與 old2～old99 的迴圈"))
    out.append((".removing（main.rs:251-261）", True, "不在清單：只出現在 %LOCALAPPDATA% 底下，見檔頭"))
    return out


def main() -> int:
    files, folders, body = listed()
    consts = all_constants()
    bare, paths = written_names(consts)
    missing = []
    for name, where in sorted(bare.items()):
        if not bare_covered(name, files, folders) and name not in NOT_LISTED:
            missing.append((name, where))
    for path, where in sorted(paths.items()):
        if not path_covered(path, files, folders) and not path_not_listed(path):
            missing.append((path, where))
    spots = blind_spots(files, body)
    print(f"啟動器與遊戲寫的名字：常數 {len(bare)} 個、路徑 {len(paths)} 個；解除安裝清單：檔 {len(files)}、資料夾 {len(folders)}")
    bad_spots = [(n, w) for n, ok, w in spots if not ok]
    if missing or bad_spots:
        if missing:
            print("不在清單、也不在刻意不列的名單（NOT_LISTED）裡：")
            for name, where in missing:
                print(f"  {name}（{where}）")
        for name, want in bad_spots:
            print(f"已知盲點 {name}：清單上找不到 {want}")
        return 1
    print(f"全部涵蓋（已知盲點 {len(spots)} 項都對到清單）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
