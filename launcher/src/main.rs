//! One-file launcher of `ssd-recompiled` (Dragon Ball Z: Super Saiya Densetsu, recompiled) with the
//! Traditional Chinese translation built in. Double-click to play:
//!
//! 1. The embedded files (the game, the tables of Taiwan's `zh-tw.tsv`, `ui-zh-tw.tsv` and Hong Kong's `zh-hk.tsv`,
//!    `ui-zh-hk.tsv`, `fonts/`, the licences, and with
//!    the `bundle-ips` feature `dbz102.ips`) are unpacked once to
//!    `%LOCALAPPDATA%\dbz-ssd-zhtw\app\<BUNDLE_ID>\`. The game finds the translation and the font
//!    next to its own executable (upstream `ssd-app/src/lang.rs`, `places()`).
//!    The translation's files are also written into the game's data folder (same bytes: skipped;
//!    a table that differs in its `#font` line only keeps the player's `#font`; else the old file
//!    is kept as `.bak`), which upstream reads first. What each file got is in `last-run.log`.
//!    Before that, once per data folder, the player of an earlier launcher (which ran the game in `zh-tw` every time) whose
//!    PC file holds no language or one other than `zh-tw` (`zh-hk` too) is given `zh-tw` there ([`settle_language`]).
//!    The game's data folder (saves, save states, the cached ROM) is `save\` beside this launcher's
//!    `.exe`, passed to the game as `SSD_DATA_DIR` ([`resolve_data_dir`]): a `SSD_DATA_DIR` the
//!    player set is kept, and a `save\` that cannot be written falls back to upstream's default
//!    `%LOCALAPPDATA%\dbz-ssd-recompiled`. The first time, the saves of that default folder are
//!    copied into `save\` ([`migrate`]).
//!    Then the old copies are removed ([`clean_up`]): the unpacked folders of other bundles, and,
//!    once every file of its `saves\` and `states\` is in `save\` with the same SHA-256, the old
//!    data folder. `--selftest` only lists what would be removed.
//!    The bundled `art-override/` pictures (Super Vegeta without his shoulder pads, drawn by the game in place of the HD
//!    pack's: patch 0047) are written once into the data folder's `art-override\` ([`install_art_override`]): a picture
//!    the player removed or changed is left so. `--selftest` only says what it would do.
//! 2. Without a cached ROM (`<data dir>\cache\rom\canonical.sfc`), the player picks the ROM and the
//!    game is run once headless to cache it.
//! 3. The game runs: `--mode enhanced --hd-scale 4`, plus `--install-hd-pack` when an
//!    HD pack sits next to this launcher, and `--debug` (the game's debug tools) for a private copy
//!    ([`debug_args`]). No `--lang`: the game's first page asks a new player Taiwan or Hong Kong and keeps the choice.
//!
//! Before step 3: a newer release found by the background check is offered (`update.rs`, only in a build given an
//! update source), and without an HD pack the player is asked whether to download upstream's (`hd_offer.rs`).
//!
//! A private copy may carry the ROM and the IPS appended to the `.exe` (`overlay.rs`, in the format
//! that file describes): then step 2 uses them and asks nothing. An overlay IPS wins over the
//! embedded one.
//!
//! `--selftest` unpacks, checks every file's SHA-256 and prints the paths, without a dialog or the game.

#![windows_subsystem = "windows"]

use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::OnceLock;

/// One embedded file, unpacked to `path` under the app folder.
pub struct Asset {
    pub path: &'static str,
    pub bytes: &'static [u8],
    pub sha256: &'static str,
    pub len: usize,
}

include!(concat!(env!("OUT_DIR"), "/assets.rs"));

mod hd_offer;
mod overlay;
mod preset;
mod progress;
mod update;
mod update_manifest;

const TITLE: &str = "七龍珠Z 超賽亞傳說 繁中版";
const GAME_EXE: &str = "ssd-recompiled.exe";
const IPS_FILE: &str = "dbz102.ips";
/// Written last into an unpacked folder: its presence means every file was written.
const DONE_MARKER: &str = ".unpacked";
/// In the launcher's folder once the credits were shown: they are shown on the first run only.
const CREDITS_MARKER: &str = ".credits-shown";
const CREDITS: &str = "DBZ 超賽亞傳說 繁中版　繁體中文化 By Ming Liu\n\n本程式基於 JackRushante/DBZ-Super-Saiya-Densetsu-Rust（GPL-3）";
/// Windows `CREATE_NO_WINDOW`: the game is a console program; no black window behind it.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// The launcher's own folder: `%LOCALAPPDATA%\dbz-ssd-zhtw`.
fn launcher_root() -> PathBuf {
    local_app_data().join("dbz-ssd-zhtw")
}

/// The folder beside this launcher's `.exe` that holds the game's data.
const SAVE_DIR: &str = "save";

/// Upstream's default data folder (`paths::data_dir` without `SSD_DATA_DIR`): where the saves of
/// earlier versions are.
fn upstream_data_dir() -> PathBuf {
    local_app_data().join("dbz-ssd-recompiled")
}

/// The game's data folder of this run, given to the game as `SSD_DATA_DIR`.
struct DataDir {
    path: PathBuf,
    /// `save\` beside the `.exe`: the only case the old saves are migrated into.
    beside_exe: bool,
    /// `save\` beside the `.exe` could not be used (its folder unknown, or not writable): upstream's folder.
    fallback: bool,
    /// Why this folder, for `last-run.log` and the selftest.
    note: String,
}

/// The data folder: `SSD_DATA_DIR` when the player set it (kept as is, as upstream reads it), else
/// `<exe_dir>\save` (created), else — the `.exe`'s folder unknown, or `save\` not creatable or
/// writable — `fallback`.
fn resolve_data_dir(env: Option<OsString>, exe_dir: Option<&Path>, fallback: PathBuf) -> DataDir {
    if let Some(dir) = env {
        let path = PathBuf::from(dir);
        let note = format!("data_dir {} (SSD_DATA_DIR set by the player, kept)", path.display());
        return DataDir { path, beside_exe: false, fallback: false, note };
    }
    let Some(exe_dir) = exe_dir else {
        let note = format!("data_dir {} (fallback: the launcher's folder is unknown)", fallback.display());
        return DataDir { path: fallback, beside_exe: false, fallback: true, note };
    };
    let target = exe_dir.join(SAVE_DIR);
    match ensure_writable(&target) {
        Ok(()) => {
            let note = format!("data_dir {} (beside the launcher)", target.display());
            DataDir { path: target, beside_exe: true, fallback: false, note }
        }
        Err(e) => {
            let note = format!(
                "data_dir {} (fallback: {} cannot be written: {e})",
                fallback.display(),
                target.display()
            );
            DataDir { path: fallback, beside_exe: false, fallback: true, note }
        }
    }
}

/// Creates `dir` if needed and checks a file can be written in it.
fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    std::fs::write(&probe, b"ok")?;
    std::fs::remove_file(&probe)
}

static DATA_DIR: OnceLock<DataDir> = OnceLock::new();

/// The data folder of this run, resolved once.
fn data_dir() -> &'static DataDir {
    DATA_DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        resolve_data_dir(
            std::env::var_os("SSD_DATA_DIR"),
            exe.as_deref().and_then(Path::parent),
            upstream_data_dir(),
        )
    })
}

/// The game's data folder: the one the game is told by `SSD_DATA_DIR` (see [`game_command`]).
fn game_data_dir() -> PathBuf {
    data_dir().path.clone()
}

/// First run with `save\` beside the `.exe`: when `new` has no `saves\ssd.sav` and `old` has a
/// `saves\` folder, copies `old`'s `saves\` (the battery save and every sidecar, `ssd.mods` too),
/// `states\` (each `slotN.ssdstate` with its `slotN.hd`), `mods\` (the installed data packs, 0.3.x),
/// `cache\mods\` (the packs unpacked) and `cache\rom\canonical.sfc` into `new`. Only copies: `old` is left as it is, and a file already in
/// `new` is not overwritten. `saves\ssd.sav` is copied last (through a `.part` file), so its presence
/// means the copy is complete. Returns one line for the log.
fn migrate(old: &Path, new: &Path) -> Result<String, String> {
    let sav = Path::new("saves").join("ssd.sav");
    if new.join(&sav).exists() {
        return Ok(format!("migration not needed: {} exists", new.join(&sav).display()));
    }
    if !old.join("saves").is_dir() {
        return Ok(format!("migration none: no saves in {}", old.display()));
    }
    let err = |e: std::io::Error| format!("無法把舊存檔從 {} 複製到 {}：{e}（舊存檔沒有動）", old.display(), new.display());
    let mut copied = 0usize;
    let mut kept = 0usize;
    copy_tree(&old.join("states"), &new.join("states"), None, &mut copied, &mut kept).map_err(err)?;
    copy_tree(&old.join("mods"), &new.join("mods"), None, &mut copied, &mut kept).map_err(err)?;
    let unpacked = Path::new("cache").join("mods");
    copy_tree(&old.join(&unpacked), &new.join(&unpacked), None, &mut copied, &mut kept).map_err(err)?;
    let rom = Path::new("cache").join("rom").join("canonical.sfc");
    copy_file(&old.join(&rom), &new.join(&rom), &mut copied, &mut kept).map_err(err)?;
    copy_tree(&old.join("saves"), &new.join("saves"), Some("ssd.sav"), &mut copied, &mut kept).map_err(err)?;
    let (from, to) = (old.join(&sav), new.join(&sav));
    if from.is_file() {
        let mut part = to.clone().into_os_string();
        part.push(".part");
        std::fs::create_dir_all(new.join("saves")).map_err(err)?;
        std::fs::copy(&from, &part).and_then(|_| std::fs::rename(&part, &to)).map_err(err)?;
        copied += 1;
    }
    Ok(format!(
        "migrated from {} to {}: {copied} files copied, {kept} already there kept",
        old.display(),
        new.display()
    ))
}

/// Copies `from` to `to` when `from` is a file and `to` does not exist.
fn copy_file(from: &Path, to: &Path, copied: &mut usize, kept: &mut usize) -> std::io::Result<()> {
    if !from.is_file() {
        return Ok(());
    }
    if to.exists() {
        *kept += 1;
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)?;
    *copied += 1;
    Ok(())
}

/// Copies every file under `from` into `to` with [`copy_file`]; `skip` names a file of `from`'s top
/// level left out. Nothing when `from` is not a folder.
fn copy_tree(from: &Path, to: &Path, skip: Option<&str>, copied: &mut usize, kept: &mut usize) -> std::io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.is_some_and(|s| name == s) {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if src.is_dir() {
            copy_tree(&src, &dst, None, copied, kept)?;
        } else {
            copy_file(&src, &dst, copied, kept)?;
        }
    }
    Ok(())
}

/// The data folder's line and, when it is `save\` beside the `.exe`, the migration's from `old`
/// (upstream's default folder).
fn prepare_data_dir(data: &DataDir, old: &Path) -> Result<Vec<String>, String> {
    let migration = if data.beside_exe {
        migrate(old, &data.path)?
    } else {
        "migration skipped: the data folder is not save\\ beside the launcher".to_owned()
    };
    Ok(vec![data.note.clone(), migration])
}

/// Suffix a folder is renamed to before it is removed: a folder half removed is never taken for a
/// live one, and is removed on the next run.
const REMOVING: &str = ".removing";

/// Removes `dir`: renamed to `<dir>.removing` first (fails, leaving `dir` whole, while a file in it
/// is in use), then removed. A `<…>.removing` folder is removed directly.
fn remove_by_rename(dir: &Path) -> std::io::Result<()> {
    if dir.as_os_str().to_string_lossy().ends_with(REMOVING) {
        return std::fs::remove_dir_all(dir);
    }
    let mut gone = dir.as_os_str().to_owned();
    gone.push(REMOVING);
    std::fs::rename(dir, &gone)?;
    std::fs::remove_dir_all(&gone)
}

/// Removes, with `remove`, every folder in `apps` but the bundle `current` (and a `<current>.tmp-…`
/// folder another copy of the launcher may be unpacking into). A folder that cannot be removed is
/// skipped and tried again next run. `apply` false: only lists them. One line per folder.
fn clean_bundles(apps: &Path, current: &str, apply: bool, remove: &dyn Fn(&Path) -> std::io::Result<()>) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(apps) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            name != current && !name.starts_with(&format!("{current}.tmp-"))
        })
        .collect();
    dirs.sort();
    dirs.iter()
        .map(|dir| {
            if !apply {
                return format!("cleanup would remove old bundle {}", dir.display());
            }
            match remove(dir) {
                Ok(()) => format!("cleanup removed old bundle {}", dir.display()),
                Err(e) => format!("cleanup skipped old bundle {}: {e} (tried again next run)", dir.display()),
            }
        })
        .collect()
}

/// Every file under `dir`, as paths relative to `base`.
fn files_under(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            files_under(base, &path, out)?;
        } else {
            out.push(path.strip_prefix(base).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}

fn sha256_of(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| hex(&Sha256::digest(&b)))
}

/// The sub-folders of the old data folder the launcher brings into `save\` ([`migrate`]): the only ones its cleanup
/// removes. Everything else there (`cache\`, the ROM's copy, a translation, what upstream's own build keeps in the same
/// folder) stays.
const MOVED: [&str; 3] = ["saves", "states", "mods"];

/// Removes the old data folder's `saves\`, `states\` and `mods\` (with `remove`) when `new` (`save\` beside the
/// `.exe`) has `saves\ssd.sav` and every file under them (the installed data packs in `mods\`), each with the same
/// SHA-256; else removes nothing and says which file differs. The rest of `old` stays (upstream's build uses the same
/// folder). A `<old>.removing` (an earlier launcher removed the whole folder) or `<old>\<sub>.removing` left by an
/// earlier run is removed. `apply` false: only says what would be done.
fn clean_old_data_dir(old: &Path, new: &Path, apply: bool, remove: &dyn Fn(&Path) -> std::io::Result<()>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut leftovers = Vec::new();
    let mut whole = old.as_os_str().to_owned();
    whole.push(REMOVING);
    leftovers.push(PathBuf::from(whole));
    leftovers.extend(MOVED.iter().map(|sub| old.join(format!("{sub}{REMOVING}"))));
    for leftover in leftovers.iter().filter(|dir| dir.is_dir()) {
        lines.push(if !apply {
            format!("cleanup would remove leftover {}", leftover.display())
        } else {
            match remove(leftover) {
                Ok(()) => format!("cleanup removed leftover {}", leftover.display()),
                Err(e) => format!("cleanup skipped leftover {}: {e} (tried again next run)", leftover.display()),
            }
        });
    }
    if !old.exists() {
        lines.push(format!("cleanup old data folder none: {} does not exist", old.display()));
        return lines;
    }
    let keep = |why: String| format!("cleanup kept old data folder {}: {why}", old.display());
    let sav = new.join("saves").join("ssd.sav");
    if !sav.is_file() {
        lines.push(keep(format!("{} does not exist", sav.display())));
        return lines;
    }
    let subs: Vec<PathBuf> = MOVED.iter().map(|sub| old.join(sub)).filter(|dir| dir.is_dir()).collect();
    if subs.is_empty() {
        lines.push(format!("cleanup old data folder none: {} has no saves, states or mods", old.display()));
        return lines;
    }
    let mut files = Vec::new();
    for sub in &subs {
        if let Err(e) = files_under(old, sub, &mut files) {
            lines.push(keep(format!("cannot list {}: {e}", sub.display())));
            return lines;
        }
    }
    files.sort();
    for rel in &files {
        let theirs = sha256_of(&old.join(rel));
        let ours = sha256_of(&new.join(rel));
        if ours.is_none() {
            lines.push(keep(format!("{} is not in {}", rel.display(), new.display())));
            return lines;
        }
        if theirs != ours {
            lines.push(keep(format!("{} differs from {}", old.join(rel).display(), new.join(rel).display())));
            return lines;
        }
    }
    for sub in &subs {
        lines.push(if !apply {
            format!("cleanup would remove old data folder {} ({} save files all in {}; the rest stays)", sub.display(), files.len(), new.display())
        } else {
            match remove(sub) {
                Ok(()) => format!("cleanup removed old data folder {} ({} save files all in {}; the rest stays)", sub.display(), files.len(), new.display()),
                Err(e) => format!("cleanup skipped old data folder {}: {e} (tried again next run)", sub.display()),
            }
        });
    }
    lines
}

/// The cleanup after the data folder is ready (see [`prepare_data_dir`]): the other bundles in
/// `apps`, and the old data folder `old` only when the data folder is `save\` beside the `.exe`
/// (a `SSD_DATA_DIR` the player set may be `old` itself; a fallback is `old`).
fn clean_up(data: &DataDir, old: &Path, apps: &Path, current: &str, apply: bool, remove: &dyn Fn(&Path) -> std::io::Result<()>) -> Vec<String> {
    let mut lines = clean_bundles(apps, current, apply, remove);
    if data.beside_exe {
        lines.extend(clean_old_data_dir(old, &data.path, apply, remove));
    } else {
        lines.push(format!("cleanup old data folder left: the data folder {} is not save\\ beside the launcher", data.path.display()));
    }
    lines
}

/// What the player is told once the ROM is set up: where the game keeps its data, its battery save and
/// its copy of the ROM, and that the folder is not to be shared with the ROM's copy in it; when `save\` beside the
/// `.exe` could not be used, that the data went to upstream's folder instead.
fn folders_text(data: &DataDir) -> String {
    let dir = &data.path;
    let mut text = format!(
        "設定完成。\n\n資料夾：{}\n存檔：{}\nROM 複本：{}\n\n分享或複製這個資料夾給別人之前，請先刪掉 cache\\rom（裡面是 ROM 的複本）。",
        dir.display(),
        dir.join("saves").join("ssd.sav").display(),
        dir.join("cache").join("rom").join("canonical.sfc").display(),
    );
    if data.fallback {
        text.push_str(&format!(
            "\n\n注意：exe 旁邊的 {SAVE_DIR} 資料夾無法寫入，存檔改放在上面這個資料夾。"
        ));
    }
    text
}

/// The CRC-32 (ISO-HDLC, as upstream `ssd-rom` identifies ROMs) of `bytes`.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// The ROMs upstream `ssd-rom` (`identify.rs`) accepts, by the CRC-32 of the image without a copier header.
const ROM_CANONICAL: u32 = 0xA8CF_4F2A;
const ROM_KLEPTO_UNFIXED: u32 = 0x3BB0_2A05;
const ROM_JP_REV1: u32 = 0xADE7_B968;
/// The size of the game's image: 1 MiB; a copier header adds 512 bytes.
const ROM_SIZE: usize = 0x10_0000;

/// What the launcher can tell the player about the ROM `bytes` they chose, before or after the game
/// refused it: its header, which ROM it is, and the IPS a Japanese ROM needs.
fn rom_diagnosis(bytes: &[u8], has_ips: bool) -> String {
    let mut lines = Vec::new();
    let image = if bytes.len() == ROM_SIZE + 512 {
        lines.push(format!("這個 ROM 是 {} bytes，前面有 512 bytes 的檔頭；程式會自動去掉。", bytes.len()));
        &bytes[512..]
    } else {
        bytes
    };
    if image.len() != ROM_SIZE {
        lines.push(format!(
            "大小是 {} bytes，不是這個遊戲的 ROM（應為 {} bytes，或含檔頭 {} bytes）。",
            bytes.len(),
            ROM_SIZE,
            ROM_SIZE + 512
        ));
        return lines.join("\n");
    }
    match crc32(image) {
        ROM_CANONICAL | ROM_KLEPTO_UNFIXED => lines.push("這是 Klepto v1.02 英文版 ROM，不需要 IPS。".to_owned()),
        ROM_JP_REV1 if has_ips => lines.push("這是日版 Rev 1 ROM，會套用 dbz102.ips。".to_owned()),
        ROM_JP_REV1 => lines.push(
            "這是日版 Rev 1 ROM，需要 dbz102.ips（Klepto 英文化 v1.02 的修補檔）才能用：請準備好 dbz102.ips 再執行一次，選 ROM 後會詢問 IPS。"
                .to_owned(),
        ),
        other => lines.push(format!(
            "CRC32 是 {other:08X}，不是可用的版本（可能是日版 Rev 0 或其他版本）。可用的是日版 Rev 1（CRC32 ADE7B968，要搭配 dbz102.ips）或 Klepto v1.02 英文版。"
        )),
    }
    lines.join("\n")
}

fn cached_rom() -> PathBuf {
    game_data_dir().join("cache").join("rom").join("canonical.sfc")
}

/// The translation's files, also written into the game's data folder: upstream `lang.rs` reads the
/// data folder before the exe's, so a copy left there by hand would otherwise win over this one.
/// Both languages, Taiwan (`zh-tw`) and Hong Kong (`zh-hk`, the same text with the Hong Kong names), each with the tables of
/// its game text and of its menus. A `zh-hk.tsv` the early launchers left there (the Taiwan table under the code before the
/// game's patch 0022) is replaced by the Hong Kong one like any other old file, kept as `.bak` ([`sync_into`]).
const SYNCED: &[&str] = &[
    "zh-tw.tsv",
    "ui-zh-tw.tsv",
    "zh-hk.tsv",
    "ui-zh-hk.tsv",
    "fonts/LXGWWenKaiTC-Regular.ttf",
    "fonts/OFL.txt",
];

/// The table `old` (the data folder's) as the bundled table `bundled` with the player's `#font` lines: `Some` when the two differ
/// in their `#font` lines only (a player who set `#font<TAB>NotoSansTC` to go back to Noto Sans TC keeps it), compared line by
/// line without a byte order mark or carriage returns; `None` when any other line differs. The bundled text is kept as it
/// is, its `#font` lines replaced by the player's (taken out when the player has none).
fn font_kept(old: &[u8], bundled: &[u8]) -> Option<Vec<u8>> {
    let text = |bytes: &[u8]| -> Option<String> {
        let text = std::str::from_utf8(bytes).ok()?;
        Some(text.strip_prefix('\u{feff}').unwrap_or(text).to_owned())
    };
    let (old, bundled) = (text(old)?, text(bundled)?);
    let is_font = |line: &str| line.starts_with("#font\t") || line == "#font";
    let rest = |text: &str| text.lines().filter(|line| !is_font(line)).map(str::to_owned).collect::<Vec<_>>();
    if rest(&old) != rest(&bundled) {
        return None;
    }
    let mut theirs: Vec<&str> = old.lines().filter(|line| is_font(line)).collect();
    let mut out = String::with_capacity(bundled.len());
    for line in bundled.split_inclusive('\n') {
        if is_font(line.trim_end_matches(['\n', '\r'])) {
            // the player's #font in place of the bundled one, at the bundled one's place
            if !theirs.is_empty() {
                out.push_str(theirs.remove(0));
                out.push('\n');
            }
        } else {
            out.push_str(line);
        }
    }
    if !theirs.is_empty() {
        // a #font the bundled table has not: put after the first line (the #name), as the bundled tables have it
        let first = out.find('\n').map_or(out.len(), |at| at + 1);
        let added: String = theirs.iter().map(|line| format!("{line}\n")).collect();
        out.insert_str(first, &added);
    }
    Some(out.into_bytes())
}

/// The tables of the translation, whose `#font` a player may change (see [`font_kept`]).
const TABLES: &[&str] = &["zh-tw.tsv", "ui-zh-tw.tsv", "zh-hk.tsv", "ui-zh-hk.tsv"];

/// The file in the data folder where the launcher notes the SHA-256 of each table it last wrote there (`<table> = "<sha256>"`
/// a line): a table still equal to it is the launcher's, not the player's.
const SYNC_MARKER: &str = ".launcher-sync.toml";

/// The tables earlier launchers wrote, before [`SYNC_MARKER`] (`#font` NotoSansTC), by SHA-256: with no note yet, a table
/// equal to one of them is the launcher's too.
const KNOWN_TABLES: &[(&str, &str)] = &[
    ("zh-tw.tsv", "50b4369a656cf04c1c5ccf1fe33fe1187ffafb563e37905604539c508e7f90ec"),
    ("ui-zh-tw.tsv", "996f598e822fef6aedd2b24183bd8b5555bc15f7c0e02c711ece050856c39114"),
];

/// The text of a table as compared: without a byte order mark, its lines without carriage returns.
fn table_lines(bytes: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(text.strip_prefix('\u{feff}').unwrap_or(text).lines().map(str::to_owned).collect())
}

/// The notes of [`SYNC_MARKER`] in `data`: each table's SHA-256 as the launcher last wrote it.
fn sync_notes(data: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(data.join(SYNC_MARKER))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().trim_matches('"').to_owned(), v.trim().trim_matches('"').to_owned()))
        .collect()
}

/// Writes `files` (the path under `data`, the bytes) into the folder `data`, one line per file for `last-run.log`:
/// - `unchanged`: the file there has the same bytes, or (a table) the same lines but for a byte order mark or carriage returns;
/// - `kept-font`: a table the player changed (not the one the launcher last wrote, [`SYNC_MARKER`], nor before the note an
///   earlier launcher's, `known`) that differs from the bundled one in its `#font` lines only: written again as the bundled
///   table with the player's `#font`, no `.bak` ([`font_kept`]);
/// - `replaced (old kept as .bak)`: any other difference (the old one renamed `<name>.bak`);
/// - `written`: there was none.
///
/// The SHA-256 of each bundled table is noted in [`SYNC_MARKER`] (the bundled one, also when the player's `#font` is kept).
fn sync_into(data: &Path, files: &[(&str, &[u8])], known: &[(&str, &str)]) -> Result<Vec<String>, String> {
    let notes = sync_notes(data);
    let mut written_notes = notes.clone();
    let mut lines = Vec::new();
    for &(rel, bytes) in files {
        let target = data.join(rel);
        let old = std::fs::read(&target).ok();
        let table = TABLES.contains(&rel);
        let (action, contents): (&str, Option<Vec<u8>>) = match &old {
            Some(old) if old.as_slice() == bytes => ("unchanged", None),
            Some(old) if table && table_lines(old).is_some() && table_lines(old) == table_lines(bytes) => ("unchanged", None),
            Some(old) => {
                let sha = hex(&Sha256::digest(old));
                let noted = notes.iter().find(|(k, _)| k == rel).map(|(_, v)| v.as_str());
                let the_launchers = match noted {
                    Some(noted) => noted == sha,
                    None => known.iter().any(|&(k, v)| k == rel && v == sha),
                };
                match (table && !the_launchers).then(|| font_kept(old, bytes)).flatten() {
                    Some(kept) => {
                        let write = (kept != *old).then_some(kept);
                        ("kept-font", write)
                    }
                    None => {
                        let mut bak = target.clone().into_os_string();
                        bak.push(".bak");
                        std::fs::rename(&target, &bak)
                            .map_err(|e| format!("無法備份 {}：{e}", target.display()))?;
                        ("replaced (old kept as .bak)", Some(bytes.to_vec()))
                    }
                }
            }
            None => ("written", Some(bytes.to_vec())),
        };
        if let Some(contents) = contents {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}：{e}", parent.display()))?;
            }
            std::fs::write(&target, contents).map_err(|e| format!("無法寫入 {}：{e}", target.display()))?;
        }
        if table {
            let sha = hex(&Sha256::digest(bytes));
            match written_notes.iter_mut().find(|(k, _)| k == rel) {
                Some(entry) => entry.1 = sha,
                None => written_notes.push((rel.to_owned(), sha)),
            }
        }
        lines.push(format!("sync {action}: {}", target.display()));
    }
    if written_notes != notes {
        let text: String = written_notes.iter().map(|(k, v)| format!("\"{k}\" = \"{v}\"\n")).collect();
        std::fs::write(data.join(SYNC_MARKER), text)
            .map_err(|e| format!("無法寫入 {}：{e}", data.join(SYNC_MARKER).display()))?;
    }
    Ok(lines)
}

/// The [`SYNCED`] files as bundled: the path under the data folder, the bytes.
fn bundled_synced() -> Result<Vec<(&'static str, &'static [u8])>, String> {
    SYNCED
        .iter()
        .map(|rel| {
            let asset = ASSETS.iter().find(|a| a.path == *rel).ok_or_else(|| format!("{rel} 不在程式內"))?;
            Ok((*rel, asset.bytes))
        })
        .collect()
}

/// [`settle_language`] on the data folder `data`, then the `files` written there ([`sync_into`]). In this order: the sync
/// writes [`SYNC_MARKER`], by which the settling tells an earlier player from a new one.
fn settle_and_sync(data: &Path, files: &[(&str, &[u8])]) -> Result<Vec<String>, String> {
    let mut lines = settle_language(data);
    lines.extend(sync_into(data, files, KNOWN_TABLES)?);
    Ok(lines)
}

/// The language settled, then the [`SYNCED`] files written into the data folder ([`settle_and_sync`]): a file there with other
/// bytes is replaced (the old one kept as `.bak`), but a table that differs in its `#font` only keeps the player's `#font`. One
/// line per file, for `last-run.log`.
fn sync_data_files() -> Result<Vec<String>, String> {
    settle_and_sync(&game_data_dir(), &bundled_synced()?)
}

/// The file in the data folder that says the language was settled ([`settle_language`]): its line of the log.
const LANGUAGE_MARKER: &str = ".launcher-lang";
/// The game's PC file under the data folder (`ssd-app` `options.rs`: `ssd.settings` in the saves folder).
const SETTINGS_FILE: &str = "saves/ssd.settings";
/// The first word of the PC file's header line (`ssd_menus::options`, `PC_MAGIC`).
const SETTINGS_MAGIC: &str = "ssd-settings";
/// The key of the language in the PC file (`ssd_menus::Language::KEY`).
const LANGUAGE_KEY: &str = "access.language";
/// The language the earlier launchers played in (each run's `--lang zh-tw`), kept for their players.
const EARLIER_LANGUAGE: &str = "zh-tw";

/// Once per data folder `data` (until [`LANGUAGE_MARKER`] is there): the earlier launchers ran the game with `--lang zh-tw`,
/// which overrode whatever the PC file held, and this one gives no `--lang` (the game asks a new player on its first page and
/// keeps the choice). So the player of an earlier launcher (before this run, [`SYNC_MARKER`] is in `data` or the saves folder
/// holds a file) whose PC file holds no language, or one that is not `zh-tw` (English, an index of the game's first
/// versions, `zh-hk`, another code), gets `zh-tw` there ([`settle_settings`]) and goes on in Taiwan Chinese, unasked: before
/// this release there was no Hong Kong table, so an earlier player's `zh-hk` is the early launchers' Taiwan table under that
/// code or the first versions' index 1 turned into it, played in Taiwan Chinese all the same. A new
/// player is left to the game's page. The marker is written either way, after which the player's choice is never changed:
/// - a PC file that is not one (not text, or no `ssd-settings N` header) is left as it is, to the game: one with a wrong
///   header the game moves aside as `.bad` and asks the language on its first page; one that is not text the game cannot
///   read, and plays with the defaults. The marker is written, so what the player picks is kept (an earlier player whose
///   file was damaged sees the page: the settings were lost anyway);
/// - a PC file that cannot be read or written (an error of the disk: the old value, English or an index, still holds and no
///   `--lang` hides it any more) is tried again next run, without the marker. Until a run succeeds, a language the player
///   chooses in the options meanwhile is set back to `zh-tw` once;
/// - a new player's marker that cannot be written: the next run (the sync has written [`SYNC_MARKER`] by then) takes the
///   player for an earlier one, and the language the player chose is set to `zh-tw`, run after run until the marker is
///   written. A known limit, said in the log.
///
/// Lines for `last-run.log`.
fn settle_language(data: &Path) -> Vec<String> {
    let marker = data.join(LANGUAGE_MARKER);
    if marker.exists() {
        return vec![format!("language settled before ({}): the player's choice", marker.display())];
    }
    let earlier = data.join(SYNC_MARKER).is_file()
        || std::fs::read_dir(data.join("saves")).is_ok_and(|mut files| files.next().is_some());
    let line = if earlier {
        match settle_settings(&data.join(SETTINGS_FILE)) {
            Ok(line) => line,
            Err(Unsettled::NotSettings(e)) => format!(
                "language left to the game: {e}; the settings file is not in the right format (one with a wrong header the game moves aside as .bad and asks the language on its page; one that is not text it cannot read, and plays with the defaults)"
            ),
            Err(Unsettled::Disk(e)) => return vec![format!("language settle FAILED (tried again next run): {e}")],
        }
    } else {
        "language left to the game's first page: a new data folder".to_owned()
    };
    let written = std::fs::create_dir_all(data).and_then(|()| std::fs::write(&marker, format!("{line}\n")));
    match written {
        Ok(()) => vec![line],
        Err(e) => vec![
            line,
            format!(
                "language marker FAILED: {}: {e}; until it is written, each run takes this data folder for an earlier player's and sets its language to zh-tw",
                marker.display()
            ),
        ],
    }
}

/// Why [`settle_settings`] left the PC file as it was.
#[derive(Debug)]
enum Unsettled {
    /// Not a PC file (not text, or no `ssd-settings N` header): left to the game (one with a wrong header it moves aside
    /// and asks; one that is not text it cannot read, and plays with the defaults).
    NotSettings(String),
    /// Read or written in vain: tried again next run.
    Disk(String),
}

/// The language of the PC file `file` made `zh-tw` unless it is `zh-tw` already (read as the game reads it: the last
/// `access.language` line with a value, without case, `_` as `-`). Only that line changes (each line of the key, its line end
/// kept), or one is added at the end; every other byte is kept. Without the file, one is written holding the language alone, as
/// the game writes it. Replaced through `<file>.tmp` and a rename. The line for the log.
///
/// # Errors
///
/// [`Unsettled::NotSettings`]: the file is not a PC file (left as it is). [`Unsettled::Disk`]: it cannot be read or written.
fn settle_settings(file: &Path) -> Result<String, Unsettled> {
    let disk = |path: &Path, e: std::io::Error| Unsettled::Disk(format!("{}: {e}", path.display()));
    let text = match std::fs::read(file) {
        Ok(bytes) => Some(
            String::from_utf8(bytes).map_err(|_| Unsettled::NotSettings(format!("{}: not text", file.display())))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(disk(file, e)),
    };
    let set = format!("{LANGUAGE_KEY}={EARLIER_LANGUAGE}");
    let (new, was) = match &text {
        None => (format!("{SETTINGS_MAGIC} 1\n{set}\n"), "no settings file".to_owned()),
        Some(text) => {
            let mut lines = text.split_inclusive('\n');
            let header = lines.next().unwrap_or("");
            let fine = header
                .trim()
                .strip_prefix(SETTINGS_MAGIC)
                .and_then(|version| version.trim().parse::<u32>().ok())
                .is_some_and(|version| version > 0);
            if !fine {
                return Err(Unsettled::NotSettings(format!("{}: not a {SETTINGS_MAGIC} file", file.display())));
            }
            let is_key = |line: &str| line.split_once('=').is_some_and(|(key, _)| key.trim() == LANGUAGE_KEY);
            let stored = text
                .lines()
                .skip(1)
                .filter(|line| is_key(line))
                .filter_map(|line| line.split_once('=').map(|(_, value)| value.trim()))
                .filter(|value| !value.is_empty())
                .last();
            if let Some(code) = stored.map(|value| value.to_ascii_lowercase().replace('_', "-")) {
                if code == EARLIER_LANGUAGE {
                    return Ok(format!("language kept: {code} ({})", file.display()));
                }
            }
            let mut new = String::from(header);
            let mut found = false;
            for line in lines {
                let body = line.trim_end_matches(['\n', '\r']);
                if is_key(body) {
                    found = true;
                    new.push_str(&set);
                    new.push_str(&line[body.len()..]);
                } else {
                    new.push_str(line);
                }
            }
            if !found {
                if !new.ends_with('\n') {
                    new.push('\n');
                }
                new.push_str(&set);
                new.push('\n');
            }
            (new, stored.map_or_else(|| "no language".to_owned(), |value| format!("was {value}")))
        }
    };
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| disk(parent, e))?;
    }
    let mut partial = file.as_os_str().to_owned();
    partial.push(".tmp");
    std::fs::write(&partial, &new).map_err(|e| disk(Path::new(&partial), e))?;
    std::fs::rename(&partial, file).map_err(|e| disk(file, e))?;
    Ok(format!("language set to {EARLIER_LANGUAGE} ({was}), an earlier launcher's player: {}", file.display()))
}

/// The folder, under `assets/` and in the game's data folder, of the pictures the game draws in place of the HD art pack's
/// (`<data dir>\art-override\<art id>.png`, each the size of the pack's own picture; the game's `ssd_hd::pack::ART_OVERRIDE`,
/// patch 0047): Super Vegeta without his shoulder pads.
const ART_OVERRIDE: &str = "art-override";
/// The folder in [`ART_OVERRIDE`] of Super Vegeta's own pictures of poses the pack has only Vegeta's art for
/// (`<data dir>\art-override\super-vegeta\<art id>.png`, the game's `ssd_hd::pack::SUPER_VEGETA_DIR`, patch 0049): drawn
/// only in Super Vegeta's colours, so Vegeta keeps his pads and his black hair.
const ART_SUPER_VEGETA: &str = "super-vegeta";
/// The start of the names, in [`ART_SUPER_VEGETA`], of a picture of a pose the pack has no art for and of its pose:
/// `key-<key>.png` beside `key-<key>.pose`, `<key>` the pose's 16 hexadecimal digits (the game's
/// `ssd_hd::pack::KEYED_PICTURE` and `KEYED_POSE_EXTENSION`: what the pose is, without the original sprite's pixels
/// `key-<key>.txt`, which a release never ships).
const ART_KEYED: &str = "key-";
/// The folder in [`ART_OVERRIDE`] of the normal Goku's own pictures (black hair) of poses the pack has only Super Saiyan art
/// for (`<data dir>\art-override\goku\<art id>.png`, the game's `ssd_hd::pack::NORMAL_GOKU_DIR`, patch 0051): drawn only
/// where his hair is not the Super Saiyan's, so the Super Saiyan keeps his golden hair.
const ART_GOKU: &str = "goku";
/// The folder in [`ART_OVERRIDE`] of Goku's pictures for the ending's flight with Gohan only (his torn clothes;
/// `<data dir>\art-override\ending-goku\<art id>.png`, the game's `ssd_hd::pack::ENDING_GOKU_DIR`): drawn in place of the
/// art of its name only while that flight is on screen, so the same art in the fights is left as it was.
const ART_ENDING_GOKU: &str = "ending-goku";
/// The folder in [`ART_OVERRIDE`] of pose variants (`<data dir>\art-override\variants\<art id>.png`, the game's
/// `ssd_hd::pack::VARIANTS_DIR`): each the pack's picture grown by 76 pixels on each side, drawn in every frame in place of
/// the art of its name (the Super Saiyan Goku's raised hair, for poses the pack has no variant of).
const ART_VARIANTS: &str = "variants";
/// The folder in [`ART_OVERRIDE`] of the lightning of Goku's first Super Saiyan transformation
/// (`<data dir>\art-override\ssj-lightning\frame-1.png` to `frame-4.png`, the game's `ssd_hd::lightning::DIR`): drawn over
/// the whole Enhanced picture while the transformation goes on, in turn when there are several.
const ART_SSJ_LIGHTNING: &str = "ssj-lightning";
/// The folder in [`ART_OVERRIDE`] of the art of Mode 7 planes, a folder for each plane by what it shows
/// (`<data dir>\art-override\mode7\<content key>\t<x>_<y>.png`, with `palette.txt` and the empty file `replace` it may
/// hold; the game's `ssd_hd::pack::MODE7_DIR`, which reads only those names and logs any other).
const ART_MODE7: &str = "mode7";
/// The files of a plane's folder in [`ART_MODE7`] besides its PNG tiles: the game's `ssd_hd::pack` reads them too.
const ART_MODE7_FILES: &[&str] = &["palette.txt", "replace"];
/// The file beside `art-override\` (in the data folder, so that deleting the folder keeps it) where the launcher notes each
/// picture it has written, `<file name><TAB><SHA-256>` a line: a picture noted there is not written again once the player
/// removed or changed it (or deleted the whole folder to go back to the pack's pictures), and a newer bundled picture
/// replaces only the file the launcher wrote.
const ART_MARKER: &str = ".launcher-art";
/// The note written beside the pictures in `art-override\`: where they come from, their licence, and how to go back.
const ART_LICENSE_FILE: &str = "LICENSE-ART.txt";
const ART_LICENSE: &str = "超級達爾去墊肩美術、克林光頭美術、普通悟空黑髮美術與超賽悟空豎髮美術（art-override）\r\n\r\n\
這個資料夾裡的 PNG（超級達爾的戰鬥姿勢與碎片、選指令時超級達爾的正面站姿 gallery-party-piccolo-u01-055（圖名是比克，\
畫的是超級達爾），以及龜仙屋裡布瑪與克林同一張的待機圖 field-party-groups-000／001），子資料夾 super-vegeta 裡的 PNG（美術包只有普通達爾圖的超級達爾姿勢，\
以及 GAME OVER 字幕上飛過的超級達爾 gallery-fx-1118，遊戲只在超級達爾的配色時使用，普通達爾照舊），子資料夾 goku 裡的 PNG（美術包只有超級賽亞人圖的普通悟空姿勢，\
遊戲只在悟空不是超級賽亞人髮色時使用，超級賽亞人照舊），以及子資料夾 variants 裡的 PNG（美術包沒有豎髮 variant 的\
超賽悟空姿勢，以及一般悟空龜派氣功的背面圖 gallery-party-goku-u00-170；畫布是原圖四邊各加 76 像素，遊戲每一幀都用它取代同名姿勢，\
頭髮與手可以長出原圖的框），\
子資料夾 ending-goku 裡的 PNG（遊戲只在結局悟空帶悟飯飛行那一段使用，其他場面照舊），\
都改作自上游專案 Dragon Ball Z: Super Saiya Densetsu - SNES Decomp / Classic / Remastered, Rewritten in Rust\
（JackRushante，https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust）的 HD 美術包原圖（hd-pack.zip v0.3.9）；\
variants 裡的 battle-goku-ssj-020.png 與 super-vegeta 裡以 key- 開頭的 PNG 除外，見下。\r\n\
原作：上游專案的 HD 美術包，授權 CC BY-NC-SA 4.0（https://creativecommons.org/licenses/by-nc-sa/4.0/）。\r\n\
改作：Ming Liu（以 Google Gemini 圖像模型編輯：超級達爾的圖去掉肩甲，super-vegeta 裡的圖另改成超級賽亞人的金髮；克林的兩張把美術包\
畫成有頭髮的克林改回光頭，同一張圖裡的布瑪照原圖；goku 裡的圖改成普通悟空的黑髮；\
variants 裡超賽悟空的圖把頭髮改成豎起的刺蝟頭，一般悟空的 gallery-party-goku-u00-170 換上超賽版重畫的雙手）。\
改作沿用同一授權 CC BY-NC-SA 4.0，僅限非商業使用。\r\n\r\n\
art-override/mode7/1239827fe73e508f/t0_0.png：以上游 HD 美術包的超級達爾 Mode 7 平面為底，去除墊肩後的衍生圖（2026-11）。\
原作：上游專案的 HD 美術包，授權 CC BY-NC-SA 4.0（https://creativecommons.org/licenses/by-nc-sa/4.0/）。\
改作沿用同一授權 CC BY-NC-SA 4.0，僅限非商業使用。\r\n\r\n\
art-override/mode7/cd83cbee861e14ad/t0_0.png：以上游 HD 美術包的普通悟空 Mode 7 平面為底，改成衣服破掉的超級賽亞人的衍生圖\
（開場飛行，2026-11）。原作：上游專案的 HD 美術包，授權 CC BY-NC-SA 4.0（https://creativecommons.org/licenses/by-nc-sa/4.0/）。\
改作沿用同一授權 CC BY-NC-SA 4.0，僅限非商業使用。\r\n\r\n\
art-override/variants 裡超賽悟空換成新頭的 73 張（2026-11）：battle-goku-ssj-000～012、015～018、021～023、027～030、\
032～038、045～048、050、052～063、066～069、071（53 張）與 gallery-party-goku-ssj-003、022、030、053、056、070、071、\
088、093、098、107、110、113、117、128、141、158、170、171、177（20 張）。以 Google Gemini 圖像模型\
編輯原本 art-override 裡的同名圖，把頭換成同一顆新頭（多數整張由 AI 依原圖重畫），\
Ming Liu 去背、縮放與對位。其中 gallery-party-goku-ssj-170（龜派氣功推出氣功波的背面圖）的雙手另以 Google Gemini 圖像模型\
重畫：上游美術包的原圖在這個姿勢多畫了一隻手。底圖改作自上游專案的 HD 美術包原圖，授權 CC BY-NC-SA 4.0\
（https://creativecommons.org/licenses/by-nc-sa/4.0/）。改作沿用同一授權 CC BY-NC-SA 4.0，僅限非商業使用。\r\n\r\n\
art-override/variants/gallery-party-goku-u00-170.png 與 art-override/gallery-party-goku-u00-170.png：\
一般悟空龜派氣功推出氣功波的背面圖（2026-11）。雙手取自超賽版 gallery-party-goku-ssj-170 的手\
（Ming Liu 以 Google Gemini 圖像模型重畫），貼到 HD 美術包的原圖上。底圖為上游專案的 HD 美術包原圖，授權 CC BY-NC-SA 4.0\
（https://creativecommons.org/licenses/by-nc-sa/4.0/）。改作沿用同一授權 CC BY-NC-SA 4.0，僅限非商業使用。\r\n\r\n\
art-override/gallery-party-piccolo-u01-061.png：比克蓄力姿勢（2026-11），依原作像素重繪，原作圖像權利屬原權利人，\
非商業同人圖。\r\n\r\n\
art-override/variants/battle-goku-ssj-020.png：超賽悟空集氣全身圖（穿破藍內衣的正面圖，2026-11），AI 依官方插圖重畫，\
原作圖像權利屬原權利人，非商業同人圖。\r\n\r\n\
art-override/super-vegeta 裡的 key-14b06ca30f359b50.png、key-9b36beaeedd4fd2b.png、key-9fb61f64d7d15be9.png：\
悟空龜派氣功命中超級達爾時的三個受招姿勢（2026-11），AI 依原作像素圖與既有圖重繪，原作圖像權利屬原權利人，\
非商業同人圖。同名的 .pose 只記姿勢的鍵值、大小、用到的色號與鏡像鍵值（不含原版像素），遊戲用它核對姿勢。\r\n\r\n\
art-override/super-vegeta 裡的 key-792fd8e6151f6126.png、key-2ffb009ee0521c55.png、key-ffe840f345eaed93.png、\
key-548c351e64b1725d.png、key-51afa33dcdee87e0.png、key-5a0714a076141e78.png、key-740b6f8f98f3cc54.png、\
key-10763b598c8fd2dc.png、key-0bb46e676ac56916.png、key-57f1d3ba8487754d.png、key-dc88aec8269935e7.png、\
key-ed00ab8970a6db2d.png、key-fb41e72dd32fab1a.png、key-d2047839a7642c43.png、key-c777680aa73f6d9e.png、\
key-b2bb355b0c6409f2.png：超級達爾閃開龜派氣功的背面、肉搏閃避飛出畫面邊緣、\
氣圓斬擲出、強力能量波特寫滑入與由畫面下緣升起的姿勢（2026-11），AI 依原作像素圖與既有圖重繪，畫面邊緣被裁的姿勢由同批重繪圖或\
同資料夾既有的圖裁出，原作圖像權利屬原權利人，非商業同人圖。同名的 .pose 只記姿勢的鍵值、大小、用到的色號與鏡像鍵值（不含原版像素），遊戲用它核對姿勢。\r\n\r\n\
art-override/ssj-lightning 裡的 PNG：悟空第一次變身超級賽亞人時疊在畫面上的閃電（2026-11），AI 生成的電弧素材，\
未取自原作或美術包。\r\n\r\n\
刪掉這個資料夾（save\\art-override，連同 super-vegeta、goku、ending-goku、variants、ssj-lightning 與 mode7）即回到 HD 美術包的原圖；\
只刪 variants 就只回到美術包的扁髮。啟動器不會再寫回來。\r\n";

/// Sets up `pictures` (each a file name and its bytes) in `dir` (the data folder's `art-override\`): a picture not yet in
/// [`ART_MARKER`] is written and noted, unless a file of that name with other bytes is there (the player's own: left and
/// not noted); a noted picture is only brought up to date, while its file is the one the launcher wrote. With `apply` false
/// nothing is written: the line says what would be done. One line for all the pictures.
///
/// # Errors
///
/// A file that cannot be written.
fn install_art_override(dir: &Path, pictures: &[(&str, &[u8])], apply: bool) -> Result<Vec<String>, String> {
    let err = |path: &Path, e: std::io::Error| format!("{}：{e}", path.display());
    // beside the folder, not in it: a player who deletes art-override\ to go back to the pack's pictures keeps the note
    let marker_path = dir.parent().unwrap_or(dir).join(ART_MARKER);
    let noted: Vec<(String, String)> = std::fs::read_to_string(&marker_path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(file, sha)| (file.to_owned(), sha.to_owned()))
        .collect();
    let mut new_marker = noted.clone();
    let (mut installed, mut updated, mut same) = (0, 0, 0);
    let (mut players, mut removed) = (Vec::new(), Vec::new());
    for &(file, bytes) in pictures {
        let sha = hex(&Sha256::digest(bytes));
        // `file` is a name in `dir`, `super-vegeta/<name>` (patch 0049), `goku/<name>` (patch 0051), `ending-goku/<name>`,
        // `variants/<name>` or `mode7/<content key>/<name>`
        let target = file.split('/').fold(dir.to_path_buf(), |path, part| path.join(part));
        let on_disk = std::fs::read(&target).ok().map(|b| hex(&Sha256::digest(&b)));
        let written = noted.iter().find(|(f, _)| f == file).map(|(_, s)| s.as_str());
        let write = match (written, on_disk.as_deref()) {
            (_, Some(now)) if now == sha => {
                same += 1;
                written.is_none()
            }
            (Some(written), Some(now)) if now == written => {
                updated += 1;
                true
            }
            (Some(_), Some(_)) | (None, Some(_)) => {
                players.push(file);
                continue;
            }
            (Some(_), None) => {
                removed.push(file);
                continue;
            }
            (None, None) => {
                installed += 1;
                true
            }
        };
        if !write {
            continue;
        }
        if apply && on_disk.as_deref() != Some(sha.as_str()) {
            let folder = target.parent().unwrap_or(dir);
            std::fs::create_dir_all(folder).map_err(|e| err(folder, e))?;
            std::fs::write(&target, bytes).map_err(|e| err(&target, e))?;
        }
        match new_marker.iter_mut().find(|(f, _)| f == file) {
            Some(entry) => entry.1.clone_from(&sha),
            None => new_marker.push((file.to_owned(), sha)),
        }
    }
    if apply && new_marker != noted {
        std::fs::create_dir_all(dir).map_err(|e| err(dir, e))?;
        let text: String = new_marker.iter().map(|(f, s)| format!("{f}\t{s}\n")).collect();
        std::fs::write(&marker_path, text).map_err(|e| err(&marker_path, e))?;
    }
    let verb = if apply { "" } else { "would be " };
    let mut line = format!(
        "art override {}: {installed} {verb}installed, {updated} {verb}updated, {same} same",
        dir.display()
    );
    if !players.is_empty() {
        line.push_str(&format!(", kept (changed by the player): {}", players.join(" ")));
    }
    if !removed.is_empty() {
        line.push_str(&format!(", kept removed (removed by the player): {}", removed.join(" ")));
    }
    Ok(vec![line])
}

/// The pictures of the bundle's `art-override/` set up in the game's data folder's `art-override\` (see
/// [`install_art_override`]); in a `SSD_DATA_DIR` the player set nothing is done, as [`sync_data_files`] leaves it.
fn set_up_art_override(apply: bool) -> Result<Vec<String>, String> {
    if !(data_dir().beside_exe || data_dir().fallback) {
        return Ok(vec!["art override skipped: the data folder is the player's SSD_DATA_DIR".to_owned()]);
    }
    let pictures = art_override_pictures(ASSETS.iter().map(|a| (a.path, a.bytes)));
    install_art_override(&game_data_dir().join(ART_OVERRIDE), &pictures, apply)
}

/// Of the bundle's files (`path`, bytes), the ones written into `art-override\`, by their path there: the PNGs of
/// `art-override/`, of its `super-vegeta/` (patch 0049), of its `goku/` (patch 0051), of its `ending-goku/` (the ending's
/// flight), of its `variants/` (the pose variants) and of its `ssj-lightning/` (the transformation's lightning), the PNGs, `palette.txt` and `replace` of each plane's folder `mode7/<content key>/` (the Mode 7 planes),
/// the poses `key-<key>.pose` of `super-vegeta/` ([`ART_KEYED`]: the game reads them beside `key-<key>.png`)
/// and [`ART_LICENSE_FILE`], the
/// pictures' licence and how to go back (the game reads only the PNGs, those poses and the files of the planes). A file of
/// another folder is not one of them, and neither are the pixels `key-<key>.txt` (the original sprite's, never written).
fn art_override_pictures<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Vec<(&'a str, &'a [u8])> {
    let prefix = format!("{ART_OVERRIDE}/");
    let in_folder = |file: &str| {
        !file.contains('/')
            || [ART_SUPER_VEGETA, ART_GOKU, ART_ENDING_GOKU, ART_VARIANTS, ART_SSJ_LIGHTNING].iter().any(|folder| {
                file.strip_prefix(folder)
                    .and_then(|rest| rest.strip_prefix('/'))
                    .is_some_and(|name| !name.contains('/'))
            })
    };
    // `mode7/<content key>/<name>`, one folder deeper than the others: the game checks the key and the names
    let in_mode7 = |file: &str| {
        file.strip_prefix(ART_MODE7)
            .and_then(|rest| rest.strip_prefix('/'))
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(key, name)| {
                !key.is_empty()
                    && !name.contains('/')
                    && (name.ends_with(".png") || ART_MODE7_FILES.contains(&name))
            })
    };
    // `super-vegeta/key-<key>.pose`, `<key>` 16 hexadecimal digits (either case): the name the game reads the pose of
    // `key-<key>.png` from
    let keyed_pose = |file: &str| {
        file.strip_prefix(ART_SUPER_VEGETA)
            .and_then(|rest| rest.strip_prefix('/'))
            .and_then(|name| name.strip_prefix(ART_KEYED))
            .and_then(|rest| rest.strip_suffix(".pose"))
            .is_some_and(|key| key.len() == 16 && key.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    files
        .into_iter()
        .filter_map(|(path, bytes)| Some((path.strip_prefix(prefix.as_str())?, bytes)))
        .filter(|(file, _)| {
            (file.ends_with(".png") && in_folder(file)) || in_mode7(file) || keyed_pose(file)
        })
        .chain(std::iter::once((ART_LICENSE_FILE, ART_LICENSE.as_bytes())))
        .collect()
}

/// How many of `pictures` (the files of [`art_override_pictures`]) are PNGs of `art-override\` itself and of each of its
/// folders, for the selftest: `art override folders: top N, super-vegeta N, goku N, ending-goku N, variants N,
/// ssj-lightning N, mode7 N`.
fn art_override_folders(pictures: &[(&str, &[u8])]) -> String {
    let pngs = |folder: Option<&str>| {
        pictures
            .iter()
            .filter(|(file, _)| file.ends_with(".png"))
            .filter(|(file, _)| match folder {
                None => !file.contains('/'),
                Some(folder) => file.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/')),
            })
            .count()
    };
    let mut line = format!("art override folders: top {}", pngs(None));
    for folder in [ART_SUPER_VEGETA, ART_GOKU, ART_ENDING_GOKU, ART_VARIANTS, ART_SSJ_LIGHTNING, ART_MODE7] {
        line.push_str(&format!(", {folder} {}", pngs(Some(folder))));
    }
    line
}

/// The folder of the game's data folder its voices are read from (the game's `voice::FOLDER`): the Japanese voice of Goku's
/// first Super Saiyan transformation, `voice\ssj-transform.wav`: installed from the bundle's [`VOICE_ASSET`] (build.rs
/// requires it in every build).
const VOICE: &str = "voice";
/// The file name of that voice (the game's `voice::SSJ_CLIP`).
const VOICE_SSJ_CLIP: &str = "ssj-transform.wav";
/// That voice's path in the bundle (required by build.rs).
const VOICE_ASSET: &str = "voice/ssj-transform.wav";
/// The note written in `voice\`: the file's name and what it does.
const VOICE_README_FILE: &str = "README.txt";
const VOICE_README: &str = "悟空第一次變成超級賽亞人的日文語音（voice）\r\n\r\n\
把語音檔命名為 ssj-transform.wav 放進這個資料夾（save\\voice\\ssj-transform.wav），悟空第一次變成超級賽亞人的那一刻就會播放，\
每次啟動遊戲最多播一次；讀取已經變身之後的存檔不會播。\r\n\
沒有這個檔案時遊戲照常進行，不播語音。\r\n\
格式：WAV 檔（PCM 16 位元最保險；單聲道、立體聲、任何取樣率都可以，遊戲會轉成 48 kHz 立體聲）。\
音量跟著選項「聲音」頁的音量。換了檔案要重新啟動遊戲才會讀到。\r\n";

/// Makes `dir` (the data folder's `voice\`) with [`VOICE_README`] in it: written when missing or other than the launcher's.
/// `clip` (the bundle's voice) is written as [`VOICE_SSJ_CLIP`] when none is there; one of other bytes there is the player's
/// own and is left. Without `clip` the voice is never written, only said present or absent. With `apply` false nothing is
/// written. One line.
///
/// # Errors
///
/// The folder or the note cannot be written.
fn install_voice_folder(dir: &Path, clip: Option<&[u8]>, apply: bool) -> Result<Vec<String>, String> {
    let readme = dir.join(VOICE_README_FILE);
    let action = match std::fs::read(&readme) {
        Ok(bytes) if bytes == VOICE_README.as_bytes() => "same",
        Ok(_) => "updated",
        Err(_) => "written",
    };
    if apply && action != "same" {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}：{e}", dir.display()))?;
        std::fs::write(&readme, VOICE_README).map_err(|e| format!("{}：{e}", readme.display()))?;
    }
    let verb = if apply || action == "same" { "" } else { "would be " };
    let path = dir.join(VOICE_SSJ_CLIP);
    let on_disk = std::fs::read(&path).ok();
    let clip = match (clip, on_disk) {
        (None, Some(_)) => "present".to_owned(),
        (None, None) => "absent".to_owned(),
        (Some(bytes), Some(disk)) if disk == bytes => "same as the bundle's".to_owned(),
        (Some(_), Some(_)) => "kept (the player's own)".to_owned(),
        (Some(bytes), None) => {
            if apply {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}：{e}", dir.display()))?;
                std::fs::write(&path, bytes).map_err(|e| format!("{}：{e}", path.display()))?;
                "installed from the bundle".to_owned()
            } else {
                "would be installed from the bundle".to_owned()
            }
        }
    };
    Ok(vec![format!("voice {}: {VOICE_README_FILE} {verb}{action}, {VOICE_SSJ_CLIP} {clip}", dir.display())])
}

/// The size of the bundle's voice ([`VOICE_ASSET`]), for the selftest.
///
/// # Errors
///
/// The bundle has no voice, or an empty one.
fn bundled_voice() -> Result<usize, String> {
    match ASSETS.iter().find(|a| a.path == VOICE_ASSET) {
        Some(a) if !a.bytes.is_empty() => Ok(a.bytes.len()),
        Some(_) => Err(format!("{VOICE_ASSET} is empty")),
        None => Err(format!("no {VOICE_ASSET} in the bundle")),
    }
}

/// The data folder's `voice\` set up ([`install_voice_folder`]); in a `SSD_DATA_DIR` the player set nothing is done, as
/// [`set_up_art_override`] leaves it.
fn set_up_voice_folder(apply: bool) -> Result<Vec<String>, String> {
    if !(data_dir().beside_exe || data_dir().fallback) {
        return Ok(vec!["voice folder skipped: the data folder is the player's SSD_DATA_DIR".to_owned()]);
    }
    let clip = ASSETS.iter().find(|a| a.path == VOICE_ASSET).map(|a| a.bytes);
    install_voice_folder(&game_data_dir().join(VOICE), clip, apply)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `dir` holds a complete unpack of this bundle (marker present, every file of its length).
fn is_complete(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join(DONE_MARKER)).is_ok_and(|id| id.trim() == BUNDLE_ID)
        && ASSETS.iter().all(|a| {
            std::fs::metadata(dir.join(a.path)).is_ok_and(|m| m.len() == a.len as u64)
        })
}

/// The app folder of this bundle, unpacked if it is not yet (into a temporary folder, renamed in
/// place, so a half-written folder is never used).
fn unpack() -> Result<PathBuf, String> {
    let apps = launcher_root().join("app");
    let dir = apps.join(BUNDLE_ID);
    if is_complete(&dir) {
        return Ok(dir);
    }
    let tmp = apps.join(format!("{BUNDLE_ID}.tmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let write_all = || -> std::io::Result<()> {
        for a in ASSETS {
            let path = tmp.join(a.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, a.bytes)?;
        }
        std::fs::write(tmp.join(DONE_MARKER), BUNDLE_ID)
    };
    write_all().map_err(|e| format!("無法解開檔案到 {}：{e}", tmp.display()))?;
    if dir.exists() {
        // An incomplete earlier unpack (or one being replaced).
        let _ = std::fs::remove_dir_all(&dir);
    }
    if let Err(e) = std::fs::rename(&tmp, &dir) {
        let _ = std::fs::remove_dir_all(&tmp);
        // Another copy of the launcher may have finished first.
        if !is_complete(&dir) {
            return Err(format!("無法建立 {}：{e}", dir.display()));
        }
    }
    Ok(dir)
}

fn error_box(text: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(TITLE)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

fn info_box(text: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(TITLE)
        .set_description(text)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// The last `max` characters of `text`, for a message box.
fn tail(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.trim().chars().collect();
    let start = chars.len().saturating_sub(max);
    chars[start..].iter().collect()
}

fn game_command(app: &Path) -> Command {
    let mut cmd = Command::new(app.join(GAME_EXE));
    cmd.current_dir(app);
    cmd.env("SSD_DATA_DIR", game_data_dir());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// The game's arguments for its debug tools: `--debug` for a private copy (one whose `.exe` has an overlay), none
/// for the public one. A damaged overlay counts as none here.
fn debug_args(overlay: &Result<Vec<overlay::Segment>, String>) -> &'static [&'static str] {
    match overlay {
        Ok(segments) if !segments.is_empty() => &["--debug"],
        _ => &[],
    }
}

/// The overlay of this launcher's `.exe` (none when it cannot be found or read as one).
fn own_overlay() -> Result<Vec<overlay::Segment>, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(overlay::read(&exe)?.unwrap_or_default())
}

/// A folder that removes itself: the ROM and IPS of the overlay are written here for the setup
/// only.
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The step every first-launch ROM goes through: a Japanese Rev 1 ROM without an IPS is refused; else the game
/// caches the ROM with one headless frame (`cache\rom\canonical.sfc`). `Err`: what to tell the player.
fn cache_rom(app: &Path, rom: &Path, ips: Option<&Path>) -> Result<(), String> {
    let rom_bytes = std::fs::read(rom).map_err(|e| format!("{}：{e}", rom.display()))?;
    let image = if rom_bytes.len() == ROM_SIZE + 512 { &rom_bytes[512..] } else { &rom_bytes[..] };
    if image.len() == ROM_SIZE && crc32(image) == ROM_JP_REV1 && ips.is_none() {
        return Err(format!("ROM 設定沒有完成。\n\n{}", rom_diagnosis(&rom_bytes, false)));
    }
    let mut cmd = game_command(app);
    cmd.arg("--rom").arg(rom);
    if let Some(ips) = ips {
        cmd.arg("--ips").arg(ips);
    }
    cmd.args(["--headless-frames", "1"]);
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("無法執行 {}：{e}", app.join(GAME_EXE).display()))?;
    if !out.status.success() || !cached_rom().is_file() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "ROM 設定失敗（{}）。\n\n{}\n\n{}",
            out.status,
            rom_diagnosis(&rom_bytes, ips.is_some()),
            tail(&stderr, 1500)
        ));
    }
    Ok(())
}

/// The first launch's steps for the installer's ROM and IPS (`preset.rs`): [`cache_rom`], an error box,
/// the IPS dialog of [`set_up_rom`].
struct LivePreset<'a> {
    app: &'a Path,
}

impl preset::PresetEnv for LivePreset<'_> {
    fn cache(&mut self, rom: &Path, ips: Option<&Path>) -> Result<(), String> {
        cache_rom(self.app, rom, ips)
    }
    fn report(&mut self, text: &str) {
        error_box(text);
    }
    fn pick_ips(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("選擇 dbz102.ips（日版 ROM 需要；英文版 ROM 請按取消）")
            .add_filter("IPS patch", &["ips"])
            .pick_file()
    }
}

/// `<exe_dir>\save\setup`, where the installer copies the ROM and IPS the player chose (`preset.rs`).
fn preset_setup_dir() -> Option<PathBuf> {
    Some(preset::setup_dir(std::env::current_exe().ok()?.parent()?))
}

/// First launch: the ROM is the overlay's, else the one the installer copied (`preset.rs`; refused: said, then
/// as before), else the player picks it; the IPS is the overlay's, else the embedded one, else the player picks it
/// (or none, for the English ROM). The game caches the ROM with one headless frame ([`cache_rom`]).
/// `Ok(false)`: the player cancelled.
fn set_up_rom(app: &Path) -> Result<bool, String> {
    let segments = own_overlay()?;
    let segment = |name: &str| segments.iter().find(|s| s.name == name);
    let tmp = TempDir(std::env::temp_dir().join(format!("dbz-ssd-zhtw-setup-{}", std::process::id())));
    let write_tmp = |name: &str, bytes: &[u8]| -> Result<PathBuf, String> {
        std::fs::create_dir_all(&tmp.0).map_err(|e| format!("{}：{e}", tmp.0.display()))?;
        let path = tmp.0.join(name);
        std::fs::write(&path, bytes).map_err(|e| format!("{}：{e}", path.display()))?;
        Ok(path)
    };
    // The installer's copies, when this copy carries no ROM of its own
    if segment("rom").is_none() {
        if let Some(setup) = preset_setup_dir() {
            let fixed_ips = match segment("ips") {
                Some(seg) => Some(write_tmp(IPS_FILE, &seg.bytes)?),
                None => HAS_IPS.then(|| app.join(IPS_FILE)),
            };
            let mut lines = Vec::new();
            let outcome = preset::run(false, &setup, fixed_ips.as_deref(), &mut LivePreset { app }, &mut lines);
            lines.iter().for_each(|l| log_line(l));
            if outcome == preset::Outcome::Done {
                drop(tmp);
                info_box(&folders_text(data_dir()));
                return Ok(true);
            }
        }
    }
    let rom = if let Some(seg) = segment("rom") {
        write_tmp("rom.sfc", &seg.bytes)?
    } else {
        info_box("第一次執行：請選擇遊戲 ROM（.sfc／.smc）。\n\n可用：日版 Rev 1 ROM，或 Klepto v1.02 英文版 ROM。");
        let Some(rom) = rfd::FileDialog::new()
            .set_title("選擇 ROM")
            .add_filter("SNES ROM", &["sfc", "smc"])
            .pick_file()
        else {
            return Ok(false);
        };
        rom
    };
    let ips = if let Some(seg) = segment("ips") {
        Some(write_tmp(IPS_FILE, &seg.bytes)?)
    } else if HAS_IPS {
        Some(app.join(IPS_FILE))
    } else {
        rfd::FileDialog::new()
            .set_title("選擇 dbz102.ips（日版 ROM 需要；英文版 ROM 請按取消）")
            .add_filter("IPS patch", &["ips"])
            .pick_file()
    };
    let cached = cache_rom(app, &rom, ips.as_deref());
    drop(tmp);
    cached?;
    info_box(&folders_text(data_dir()));
    Ok(true)
}

/// An HD art pack next to this launcher, as upstream `ssd_hdpack::install::find_source` looks for
/// one next to the game: `hd-pack.zip`, a folder `hd-pack`, or a zip with `hd-pack` in its name.
fn hd_pack_beside_launcher() -> Option<PathBuf> {
    hd_pack_in(std::env::current_exe().ok()?.parent()?)
}

/// The HD art pack in `dir`, found as [`hd_pack_beside_launcher`] says.
fn hd_pack_in(dir: &Path) -> Option<PathBuf> {
    for name in ["hd-pack.zip", "hd-pack"] {
        let path = dir.join(name);
        if path.is_file() || path.join("hd-pack.toml").is_file() {
            return Some(path);
        }
    }
    let mut zips: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
            p.is_file() && name.contains("hd-pack") && name.ends_with(".zip")
        })
        .collect();
    zips.sort();
    zips.pop()
}

/// The HD art pack the launcher expects: upstream v0.3.9's `hd-pack.zip` (484,121,119 bytes; v0.3.10 to v0.3.15 ship no
/// newer one), whose `hd-pack.toml` says `digest = "0EF46E0B"` and `revision = "2"`. An older pack lacks the art the
/// game's patches use (Super Saiyan Goku's raised hair, the portraits fixed by patch 0025).
const HD_PACK_DIGEST: &str = "0EF46E0B";
const HD_PACK_REVISION: &str = "2";
const HD_PACK_ZIP_BYTES: u64 = 484_121_119;
const HD_PACK_OLD: &str = "HD 美術包不是 v0.3.9，超賽悟空豎髮與頭像修正不會生效，請更新";
/// The file in the launcher's folder that holds the digest of the last pack the player was told about, so the dialog
/// comes once per pack (the log says it every time).
const HD_PACK_WARNED: &str = ".hd-pack-warned";

/// The value of the top-level key `key` (before the first `[table]`) of a TOML text, as a string (`key = "value"`).
fn toml_top_string(text: &str, key: &str) -> Option<String> {
    text.lines()
        .take_while(|line| !line.trim_start().starts_with('['))
        .filter_map(|line| line.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"').to_owned())
}

/// The bytes of the entry `name` (at the root) of the zip at `path`, stored or deflated; `Ok(None)` when it has none.
/// Reads only the end of the file, its central directory and the one entry (a pack is half a gigabyte).
fn zip_entry(path: &Path, name: &str) -> Result<Option<Vec<u8>>, String> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let tail_len = len.min(22 + 0xFFFF);
    let mut tail = vec![0; tail_len as usize];
    file.seek(SeekFrom::Start(len - tail_len)).map_err(|e| e.to_string())?;
    file.read_exact(&mut tail).map_err(|e| e.to_string())?;
    let eocd = (0..=tail.len().saturating_sub(22))
        .rev()
        .find(|&i| tail[i..i + 4] == [0x50, 0x4B, 0x05, 0x06])
        .ok_or("not a zip (no end of central directory)")?;
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as usize;
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as u64;
    let (count, cd_size, cd_offset) = (u16_at(&tail, eocd + 10), u32_at(&tail, eocd + 12), u32_at(&tail, eocd + 16));
    let mut cd = vec![0; cd_size as usize];
    file.seek(SeekFrom::Start(cd_offset)).map_err(|e| e.to_string())?;
    file.read_exact(&mut cd).map_err(|e| e.to_string())?;
    let mut at = 0;
    for _ in 0..count {
        if cd.len() < at + 46 || cd[at..at + 4] != [0x50, 0x4B, 0x01, 0x02] {
            return Err("a bad central directory".to_owned());
        }
        let (method, csize, size) = (u16_at(&cd, at + 10), u32_at(&cd, at + 20), u32_at(&cd, at + 24) as usize);
        let (name_len, extra_len, comment_len) = (u16_at(&cd, at + 28), u16_at(&cd, at + 30), u16_at(&cd, at + 32));
        let local = u32_at(&cd, at + 42);
        let entry_name = &cd[at + 46..at + 46 + name_len];
        at += 46 + name_len + extra_len + comment_len;
        if entry_name != name.as_bytes() {
            continue;
        }
        let mut header = [0; 30];
        file.seek(SeekFrom::Start(local)).map_err(|e| e.to_string())?;
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        let skip = u16_at(&header, 26) + u16_at(&header, 28);
        let mut data = vec![0; csize as usize];
        file.seek(SeekFrom::Start(local + 30 + skip as u64)).map_err(|e| e.to_string())?;
        file.read_exact(&mut data).map_err(|e| e.to_string())?;
        return match method {
            0 => Ok(Some(data)),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(&data, size.max(1))
                .map(Some)
                .map_err(|e| format!("{name}: cannot be inflated ({e:?})")),
            other => Err(format!("{name}: compression method {other}")),
        };
    }
    Ok(None)
}

/// What the launcher says of the HD pack `pack` (a `hd-pack.zip` or a `hd-pack` folder, `None` for none): `None` for
/// no pack or the expected one ([`HD_PACK_DIGEST`], [`HD_PACK_REVISION`]), else the log line and the pack's digest.
fn hd_pack_check(pack: Option<&Path>) -> Option<(String, String)> {
    let pack = pack?;
    let manifest = if pack.is_dir() {
        std::fs::read(pack.join("hd-pack.toml")).map(Some).map_err(|e| e.to_string())
    } else {
        zip_entry(pack, "hd-pack.toml")
    };
    let text = match manifest {
        Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
        Ok(None) => return Some((format!("{HD_PACK_OLD}: {} has no hd-pack.toml", pack.display()), String::new())),
        Err(e) => return Some((format!("{HD_PACK_OLD}: {} cannot be read: {e}", pack.display()), String::new())),
    };
    let digest = toml_top_string(&text, "digest").unwrap_or_default();
    let revision = toml_top_string(&text, "revision").unwrap_or_default();
    if digest.eq_ignore_ascii_case(HD_PACK_DIGEST) && revision == HD_PACK_REVISION {
        return None;
    }
    let size = std::fs::metadata(pack).ok().filter(|m| m.is_file()).map_or(String::new(), |m| format!(", {} bytes", m.len()));
    Some((
        format!(
            "{HD_PACK_OLD}: {} digest {digest:?} revision {revision:?}{size} (v0.3.9: digest {HD_PACK_DIGEST} revision {HD_PACK_REVISION}, {HD_PACK_ZIP_BYTES} bytes)",
            pack.display()
        ),
        digest,
    ))
}

/// The game's arguments for a play: the Enhanced mode, and the debug tools of a copy whose `.exe` has this `overlay`
/// ([`debug_args`]). [`play`] adds the HD pack. No `--lang`: the language is the one the player chose (the game's first page
/// asks a new player; an earlier launcher's player was given `zh-tw` once, [`settle_language`]), which a `--lang` would
/// override every run.
fn play_args(overlay: &Result<Vec<overlay::Segment>, String>) -> Vec<&'static str> {
    let mut args = vec!["--mode", "enhanced", "--hd-scale", "4"];
    args.extend(debug_args(overlay));
    args
}

/// Runs the game; its output is appended to `last-run.log`, after the lines [`main`] wrote there
/// (the data folder, the migration, the cleanup).
fn play(app: &Path) -> Result<(), String> {
    let log_path = launcher_root().join("last-run.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("無法寫入 {}：{e}", log_path.display()))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = game_command(app);
    cmd.args(play_args(&own_overlay()));
    let pack = hd_pack_beside_launcher();
    if let Some((line, digest)) = hd_pack_check(pack.as_deref()) {
        // Said, and the game starts all the same: an older pack still draws most of the art
        let _ = writeln!(&log, "{line}");
        let warned = launcher_root().join(HD_PACK_WARNED);
        if std::fs::read_to_string(&warned).map_or(true, |seen| seen.trim() != digest) {
            info_box(&format!("{HD_PACK_OLD}。\n\n需要上游 v0.3.9 的 hd-pack.zip（{HD_PACK_ZIP_BYTES} 位元組）。\n{}", pack.as_deref().map_or_else(String::new, |p| p.display().to_string())));
            let _ = std::fs::write(&warned, &digest);
        }
    }
    if let Some(pack) = pack {
        cmd.arg("--install-hd-pack").arg(pack);
    }
    let status = cmd
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err)
        .status()
        .map_err(|e| format!("無法執行 {}：{e}", app.join(GAME_EXE).display()))?;
    if !status.success() {
        let text = std::fs::read_to_string(&log_path).unwrap_or_default();
        return Err(format!(
            "遊戲結束時回報錯誤（{status}）。\n紀錄：{}\n\n{}",
            log_path.display(),
            tail(&text, 1500)
        ));
    }
    Ok(())
}

/// Appends `line` to `last-run.log` (before the game's output).
fn log_line(line: &str) {
    if let Ok(log) = std::fs::OpenOptions::new().create(true).append(true).open(launcher_root().join("last-run.log")) {
        let _ = writeln!(&log, "{line}");
    }
}

/// A question with the buttons `labels` in a task dialog (`Some(index)` clicked, `None` closed); without a
/// task dialog (no comctl32 v6), rfd's `fallback` buttons, whose result `map` turns into the same.
fn ask_with(text_buttons: &str, text_yes_no: &str, labels: &[&str], fallback: rfd::MessageButtons, map: impl Fn(rfd::MessageDialogResult) -> Option<usize>) -> Option<usize> {
    if let Some(clicked) = progress::ask(TITLE, text_buttons, labels) {
        return clicked;
    }
    map(rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(TITLE)
        .set_description(text_yes_no)
        .set_buttons(fallback)
        .show())
}

/// Before the game: the previous release left by an update is removed (this one has started), then a newer
/// release is asked about — the one this start's check found, if it has answered already (the game never waits
/// for it), else the one kept from an earlier check. "Update now" downloads it with a progress dialog. `true`: the
/// new release was put in place and started, and this one must end.
fn update_before_play(startup: update::Startup) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    update::clean_after_update(&exe).iter().for_each(|l| log_line(l));
    let update::Startup { stored, check } = startup;
    if check.is_none() && stored.is_none() {
        return false;
    }
    let settings_path = launcher_root().join(update::SETTINGS_FILE);
    let mut settings = update::read_settings(&settings_path);
    let (offer, lines) = update::decide(update::poll(check), stored, &settings);
    lines.iter().for_each(|l| log_line(l));
    let Some(offer) = offer else { return false };
    let version = offer.manifest().version;
    let clicked = ask_with(
        &update::prompt_text(offer.manifest(), &settings_path, false),
        &update::prompt_text(offer.manifest(), &settings_path, true),
        &update::prompt_buttons(),
        rfd::MessageButtons::YesNo,
        |r| (r == rfd::MessageDialogResult::Yes).then_some(0),
    );
    if clicked != Some(0) {
        settings.skipped_version = version;
        let _ = update::write_settings(&settings_path, &settings);
        log_line(&format!("update check: v{version} offered, answered later"));
        return false;
    }
    let Some(source) = update::source() else { return false };
    let job_exe = exe.clone();
    let result = progress::with_progress(TITLE, &format!("正在更新到 v{version}"), move |p| {
        update::apply(&update::Http::download(), &source, &offer, &job_exe, p)
    })
    .and_then(|r| r)
    .and_then(|()| update::restart(&update::files(&exe)));
    match result {
        Ok(()) => {
            log_line(&format!("update: v{version} put in place, started"));
            true
        }
        Err(e) if e == progress::CANCELLED => {
            log_line(&format!("update to v{version} cancelled by the player"));
            false
        }
        Err(e) => {
            log_line(&format!("update to v{version} FAILED: {e}"));
            error_box(&format!("更新失敗，繼續使用目前的版本 {RELEASE_NAME}。\n\n{e}"));
            false
        }
    }
}

/// Before the game: without an HD art pack beside the launcher, the player is asked whether to download
/// upstream's ([`hd_offer`]); nothing connects unless they say "download", which shows a progress dialog.
fn hd_pack_before_play() {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else { return };
    if let Some(line) = hd_offer::clean_stale(&dir) {
        log_line(&line);
    }
    let ask = || {
        hd_offer::answer_of(ask_with(
            &hd_offer::prompt_text(&hd_offer::OFFICIAL, &dir, false),
            &hd_offer::prompt_text(&hd_offer::OFFICIAL, &dir, true),
            &hd_offer::prompt_buttons(),
            rfd::MessageButtons::YesNoCancel,
            |r| match r {
                rfd::MessageDialogResult::Yes => Some(0),
                rfd::MessageDialogResult::No => Some(2),
                _ => Some(1),
            },
        ))
    };
    let job_dir = dir.clone();
    let download = move || {
        progress::with_progress(TITLE, "正在下載 HD 美術包", move |p| {
            hd_offer::download(&update::Http::download(), &hd_offer::OFFICIAL, &job_dir, p)
        })
        .and_then(|r| r)
    };
    let done = hd_offer::run(hd_pack_in(&dir).is_some(), &launcher_root().join(hd_offer::STATE_FILE), ask, download);
    match &done {
        hd_offer::Done::Failed(e) => {
            error_box(&format!("HD 美術包下載失敗，這次不使用美術包（下次開啟會再問）。\n\n{e}"));
        }
        // no task dialog showed the download: say how it ended
        hd_offer::Done::Downloaded(path) if !progress::task_dialog_available() => {
            info_box(&format!("HD 美術包下載完成：\n{}", path.display()));
        }
        _ => {}
    }
    if done != hd_offer::Done::Present {
        log_line(&format!("hd pack offer: {done:?}"));
    }
}

/// `--hd-pack`: for a player who answered "not now" or "never" to the HD art pack and changed their mind (a
/// shortcut to the launcher with this argument; the installer makes one). Whatever was answered before: the
/// v0.3.9 pack already beside the launcher is said so; else a question ("下載" / "取消"), and on "下載" the same
/// download as at start (progress dialog, cancel, SHA-256, the disk read back, one launcher at a time). The result
/// in a message box; the game is not started. Nothing connects before "下載".
fn hd_pack_on_demand() -> ExitCode {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) else {
        error_box("找不到啟動器所在的資料夾。");
        return ExitCode::FAILURE;
    };
    let _ = std::fs::create_dir_all(launcher_root());
    if let Some(line) = hd_offer::clean_stale(&dir) {
        log_line(&line);
    }
    let pack = hd_pack_in(&dir);
    // the v0.3.9 pack (hd_pack_check says nothing of it); an older one is replaced by the download
    let valid = pack.as_deref().is_some_and(|p| hd_pack_check(Some(p)).is_none());
    let old = pack.as_deref().filter(|_| !valid);
    let confirm = || {
        ask_with(
            &hd_offer::demand_text(&hd_offer::OFFICIAL, &dir, old, false),
            &hd_offer::demand_text(&hd_offer::OFFICIAL, &dir, old, true),
            &hd_offer::demand_buttons(),
            rfd::MessageButtons::OkCancel,
            |r| (r == rfd::MessageDialogResult::Ok).then_some(0),
        ) == Some(0)
    };
    let job_dir = dir.clone();
    let download = move || {
        progress::with_progress(TITLE, "正在下載 HD 美術包", move |p| {
            hd_offer::download(&update::Http::download(), &hd_offer::OFFICIAL, &job_dir, p)
        })
        .and_then(|r| r)
    };
    let done = hd_offer::run_on_demand(valid, &launcher_root().join(hd_offer::STATE_FILE), confirm, download);
    log_line(&format!("--hd-pack: {done:?}"));
    match done {
        hd_offer::Demand::AlreadyThere => {
            info_box(&format!("exe 旁已經有 v0.3.9 的 HD 美術包，不用再下載：\n{}", pack.unwrap_or_default().display()));
        }
        hd_offer::Demand::Declined => info_box("沒有下載 HD 美術包。"),
        hd_offer::Demand::Cancelled => info_box("已取消下載，沒有留下任何檔。"),
        hd_offer::Demand::Downloaded(placed) => {
            let kept = placed.backup.map_or_else(String::new, |b| {
                format!("\n\n原本的 hd-pack.zip 改名保留為：\n{}\n（啟動器不會自動刪除；不需要時可以自己刪掉。）", b.display())
            });
            info_box(&format!("HD 美術包下載完成，下次開啟遊戲時就會使用：\n{}{kept}", placed.path.display()));
        }
        hd_offer::Demand::Failed(e) => {
            error_box(&format!("HD 美術包下載失敗。\n\n{e}"));
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

/// `--help`: the arguments, in a message box (a windows-subsystem program has no console) and on stdout.
fn help_text() -> String {
    format!(
        "{TITLE} {RELEASE_NAME}\n\n\
         不加參數：開始遊戲。\n\
         --hd-pack：下載 HD 美術包（上游 v0.3.9 的 hd-pack.zip），不開始遊戲。先前選過「這次不要」或「不再詢問」也可以用；\
         exe 旁已經有就不會再下載。\n\
         --selftest：檢查內嵌的檔案並列出路徑，不開視窗、不開始遊戲（結果在 %LOCALAPPDATA%\\dbz-ssd-zhtw\\selftest.txt）。\n\
         --help：顯示這段說明。"
    )
}

/// `--selftest`: unpack, check every file's SHA-256, print the paths; no dialog, no game. Also
/// written to `%LOCALAPPDATA%\dbz-ssd-zhtw\selftest.txt` (a windows-subsystem program may have no
/// console to print to).
fn selftest() -> ExitCode {
    let mut report = String::new();
    let mut ok = true;
    report.push_str(&format!("launcher {} bundle {BUNDLE_ID} ips_embedded={HAS_IPS}\n", env!("CARGO_PKG_VERSION")));
    match unpack() {
        Ok(app) => {
            report.push_str(&format!("app_dir {}\n", app.display()));
            for a in ASSETS {
                let path = app.join(a.path);
                let got = std::fs::read(&path).map(|b| hex(&Sha256::digest(&b)));
                let good = got.as_deref().is_ok_and(|s| s == a.sha256);
                ok &= good;
                report.push_str(&format!(
                    "{} {:>9} {} {}\n",
                    if good { "OK  " } else { "FAIL" },
                    a.len,
                    &a.sha256[..16],
                    a.path
                ));
            }
        }
        Err(e) => {
            ok = false;
            report.push_str(&format!("unpack FAILED: {e}\n"));
        }
    }
    match own_overlay() {
        Ok(segments) if segments.is_empty() => report.push_str("overlay none\n"),
        Ok(segments) => {
            report.push_str(&format!("overlay yes ({} segments)\n", segments.len()));
            for seg in &segments {
                report.push_str(&format!(
                    "overlay {} {} bytes sha256 {}\n",
                    seg.name,
                    seg.bytes.len(),
                    seg.sha256
                ));
            }
        }
        Err(e) => {
            ok = false;
            report.push_str(&format!("overlay FAILED: {e}\n"));
        }
    }
    match prepare_data_dir(data_dir(), &upstream_data_dir()) {
        Ok(lines) => {
            lines.iter().for_each(|l| report.push_str(&format!("{l}\n")));
            // Dry run: nothing is removed by a selftest.
            let apps = launcher_root().join("app");
            clean_up(data_dir(), &upstream_data_dir(), &apps, BUNDLE_ID, false, &remove_by_rename)
                .iter()
                .for_each(|l| report.push_str(&format!("{l}\n")));
        }
        Err(e) => {
            ok = false;
            report.push_str(&format!("migration FAILED: {e}\n"));
        }
    }
    let data = game_data_dir();
    report.push_str(&format!("game_data_dir {}\n", data.display()));
    report.push_str(&format!("cached_rom {} exists={}\n", cached_rom().display(), cached_rom().is_file()));
    match sync_data_files() {
        Ok(lines) => lines.iter().for_each(|l| report.push_str(&format!("{l}\n"))),
        Err(e) => {
            ok = false;
            report.push_str(&format!("sync FAILED: {e}\n"));
        }
    }
    match set_up_art_override(false) {
        Ok(lines) => lines.iter().for_each(|l| report.push_str(&format!("{l}\n"))),
        Err(e) => {
            ok = false;
            report.push_str(&format!("art override FAILED: {e}\n"));
        }
    }
    let bundled = art_override_pictures(ASSETS.iter().map(|a| (a.path, a.bytes)));
    report.push_str(&format!("{}\n", art_override_folders(&bundled)));
    match set_up_voice_folder(false) {
        Ok(lines) => lines.iter().for_each(|l| report.push_str(&format!("{l}\n"))),
        Err(e) => {
            ok = false;
            report.push_str(&format!("voice folder FAILED: {e}\n"));
        }
    }
    match bundled_voice() {
        Ok(len) => report.push_str(&format!("voice bundled {VOICE_ASSET} {len} bytes\n")),
        Err(e) => {
            ok = false;
            report.push_str(&format!("voice bundled FAILED: {e}\n"));
        }
    }
    report.push_str(&format!(
        "hd_pack_beside_launcher {}\n",
        hd_pack_beside_launcher().map_or_else(|| "none".to_owned(), |p| p.display().to_string())
    ));
    report.push_str(&format!(
        "hd_pack_version {}\n",
        hd_pack_check(hd_pack_beside_launcher().as_deref()).map_or_else(|| "ok or none".to_owned(), |(line, _)| line)
    ));
    // What the update check would do: said only, nothing connects in a selftest
    report.push_str(&format!(
        "release {RELEASE_NAME} ({RELEASE}) update_source {} check_updates {}\n",
        update::source().map_or_else(|| "none".to_owned(), |s| s.manifest_url),
        update::read_settings(&launcher_root().join(update::SETTINGS_FILE)).check_updates
    ));
    report.push_str(&format!("result {}\n", if ok { "PASS" } else { "FAIL" }));
    let _ = std::io::stdout().write_all(report.as_bytes());
    let _ = std::fs::create_dir_all(launcher_root());
    let _ = std::fs::write(launcher_root().join("selftest.txt"), &report);
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn main() -> ExitCode {
    if std::env::args().skip(1).any(|a| a == "--selftest") {
        return selftest();
    }
    if std::env::args().skip(1).any(|a| a == "--help" || a == "-h" || a == "/?") {
        let text = help_text();
        let _ = std::io::stdout().write_all(text.as_bytes());
        info_box(&text);
        return ExitCode::SUCCESS;
    }
    if std::env::args().skip(1).any(|a| a == "--hd-pack") {
        return hd_pack_on_demand();
    }
    // In the background while the files are unpacked; nothing at all without a source (see update.rs)
    let update_check = update::start(update::source(), &launcher_root(), std::time::SystemTime::now(), || {
        Box::new(update::Http::check())
    });
    let app = match unpack() {
        Ok(app) => app,
        Err(e) => {
            error_box(&e);
            return ExitCode::FAILURE;
        }
    };
    let mut notes = match prepare_data_dir(data_dir(), &upstream_data_dir()) {
        Ok(notes) => notes,
        Err(e) => {
            error_box(&e);
            return ExitCode::FAILURE;
        }
    };
    notes.extend(clean_up(data_dir(), &upstream_data_dir(), &launcher_root().join("app"), BUNDLE_ID, true, &remove_by_rename));
    // Written now, not when the game starts: a run that stops at the ROM dialog keeps its lines.
    let _ = std::fs::create_dir_all(launcher_root());
    let _ = std::fs::write(launcher_root().join("last-run.log"), notes.iter().map(|l| format!("{l}\n")).collect::<String>());
    match sync_data_files() {
        Ok(lines) => notes.extend(lines),
        Err(e) => {
            notes.push(format!("sync FAILED: {e}"));
            let _ = std::fs::write(launcher_root().join("last-run.log"), notes.iter().map(|l| format!("{l}\n")).collect::<String>());
            error_box(&e);
            return ExitCode::FAILURE;
        }
    }
    // The pictures the game draws in place of the HD pack's (Super Vegeta without his shoulder pads): not worth stopping for
    match set_up_art_override(true) {
        Ok(lines) => notes.extend(lines),
        Err(e) => notes.push(format!("art override FAILED: {e}")),
    }
    // The folder of the transformation's voice and the bundle's WAV in it: not worth stopping for either
    match set_up_voice_folder(true) {
        Ok(lines) => notes.extend(lines),
        Err(e) => notes.push(format!("voice folder FAILED: {e}")),
    }
    // The sync's lines too (which table kept the player's #font, which was replaced)
    let _ = std::fs::write(launcher_root().join("last-run.log"), notes.iter().map(|l| format!("{l}\n")).collect::<String>());
    let marker = launcher_root().join(CREDITS_MARKER);
    if !marker.exists() {
        info_box(CREDITS);
        let _ = std::fs::write(&marker, BUNDLE_ID);
    }
    if cached_rom().is_file() && data_dir().fallback {
        // `save\` beside the exe could not be used this time: the player is told where the data went
        info_box(&folders_text(data_dir()));
    }
    if cached_rom().is_file() {
        // The ROM is cached: the installer's copies are not read, only removed if any are left
        if let Some(setup) = preset_setup_dir() {
            let mut lines = Vec::new();
            preset::run(true, &setup, None, &mut LivePreset { app: &app }, &mut lines);
            lines.iter().for_each(|l| log_line(l));
        }
    }
    if !cached_rom().is_file() {
        match set_up_rom(&app) {
            Ok(true) => {}
            Ok(false) => return ExitCode::SUCCESS,
            Err(e) => {
                error_box(&e);
                return ExitCode::FAILURE;
            }
        }
    }
    if update_before_play(update_check) {
        return ExitCode::SUCCESS;
    }
    hd_pack_before_play();
    if let Err(e) = play(&app) {
        error_box(&e);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_names_every_argument() {
        let text = help_text();
        for arg in ["--hd-pack", "--selftest", "--help"] {
            assert!(text.contains(arg), "{arg}: {text}");
        }
        assert!(text.contains("不再詢問") && text.contains("不開始遊戲"), "{text}");
        assert!(text.starts_with(&format!("{TITLE} {RELEASE_NAME}\n")), "the title names the release as players see it: {text}");
    }

    /// A fresh empty folder under the system's temp folder, removed afterwards.
    fn scratch(name: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("dbz-launcher-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn the_voice_folder_gets_its_note_once_and_the_voice_is_never_written() {
        let dir = scratch("voice");
        let voice = dir.0.join(VOICE);
        let dry = install_voice_folder(&voice, None, false).unwrap();
        assert!(dry[0].contains("README.txt would be written, ssj-transform.wav absent"), "{dry:?}");
        assert!(!voice.exists(), "a dry run writes nothing");
        let first = install_voice_folder(&voice, None, true).unwrap();
        assert!(first[0].contains("README.txt written"), "{first:?}");
        assert_eq!(std::fs::read(voice.join(VOICE_README_FILE)).unwrap(), VOICE_README.as_bytes());
        let again = install_voice_folder(&voice, None, true).unwrap();
        assert!(again[0].contains("README.txt same, ssj-transform.wav absent"), "twice is the same: {again:?}");
        put(&voice.join(VOICE_SSJ_CLIP), b"RIFF....WAVE");
        std::fs::write(voice.join(VOICE_README_FILE), b"old").unwrap();
        let later = install_voice_folder(&voice, None, true).unwrap();
        assert!(later[0].contains("README.txt updated, ssj-transform.wav present"), "{later:?}");
        assert_eq!(std::fs::read(voice.join(VOICE_SSJ_CLIP)).unwrap(), b"RIFF....WAVE", "the player's voice untouched");
        assert!(VOICE_README.contains("ssj-transform.wav"), "the note names the file");
    }

    #[test]
    fn only_a_private_copy_runs_the_game_with_its_debug_tools() {
        let segment = |name: &str| overlay::Segment {
            name: name.to_owned(),
            bytes: vec![1, 2, 3],
            sha256: String::new(),
        };
        assert_eq!(debug_args(&Ok(vec![segment("rom"), segment("ips")])), ["--debug"], "private copy");
        assert_eq!(debug_args(&Ok(vec![segment("ips")])), ["--debug"], "an overlay of the IPS only");
        assert!(debug_args(&Ok(Vec::new())).is_empty(), "public copy: no overlay");
        assert!(debug_args(&Err("overlay: 長度表損壞".to_owned())).is_empty(), "a damaged overlay");
        // The test binary has no overlay: like the public launcher, it asks for no debug tools.
        assert!(debug_args(&own_overlay()).is_empty());
        // The game is started with them: `--debug` for the private copy only.
        let public = ["--mode", "enhanced", "--hd-scale", "4"];
        assert_eq!(play_args(&Ok(Vec::new())), public, "public copy");
        assert_eq!(play_args(&Err("overlay: 長度表損壞".to_owned())), public, "a damaged overlay");
        assert_eq!(play_args(&Ok(vec![segment("rom")])), [&public[..], &["--debug"]].concat(), "private copy");
    }

    #[test]
    fn every_build_bundles_the_voice() {
        assert!(bundled_voice().is_ok_and(|len| len > 0), "build.rs requires voice/ssj-transform.wav");
    }

    #[test]
    fn the_bundles_voice_is_installed_once_and_the_players_own_is_kept() {
        let dir = scratch("voice-bundled");
        let voice = dir.0.join(VOICE);
        let dry = install_voice_folder(&voice, Some(b"RIFF bundled"), false).unwrap();
        assert!(dry[0].contains("ssj-transform.wav would be installed from the bundle"), "{dry:?}");
        assert!(!voice.exists(), "a dry run writes nothing");
        let first = install_voice_folder(&voice, Some(b"RIFF bundled"), true).unwrap();
        assert!(first[0].contains("README.txt written, ssj-transform.wav installed from the bundle"), "{first:?}");
        assert_eq!(std::fs::read(voice.join(VOICE_SSJ_CLIP)).unwrap(), b"RIFF bundled");
        let again = install_voice_folder(&voice, Some(b"RIFF bundled"), true).unwrap();
        assert!(again[0].contains("README.txt same, ssj-transform.wav same as the bundle's"), "{again:?}");
        std::fs::write(voice.join(VOICE_SSJ_CLIP), b"RIFF mine").unwrap();
        let later = install_voice_folder(&voice, Some(b"RIFF bundled"), true).unwrap();
        assert!(later[0].contains("ssj-transform.wav kept (the player's own)"), "{later:?}");
        assert_eq!(std::fs::read(voice.join(VOICE_SSJ_CLIP)).unwrap(), b"RIFF mine", "the player's voice untouched");
    }

    #[test]
    fn the_launchers_credits_name_no_one_language() {
        // Shown before the player chooses Taiwan or Hong Kong, by one launcher for both: 繁體中文化, no Taiwan-only wording
        assert_eq!(
            CREDITS,
            "DBZ 超賽亞傳說 繁中版　繁體中文化 By Ming Liu\n\n本程式基於 JackRushante/DBZ-Super-Saiya-Densetsu-Rust（GPL-3）"
        );
        // the exe's version information (Properties > Details), set by build.rs on Windows only
        let build = include_str!("../build.rs");
        assert!(build.contains(r#".set("FileDescription", "繁體中文化 By Ming Liu")"#), "build.rs FileDescription");
        assert!(!build.contains("台式") && !CREDITS.contains("台式"));
    }

    #[test]
    fn a_play_no_longer_names_a_language() {
        // The game asks a new player (its first page) and keeps the choice; a `--lang` would override it every run.
        for overlay in [Ok(Vec::new()), Err("overlay: 長度表損壞".to_owned())] {
            let args = play_args(&overlay);
            assert!(!args.contains(&"--lang"), "{args:?}");
            assert!(!args.iter().any(|a| a.starts_with("zh-")), "{args:?}");
        }
    }

    #[test]
    fn both_languages_are_synced_with_their_menus() {
        for rel in ["zh-tw.tsv", "ui-zh-tw.tsv", "zh-hk.tsv", "ui-zh-hk.tsv"] {
            assert!(SYNCED.contains(&rel), "{rel} is written into the data folder");
            assert!(TABLES.contains(&rel), "{rel} keeps a player's #font");
            assert!(ASSETS.iter().any(|a| a.path == rel), "{rel} is bundled (build.rs REQUIRED)");
        }
        assert_eq!(bundled_synced().unwrap().len(), SYNCED.len(), "every synced file is bundled");
    }

    /// The embedded bytes of `rel`.
    fn bundled(rel: &str) -> &'static [u8] {
        ASSETS.iter().find(|a| a.path == rel).unwrap().bytes
    }

    #[test]
    fn an_early_launchers_hong_kong_table_is_replaced() {
        // The early launchers wrote the Taiwan tables under the code zh-hk: now the bundled Hong Kong tables take their
        // place, the old ones kept as .bak
        let dir = scratch("early-hk");
        let data = dir.0.clone();
        put(&data.join("zh-hk.tsv"), b"#name\tthe early launcher's Taiwan table\n");
        put(&data.join("ui-zh-hk.tsv"), b"#name\tthe early launcher's Taiwan menus\n");
        let lines = settle_and_sync(&data, &bundled_synced().unwrap()).unwrap();
        for (rel, old) in [
            ("zh-hk.tsv", &b"#name\tthe early launcher's Taiwan table\n"[..]),
            ("ui-zh-hk.tsv", &b"#name\tthe early launcher's Taiwan menus\n"[..]),
        ] {
            assert_eq!(std::fs::read(data.join(rel)).unwrap(), bundled(rel), "{rel}");
            assert_eq!(std::fs::read(data.join(format!("{rel}.bak"))).unwrap(), old, "{rel}");
            let line = format!("sync replaced (old kept as .bak): {}", data.join(rel).display());
            assert!(lines.contains(&line), "{lines:?}");
        }
        assert_eq!(std::fs::read(data.join("zh-tw.tsv")).unwrap(), bundled("zh-tw.tsv"));
        assert_ne!(bundled("zh-hk.tsv"), bundled("zh-tw.tsv"), "two languages, two tables");
    }

    const SETTINGS_PATH: &str = "saves/ssd.settings";

    /// The data folder of a player of an earlier launcher: its sync note is there.
    fn earlier_player(name: &str) -> TempDir {
        let dir = scratch(name);
        put(&dir.0.join(SYNC_MARKER), b"\"zh-tw.tsv\" = \"00\"\n");
        dir
    }

    fn read_settings(data: &Path) -> Option<String> {
        std::fs::read_to_string(data.join(SETTINGS_PATH)).ok()
    }

    #[test]
    fn an_old_player_with_english_stored_stays_taiwan() {
        // The launcher's --lang zh-tw hid what the file held: English, never seen
        let dir = earlier_player("settle-en");
        put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=en\n");
        let lines = settle_language(&dir.0);
        assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\naccess.language=zh-tw\n", "{lines:?}");
        assert!(dir.0.join(LANGUAGE_MARKER).is_file(), "{lines:?}");
        assert!(!dir.0.join("saves/ssd.settings.tmp").exists(), "no partial file left");
    }

    #[test]
    fn an_old_player_with_the_first_versions_index_stays_taiwan() {
        // a player known by the saves alone (no sync note): the index 1 of the first versions reads as zh-hk in the game
        let dir = scratch("settle-index");
        put(&dir.0.join("saves/ssd.sav"), b"battery");
        put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=1\n");
        let lines = settle_language(&dir.0);
        assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\naccess.language=zh-tw\n", "{lines:?}");
        assert!(dir.0.join(LANGUAGE_MARKER).is_file());
    }

    #[test]
    fn an_old_player_with_nothing_stored_stays_taiwan() {
        let dir = earlier_player("settle-none");
        put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\nvideo.scale=4\n");
        settle_language(&dir.0);
        assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\nvideo.scale=4\naccess.language=zh-tw\n");
        // no settings file at all: one with the language alone, as the game writes it
        let bare = earlier_player("settle-nofile");
        let lines = settle_language(&bare.0);
        assert_eq!(read_settings(&bare.0).unwrap(), "ssd-settings 1\naccess.language=zh-tw\n", "{lines:?}");
        assert!(bare.0.join(LANGUAGE_MARKER).is_file());
    }

    #[test]
    fn a_new_install_is_left_to_the_games_page() {
        for (name, empty_saves) in [("settle-new", false), ("settle-new-empty-saves", true)] {
            let dir = scratch(name);
            if empty_saves {
                std::fs::create_dir_all(dir.0.join("saves")).unwrap();
            }
            let lines = settle_language(&dir.0);
            assert!(read_settings(&dir.0).is_none(), "{name}: the game asks: {lines:?}");
            assert!(dir.0.join(LANGUAGE_MARKER).is_file(), "{name}: settled once: {lines:?}");
            // the game's page wrote the player's choice: never changed
            put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=zh-hk\n");
            settle_language(&dir.0);
            assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\naccess.language=zh-hk\n", "{name}");
        }
    }

    #[test]
    fn the_settling_runs_once() {
        let dir = earlier_player("settle-once");
        put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=en\n");
        settle_language(&dir.0);
        assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\naccess.language=zh-tw\n");
        for chosen in ["zh-hk", "en"] {
            let text = format!("ssd-settings 1\naccess.language={chosen}\n");
            put(&dir.0.join(SETTINGS_PATH), text.as_bytes());
            settle_language(&dir.0);
            assert_eq!(read_settings(&dir.0).unwrap(), text, "the player's {chosen} kept");
        }
        // an earlier player already in zh-tw: the file is not written, the marker is
        let tw = earlier_player("settle-tw");
        put(&tw.0.join(SETTINGS_PATH), b"ssd-settings 1\r\naccess.language=zh-tw\r\n");
        settle_language(&tw.0);
        assert_eq!(read_settings(&tw.0).unwrap(), "ssd-settings 1\r\naccess.language=zh-tw\r\n", "left as it was");
        assert!(tw.0.join(LANGUAGE_MARKER).is_file());
    }

    #[test]
    fn an_old_player_with_hong_kong_stored_stays_taiwan() {
        // Before this release there was no Hong Kong table: an earlier player's zh-hk is the early launchers' Taiwan table
        // under that code, or the first versions' index 1 turned into it, and was played in Taiwan Chinese
        let dir = earlier_player("settle-hk");
        put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=zh-hk\n");
        settle_language(&dir.0);
        assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\naccess.language=zh-tw\n");
        assert!(dir.0.join(LANGUAGE_MARKER).is_file());
        // settled: Hong Kong chosen afterwards is kept
        put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=zh-hk\n");
        settle_language(&dir.0);
        assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\naccess.language=zh-hk\n");
    }

    #[test]
    fn other_settings_are_kept() {
        let dir = earlier_player("settle-others");
        let before = "ssd-settings 1\r\nvideo.scale=4\r\naudio.music = 7\n access.language = en \r\nx.unknown=word\nnonsense";
        put(&dir.0.join(SETTINGS_PATH), before.as_bytes());
        settle_language(&dir.0);
        let after = "ssd-settings 1\r\nvideo.scale=4\r\naudio.music = 7\naccess.language=zh-tw\r\nx.unknown=word\nnonsense";
        assert_eq!(read_settings(&dir.0).unwrap(), after);
        // without the key: added after the last line, the others as they were
        let none = earlier_player("settle-others-none");
        put(&none.0.join(SETTINGS_PATH), b"ssd-settings 1\r\nvideo.scale=4");
        settle_language(&none.0);
        assert_eq!(read_settings(&none.0).unwrap(), "ssd-settings 1\r\nvideo.scale=4\naccess.language=zh-tw\n");
    }

    #[test]
    fn a_damaged_settings_file_is_left_to_the_game_and_the_players_choice_kept() {
        // A PC file with a wrong header the game moves aside (.bad) and asks the language on its page; one that is not text
        // it cannot read, and plays with the defaults. Either way the launcher leaves the file to the game: what the player
        // picks later is a choice, never an old value to settle
        for (name, damaged) in [
            ("settle-damaged", &b"not a settings file\naccess.language=en\n"[..]),
            ("settle-not-text", &b"ssd-settings 1\naccess.language=\xff\xfe\n"[..]),
        ] {
            let dir = earlier_player(name);
            put(&dir.0.join(SETTINGS_PATH), damaged);
            let lines = settle_language(&dir.0);
            assert!(lines.iter().any(|l| l.contains("left to the game")), "{name}: {lines:?}");
            assert!(
                lines.iter().any(|l| l.contains("a wrong header the game moves aside") && l.contains("not text it cannot read")),
                "{name}: the log says what the game does with each: {lines:?}"
            );
            assert!(!lines.iter().any(|l| l.contains("keeps it aside as .bad and asks")), "{name}: {lines:?}");
            assert_eq!(std::fs::read(dir.0.join(SETTINGS_PATH)).unwrap(), damaged, "{name}: left as it is");
            assert!(dir.0.join(LANGUAGE_MARKER).is_file(), "{name}: settled: {lines:?}");
            // the file is gone or replaced later (here: by hand), and the player picks Hong Kong
            std::fs::remove_file(dir.0.join(SETTINGS_PATH)).unwrap();
            put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=zh-hk\n");
            settle_language(&dir.0);
            assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\naccess.language=zh-hk\n", "{name}");
        }
    }

    #[test]
    fn a_settings_file_that_cannot_be_read_or_written_is_tried_again() {
        // a folder in place of the file: reading it fails (not a missing file), as writing over a folder in place of the
        // partial file does; the old value still holds, so the next run tries again
        for (name, blocked) in [("settle-unreadable", SETTINGS_PATH), ("settle-unwritable", "saves/ssd.settings.tmp")] {
            let dir = earlier_player(name);
            if blocked != SETTINGS_PATH {
                put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=en\n");
            }
            std::fs::create_dir_all(dir.0.join(blocked)).unwrap();
            let lines = settle_language(&dir.0);
            assert!(lines.iter().any(|l| l.contains("FAILED")), "{name}: {lines:?}");
            assert!(!dir.0.join(LANGUAGE_MARKER).exists(), "{name}: tried again next run");
            std::fs::remove_dir(dir.0.join(blocked)).unwrap();
            put(&dir.0.join(SETTINGS_PATH), b"ssd-settings 1\naccess.language=en\n");
            settle_language(&dir.0);
            assert_eq!(read_settings(&dir.0).unwrap(), "ssd-settings 1\naccess.language=zh-tw\n", "{name}");
            assert!(dir.0.join(LANGUAGE_MARKER).is_file(), "{name}");
        }
    }

    #[test]
    fn the_language_is_settled_before_the_tables_are_synced() {
        // The sync writes its note: settled after it, a new install would be taken for an earlier player
        let new = scratch("settle-order-new");
        settle_and_sync(&new.0, &bundled_synced().unwrap()).unwrap();
        assert!(new.0.join(SYNC_MARKER).is_file(), "the sync ran");
        assert!(read_settings(&new.0).is_none(), "a new install is left to the game's page");
        settle_and_sync(&new.0, &bundled_synced().unwrap()).unwrap();
        assert!(read_settings(&new.0).is_none(), "and so on the next run");
        let old = earlier_player("settle-order-old");
        settle_and_sync(&old.0, &bundled_synced().unwrap()).unwrap();
        assert_eq!(read_settings(&old.0).unwrap(), "ssd-settings 1\naccess.language=zh-tw\n");
    }

    #[test]
    fn the_data_folder_is_save_beside_the_exe_and_is_created() {
        let t = scratch("default");
        let exe_dir = t.0.join("game");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let data = resolve_data_dir(None, Some(&exe_dir), t.0.join("localappdata"));
        assert_eq!(data.path, exe_dir.join("save"));
        assert!(data.beside_exe);
        assert!(data.path.is_dir());
        assert_eq!(std::fs::read_dir(&data.path).unwrap().count(), 0, "the write probe is removed");
    }

    #[test]
    fn a_ssd_data_dir_the_player_set_is_kept() {
        let t = scratch("env");
        let exe_dir = t.0.join("game");
        let mine = t.0.join("mine");
        let data = resolve_data_dir(Some(mine.clone().into_os_string()), Some(&exe_dir), t.0.join("localappdata"));
        assert_eq!(data.path, mine);
        assert!(!data.beside_exe, "no migration into a folder the player chose");
        assert!(!exe_dir.join("save").exists(), "save\\ is not created");
        assert!(data.note.contains("SSD_DATA_DIR"));
    }

    #[test]
    fn a_save_folder_that_cannot_be_written_falls_back_to_upstreams_default() {
        let t = scratch("readonly");
        let exe_dir = t.0.join("game");
        // A file named `save` where the folder would be: it cannot be created.
        put(&exe_dir.join("save"), b"not a folder");
        let fallback = t.0.join("localappdata").join("dbz-ssd-recompiled");
        let data = resolve_data_dir(None, Some(&exe_dir), fallback.clone());
        assert_eq!(data.path, fallback);
        assert!(!data.beside_exe);
        assert!(data.note.contains("fallback"), "{}", data.note);
    }

    #[test]
    fn the_first_run_copies_the_old_saves_states_and_rom_and_leaves_the_old_ones() {
        let t = scratch("migrate");
        let (old, new) = (t.0.join("old"), t.0.join("new"));
        let files: [(&str, &[u8]); 5] = [
            ("saves/ssd.sav", b"battery"),
            ("saves/ssd.bag", b"bag"),
            ("saves/ssd.cheats", b"cheats"),
            ("states/slot1.ssdstate", b"state 1"),
            ("cache/rom/canonical.sfc", b"rom"),
        ];
        for (rel, bytes) in files {
            put(&old.join(rel), bytes);
        }
        std::fs::create_dir_all(&new).unwrap();
        let line = migrate(&old, &new).unwrap();
        assert!(line.starts_with("migrated from"), "{line}");
        for (rel, bytes) in files {
            assert_eq!(std::fs::read(new.join(rel)).unwrap(), bytes, "{rel} copied");
            assert_eq!(std::fs::read(old.join(rel)).unwrap(), bytes, "{rel} left in the old folder");
        }
        assert!(!new.join("saves/ssd.sav.part").exists());
    }

    #[test]
    fn the_launcher_migrates_into_save_beside_the_exe_only() {
        let t = scratch("prepare");
        let old = t.0.join("old");
        put(&old.join("saves/ssd.sav"), b"battery");
        let exe_dir = t.0.join("game");
        let beside = resolve_data_dir(None, Some(&exe_dir), t.0.join("fallback"));
        let lines = prepare_data_dir(&beside, &old).unwrap();
        assert!(lines[1].starts_with("migrated from"), "{lines:?}");
        assert_eq!(std::fs::read(exe_dir.join("save/saves/ssd.sav")).unwrap(), b"battery");
        let mine = t.0.join("mine");
        let chosen = resolve_data_dir(Some(mine.clone().into_os_string()), Some(&exe_dir), t.0.join("fallback"));
        let lines = prepare_data_dir(&chosen, &old).unwrap();
        assert!(lines[1].starts_with("migration skipped"), "{lines:?}");
        assert!(!mine.exists(), "nothing copied into a folder the player chose");
    }

    #[test]
    fn no_migration_when_the_save_folder_already_has_a_battery_save() {
        let t = scratch("has-save");
        let (old, new) = (t.0.join("old"), t.0.join("new"));
        put(&old.join("saves/ssd.sav"), b"old battery");
        put(&old.join("states/slot1.ssdstate"), b"old state");
        put(&new.join("saves/ssd.sav"), b"new battery");
        let line = migrate(&old, &new).unwrap();
        assert!(line.starts_with("migration not needed"), "{line}");
        assert_eq!(std::fs::read(new.join("saves/ssd.sav")).unwrap(), b"new battery");
        assert!(!new.join("states").exists(), "nothing copied");
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn the_crc32_is_upstreams() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn the_folders_text_names_the_folder_the_save_and_the_roms_copy() {
        let dir = scratch("folders");
        let root = dir.0.as_path();
        let data = beside(root);
        let text = folders_text(&data);
        assert!(text.contains(&data.path.display().to_string()));
        assert!(text.contains(&data.path.join("saves").join("ssd.sav").display().to_string()));
        assert!(text.contains("cache\\rom"), "the copy of the ROM to delete before sharing");
        assert!(!text.contains("無法寫入"));
        let fallen = resolve_data_dir(None, None, root.join("fallback"));
        assert!(fallen.fallback && folders_text(&fallen).contains("無法寫入"), "a fallback is said");
        let set = resolve_data_dir(Some(root.join("mine").into_os_string()), None, root.join("fallback"));
        assert!(!set.fallback, "a folder the player set is no fallback");
    }

    #[test]
    fn the_rom_diagnosis_says_header_version_and_ips() {
        let mut jp = vec![0_u8; ROM_SIZE];
        jp[0] = 1;
        // (a made-up image: its CRC is no known ROM's)
        assert!(rom_diagnosis(&jp, true).contains("不是可用的版本"));
        assert!(rom_diagnosis(&[0; 100], true).contains("不是這個遊戲的 ROM"));
        let mut headered = vec![0_u8; 512];
        headered.extend_from_slice(&jp);
        assert!(rom_diagnosis(&headered, true).contains("512 bytes 的檔頭"));
        assert!(!rom_diagnosis(&jp, true).contains("檔頭"));
        // The repo's Japanese Rev 1 ROM, where it is (it is not tracked)
        let rev1 = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rom/dbz_jp_rev1.sfc");
        if let Ok(rom) = std::fs::read(rev1) {
            assert!(rom_diagnosis(&rom, false).contains("需要 dbz102.ips"));
            assert!(rom_diagnosis(&rom, true).contains("會套用 dbz102.ips"));
        }
    }

    /// A data folder `save\` beside an exe in `root\game`.
    fn beside(root: &Path) -> DataDir {
        resolve_data_dir(None, Some(&root.join("game")), root.join("fallback"))
    }

    /// `old` with a battery save, its sidecars, a state with its HD part, an installed data pack and
    /// its unpacked copy, a cached ROM and a translation; `new` with the same save files.
    fn migrated(root: &Path) -> (PathBuf, DataDir) {
        let old = root.join("lad").join("dbz-ssd-recompiled");
        for (rel, bytes) in [
            ("saves/ssd.sav", "battery"),
            ("saves/ssd.bag", "bag"),
            ("saves/ssd.mods", "mods state"),
            ("states/slot1.ssdstate", "state"),
            ("states/slot1.hd", "state hd"),
            ("mods/free-progression.zip", "pack"),
            ("cache/mods/free-progression-1.0.0-4/pack.toml", "unpacked"),
        ] {
            put(&old.join(rel), bytes.as_bytes());
        }
        put(&old.join("cache/rom/canonical.sfc"), b"rom");
        put(&old.join("zh-tw.tsv"), b"tsv");
        let data = beside(root);
        migrate(&old, &data.path).unwrap();
        (old, data)
    }

    #[test]
    fn only_the_other_bundles_are_removed() {
        let t = scratch("bundles");
        let apps = t.0.join("app");
        for name in ["14408ddb1e82", "b4edeb2868ad", "cur", "cur.tmp-77"] {
            put(&apps.join(name).join("ssd-recompiled.exe"), b"exe");
        }
        let lines = clean_bundles(&apps, "cur", true, &remove_by_rename);
        assert_eq!(names(&apps), ["cur", "cur.tmp-77"], "{lines:?}");
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.starts_with("cleanup removed old bundle")), "{lines:?}");
    }

    #[test]
    fn a_bundle_in_use_is_skipped_and_the_others_still_removed() {
        let t = scratch("in-use");
        let apps = t.0.join("app");
        for name in ["aaa", "bbb", "cur"] {
            put(&apps.join(name).join("f"), b"x");
        }
        let in_use = |dir: &Path| -> std::io::Result<()> {
            if dir.ends_with("aaa") {
                return Err(std::io::Error::other("in use"));
            }
            remove_by_rename(dir)
        };
        let lines = clean_bundles(&apps, "cur", true, &in_use);
        assert_eq!(names(&apps), ["aaa", "cur"]);
        assert!(lines[0].starts_with("cleanup skipped old bundle") && lines[0].contains("in use"), "{lines:?}");
        assert!(lines[1].starts_with("cleanup removed old bundle"), "{lines:?}");
    }

    #[test]
    fn a_dry_run_removes_nothing() {
        let t = scratch("dry");
        let (old, data) = migrated(&t.0);
        let apps = t.0.join("app");
        put(&apps.join("old1/f"), b"x");
        put(&apps.join("cur/f"), b"x");
        let leftover = t.0.join("lad").join("dbz-ssd-recompiled.removing");
        put(&leftover.join("x"), b"x");
        let lines = clean_up(&data, &old, &apps, "cur", false, &remove_by_rename);
        assert_eq!(names(&apps), ["cur", "old1"]);
        assert!(leftover.is_dir(), "{lines:?}");
        assert!(old.join("saves/ssd.sav").is_file());
        assert!(lines.iter().any(|l| l.starts_with("cleanup would remove old bundle")), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("cleanup would remove old data folder")), "{lines:?}");
    }

    #[test]
    fn a_migration_brings_the_data_packs_and_the_states_hd_parts() {
        let t = scratch("migrate-mods");
        let (_, data) = migrated(&t.0);
        for (rel, bytes) in [
            ("saves/ssd.mods", "mods state"),
            ("states/slot1.hd", "state hd"),
            ("mods/free-progression.zip", "pack"),
            ("cache/mods/free-progression-1.0.0-4/pack.toml", "unpacked"),
        ] {
            assert_eq!(std::fs::read(data.path.join(rel)).ok().as_deref(), Some(bytes.as_bytes()), "{rel}");
        }
    }

    #[test]
    fn the_old_data_folder_is_kept_while_a_data_pack_is_not_in_the_new_one() {
        let t = scratch("clean-mods");
        let (old, data) = migrated(&t.0);
        std::fs::remove_file(data.path.join("mods/free-progression.zip")).unwrap();
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines.iter().any(|l| l.starts_with("cleanup kept old data folder")), "{lines:?}");
        assert!(old.join("mods/free-progression.zip").is_file(), "the pack is not lost");
        // The unpacked copy is the game's cache: one missing does not keep the old folder.
        put(&data.path.join("mods/free-progression.zip"), b"pack");
        std::fs::remove_dir_all(data.path.join("cache/mods")).unwrap();
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines.iter().any(|l| l.starts_with("cleanup removed old data folder")), "{lines:?}");
    }

    #[test]
    fn the_old_data_folder_is_removed_when_every_save_file_matches() {
        let t = scratch("clean-ok");
        let (old, data) = migrated(&t.0);
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines.iter().any(|l| l.starts_with("cleanup removed old data folder")), "{lines:?}");
        // Only the three sub-folders brought into save\ go: upstream's build uses the same folder
        assert_eq!(names(&old), ["cache", "zh-tw.tsv"], "saves, states and mods go; the rest stays");
        assert_eq!(std::fs::read(old.join("cache/rom/canonical.sfc")).unwrap(), b"rom");
        assert_eq!(names(&t.0.join("lad")), ["dbz-ssd-recompiled"], "no .removing left");
        assert_eq!(std::fs::read(data.path.join("saves/ssd.sav")).unwrap(), b"battery");
        // The next run finds nothing more to remove, and removes nothing else
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines.iter().any(|l| l.starts_with("cleanup old data folder none") && l.contains("no saves")), "{lines:?}");
        assert_eq!(names(&old), ["cache", "zh-tw.tsv"]);
    }

    #[test]
    fn a_sub_folder_left_half_removed_is_removed_next_time_and_nothing_else() {
        let t = scratch("leftover-sub");
        let (old, data) = migrated(&t.0);
        put(&old.join("saves.removing/x"), b"x");
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines[0].starts_with("cleanup removed leftover"), "{lines:?}");
        assert_eq!(names(&old), ["cache", "zh-tw.tsv"], "{lines:?}");
    }

    #[test]
    fn a_save_file_that_differs_keeps_the_whole_old_folder() {
        let t = scratch("clean-differs");
        let (old, data) = migrated(&t.0);
        put(&data.path.join("states/slot1.ssdstate"), b"played on");
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines.iter().any(|l| l.starts_with("cleanup kept old data folder") && l.contains("slot1.ssdstate")), "{lines:?}");
        assert!(old.join("saves/ssd.sav").is_file() && old.join("zh-tw.tsv").is_file());
    }

    #[test]
    fn a_save_file_missing_from_save_keeps_the_old_folder() {
        let t = scratch("clean-missing");
        let (old, data) = migrated(&t.0);
        std::fs::remove_file(data.path.join("saves/ssd.bag")).unwrap();
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines.iter().any(|l| l.contains("ssd.bag") && l.contains("is not in")), "{lines:?}");
        assert!(old.join("saves/ssd.bag").is_file());
    }

    #[test]
    fn no_battery_save_in_save_keeps_the_old_folder() {
        let t = scratch("clean-no-sav");
        let old = t.0.join("lad").join("dbz-ssd-recompiled");
        put(&old.join("cache/rom/canonical.sfc"), b"rom");
        let data = beside(&t.0);
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines.iter().any(|l| l.starts_with("cleanup kept old data folder") && l.contains("ssd.sav")), "{lines:?}");
        assert!(old.join("cache/rom/canonical.sfc").is_file());
    }

    #[test]
    fn a_ssd_data_dir_the_player_set_keeps_the_old_folder_but_not_the_old_bundles() {
        let t = scratch("clean-env");
        let (old, _) = migrated(&t.0);
        let apps = t.0.join("app");
        put(&apps.join("old1/f"), b"x");
        put(&apps.join("cur/f"), b"x");
        let mine = t.0.join("game").join("save");
        let data = resolve_data_dir(Some(mine.into_os_string()), Some(&t.0.join("game")), t.0.join("fallback"));
        let lines = clean_up(&data, &old, &apps, "cur", true, &remove_by_rename);
        assert!(old.join("saves/ssd.sav").is_file(), "{lines:?}");
        assert_eq!(names(&apps), ["cur"]);
        assert!(lines.iter().any(|l| l.starts_with("cleanup old data folder left")), "{lines:?}");
    }

    /// A removal that fails half way leaves `<dir>.removing`, never a half folder under the old
    /// name. (A sub-folder without write permission: its file cannot be removed.)
    #[cfg(unix)]
    #[test]
    fn a_removal_failing_half_way_leaves_only_the_removing_folder() {
        use std::os::unix::fs::PermissionsExt as _;
        let t = scratch("half");
        let dir = t.0.join("dbz-ssd-recompiled");
        put(&dir.join("saves/ssd.sav"), b"battery");
        put(&dir.join("locked/f"), b"x");
        let locked = dir.join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = remove_by_rename(&dir);
        let gone = t.0.join("dbz-ssd-recompiled.removing");
        let _ = std::fs::set_permissions(gone.join("locked"), std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        assert!(result.is_err());
        assert!(!dir.exists(), "no half folder under the old name");
        assert!(gone.is_dir());
        remove_by_rename(&gone).unwrap();
        assert!(!gone.exists());
    }

    #[test]
    fn a_folder_left_half_removed_is_removed_next_time() {
        let t = scratch("leftover");
        let (old, data) = migrated(&t.0);
        let leftover = t.0.join("lad").join("dbz-ssd-recompiled.removing");
        put(&leftover.join("cache/x"), b"x");
        let lines = clean_old_data_dir(&old, &data.path, true, &remove_by_rename);
        assert!(lines[0].starts_with("cleanup removed leftover"), "{lines:?}");
        assert_eq!(names(&t.0.join("lad")), ["dbz-ssd-recompiled"], "the leftover goes; the old folder's rest stays");
    }

    #[test]
    fn no_migration_without_old_saves() {
        let t = scratch("no-old");
        let (old, new) = (t.0.join("old"), t.0.join("new"));
        std::fs::create_dir_all(&new).unwrap();
        let line = migrate(&old, &new).unwrap();
        assert!(line.starts_with("migration none"), "{line}");
        assert_eq!(std::fs::read_dir(&new).unwrap().count(), 0, "the save folder is left empty");
        assert!(!old.exists(), "the old folder is not created");
    }

    #[test]
    fn a_migration_does_not_overwrite_a_file_already_in_the_save_folder() {
        let t = scratch("keep");
        let (old, new) = (t.0.join("old"), t.0.join("new"));
        put(&old.join("saves/ssd.sav"), b"old battery");
        put(&old.join("states/slot1.ssdstate"), b"old state");
        put(&new.join("states/slot1.ssdstate"), b"new state");
        let line = migrate(&old, &new).unwrap();
        assert!(line.contains("1 already there kept"), "{line}");
        assert_eq!(std::fs::read(new.join("states/slot1.ssdstate")).unwrap(), b"new state");
        assert_eq!(std::fs::read(new.join("saves/ssd.sav")).unwrap(), b"old battery");
    }

    /// A zip of `entries` (name, bytes, deflated), as a pack's `hd-pack.zip` is laid out.
    fn zip_of(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let (mut out, mut cd) = (Vec::new(), Vec::new());
        for &(name, bytes, deflate) in entries {
            let data = if deflate { miniz_oxide::deflate::compress_to_vec(bytes, 6) } else { bytes.to_vec() };
            let method: u16 = if deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            let header = |sig: &[u8], out: &mut Vec<u8>| {
                out.extend_from_slice(sig);
                if sig[2] == 1 {
                    out.extend_from_slice(&20_u16.to_le_bytes());
                }
                out.extend_from_slice(&20_u16.to_le_bytes());
                out.extend_from_slice(&0_u16.to_le_bytes());
                out.extend_from_slice(&method.to_le_bytes());
                out.extend_from_slice(&[0; 8]); // time, date, crc (not read)
                out.extend_from_slice(&(data.len() as u32).to_le_bytes());
                out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                out.extend_from_slice(&(name.len() as u16).to_le_bytes());
                out.extend_from_slice(&0_u16.to_le_bytes());
            };
            header(&[0x50, 0x4B, 0x03, 0x04], &mut out);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&data);
            header(&[0x50, 0x4B, 0x01, 0x02], &mut cd);
            cd.extend_from_slice(&[0; 10]); // comment length, disk, internal and external attributes
            cd.extend_from_slice(&offset.to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06, 0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes());
        out
    }

    const NEW_PACK: &str = "format = 1\nname = \"Dragon Ball Z: Super Saiya Densetsu HD art pack\"\nrevision = \"2\"\nrom_crc32 = \"A8CF4F2A\"\ndigest = \"0EF46E0B\"\n\n[counts]\n\"pose variants\" = 27\n";

    #[test]
    fn the_v039_hd_pack_is_said_nothing_of() {
        let t = scratch("hd-pack-new");
        let zip = t.0.join("hd-pack.zip");
        put(&zip, &zip_of(&[("LICENSE-ART.md", b"art", true), ("hd-pack.toml", NEW_PACK.as_bytes(), true)]));
        assert_eq!(hd_pack_check(Some(&zip)), None);
        let stored = t.0.join("stored.zip");
        put(&stored, &zip_of(&[("hd-pack.toml", NEW_PACK.as_bytes(), false)]));
        assert_eq!(hd_pack_check(Some(&stored)), None, "a stored entry");
        put(&t.0.join("hd-pack").join("hd-pack.toml"), NEW_PACK.as_bytes());
        assert_eq!(hd_pack_check(Some(&t.0.join("hd-pack"))), None, "an unpacked pack folder");
    }

    #[test]
    fn an_older_hd_pack_is_said_and_the_game_still_starts() {
        let t = scratch("hd-pack-old");
        let zip = t.0.join("hd-pack.zip");
        let old = NEW_PACK.replace("0EF46E0B", "5A1B2C3D").replace("revision = \"2\"", "revision = \"1\"");
        put(&zip, &zip_of(&[("hd-pack.toml", old.as_bytes(), true)]));
        let (line, digest) = hd_pack_check(Some(&zip)).unwrap();
        assert!(line.starts_with(HD_PACK_OLD) && line.contains("\"5A1B2C3D\"") && line.contains("484121119"), "{line}");
        assert_eq!(digest, "5A1B2C3D");
        // The same digest with another revision is not the v0.3.9 pack either
        let other = NEW_PACK.replace("revision = \"2\"", "revision = \"3\"");
        put(&zip, &zip_of(&[("hd-pack.toml", other.as_bytes(), true)]));
        assert!(hd_pack_check(Some(&zip)).is_some());
        // A zip without a manifest, or not a zip
        put(&zip, &zip_of(&[("readme.txt", b"x", false)]));
        assert!(hd_pack_check(Some(&zip)).unwrap().0.contains("no hd-pack.toml"));
        put(&zip, b"not a zip at all");
        assert!(hd_pack_check(Some(&zip)).unwrap().0.contains("cannot be read"));
    }

    /// The real pack: `DBZ_HD_PACK=<hd-pack.zip of v0.3.9> cargo test -- --ignored`.
    #[test]
    #[ignore = "needs upstream v0.3.9's hd-pack.zip (DBZ_HD_PACK)"]
    fn the_real_v039_pack_reads_as_the_expected_one() {
        let pack = PathBuf::from(std::env::var_os("DBZ_HD_PACK").expect("DBZ_HD_PACK"));
        assert_eq!(std::fs::metadata(&pack).unwrap().len(), HD_PACK_ZIP_BYTES);
        assert_eq!(hd_pack_check(Some(&pack)), None);
    }

    #[test]
    fn no_hd_pack_is_said_nothing_of() {
        assert_eq!(hd_pack_check(None), None);
    }

    const BUNDLED: &str = "#name\t繁體中文（台灣）\n#font\tLXGWWenKaiTC\nkey\tzh\tnote\nhelp:0\t能讓一個人的ＨＰ\tk0:0\n";

    #[test]
    fn a_table_that_differs_in_its_font_only_keeps_the_players_font_and_no_bak() {
        let t = scratch("sync-font");
        let theirs = BUNDLED.replace("#font\tLXGWWenKaiTC", "#font\tNotoSansTC");
        put(&t.0.join("zh-tw.tsv"), theirs.as_bytes());
        // Saved again by Notepad: CRLF and a byte order mark, still the same table
        let crlf = format!("\u{feff}{}", BUNDLED.replace("#font\tLXGWWenKaiTC", "#font\tNotoSansTC").replace('\n', "\r\n"));
        put(&t.0.join("ui-zh-tw.tsv"), crlf.as_bytes());
        let lines = sync_into(&t.0, &[("zh-tw.tsv", BUNDLED.as_bytes()), ("ui-zh-tw.tsv", BUNDLED.as_bytes())], &[]).unwrap();
        assert!(lines.iter().all(|l| l.starts_with("sync kept-font: ")), "{lines:?}");
        assert_eq!(std::fs::read_to_string(t.0.join("zh-tw.tsv")).unwrap(), theirs, "left as it is");
        assert_eq!(std::fs::read_to_string(t.0.join("ui-zh-tw.tsv")).unwrap(), theirs, "the bundled table, the player's #font");
        assert!(!t.0.join("zh-tw.tsv.bak").exists() && !t.0.join("ui-zh-tw.tsv.bak").exists(), "no .bak");
        // Run again: nothing to do but say it
        let again = sync_into(&t.0, &[("zh-tw.tsv", BUNDLED.as_bytes())], &[]).unwrap();
        assert_eq!(again, [format!("sync kept-font: {}", t.0.join("zh-tw.tsv").display())]);
    }

    #[test]
    fn a_table_with_another_row_changed_is_replaced_and_kept_as_bak() {
        let t = scratch("sync-row");
        let theirs = BUNDLED.replace("#font\tLXGWWenKaiTC", "#font\tNotoSansTC").replace("能讓一個人", "能讓某人");
        put(&t.0.join("zh-tw.tsv"), theirs.as_bytes());
        let lines = sync_into(&t.0, &[("zh-tw.tsv", BUNDLED.as_bytes())], &[]).unwrap();
        assert_eq!(lines, [format!("sync replaced (old kept as .bak): {}", t.0.join("zh-tw.tsv").display())]);
        assert_eq!(std::fs::read_to_string(t.0.join("zh-tw.tsv")).unwrap(), BUNDLED);
        assert_eq!(std::fs::read_to_string(t.0.join("zh-tw.tsv.bak")).unwrap(), theirs);
        // A font file is never merged: other bytes are replaced
        put(&t.0.join("fonts/x.ttf"), b"old font");
        let font = sync_into(&t.0, &[("fonts/x.ttf", b"new font".as_slice())], &[]).unwrap();
        assert!(font[0].starts_with("sync replaced"), "{font:?}");
    }

    #[test]
    fn every_synced_file_has_its_line_for_the_log() {
        let t = scratch("sync-log");
        put(&t.0.join("same.tsv"), b"x");
        let lines = sync_into(&t.0, &[("same.tsv", b"x".as_slice()), ("new.tsv", b"y".as_slice())], &[]).unwrap();
        assert_eq!(
            lines,
            [
                format!("sync unchanged: {}", t.0.join("same.tsv").display()),
                format!("sync written: {}", t.0.join("new.tsv").display()),
            ]
        );
    }

    #[test]
    fn the_font_line_alone_is_what_may_differ() {
        let noto = BUNDLED.replace("LXGWWenKaiTC", "NotoSansTC");
        assert_eq!(font_kept(noto.as_bytes(), BUNDLED.as_bytes()), Some(noto.clone().into_bytes()));
        let none = BUNDLED.replace("#font\tLXGWWenKaiTC\n", "");
        assert_eq!(font_kept(none.as_bytes(), BUNDLED.as_bytes()), Some(none.clone().into_bytes()), "no #font: kept so");
        assert_eq!(font_kept(BUNDLED.as_bytes(), none.as_bytes()), Some(BUNDLED.as_bytes().to_vec()), "a #font the bundled has not");
        assert_eq!(font_kept(b"#font\tX\nother\n", BUNDLED.as_bytes()), None);
    }

    #[test]
    fn an_earlier_launchers_table_is_replaced_on_an_upgrade_not_kept_as_the_players() {
        let t = scratch("sync-upgrade");
        // An earlier release's table: the same as the bundled one but for its #font, as the launcher wrote it (no note yet)
        let v32 = BUNDLED.replace("#font\tLXGWWenKaiTC", "#font\tNotoSansTC");
        put(&t.0.join("ui-zh-tw.tsv"), v32.as_bytes());
        let known = hex(&Sha256::digest(v32.as_bytes()));
        let lines = sync_into(&t.0, &[("ui-zh-tw.tsv", BUNDLED.as_bytes())], &[("ui-zh-tw.tsv", known.as_str())]).unwrap();
        assert!(lines[0].starts_with("sync replaced"), "{lines:?}");
        assert_eq!(std::fs::read_to_string(t.0.join("ui-zh-tw.tsv")).unwrap(), BUNDLED);
        // The note: the bundled table's SHA-256
        let noted = std::fs::read_to_string(t.0.join(SYNC_MARKER)).unwrap();
        assert_eq!(noted, format!("\"ui-zh-tw.tsv\" = \"{}\"\n", hex(&Sha256::digest(BUNDLED.as_bytes()))));
    }

    #[test]
    fn a_table_equal_to_the_one_the_launcher_noted_is_replaced_and_a_changed_one_kept() {
        let t = scratch("sync-noted");
        let older = BUNDLED.replace("#font\tLXGWWenKaiTC", "#font\tOlderFont");
        put(&t.0.join("zh-tw.tsv"), older.as_bytes());
        put(&t.0.join(SYNC_MARKER), format!("\"zh-tw.tsv\" = \"{}\"\n", hex(&Sha256::digest(older.as_bytes()))).as_bytes());
        let lines = sync_into(&t.0, &[("zh-tw.tsv", BUNDLED.as_bytes())], &[]).unwrap();
        assert!(lines[0].starts_with("sync replaced"), "the launcher's own table: {lines:?}");
        // The player changes #font after that: kept, now and at the next start
        let theirs = BUNDLED.replace("#font\tLXGWWenKaiTC", "#font\tNotoSansTC");
        std::fs::write(t.0.join("zh-tw.tsv"), &theirs).unwrap();
        for _ in 0..2 {
            let kept = sync_into(&t.0, &[("zh-tw.tsv", BUNDLED.as_bytes())], &[]).unwrap();
            assert!(kept[0].starts_with("sync kept-font"), "{kept:?}");
            assert_eq!(std::fs::read_to_string(t.0.join("zh-tw.tsv")).unwrap(), theirs);
        }
    }

    #[test]
    fn a_table_that_differs_in_its_line_ends_or_bom_only_is_unchanged() {
        let t = scratch("sync-crlf");
        let crlf = format!("\u{feff}{}", BUNDLED.replace('\n', "\r\n"));
        put(&t.0.join("zh-tw.tsv"), crlf.as_bytes());
        let lines = sync_into(&t.0, &[("zh-tw.tsv", BUNDLED.as_bytes())], &[]).unwrap();
        assert!(lines[0].starts_with("sync unchanged"), "{lines:?}");
        assert_eq!(std::fs::read(t.0.join("zh-tw.tsv")).unwrap(), crlf.as_bytes(), "left as it is");
    }

    #[test]
    fn the_bundled_art_override_is_written_once_and_a_picture_the_player_removed_or_changed_is_left() {
        let t = scratch("art-override");
        let dir = t.0.join(ART_OVERRIDE);
        let pictures: &[(&str, &[u8])] = &[("battle-super-vegeta-000.png", b"pads off 0"), ("battle-super-vegeta-001.png", b"pads off 1")];
        // A dry run writes nothing
        let lines = install_art_override(&dir, pictures, false).unwrap();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("2 would be installed"), "{lines:?}");
        assert!(!dir.exists());
        // The first run writes both and notes them
        let lines = install_art_override(&dir, pictures, true).unwrap();
        assert!(lines[0].contains("2 installed, 0 updated, 0 same"), "{lines:?}");
        assert_eq!(std::fs::read(dir.join("battle-super-vegeta-000.png")).unwrap(), b"pads off 0");
        let marker = std::fs::read_to_string(t.0.join(ART_MARKER)).unwrap();
        assert_eq!(marker.lines().count(), 2);
        assert!(marker.contains(&format!("battle-super-vegeta-001.png\t{}", hex(&Sha256::digest(b"pads off 1")))));
        let lines = install_art_override(&dir, pictures, true).unwrap();
        assert!(lines[0].contains("0 installed, 0 updated, 2 same"), "{lines:?}");
        // The player removes one and changes the other: neither comes back, now or with a newer bundle
        std::fs::remove_file(dir.join("battle-super-vegeta-000.png")).unwrap();
        std::fs::write(dir.join("battle-super-vegeta-001.png"), b"mine").unwrap();
        let newer: &[(&str, &[u8])] = &[("battle-super-vegeta-000.png", b"newer 0"), ("battle-super-vegeta-001.png", b"newer 1")];
        for bundle in [pictures, newer] {
            let lines = install_art_override(&dir, bundle, true).unwrap();
            assert!(lines[0].contains("kept (changed by the player): battle-super-vegeta-001.png"), "{lines:?}");
            assert!(lines[0].contains("kept removed (removed by the player): battle-super-vegeta-000.png"), "{lines:?}");
            assert!(!dir.join("battle-super-vegeta-000.png").exists());
            assert_eq!(std::fs::read(dir.join("battle-super-vegeta-001.png")).unwrap(), b"mine");
        }
    }

    #[test]
    fn a_newer_bundled_picture_replaces_the_launchers_own_and_a_players_own_picture_is_never_noted() {
        let t = scratch("art-override-newer");
        let dir = t.0.join(ART_OVERRIDE);
        put(&dir.join("battle-super-vegeta-002.png"), b"the player's own");
        let first: &[(&str, &[u8])] = &[("battle-super-vegeta-000.png", b"v1"), ("battle-super-vegeta-002.png", b"v1")];
        install_art_override(&dir, first, true).unwrap();
        assert_eq!(std::fs::read(dir.join("battle-super-vegeta-002.png")).unwrap(), b"the player's own");
        let newer: &[(&str, &[u8])] = &[("battle-super-vegeta-000.png", b"v2"), ("battle-super-vegeta-002.png", b"v2")];
        let lines = install_art_override(&dir, newer, true).unwrap();
        assert!(lines[0].contains("0 installed, 1 updated, 0 same, kept (changed by the player): battle-super-vegeta-002.png"), "{lines:?}");
        assert_eq!(std::fs::read(dir.join("battle-super-vegeta-000.png")).unwrap(), b"v2");
        assert_eq!(std::fs::read(dir.join("battle-super-vegeta-002.png")).unwrap(), b"the player's own");
        let marker = std::fs::read_to_string(t.0.join(ART_MARKER)).unwrap();
        assert_eq!(marker, format!("battle-super-vegeta-000.png\t{}\n", hex(&Sha256::digest(b"v2"))));
    }

    #[test]
    fn the_art_licence_is_written_with_the_pictures_and_says_how_to_go_back() {
        let t = scratch("art-licence");
        let dir = t.0.join("art-override");
        let pictures: &[(&str, &[u8])] = &[("battle-super-vegeta-000.png", b"png"), (ART_LICENSE_FILE, ART_LICENSE.as_bytes())];
        install_art_override(&dir, pictures, true).unwrap();
        let text = std::fs::read_to_string(dir.join(ART_LICENSE_FILE)).unwrap();
        for said in ["CC BY-NC-SA 4.0", "Ming Liu", "Gemini", "非商業", "save\\art-override", "原圖", "克林", "光頭", "field-party-groups", "u01-055"] {
            assert!(text.contains(said), "{said}");
        }
        // The player deletes the whole folder to go back to the pack's pictures: the note is beside it, nothing comes back
        assert!(t.0.join(ART_MARKER).is_file() && !dir.join(ART_MARKER).exists(), "the note is beside the folder");
        std::fs::remove_dir_all(&dir).unwrap();
        for _ in 0..2 {
            let line = install_art_override(&dir, pictures, true).unwrap();
            assert!(line[0].contains("kept removed"), "{line:?}");
            assert!(!dir.exists(), "not written again");
        }
    }

    #[test]
    fn the_pictures_of_super_vegetas_folder_are_bundled_and_nothing_of_another_folder_is() {
        let files: &[(&str, &[u8])] = &[
            ("art-override/battle-super-vegeta-000.png", b"a"),
            ("art-override/super-vegeta/battle-vegeta-803.png", b"b"),
            ("art-override/super-vegeta/notes.txt", b"c"),
            ("art-override/other/battle-x-000.png", b"d"),
            ("art-override/super-vegeta/deeper/battle-y-000.png", b"e"),
            ("fonts/OFL.txt", b"f"),
        ];
        let names: Vec<&str> = art_override_pictures(files.iter().copied()).iter().map(|(file, _)| *file).collect();
        assert_eq!(
            names,
            ["battle-super-vegeta-000.png", "super-vegeta/battle-vegeta-803.png", ART_LICENSE_FILE]
        );
    }

    #[test]
    fn the_poses_of_super_vegetas_keyed_pictures_are_bundled_and_written_beside_them_and_never_their_pixels() {
        // The game reads `super-vegeta\key-<key>.png` (a pose the pack has no art for) only with its pose
        // `key-<key>.pose` beside it: those are written into `art-override\super-vegeta\` too. The pixels `key-<key>.txt`
        // are the original sprite's: never written, even when bundled (build.rs refuses them in assets/). Another file, a
        // name that is not 16 hexadecimal digits, another extension's case and the same name in another folder are not
        let key = "key-9fb61f64d7d15be9";
        let pose = b"key=9fb61f64d7d15be9 size=2x2 colours=0006 mirror=9fb61f64d7d15be9\n";
        let files: &[(&str, &[u8])] = &[
            ("art-override/super-vegeta/key-9fb61f64d7d15be9.png", b"art"),
            ("art-override/super-vegeta/key-9fb61f64d7d15be9.pose", pose),
            ("art-override/super-vegeta/key-9fb61f64d7d15be9.txt", b"0110\n1221\n"),
            ("art-override/super-vegeta/key-9FB61F64D7D15BE9.pose", b"upper"),
            ("art-override/super-vegeta/key-9fb61f64d7d15be.pose", b"15 digits"),
            ("art-override/super-vegeta/key-9fb61f64d7d15bez.pose", b"not hex"),
            ("art-override/super-vegeta/key-9fb61f64d7d15be9.POSE", b"upper extension"),
            ("art-override/super-vegeta/notes.txt", b"notes"),
            ("art-override/super-vegeta/deeper/key-9fb61f64d7d15be9.pose", b"deeper"),
            ("art-override/goku/key-9fb61f64d7d15be9.pose", b"goku"),
            ("art-override/key-9fb61f64d7d15be9.pose", b"top"),
        ];
        let pictures = art_override_pictures(files.iter().copied());
        let names: Vec<&str> = pictures.iter().map(|(file, _)| *file).collect();
        assert_eq!(
            names,
            [
                "super-vegeta/key-9fb61f64d7d15be9.png",
                "super-vegeta/key-9fb61f64d7d15be9.pose",
                "super-vegeta/key-9FB61F64D7D15BE9.pose",
                ART_LICENSE_FILE
            ]
        );
        assert!(!names.iter().any(|name| name.ends_with(".txt") && name.contains("key-")), "{names:?}");
        // the selftest's line counts pictures, not their poses
        assert_eq!(
            art_override_folders(&pictures),
            "art override folders: top 0, super-vegeta 1, goku 0, ending-goku 0, variants 0, ssj-lightning 0, mode7 0"
        );
        // written without the upper-case name: on Windows (and a Mac's disk) it is the same file
        let pictures: Vec<(&str, &[u8])> =
            pictures.into_iter().filter(|(file, _)| !file.contains("9FB6")).collect();
        let t = scratch("art-keyed");
        let dir = t.0.join("art-override");
        let line = install_art_override(&dir, &pictures, true).unwrap();
        assert!(line[0].contains("3 installed"), "{line:?}");
        let sv = dir.join("super-vegeta");
        assert_eq!(std::fs::read(sv.join(format!("{key}.pose"))).unwrap(), pose);
        assert_eq!(std::fs::read(sv.join(format!("{key}.png"))).unwrap(), b"art");
        assert!(!sv.join(format!("{key}.txt")).exists(), "never the pixels");
        assert!(!dir.join(format!("{key}.pose")).exists(), "not beside the folder");
        let note = std::fs::read_to_string(t.0.join(ART_MARKER)).unwrap();
        assert!(note.contains(&format!("super-vegeta/{key}.pose\t")), "{note}");
    }

    #[test]
    fn the_art_licence_names_the_keyed_poses_and_leaves_them_out_of_the_cc_licence() {
        // The three poses of Goku's Kamehameha are AI redrawings, not edits of the pack's pictures: named on their own
        // line with their rights, and left out of the CC BY-NC-SA paragraph by name. GAME OVER's flight
        // (`gallery-fx-1118`) is an edit of the pack's picture: named in that paragraph
        let said = ART_LICENSE
            .split("\r\n\r\n")
            .find(|p| p.starts_with("art-override/super-vegeta 裡的 key-"))
            .unwrap_or_else(|| panic!("{ART_LICENSE}"));
        for key in
            ["key-14b06ca30f359b50.png", "key-9b36beaeedd4fd2b.png", "key-9fb61f64d7d15be9.png"]
        {
            assert!(said.contains(key), "{key}: {said}");
        }
        for words in [
            "龜派氣功",
            "AI 依原作像素圖與既有圖重繪",
            "原作圖像權利屬原權利人",
            "非商業同人圖",
            ".pose",
            "不含原版像素",
        ] {
            assert!(said.contains(words), "{words}: {said}");
        }
        assert!(!said.contains(".txt"), "the pixels are never shipped: {said}");
        assert!(!said.contains("CC BY-NC-SA"), "{said}");
        let cc = ART_LICENSE.split("\r\n\r\n").nth(1).unwrap();
        assert!(cc.contains("CC BY-NC-SA 4.0") && cc.contains("gallery-fx-1118"), "{cc}");
        assert!(
            cc.contains("battle-goku-ssj-020.png 與 super-vegeta 裡以 key- 開頭的 PNG 除外"),
            "{cc}"
        );
    }

    #[test]
    fn the_art_licence_names_the_dodge_kienzan_and_energy_wave_poses() {
        // The back Super Vegeta turns when he dodges the Kamehameha, the poses cut by the screen's edge in a
        // dodge, Kienzan's throw and the slide-in of the Energy Wave close-up. Every keyed picture is named with its
        // rights, outside the CC BY-NC-SA paragraph (which leaves every key- picture out)
        let said = ART_LICENSE
            .split("\r\n\r\n")
            .find(|p| p.contains("key-792fd8e6151f6126.png"))
            .unwrap_or_else(|| panic!("{ART_LICENSE}"));
        for key in [
            "792fd8e6151f6126",
            "2ffb009ee0521c55",
            "ffe840f345eaed93",
            "548c351e64b1725d",
            "51afa33dcdee87e0",
            "5a0714a076141e78",
            "740b6f8f98f3cc54",
            "10763b598c8fd2dc",
            "0bb46e676ac56916",
            "57f1d3ba8487754d",
            "dc88aec8269935e7",
            "ed00ab8970a6db2d",
            "fb41e72dd32fab1a",
            "d2047839a7642c43",
            "c777680aa73f6d9e",
            "b2bb355b0c6409f2",
        ] {
            assert!(said.contains(&format!("key-{key}.png")), "{key}: {said}");
        }
        assert_eq!(said.matches("key-").count(), 16, "{said}");
        for words in [
            "龜派氣功",
            "氣圓斬",
            "強力能量波",
            "AI 依原作像素圖與既有圖重繪",
            "原作圖像權利屬原權利人",
            "非商業同人圖",
            ".pose",
            "不含原版像素",
        ] {
            assert!(said.contains(words), "{words}: {said}");
        }
        assert!(!said.contains(".txt"), "the pixels are never shipped: {said}");
        assert!(!said.contains("CC BY-NC-SA"), "{said}");
    }

    /// The numbers a list like `000～012、015、…` names, up to the first `（`.
    fn numbers_listed(list: &str) -> Vec<u32> {
        let list = list.split('（').next().unwrap();
        let mut out = Vec::new();
        for item in list.split('、') {
            let item = item.trim();
            match item.split_once('～') {
                Some((a, b)) => out.extend(a.parse::<u32>().unwrap()..=b.parse::<u32>().unwrap()),
                None => out.push(item.parse().unwrap()),
            }
        }
        out
    }

    #[test]
    fn the_art_licence_names_the_new_heads_of_super_saiyan_goku_under_the_cc_licence() {
        // 73 of variants' Super Saiyan Goku pictures have one new head, made by AI (Gemini) editing the earlier
        // art-override picture of the same name, which is an edit of the pack's picture: so they stay under CC BY-NC-SA
        // and out of the exceptions, each named by number. 020 (redrawn from an official illustration) keeps its own
        // paragraph and its exception; the close-ups 019 and 070 were not changed
        let said = ART_LICENSE
            .split("\r\n\r\n")
            .find(|p| p.contains("超賽悟空換成新頭"))
            .unwrap_or_else(|| panic!("{ART_LICENSE}"));
        let battle = said.split("battle-goku-ssj-").nth(1).unwrap();
        let battle = numbers_listed(battle);
        assert_eq!(battle.len(), 53, "{battle:?}");
        for not_changed in [19, 20, 70] {
            assert!(!battle.contains(&not_changed), "{not_changed}: {battle:?}");
        }
        let gallery = numbers_listed(said.split("gallery-party-goku-ssj-").nth(1).unwrap());
        assert_eq!(gallery.len(), 20, "{gallery:?}");
        for n in [56, 117, 158] {
            assert!(gallery.contains(&n), "{n}: {gallery:?}");
        }
        for words in [
            "73 張",
            "（53 張）",
            "（20 張）",
            "Gemini",
            "Ming Liu",
            "HD 美術包",
            "CC BY-NC-SA 4.0",
            "非商業",
        ] {
            assert!(said.contains(words), "{words}: {said}");
        }
        assert!(!said.contains("原作圖像權利屬原權利人"), "{said}");
        let cc = ART_LICENSE.split("\r\n\r\n").nth(1).unwrap();
        assert!(
            cc.contains("battle-goku-ssj-020.png 與 super-vegeta 裡以 key- 開頭的 PNG 除外"),
            "{cc}"
        );
        assert!(
            ART_LICENSE
                .contains("art-override/variants/battle-goku-ssj-020.png：超賽悟空集氣全身圖")
        );
    }

    #[test]
    fn the_art_licence_names_the_hands_of_normal_gokus_kamehameha_under_the_cc_licence() {
        // The back view of the normal (black-haired) Goku pushing out the Kamehameha wave, in variants and at the top:
        // the pack's picture with the hands of the Super Saiyan picture (gallery-party-goku-ssj-170, redrawn with
        // Gemini) put on it, so it stays under CC BY-NC-SA. The CC paragraph's words on variants cover it too
        let said = ART_LICENSE
            .split("\r\n\r\n")
            .find(|p| p.starts_with("art-override/variants/gallery-party-goku-u00-170.png"))
            .unwrap_or_else(|| panic!("{ART_LICENSE}"));
        for words in [
            "art-override/variants/gallery-party-goku-u00-170.png 與 art-override/gallery-party-goku-u00-170.png：",
            "一般悟空龜派氣功",
            "gallery-party-goku-ssj-170",
            "Ming Liu",
            "Google Gemini 圖像模型",
            "HD 美術包的原圖",
            "CC BY-NC-SA 4.0",
            "改作沿用同一授權",
            "僅限非商業使用",
        ] {
            assert!(said.contains(words), "{words}: {said}");
        }
        assert!(!said.contains("原作圖像權利屬原權利人"), "{said}");
        let cc = ART_LICENSE.split("\r\n\r\n").nth(1).unwrap();
        assert!(cc.contains("一般悟空龜派氣功的背面圖 gallery-party-goku-u00-170"), "{cc}");
        assert!(cc.contains("一般悟空的 gallery-party-goku-u00-170"), "{cc}");
    }

    #[test]
    fn the_pictures_of_gokus_folder_are_bundled_beside_super_vegetas() {
        // Patch 0051: `art-override/goku/<name>.png` is written into `art-override\goku\`; deeper folders and other files
        // are not, nor a folder whose name only starts with goku
        let files: &[(&str, &[u8])] = &[
            ("art-override/battle-super-vegeta-000.png", b"a"),
            ("art-override/super-vegeta/battle-vegeta-803.png", b"b"),
            ("art-override/goku/battle-goku-ssj-020.png", b"c"),
            ("art-override/goku/notes.txt", b"d"),
            ("art-override/goku/deeper/battle-goku-ssj-019.png", b"e"),
            ("art-override/gokux/battle-goku-ssj-019.png", b"f"),
        ];
        let names: Vec<&str> = art_override_pictures(files.iter().copied()).iter().map(|(file, _)| *file).collect();
        assert_eq!(
            names,
            [
                "battle-super-vegeta-000.png",
                "super-vegeta/battle-vegeta-803.png",
                "goku/battle-goku-ssj-020.png",
                ART_LICENSE_FILE
            ]
        );
        let t = scratch("art-goku");
        let dir = t.0.join("art-override");
        let pictures: &[(&str, &[u8])] = &[("goku/battle-goku-ssj-020.png", b"black"), (ART_LICENSE_FILE, ART_LICENSE.as_bytes())];
        let line = install_art_override(&dir, pictures, true).unwrap();
        assert!(line[0].contains("2 installed"), "{line:?}");
        assert_eq!(std::fs::read(dir.join("goku").join("battle-goku-ssj-020.png")).unwrap(), b"black");
        let text = std::fs::read_to_string(dir.join(ART_LICENSE_FILE)).unwrap();
        for said in ["goku", "黑髮", "super-vegeta"] {
            assert!(text.contains(said), "{said}");
        }
    }

    #[test]
    fn the_pictures_of_the_lightning_folder_are_bundled_and_written_into_their_folder() {
        // `art-override/ssj-lightning/frame-<n>.png` is written into `art-override\ssj-lightning\` (the game draws it over
        // the picture while Goku first becomes a Super Saiyan); deeper folders and other files are not
        let files: &[(&str, &[u8])] = &[
            ("art-override/ssj-lightning/frame-1.png", b"a"),
            ("art-override/ssj-lightning/frame-2.png", b"b"),
            ("art-override/ssj-lightning/notes.txt", b"c"),
            ("art-override/ssj-lightning/deeper/frame-3.png", b"d"),
            ("art-override/ssj-lightningx/frame-1.png", b"e"),
        ];
        let pictures = art_override_pictures(files.iter().copied());
        let names: Vec<&str> = pictures.iter().map(|(file, _)| *file).collect();
        assert_eq!(names, ["ssj-lightning/frame-1.png", "ssj-lightning/frame-2.png", ART_LICENSE_FILE]);
        assert_eq!(
            art_override_folders(&pictures),
            "art override folders: top 0, super-vegeta 0, goku 0, ending-goku 0, variants 0, ssj-lightning 2, mode7 0"
        );
        let t = scratch("art-ssj-lightning");
        let dir = t.0.join("art-override");
        install_art_override(&dir, &pictures, true).unwrap();
        assert_eq!(std::fs::read(dir.join("ssj-lightning").join("frame-2.png")).unwrap(), b"b");
    }

    #[test]
    fn the_pictures_of_the_ending_goku_folder_are_bundled_and_written_into_their_folder() {
        // `art-override/ending-goku/<name>.png` is written into `art-override\ending-goku\` (the game draws it only in the
        // ending's flight); deeper folders and other files are not, nor a folder whose name only starts with it. The
        // selftest's line counts the folder's pictures
        let files: &[(&str, &[u8])] = &[
            ("art-override/goku/battle-goku-ssj-020.png", b"a"),
            ("art-override/ending-goku/battle-goku-025.png", b"b"),
            ("art-override/ending-goku/notes.txt", b"c"),
            ("art-override/ending-goku/deeper/battle-goku-024.png", b"d"),
            ("art-override/ending-gokux/battle-goku-023.png", b"e"),
        ];
        let pictures = art_override_pictures(files.iter().copied());
        let names: Vec<&str> = pictures.iter().map(|(file, _)| *file).collect();
        assert_eq!(names, ["goku/battle-goku-ssj-020.png", "ending-goku/battle-goku-025.png", ART_LICENSE_FILE]);
        assert_eq!(
            art_override_folders(&pictures),
            "art override folders: top 0, super-vegeta 0, goku 1, ending-goku 1, variants 0, ssj-lightning 0, mode7 0"
        );
        let t = scratch("art-ending-goku");
        let dir = t.0.join("art-override");
        let line = install_art_override(&dir, &pictures, true).unwrap();
        assert!(line[0].contains("3 installed"), "{line:?}");
        assert_eq!(std::fs::read(dir.join("ending-goku").join("battle-goku-025.png")).unwrap(), b"b");
        assert!(!dir.join("battle-goku-025.png").exists(), "not beside the folder: that would replace it everywhere");
        let again = install_art_override(&dir, &pictures, true).unwrap();
        assert!(again[0].contains("0 installed, 0 updated, 3 same"), "{again:?}");
    }

    #[test]
    fn super_vegetas_pictures_are_written_into_their_folder_once_and_deleting_art_override_keeps_them_away() {
        let t = scratch("art-super-vegeta");
        let dir = t.0.join("art-override");
        let pictures: &[(&str, &[u8])] = &[
            ("battle-super-vegeta-000.png", b"root"),
            ("super-vegeta/battle-vegeta-803.png", b"wave"),
            (ART_LICENSE_FILE, ART_LICENSE.as_bytes()),
        ];
        let line = install_art_override(&dir, pictures, true).unwrap();
        assert!(line[0].contains("3 installed"), "{line:?}");
        assert_eq!(std::fs::read(dir.join("super-vegeta").join("battle-vegeta-803.png")).unwrap(), b"wave");
        let note = std::fs::read_to_string(t.0.join(ART_MARKER)).unwrap();
        assert!(note.contains("super-vegeta/battle-vegeta-803.png\t"), "{note}");
        let text = std::fs::read_to_string(dir.join(ART_LICENSE_FILE)).unwrap();
        for said in ["super-vegeta", "金髮", "save\\art-override", "原圖"] {
            assert!(text.contains(said), "{said}");
        }
        // Only the subfolder deleted: it is not written again either
        std::fs::remove_dir_all(dir.join("super-vegeta")).unwrap();
        let line = install_art_override(&dir, pictures, true).unwrap();
        assert!(line[0].contains("kept removed") && line[0].contains("super-vegeta/"), "{line:?}");
        assert!(!dir.join("super-vegeta").exists());
        // The whole folder deleted, to go back to the pack's pictures: nothing comes back, twice
        std::fs::remove_dir_all(&dir).unwrap();
        for _ in 0..2 {
            let line = install_art_override(&dir, pictures, true).unwrap();
            assert!(line[0].contains("kept removed"), "{line:?}");
            assert!(!dir.exists(), "not written again");
        }
    }

    #[test]
    fn the_pose_variants_are_bundled_written_into_their_folder_once_and_a_removed_one_is_not_written_again() {
        // `art-override/variants/<name>.png` is written into `art-override\variants\`; deeper folders and other files are
        // not, nor a folder whose name only starts with variants
        let files: &[(&str, &[u8])] = &[
            ("art-override/battle-super-vegeta-000.png", b"a"),
            ("art-override/goku/battle-goku-ssj-020.png", b"b"),
            ("art-override/variants/battle-goku-ssj-071.png", b"c"),
            ("art-override/variants/battle-goku-ssj-051.png", b"d"),
            ("art-override/variants/notes.txt", b"e"),
            ("art-override/variants/deeper/battle-goku-ssj-008.png", b"f"),
            ("art-override/variantsx/battle-goku-ssj-008.png", b"g"),
        ];
        let names: Vec<&str> = art_override_pictures(files.iter().copied()).iter().map(|(file, _)| *file).collect();
        assert_eq!(
            names,
            [
                "battle-super-vegeta-000.png",
                "goku/battle-goku-ssj-020.png",
                "variants/battle-goku-ssj-071.png",
                "variants/battle-goku-ssj-051.png",
                ART_LICENSE_FILE
            ]
        );
        let t = scratch("art-variants");
        let dir = t.0.join(ART_OVERRIDE);
        let pictures: &[(&str, &[u8])] = &[
            ("variants/battle-goku-ssj-071.png", b"wave"),
            ("variants/battle-goku-ssj-051.png", b"power"),
            (ART_LICENSE_FILE, ART_LICENSE.as_bytes()),
        ];
        let line = install_art_override(&dir, pictures, true).unwrap();
        assert!(line[0].contains("3 installed"), "{line:?}");
        assert_eq!(std::fs::read(dir.join(ART_VARIANTS).join("battle-goku-ssj-071.png")).unwrap(), b"wave");
        assert_eq!(std::fs::read(dir.join(ART_VARIANTS).join("battle-goku-ssj-051.png")).unwrap(), b"power");
        let note = std::fs::read_to_string(t.0.join(ART_MARKER)).unwrap();
        assert!(note.contains("variants/battle-goku-ssj-071.png\t"), "{note}");
        let text = std::fs::read_to_string(dir.join(ART_LICENSE_FILE)).unwrap();
        for said in ["variants", "76", "豎", "save\\art-override", "原圖"] {
            assert!(text.contains(said), "{said}");
        }
        // One removed by the player: not written again, now or with a newer bundle; the other is brought up to date
        std::fs::remove_file(dir.join(ART_VARIANTS).join("battle-goku-ssj-071.png")).unwrap();
        let newer: &[(&str, &[u8])] =
            &[("variants/battle-goku-ssj-071.png", b"wave 2"), ("variants/battle-goku-ssj-051.png", b"power 2")];
        let line = install_art_override(&dir, newer, true).unwrap();
        assert!(line[0].contains("1 updated") && line[0].contains("kept removed (removed by the player): variants/battle-goku-ssj-071.png"), "{line:?}");
        assert!(!dir.join(ART_VARIANTS).join("battle-goku-ssj-071.png").exists());
        assert_eq!(std::fs::read(dir.join(ART_VARIANTS).join("battle-goku-ssj-051.png")).unwrap(), b"power 2");
        // The whole subfolder deleted: nothing of it comes back, twice
        std::fs::remove_dir_all(dir.join(ART_VARIANTS)).unwrap();
        for _ in 0..2 {
            let line = install_art_override(&dir, newer, true).unwrap();
            assert!(line[0].contains("kept removed") && line[0].contains("variants/battle-goku-ssj-051.png"), "{line:?}");
            assert!(!dir.join(ART_VARIANTS).exists(), "not written again");
        }
    }

    #[test]
    fn a_mode7_planes_tiles_palette_and_replace_are_written_two_folders_deep() {
        // `art-override/mode7/<content key>/` (the game's `ssd_hd::pack::MODE7_DIR`): its PNGs, `palette.txt` and the empty
        // `replace` are written into `art-override\mode7\<content key>\`, beside the pictures of one folder; a file of
        // `mode7/` itself, of a folder deeper, of another name, or of a folder whose name only starts with mode7 is not
        let files: &[(&str, &[u8])] = &[
            ("art-override/battle-super-vegeta-000.png", b"a"),
            ("art-override/goku/battle-goku-ssj-020.png", b"b"),
            ("art-override/mode7/1239827fe73e508f/deeper/t1_0.png", b"c"),
            ("art-override/mode7/1239827fe73e508f/notes.txt", b"d"),
            ("art-override/mode7/1239827fe73e508f/palette.txt", b"000000\n"),
            ("art-override/mode7/1239827fe73e508f/replace", b""),
            ("art-override/mode7/1239827fe73e508f/t0_0.png", b"tile"),
            ("art-override/mode7/replace", b""),
            ("art-override/mode7/t0_0.png", b"e"),
            ("art-override/mode7x/1239827fe73e508f/t0_0.png", b"f"),
        ];
        let pictures = art_override_pictures(files.iter().copied());
        let names: Vec<&str> = pictures.iter().map(|(file, _)| *file).collect();
        assert_eq!(
            names,
            [
                "battle-super-vegeta-000.png",
                "goku/battle-goku-ssj-020.png",
                "mode7/1239827fe73e508f/palette.txt",
                "mode7/1239827fe73e508f/replace",
                "mode7/1239827fe73e508f/t0_0.png",
                ART_LICENSE_FILE
            ]
        );
        let t = scratch("art-mode7");
        let dir = t.0.join(ART_OVERRIDE);
        let line = install_art_override(&dir, &pictures, true).unwrap();
        assert!(line[0].contains("6 installed"), "{line:?}");
        let plane = dir.join("mode7").join("1239827fe73e508f");
        let mut in_plane: Vec<String> = std::fs::read_dir(&plane)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        in_plane.sort();
        assert_eq!(in_plane, ["palette.txt", "replace", "t0_0.png"]);
        assert_eq!(std::fs::read(plane.join("t0_0.png")).unwrap(), b"tile");
        assert_eq!(std::fs::read(plane.join("replace")).unwrap(), b"");
        assert_eq!(std::fs::read(plane.join("palette.txt")).unwrap(), b"000000\n");
        assert_eq!(std::fs::read(dir.join("battle-super-vegeta-000.png")).unwrap(), b"a");
        assert_eq!(std::fs::read(dir.join("goku").join("battle-goku-ssj-020.png")).unwrap(), b"b");
        assert!(
            !dir.join("mode7").join("t0_0.png").exists()
                && !dir.join("mode7").join("replace").exists()
        );
        let note = std::fs::read_to_string(t.0.join(ART_MARKER)).unwrap();
        assert!(
            note.contains("mode7/1239827fe73e508f/replace\t")
                && note.contains("mode7/1239827fe73e508f/t0_0.png\t"),
            "{note}"
        );
        // The licence names the plane's picture: what it is drawn from, and under which licence
        let text = std::fs::read_to_string(dir.join(ART_LICENSE_FILE)).unwrap();
        let said =
            text.lines().find(|l| l.starts_with("art-override/mode7/1239827fe73e508f/t0_0.png："));
        assert!(
            said.is_some_and(|l| l.contains("Mode 7 平面") && l.contains("CC BY-NC-SA 4.0")),
            "{text}"
        );
        // and the Super Saiyan Goku's plane of the opening flight: the normal Goku's plane of the pack, made a Super
        // Saiyan with torn clothes, under the pack's licence
        let goku =
            text.lines().find(|l| l.starts_with("art-override/mode7/cd83cbee861e14ad/t0_0.png："));
        assert!(
            goku.is_some_and(|l| l.contains("普通悟空 Mode 7 平面")
                && l.contains("超級賽亞人")
                && l.contains("CC BY-NC-SA 4.0")),
            "{text}"
        );
        assert!(text.contains("ssj-lightning 與 mode7）"), "deleting art-override names mode7: {text}");
        // Again: nothing is written; a `replace` the player removed is not written back
        let line = install_art_override(&dir, &pictures, true).unwrap();
        assert!(line[0].contains("0 installed") && line[0].contains("6 same"), "{line:?}");
        std::fs::remove_file(plane.join("replace")).unwrap();
        let line = install_art_override(&dir, &pictures, true).unwrap();
        assert!(
            line[0].contains("kept removed") && line[0].contains("mode7/1239827fe73e508f/replace"),
            "{line:?}"
        );
        assert!(!plane.join("replace").exists());
    }
}
