//! Without an HD art pack beside the launcher, the player is asked whether to download it: upstream's
//! `hd-pack.zip` of v0.3.9 (not ours: JackRushante/DBZ-Super-Saiya-Densetsu-Rust, CC BY-NC-SA 4.0), from its
//! GitHub release, to `hd-pack.zip` beside this `.exe`, where the launcher and the game look for it.
//!
//! Nothing is requested before the player answers "download". The download uses the update's transport and checks
//! ([`crate::update::fetch_verified`]): https only, GitHub's release hosts only, at most the pack's size, an idle
//! timeout, and the SHA-256 built in here — the one GitHub publishes for the asset (the API's `digest`), the same
//! as a checked copy — checked while it comes and again from the disk before the rename. It goes to
//! `hd-pack.zip.part`, created for this launcher alone ([`crate::update::claim`]); bytes that are not the pack's,
//! a cancel, or a stall remove it. Nothing is ever unpacked or run. The pack does not touch `art-override\`
//! (ours): the game draws those pictures in place of the pack's (`Pack::with_art_override`).
//!
//! When upstream's release cannot give the pack (no connection, a 404 or other status, a redirect off the
//! hosts, a stall, bytes that are not the pack), the same file is downloaded from [`MIRROR_URL`] (our public fork's
//! release, also on `github.com`) into the emptied partial file, checked exactly as upstream's. A cancel, another
//! launcher downloading ([`crate::update::BUSY`]) or a failed write does not go to the mirror.

use crate::progress::{Phase, ProgressSink, CANCELLED};
use crate::update::{self, Transport};
use std::path::{Path, PathBuf};

/// One file to download: where, where else when it cannot be had there, how big, its SHA-256.
#[derive(Debug, Clone)]
pub struct Pack {
    pub url: &'static str,
    /// Tried when `url` cannot be had ([`mirror_may_help`]); the same size and SHA-256 are required of it.
    pub mirror: Option<&'static str>,
    pub size: u64,
    pub sha256: [u8; 32],
}

/// The same `hd-pack.zip`, on a release of our public fork, for when upstream's release is gone. On
/// `github.com` (in [`update::DOWNLOAD_HOSTS`]): no host is added for it.
pub const MIRROR_URL: &str = "https://github.com/lming888/DBZ-Super-Saiya-Densetsu-Rust/releases/download/hd-pack-v0.3.9/hd-pack.zip";

/// Upstream v0.3.9's `hd-pack.zip` (the one [`crate::HD_PACK_DIGEST`] names).
pub const OFFICIAL: Pack = Pack {
    url: "https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust/releases/download/v0.3.9/hd-pack.zip",
    mirror: Some(MIRROR_URL),
    size: crate::HD_PACK_ZIP_BYTES,
    sha256: [
        0xe5, 0x5c, 0xb0, 0x5d, 0x4c, 0xb9, 0x89, 0x04, 0xd4, 0x89, 0x3b, 0x35, 0x97, 0xa9, 0x0b, 0x72, 0x59, 0x89, 0x3c, 0x34,
        0x79, 0x7f, 0x3b, 0x8e, 0x5b, 0x5c, 0x1c, 0xd0, 0xb6, 0x21, 0xd4, 0xcc,
    ],
};
/// The release page, named in the question.
pub const RELEASE_PAGE: &str = "https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust/releases/tag/v0.3.9";
/// In the launcher's folder: the player's last answer.
pub const STATE_FILE: &str = "hd-pack-offer.txt";
/// The name the launcher and the game look for beside the `.exe` (`hd_pack_in`, upstream `install::find_source`).
pub const FILE_NAME: &str = "hd-pack.zip";
/// The download until it is checked.
pub const PART_NAME: &str = "hd-pack.zip.part";
/// What an existing `hd-pack.zip` is renamed to before the download takes its place (then `.old2`, `.old3`, …):
/// not a `.zip`, so neither the launcher (`hd_pack_in`: a name with `hd-pack` ending in `.zip`) nor the game
/// (upstream `install::find_source`: extension `zip`) takes it for a pack. Never removed by the launcher.
pub const BACKUP_NAME: &str = "hd-pack.zip.old";

/// Whether anything is at `path`, a dangling link included (`Path::exists` follows links).
fn taken(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// Where `<exe_dir>\hd-pack.zip` is kept when a download replaces it: the first of `hd-pack.zip.old`,
/// `hd-pack.zip.old2`, … that is not taken (`None` after 99).
pub fn backup_path(exe_dir: &Path) -> Option<PathBuf> {
    (1..100)
        .map(|n| exe_dir.join(if n == 1 { BACKUP_NAME.to_owned() } else { format!("{BACKUP_NAME}{n}") }))
        // guard: a backup never overwrites a file
        .find(|p| !taken(p))
}

/// Where a download landed, and the name the `hd-pack.zip` it replaced is kept under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
}

/// The player's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Download it now.
    Download,
    /// Not this time: asked again at the next start.
    NotNow,
    /// Never asked again.
    Never,
}

impl Answer {
    fn word(self) -> &'static str {
        match self {
            Answer::Download => "download",
            Answer::NotNow => "not-now",
            Answer::Never => "never",
        }
    }
}

/// The last answer saved in `state` (`None`: never asked, or a file not understood).
pub fn read_state(state: &Path) -> Option<Answer> {
    let text = std::fs::read_to_string(state).ok()?;
    let word = text.lines().find_map(|l| l.trim().strip_prefix("answer = "))?;
    [Answer::Download, Answer::NotNow, Answer::Never].into_iter().find(|a| a.word() == word.trim())
}

pub fn write_state(state: &Path, answer: Answer) -> std::io::Result<()> {
    if let Some(dir) = state.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        state,
        format!(
            "# 七龍珠Z 超賽亞傳說 繁中版：要不要下載 HD 美術包的最後回答\n\
             # download＝下載、not-now＝這次不要（下次再問）、never＝不再詢問；刪掉這個檔就會再問\n\
             answer = {}\n",
            answer.word()
        ),
    )
}

/// What happened, for `last-run.log`.
#[derive(Debug, PartialEq, Eq)]
pub enum Done {
    /// A pack is there: nothing asked.
    Present,
    /// The player said never before: nothing asked.
    NotAsked,
    Answered(Answer),
    Downloaded(PathBuf),
    /// The player cancelled the download: asked again next time.
    Cancelled,
    Failed(String),
}

/// Asks (unless a pack is there or the player said never), saves the answer, downloads on "download" (`download`
/// is [`download`] behind the progress dialog). `ask` and `download` are called only when needed: no request is
/// made unless `ask` said [`Answer::Download`].
pub fn run(
    pack_present: bool,
    state: &Path,
    ask: impl FnOnce() -> Answer,
    download: impl FnOnce() -> Result<Placed, String>,
) -> Done {
    if pack_present {
        return Done::Present;
    }
    if read_state(state) == Some(Answer::Never) {
        return Done::NotAsked;
    }
    let answer = ask();
    let _ = write_state(state, answer);
    // guard: no request before "download"
    if answer != Answer::Download {
        return Done::Answered(answer);
    }
    match download() {
        Ok(placed) => Done::Downloaded(placed.path),
        Err(e) if e == CANCELLED => Done::Cancelled,
        Err(e) => Done::Failed(e),
    }
}

/// What `--hd-pack` did, for the message box and `last-run.log`.
#[derive(Debug, PartialEq, Eq)]
pub enum Demand {
    /// The v0.3.9 pack is beside the launcher already: nothing asked, nothing downloaded.
    AlreadyThere,
    /// The player said no: nothing downloaded, nothing changed.
    Declined,
    /// Downloaded; the `hd-pack.zip` that was there is kept under `backup`.
    Downloaded(Placed),
    /// The player cancelled the download: no file left.
    Cancelled,
    Failed(String),
}

/// `--hd-pack`, for a player who answered "not now" or "never" and changed their mind: asks (`confirm`, whatever
/// was answered before) unless the pack is there already, and downloads only on yes. After a download the saved
/// answer becomes "download", so the question at start agrees (and is not asked: the pack is there).
pub fn run_on_demand(
    valid_pack_present: bool,
    state: &Path,
    confirm: impl FnOnce() -> bool,
    download: impl FnOnce() -> Result<Placed, String>,
) -> Demand {
    // guard: on demand, a pack already there is not downloaded again
    if valid_pack_present {
        return Demand::AlreadyThere;
    }
    // guard: on demand, no request before "download"
    if !confirm() {
        return Demand::Declined;
    }
    match download() {
        Ok(placed) => {
            // guard: the saved answer agrees with the pack now there
            let _ = write_state(state, Answer::Download);
            Demand::Downloaded(placed)
        }
        Err(e) if e == CANCELLED => Demand::Cancelled,
        Err(e) => Demand::Failed(e),
    }
}

/// The question of `--hd-pack`: "下載" / "取消" in a task dialog, or (`yes_no`) rfd's OK / cancel. `old`: a pack
/// beside the launcher that is not v0.3.9: `hd-pack.zip` itself, or another one (left as it is; `hd-pack.zip` is
/// taken first). Which backup name an `hd-pack.zip` gets is known only when the download is placed ([`place`]).
pub fn demand_text(pack: &Pack, exe_dir: &Path, old: Option<&Path>, yes_no: bool) -> String {
    let target = exe_dir.join(FILE_NAME);
    let old = match old {
        None => String::new(),
        Some(p) if p == target => "現有的 hd-pack.zip 不是 v0.3.9。\n\n".to_owned(),
        Some(p) => format!("exe 旁的美術包不是 v0.3.9：{}\n新下載的 hd-pack.zip 會優先使用，這個檔不會被改動。\n\n", p.display()),
    };
    let kept = format!(
        "放新檔時若 exe 旁已有 hd-pack.zip，會先把它改名保留為 {BACKUP_NAME}（已被佔用就用 {BACKUP_NAME}2、{BACKUP_NAME}3……），\
         不覆蓋任何檔，啟動器也不會自動刪除；實際的檔名在完成時告知。\n\n"
    );
    let buttons = if yes_no { "「確定」：下載。「取消」：不下載。\n\n" } else { "" };
    format!(
        "要下載 HD 高畫質美術包嗎？\n\n{old}\
         美術包是上游作者 JackRushante（DBZ-Super-Saiya-Densetsu-Rust）的作品，不是本繁中版的內容，\
         授權 CC BY-NC-SA 4.0。從上游的 GitHub 發行頁下載 v0.3.9 的 hd-pack.zip（上游抓不到時會改從備援下載；約 {:.0} MB，可能要幾分鐘，\
         可隨時取消）。不論從哪裡下載，啟動器都會自己核對檔案大小與 SHA-256，和上游原檔完全相同才放在：\n{}\n\n{kept}{buttons}按下載之前不會連網。發行頁：{RELEASE_PAGE}",
        pack.size as f64 / (1024.0 * 1024.0),
        exe_dir.join(FILE_NAME).display()
    )
}

/// The task dialog's buttons of `--hd-pack`: "download" first.
pub fn demand_buttons() -> [&'static str; 2] {
    ["下載", "取消"]
}

/// Downloads `pack` to `<exe_dir>\hd-pack.zip` through `<exe_dir>\hd-pack.zip.part`, which becomes it only when its
/// size and SHA-256 are right, read back from the disk before the rename and again after it; removed otherwise. An
/// `hd-pack.zip` there when the new one is placed (before the download, or put there during it) is kept under
/// [`backup_path`], and put back when the new one cannot be placed. Another launcher downloading it: [`update::BUSY`].
pub fn download(transport: &dyn Transport, pack: &Pack, exe_dir: &Path, sink: &dyn ProgressSink) -> Result<Placed, String> {
    // tried a few times, as the update's rename: an antivirus may hold a file for a moment
    download_with(transport, pack, exe_dir, sink, &|from, to| update::retry(update::RENAME_PAUSE, || std::fs::rename(from, to)))
}

fn download_with(
    transport: &dyn Transport,
    pack: &Pack,
    exe_dir: &Path,
    sink: &dyn ProgressSink,
    rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<Placed, String> {
    let part = exe_dir.join(PART_NAME);
    let target = exe_dir.join(FILE_NAME);
    let allowed: Vec<String> = update::DOWNLOAD_HOSTS.iter().map(|h| (*h).to_owned()).collect();
    let mut out = update::claim(&part)?;
    let result = (|| {
        let io = |e: std::io::Error| format!("{}：{e}", part.display());
        fetch_either(transport, pack, &allowed, &mut out, &part, sink)?;
        sink.phase(Phase::Verifying);
        out.sync_all().map_err(io)?;
        update::verify_on_disk(&part, pack.size, &pack.sha256)
    })();
    drop(out);
    let result = result.and_then(|()| {
        sink.phase(Phase::Installing);
        place(&part, &target, exe_dir, pack, rename)
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result.map(|backup| Placed { path: target, backup })
}

/// Downloads `pack` from `pack.url` into `out` (at `part`); when it cannot be had there
/// ([`mirror_may_help`]), from `pack.mirror` instead, into `out` emptied first. Both are checked alike
/// ([`update::fetch_verified`]: the same hosts, size and SHA-256).
fn fetch_either(
    transport: &dyn Transport,
    pack: &Pack,
    allowed: &[String],
    out: &mut std::fs::File,
    part: &Path,
    sink: &dyn ProgressSink,
) -> Result<(), String> {
    let fetch = |url: &str, out: &mut std::fs::File| update::fetch_verified(transport, url, allowed, pack.size, &pack.sha256, out, part, sink);
    let upstream = match fetch(pack.url, out) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    let Some(mirror) = pack.mirror.filter(|_| mirror_may_help(&upstream, part)) else { return Err(upstream) };
    // guard: the mirror's bytes start an emptied partial file
    {
        use std::io::{Seek, SeekFrom};
        out.set_len(0).and_then(|()| out.seek(SeekFrom::Start(0))).map_err(|e| format!("{}：{e}", part.display()))?;
    }
    match fetch(mirror, out) {
        Ok(()) => Ok(()),
        Err(e) if e == CANCELLED => Err(e),
        Err(e) => Err(format!("上游發行頁與備援網址都試過，都沒有取得正確的美術包。\n上游：{upstream}\n備援：{e}")),
    }
}

/// Whether `err`, from [`update::fetch_verified`] writing to `part`, means the pack could not be had (no connection,
/// a status but 200, a host off the list, a stall, bytes that are not the pack's): the mirror may have it. Not a
/// cancel (the player's), nor a failed write to `part` (a full disk, no permission: the mirror's bytes would fail
/// the same way), which `fetch_verified` reports as `<part>：<error>`.
fn mirror_may_help(err: &str, part: &Path) -> bool {
    // guard: no mirror after a cancel or a failed write
    err != CANCELLED && !err.starts_with(&format!("{}：", part.display()))
}

/// Puts the checked `part` at `target`. Whatever is at `target` at this moment (there before the download or put
/// there during it; a dangling link too) is first renamed to [`backup_path`]; the new file is checked again where it
/// landed (the partial file is not held between its check and the rename). Anything failing: the new file goes, the
/// old one is back at `target`, untouched. `Ok`: the backup's path, if one was made.
fn place(part: &Path, target: &Path, exe_dir: &Path, pack: &Pack, rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>) -> Result<Option<PathBuf>, String> {
    // decided now, not when the download began: the player may have put a file there meanwhile
    // guard: whatever is at the target now is kept
    let backup = if taken(target) {
        let backup = backup_path(exe_dir)
            .ok_or_else(|| format!("無法保留現有的 {}：{BACKUP_NAME} 到 {BACKUP_NAME}99 都已被佔用，沒有放新檔。", target.display()))?;
        // guard: the existing pack is kept
        rename(target, &backup).map_err(|e| format!("無法把現有的 {} 改名保留為 {}，沒有放新檔：{e}", target.display(), backup.display()))?;
        Some(backup)
    } else {
        None
    };
    let put_back = || {
        if let Some(backup) = &backup {
            // guard: the existing pack is put back
            let _ = rename(backup, target);
        }
    };
    if let Err(e) = rename(part, target) {
        put_back();
        return Err(format!("{}：{e}", target.display()));
    }
    // guard: the renamed pack checked again
    if let Err(e) = update::verify_on_disk(target, pack.size, &pack.sha256) {
        let _ = std::fs::remove_file(target);
        put_back();
        return Err(e);
    }
    Ok(backup)
}

/// At start: a `hd-pack.zip.part` left by a launcher that was killed is removed.
pub fn clean_stale(exe_dir: &Path) -> Option<String> {
    update::remove_stale(&exe_dir.join(PART_NAME))
}

/// The question: [`prompt_buttons`] in a task dialog, or (`yes_no`) rfd's yes / no / cancel.
pub fn prompt_text(pack: &Pack, exe_dir: &Path, yes_no: bool) -> String {
    let buttons = if yes_no {
        "「是」：下載\n「否」：不要，以後不再詢問\n「取消」：這次先不要，下次開啟再問\n\n"
    } else {
        "「這次不要」：下次開啟再問。「不再詢問」：以後不再問。\n\n"
    };
    format!(
        "沒有找到 HD 高畫質美術包。要下載嗎？\n\n\
         美術包是上游作者 JackRushante（DBZ-Super-Saiya-Densetsu-Rust）的作品，不是本繁中版的內容，\
         授權 CC BY-NC-SA 4.0。從上游的 GitHub 發行頁下載 v0.3.9 的 hd-pack.zip（上游抓不到時會改從備援下載；約 {:.0} MB，可能要幾分鐘，\
         可隨時取消）。不論從哪裡下載，啟動器都會自己核對檔案大小與 SHA-256，和上游原檔完全相同才放在：\n{}\n\n\
         {buttons}不下載也能玩，畫面用原版像素。發行頁：{RELEASE_PAGE}",
        pack.size as f64 / (1024.0 * 1024.0),
        exe_dir.join(FILE_NAME).display()
    )
}

/// The task dialog's buttons, in [`answer_of`]'s order.
pub fn prompt_buttons() -> [&'static str; 3] {
    ["下載", "這次不要", "不再詢問"]
}

/// The answer of the task dialog's button `clicked` (`None`: closed, which is "not now").
pub fn answer_of(clicked: Option<usize>) -> Answer {
    match clicked {
        Some(0) => Answer::Download,
        Some(2) => Answer::Never,
        _ => Answer::NotNow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::NoProgress;
    use crate::update::tests::{scratch, Fake, Recorder};
    use crate::update::BUSY;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    const URL: &str = "https://github.com/up/stream/releases/download/v0.3.9/hd-pack.zip";
    const ASSET: &str = "https://release-assets.githubusercontent.com/asset/9";

    const MIRROR: &str = "https://github.com/fork/stream/releases/download/hd-pack-v0.3.9/hd-pack.zip";
    const MIRROR_ASSET: &str = "https://release-assets.githubusercontent.com/asset/10";

    fn pack_of(bytes: &[u8]) -> Pack {
        Pack { url: URL, mirror: None, size: bytes.len() as u64, sha256: Sha256::digest(bytes).into() }
    }

    /// `bytes`' pack, at [`URL`] and at the mirror [`MIRROR`].
    fn mirrored(bytes: &[u8]) -> Pack {
        Pack { mirror: Some(MIRROR), ..pack_of(bytes) }
    }

    /// Why: the mirror is another GitHub release download, on a host already allowed: no host is added for it.
    #[test]
    fn the_official_pack_has_a_mirror_on_an_allowed_host() {
        assert_eq!(OFFICIAL.mirror, Some(MIRROR_URL));
        assert_ne!(MIRROR_URL, OFFICIAL.url);
        assert!(MIRROR_URL.ends_with("/hd-pack.zip"), "{MIRROR_URL}");
        assert!(update::DOWNLOAD_HOSTS.contains(&update::https_host(MIRROR_URL).unwrap().as_str()));
    }

    /// Why: when upstream's release has no pack any more (404: the release or the repo taken down), the mirror's is
    /// downloaded and placed, with nothing for the player to do.
    #[test]
    fn a_missing_upstream_pack_is_downloaded_from_the_mirror() {
        let dir = scratch("hd-mirror-404");
        let bytes = b"the pack".repeat(300);
        let fake = Fake::default().status(URL, 404).moved(MIRROR, MIRROR_ASSET).ok(MIRROR_ASSET, bytes.clone());
        let state = dir.0.join(STATE_FILE);
        let done = run(false, &state, || Answer::Download, || download(&fake, &mirrored(&bytes), &dir.0, &NoProgress));
        assert_eq!(done, Done::Downloaded(dir.0.join(FILE_NAME)));
        assert_eq!(std::fs::read(dir.0.join(FILE_NAME)).unwrap(), bytes);
        assert!(!dir.0.join(PART_NAME).exists());
        assert_eq!(fake.asked(), vec![URL, MIRROR, MIRROR_ASSET]);
    }

    /// Why: every way upstream's pack cannot be had (no connection, 410, 5xx, a redirect off the allowed hosts, a
    /// connection that stops mid-way) goes to the mirror, whose download starts again from an empty partial file.
    #[test]
    fn each_way_upstream_cannot_be_had_goes_to_the_mirror() {
        let bytes = b"the pack".repeat(300);
        let broken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cases = [
            ("unreachable", Fake::default()),
            ("410", Fake::default().status(URL, 410)),
            ("503", Fake::default().status(URL, 503)),
            ("off-list", Fake::default().moved(URL, "https://mirror.example.com/hd-pack.zip")),
            ("stopped", Fake::default().breaks(URL, bytes[..1000].to_vec(), broken.clone())),
        ];
        for (name, fake) in cases {
            let dir = scratch(&format!("hd-mirror-{name}"));
            let fake = fake.ok(MIRROR, bytes.clone());
            let placed = download(&fake, &mirrored(&bytes), &dir.0, &NoProgress).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(std::fs::read(placed.path).unwrap(), bytes, "{name}");
            assert_eq!(fake.asked().first().map(String::as_str), Some(URL), "{name}");
            assert_eq!(fake.asked().last().map(String::as_str), Some(MIRROR), "{name}");
            assert!(!dir.0.join(PART_NAME).exists(), "{name}");
        }
        assert!(broken.load(Ordering::SeqCst), "upstream's connection never stopped");
    }

    /// Why: upstream's bytes that are not the pack (a replaced asset, a byte too many) are not kept: the mirror's are
    /// written from the start of an emptied partial file, so what is placed is the mirror's pack, nothing of upstream's.
    #[test]
    fn wrong_bytes_from_upstream_go_to_the_mirror_from_an_empty_partial_file() {
        let bytes = b"the pack".repeat(300);
        for (name, upstream) in [("longer", vec![7u8; bytes.len() + 1]), ("other", vec![7u8; bytes.len()])] {
            let dir = scratch(&format!("hd-mirror-wrong-{name}"));
            let fake = Fake::default().ok(URL, upstream).ok(MIRROR, bytes.clone());
            let placed = download(&fake, &mirrored(&bytes), &dir.0, &NoProgress).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(std::fs::read(placed.path).unwrap(), bytes, "{name}");
            assert_eq!(fake.asked(), vec![URL, MIRROR], "{name}");
        }
    }

    /// Why: the mirror is trusted no more than upstream: bytes from it that are not the pack leave no file, and the
    /// message says both sources were tried.
    #[test]
    fn a_wrong_pack_from_the_mirror_leaves_no_file_and_names_both_sources() {
        let dir = scratch("hd-mirror-bad");
        let fake = Fake::default().status(URL, 404).ok(MIRROR, b"not the pack".to_vec());
        let pack = Pack { size: 12, ..mirrored(b"the real one") };
        let state = dir.0.join(STATE_FILE);
        let done = run(false, &state, || Answer::Download, || download(&fake, &pack, &dir.0, &NoProgress));
        let Done::Failed(e) = done else { panic!("{done:?}") };
        assert!(e.contains("上游") && e.contains("備援") && e.contains("HTTP 404") && e.contains("sha256 不符"), "{e}");
        assert!(!dir.0.join(FILE_NAME).exists() && !dir.0.join(PART_NAME).exists());
    }

    /// Why: with neither source reachable the player is told both were tried, each with its own reason.
    #[test]
    fn both_sources_failing_says_both_were_tried() {
        let dir = scratch("hd-mirror-both");
        let fake = Fake::default().status(URL, 404).status(MIRROR, 503);
        let err = download(&fake, &mirrored(b"the pack"), &dir.0, &NoProgress).unwrap_err();
        assert!(err.contains("上游") && err.contains("備援") && err.contains("HTTP 404") && err.contains("HTTP 503"), "{err}");
        assert!(!dir.0.join(FILE_NAME).exists() && !dir.0.join(PART_NAME).exists());
    }

    /// Why: a cancel is the player's: the mirror is not tried after a cancel of upstream's download, and a cancel of
    /// the mirror's download is a cancel (no error message); no file is left either way.
    #[test]
    fn a_cancel_does_not_try_the_mirror() {
        let bytes = vec![8u8; 400_000];
        let dir = scratch("hd-mirror-cancel");
        let fake = Fake::default().ok(URL, bytes.clone()).ok(MIRROR, bytes.clone());
        let sink = Recorder { cancel_at: Some(100_000), ..Recorder::default() };
        let done = run(false, &dir.0.join(STATE_FILE), || Answer::Download, || download(&fake, &mirrored(&bytes), &dir.0, &sink));
        assert_eq!(done, Done::Cancelled);
        assert_eq!(fake.asked(), vec![URL]);
        assert!(!dir.0.join(FILE_NAME).exists() && !dir.0.join(PART_NAME).exists());

        let other = scratch("hd-mirror-cancel-mirror");
        let fake = Fake::default().status(URL, 404).ok(MIRROR, bytes.clone());
        let sink = Recorder { cancel_at: Some(100_000), ..Recorder::default() };
        let done = run(false, &other.0.join(STATE_FILE), || Answer::Download, || download(&fake, &mirrored(&bytes), &other.0, &sink));
        assert_eq!(done, Done::Cancelled);
        assert!(!other.0.join(FILE_NAME).exists() && !other.0.join(PART_NAME).exists());
    }

    /// Why: while another launcher downloads the pack, neither source is asked.
    #[test]
    fn a_second_launcher_does_not_try_the_mirror() {
        let dir = scratch("hd-mirror-busy");
        let bytes = b"the pack".repeat(10);
        let first = update::claim(&dir.0.join(PART_NAME)).unwrap();
        let fake = Fake::default().status(URL, 404).ok(MIRROR, bytes.clone());
        assert_eq!(download(&fake, &mirrored(&bytes), &dir.0, &NoProgress).unwrap_err(), BUSY);
        assert!(fake.asked().is_empty(), "{:?}", fake.asked());
        drop(first);
    }

    /// Why: a pack upstream gives is used as it is: the mirror is never asked.
    #[test]
    fn a_pack_from_upstream_makes_no_request_to_the_mirror() {
        let dir = scratch("hd-mirror-unused");
        let bytes = b"the pack".repeat(300);
        let fake = Fake::default().moved(URL, ASSET).ok(ASSET, bytes.clone()).ok(MIRROR, bytes.clone());
        download(&fake, &mirrored(&bytes), &dir.0, &NoProgress).unwrap();
        assert_eq!(fake.asked(), vec![URL, ASSET]);
    }

    /// Why: only a pack that could not be had goes to the mirror: not a cancel, not a failed write to the partial
    /// file (a full disk, no permission: the mirror's bytes would fail the same way).
    #[test]
    fn only_a_pack_that_could_not_be_had_goes_to_the_mirror() {
        let part = Path::new("C:/Games/DBZ/hd-pack.zip.part");
        // the form of `fetch_verified`'s write error
        let write = format!("{}：{}", part.display(), std::io::Error::from(std::io::ErrorKind::StorageFull));
        assert!(!mirror_may_help(CANCELLED, part));
        assert!(!mirror_may_help(&write, part), "{write}");
        for e in ["HTTP 404：github.com", "主機不在允許清單：mirror.example.com", "下載中斷：timed out", "下載大小不符：1 位元組，應為 2", "下載檔的 sha256 不符", "connection refused"] {
            assert!(mirror_may_help(e, part), "{e}");
        }
    }

    #[test]
    fn the_official_pack_is_the_one_the_launcher_expects() {
        assert_eq!(OFFICIAL.size, 484_121_119);
        assert_eq!(crate::hex(&OFFICIAL.sha256), "e55cb05d4cb98904d4893b3597a90b7259893c34797f3b8e5b5c1cd0b621d4cc");
        assert!(OFFICIAL.url.ends_with("/v0.3.9/hd-pack.zip"));
        assert!(update::DOWNLOAD_HOSTS.contains(&update::https_host(OFFICIAL.url).unwrap().as_str()));
    }

    /// Why: a pack beside the launcher is found as the game finds it (a partial download is not one), and then
    /// nothing is asked.
    #[test]
    fn a_pack_beside_the_launcher_is_detected_and_nothing_is_asked() {
        let dir = scratch("hd-detect");
        assert_eq!(crate::hd_pack_in(&dir.0), None);
        std::fs::write(dir.0.join(PART_NAME), b"partial").unwrap();
        assert_eq!(crate::hd_pack_in(&dir.0), None, "a partial download is no pack");
        std::fs::write(dir.0.join("hd-pack-v0.3.9.zip"), b"zip").unwrap();
        assert_eq!(crate::hd_pack_in(&dir.0), Some(dir.0.join("hd-pack-v0.3.9.zip")));
        std::fs::write(dir.0.join(FILE_NAME), b"zip").unwrap();
        assert_eq!(crate::hd_pack_in(&dir.0), Some(dir.0.join(FILE_NAME)));
        let state = dir.0.join(STATE_FILE);
        let done = run(true, &state, || panic!("asked"), || panic!("downloaded"));
        assert_eq!(done, Done::Present);
        assert!(!state.exists());
    }

    /// Why: "not now" and "never" make no request; each answer is saved; after "never" nothing is asked again,
    /// after "not now" it is.
    #[test]
    fn each_answer_is_saved_and_only_download_connects() {
        let dir = scratch("hd-answers");
        let state = dir.0.join(STATE_FILE);
        let calls = Arc::new(AtomicU64::new(0));
        let dl = || {
            let c = calls.clone();
            move || {
                c.fetch_add(1, Ordering::SeqCst);
                Err("no".to_owned())
            }
        };
        assert_eq!(run(false, &state, || Answer::NotNow, dl()), Done::Answered(Answer::NotNow));
        assert_eq!(read_state(&state), Some(Answer::NotNow));
        let mut asked = false;
        run(false, &state, || { asked = true; Answer::Never }, dl());
        assert!(asked, "not now is asked again");
        assert_eq!(read_state(&state), Some(Answer::Never));
        assert_eq!(run(false, &state, || panic!("asked after never"), dl()), Done::NotAsked);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a request before the player said download");

        // download: saved, then the pack is there
        std::fs::remove_file(&state).unwrap();
        let bytes = b"the pack".repeat(100);
        let fake = Fake::default().moved(URL, ASSET).ok(ASSET, bytes.clone());
        let pack = pack_of(&bytes);
        let done = run(false, &state, || Answer::Download, || download(&fake, &pack, &dir.0, &NoProgress));
        assert_eq!(done, Done::Downloaded(dir.0.join(FILE_NAME)));
        assert_eq!(read_state(&state), Some(Answer::Download));
        assert_eq!(std::fs::read(dir.0.join(FILE_NAME)).unwrap(), bytes);
        assert!(!dir.0.join(PART_NAME).exists());
        assert_eq!(fake.asked().len(), 2);
        // the task dialog's buttons
        assert_eq!(prompt_buttons().map(|_| ()).len(), 3);
        assert_eq!([Some(0), Some(1), Some(2), None].map(answer_of), [Answer::Download, Answer::NotNow, Answer::Never, Answer::NotNow]);
    }

    /// Why: bytes whose SHA-256 is not the pack's are never left on disk (neither the zip nor the partial file).
    #[test]
    fn a_download_with_the_wrong_sha256_leaves_no_file() {
        let dir = scratch("hd-bad");
        let fake = Fake::default().ok(URL, b"not the pack".to_vec());
        let pack = Pack { size: 12, ..pack_of(b"the real one") };
        let state = dir.0.join(STATE_FILE);
        let done = run(false, &state, || Answer::Download, || download(&fake, &pack, &dir.0, &NoProgress));
        assert_eq!(done, Done::Failed("下載檔的 sha256 不符".to_owned()));
        assert!(!dir.0.join(FILE_NAME).exists() && !dir.0.join(PART_NAME).exists());
        // asked again next time: the answer was download, but there is no pack
        let mut asked = false;
        run(false, &state, || { asked = true; Answer::NotNow }, || unreachable!());
        assert!(asked);
    }

    /// Why: the pack is renamed into place only as it is on the disk, read back: a partial file changed after
    /// its bytes were checked (by another launcher, say) is removed instead. Windows: no other program can change
    /// it while the launcher holds it (from the download to its check), so the write is refused and the checked
    /// pack is placed.
    #[test]
    fn a_pack_changed_on_disk_before_the_rename_is_refused() {
        let dir = scratch("hd-tamper");
        let bytes = b"the pack".repeat(1000);
        let fake = Fake::default().ok(URL, bytes.clone());
        let sink = Recorder { tamper: Some(dir.0.join(PART_NAME)), ..Recorder::default() };
        let placed = download(&fake, &pack_of(&bytes), &dir.0, &sink);
        #[cfg(not(windows))]
        {
            assert_eq!(placed.unwrap_err(), "磁碟上的檔與驗證過的內容不同");
            assert!(!dir.0.join(FILE_NAME).exists());
        }
        #[cfg(windows)]
        {
            assert!(sink.tamper_refused.load(std::sync::atomic::Ordering::SeqCst), "the partial file was never written over");
            assert_eq!(std::fs::read(placed.unwrap().path).unwrap(), bytes);
        }
        assert!(!dir.0.join(PART_NAME).exists());
    }

    /// Why: the partial file is not held between its check and the rename; a pack that is not the checked one once
    /// renamed (replaced meanwhile) is removed, not left as hd-pack.zip.
    #[test]
    fn a_pack_replaced_between_its_check_and_the_rename_is_removed() {
        let dir = scratch("hd-race");
        let bytes = b"the pack".repeat(1000);
        let fake = Fake::default().ok(URL, bytes.clone());
        let sink = Recorder { tamper_when_installing: Some(dir.0.join(PART_NAME)), ..Recorder::default() };
        assert_eq!(download(&fake, &pack_of(&bytes), &dir.0, &sink).unwrap_err(), "磁碟上的檔與驗證過的內容不同");
        assert!(!dir.0.join(FILE_NAME).exists() && !dir.0.join(PART_NAME).exists());
    }

    /// Why: a second launcher does not download the pack again over the first one's partial file.
    #[test]
    fn a_second_launcher_is_told_the_pack_is_being_downloaded() {
        let dir = scratch("hd-busy");
        let bytes = b"the pack".repeat(10);
        let first = update::claim(&dir.0.join(PART_NAME)).unwrap();
        let fake = Fake::default().ok(URL, bytes.clone());
        assert_eq!(download(&fake, &pack_of(&bytes), &dir.0, &NoProgress).unwrap_err(), BUSY);
        assert!(fake.asked().is_empty(), "requested while the other launcher downloads");
        assert!(dir.0.join(PART_NAME).exists());
        drop(first);
    }

    /// Why: the pack is downloaded only from GitHub's release hosts, at most its size.
    #[test]
    fn a_download_off_the_list_or_too_big_leaves_no_file() {
        let dir = scratch("hd-hosts");
        let fake = Fake::default().moved(URL, "https://mirror.example.com/hd-pack.zip");
        assert!(download(&fake, &pack_of(b"x"), &dir.0, &NoProgress).unwrap_err().contains("mirror.example.com"));
        let fake = Fake::default().ok(URL, vec![1u8; 500]);
        let err = download(&fake, &pack_of(&[1u8; 100]), &dir.0, &NoProgress).unwrap_err();
        assert!(err.contains("大小"), "{err}");
        assert!(fake.pulled.load(Ordering::SeqCst) <= 101);
        assert!(!dir.0.join(FILE_NAME).exists() && !dir.0.join(PART_NAME).exists());
    }

    /// Why: the pack's progress goes forward to its size; a cancel leaves no file and asks again next time.
    #[test]
    fn the_pack_download_shows_progress_and_can_be_cancelled() {
        let dir = scratch("hd-progress");
        let bytes = vec![8u8; 400_000];
        let fake = Fake::default().ok(URL, bytes.clone());
        let sink = Recorder::default();
        download(&fake, &pack_of(&bytes), &dir.0, &sink).unwrap();
        sink.assert_monotonic_to(bytes.len() as u64);
        assert_eq!(*sink.phases.lock().unwrap(), vec![Phase::Connecting, Phase::Downloading, Phase::Verifying, Phase::Installing]);

        let other = scratch("hd-cancel");
        let fake = Fake::default().ok(URL, bytes.clone());
        let sink = Recorder { cancel_at: Some(100_000), ..Recorder::default() };
        let state = other.0.join(STATE_FILE);
        let done = run(false, &state, || Answer::Download, || download(&fake, &pack_of(&bytes), &other.0, &sink));
        assert_eq!(done, Done::Cancelled);
        assert!(!other.0.join(FILE_NAME).exists() && !other.0.join(PART_NAME).exists());
    }

    #[test]
    fn a_partial_pack_left_by_a_killed_launcher_is_removed_at_start() {
        let dir = scratch("hd-stale");
        let part = dir.0.join(PART_NAME);
        std::fs::write(&part, b"half").unwrap();
        std::fs::File::options().write(true).open(&part).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600)).unwrap();
        assert!(clean_stale(&dir.0).is_some());
        assert!(!part.exists());
        assert!(clean_stale(&dir.0).is_none());
    }

    /// Why: `--hd-pack` asks even after "never" (that is what it is for), and makes no request before "download":
    /// "cancel" leaves nothing and changes nothing.
    #[test]
    fn on_demand_asks_even_after_never_and_cancel_makes_no_request() {
        let dir = scratch("hd-demand");
        let state = dir.0.join(STATE_FILE);
        write_state(&state, Answer::Never).unwrap();
        let calls = Arc::new(AtomicU64::new(0));
        let c = calls.clone();
        let mut asked = false;
        let done = run_on_demand(false, &state, || { asked = true; false }, move || {
            c.fetch_add(1, Ordering::SeqCst);
            Err("no".to_owned())
        });
        assert!(asked, "not asked after never");
        assert_eq!(done, Demand::Declined);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a request before the player said download");
        assert!(!dir.0.join(FILE_NAME).exists() && !dir.0.join(PART_NAME).exists());
        assert_eq!(read_state(&state), Some(Answer::Never));
    }

    /// Why: with the v0.3.9 pack beside the launcher, `--hd-pack` says so: no question, no request.
    #[test]
    fn on_demand_with_the_pack_there_makes_no_request() {
        let dir = scratch("hd-demand-there");
        let done = run_on_demand(true, &dir.0.join(STATE_FILE), || panic!("asked"), || panic!("downloaded"));
        assert_eq!(done, Demand::AlreadyThere);
    }

    /// Why: after `--hd-pack` downloaded the pack, the saved "never" becomes "download" (the start agrees); a
    /// cancelled download leaves no file and the saved answer as it was.
    #[test]
    fn on_demand_downloads_and_clears_never() {
        let dir = scratch("hd-demand-dl");
        let state = dir.0.join(STATE_FILE);
        write_state(&state, Answer::Never).unwrap();
        let bytes = b"the pack".repeat(200);
        let fake = Fake::default().moved(URL, ASSET).ok(ASSET, bytes.clone());
        let pack = pack_of(&bytes);
        let done = run_on_demand(false, &state, || true, || download(&fake, &pack, &dir.0, &NoProgress));
        assert_eq!(done, Demand::Downloaded(Placed { path: dir.0.join(FILE_NAME), backup: None }));
        assert_eq!(std::fs::read(dir.0.join(FILE_NAME)).unwrap(), bytes);
        assert_eq!(read_state(&state), Some(Answer::Download));
        // at the next start: the pack is there, nothing asked
        assert_eq!(run(crate::hd_pack_in(&dir.0).is_some(), &state, || panic!("asked"), || unreachable!()), Done::Present);

        let other = scratch("hd-demand-cancel");
        let state = other.0.join(STATE_FILE);
        write_state(&state, Answer::Never).unwrap();
        let fake = Fake::default().ok(URL, vec![8u8; 300_000]);
        let sink = Recorder { cancel_at: Some(100_000), ..Recorder::default() };
        let done = run_on_demand(false, &state, || true, || download(&fake, &pack_of(&[8u8; 300_000]), &other.0, &sink));
        assert_eq!(done, Demand::Cancelled);
        assert!(!other.0.join(FILE_NAME).exists() && !other.0.join(PART_NAME).exists());
        assert_eq!(read_state(&state), Some(Answer::Never));
    }

    /// Writes `bytes` at `<dir>\hd-pack.zip` (an older pack the player put there).
    fn old_pack(dir: &Path, bytes: &[u8]) -> PathBuf {
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Why: an `hd-pack.zip` that was there (not v0.3.9, maybe the player's own) is kept, byte for byte, under
    /// `hd-pack.zip.old`, which neither the launcher nor the game take for a pack.
    #[test]
    fn a_replaced_pack_is_kept_under_the_backup_name() {
        let dir = scratch("hd-backup");
        let target = old_pack(&dir.0, b"the player's older pack");
        let bytes = b"the pack".repeat(300);
        let fake = Fake::default().ok(URL, bytes.clone());
        let placed = download(&fake, &pack_of(&bytes), &dir.0, &NoProgress).unwrap();
        let backup = dir.0.join(BACKUP_NAME);
        assert_eq!(placed, Placed { path: target.clone(), backup: Some(backup.clone()) });
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert_eq!(std::fs::read(&backup).unwrap(), b"the player's older pack");
        std::fs::remove_file(&target).unwrap();
        assert_eq!(crate::hd_pack_in(&dir.0), None, "the backup is taken for a pack");
    }

    /// Why: a backup name already taken is never overwritten: the next free one is used.
    #[test]
    fn a_taken_backup_name_is_not_overwritten() {
        let dir = scratch("hd-backup-taken");
        old_pack(&dir.0, b"older");
        std::fs::write(dir.0.join(BACKUP_NAME), b"an earlier backup").unwrap();
        assert_eq!(backup_path(&dir.0), Some(dir.0.join(format!("{BACKUP_NAME}2"))));
        let bytes = b"the pack".repeat(300);
        let fake = Fake::default().ok(URL, bytes.clone());
        let placed = download(&fake, &pack_of(&bytes), &dir.0, &NoProgress).unwrap();
        assert_eq!(placed.backup, Some(dir.0.join(format!("{BACKUP_NAME}2"))));
        assert_eq!(std::fs::read(dir.0.join(BACKUP_NAME)).unwrap(), b"an earlier backup");
        assert_eq!(std::fs::read(dir.0.join(format!("{BACKUP_NAME}2"))).unwrap(), b"older");
    }

    /// Why: when the new pack fails its check where it landed, it goes and the old one is back at hd-pack.zip.
    #[test]
    fn a_new_pack_failing_its_check_puts_the_old_one_back() {
        let dir = scratch("hd-backup-restore");
        let target = old_pack(&dir.0, b"older");
        let bytes = b"the pack".repeat(300);
        let fake = Fake::default().ok(URL, bytes.clone());
        let sink = Recorder { tamper_when_installing: Some(dir.0.join(PART_NAME)), ..Recorder::default() };
        assert!(download(&fake, &pack_of(&bytes), &dir.0, &sink).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"older");
        assert!(!dir.0.join(BACKUP_NAME).exists() && !dir.0.join(PART_NAME).exists());
    }

    /// Why: the backup is decided when the new pack is placed: an `hd-pack.zip` the player put there during the
    /// download (none at its start) is kept byte for byte, and back at its place when the new pack fails its check.
    #[test]
    fn a_pack_put_there_during_the_download_is_kept() {
        let dir = scratch("hd-backup-late");
        let target = dir.0.join(FILE_NAME);
        let bytes = b"the pack".repeat(300);
        let fake = Fake::default().ok(URL, bytes.clone());
        let sink = Recorder { create_when_installing: Some((target.clone(), b"put there meanwhile".to_vec())), ..Recorder::default() };
        let placed = download(&fake, &pack_of(&bytes), &dir.0, &sink).unwrap();
        assert_eq!(placed.backup, Some(dir.0.join(BACKUP_NAME)));
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert_eq!(std::fs::read(dir.0.join(BACKUP_NAME)).unwrap(), b"put there meanwhile");

        let other = scratch("hd-backup-late-bad");
        let target = other.0.join(FILE_NAME);
        let fake = Fake::default().ok(URL, bytes.clone());
        let sink = Recorder {
            create_when_installing: Some((target.clone(), b"put there meanwhile".to_vec())),
            tamper_when_installing: Some(other.0.join(PART_NAME)),
            ..Recorder::default()
        };
        assert!(download(&fake, &pack_of(&bytes), &other.0, &sink).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"put there meanwhile");
        assert!(!other.0.join(BACKUP_NAME).exists() && !other.0.join(PART_NAME).exists());
    }

    /// Why: a dangling link counts as taken (`exists` would say no): at the target it is kept, not replaced; as a
    /// backup name it is skipped, not overwritten.
    #[cfg(unix)]
    #[test]
    fn a_dangling_link_counts_as_taken() {
        let dir = scratch("hd-backup-link");
        let target = dir.0.join(FILE_NAME);
        std::os::unix::fs::symlink(dir.0.join("nowhere"), &target).unwrap();
        std::os::unix::fs::symlink(dir.0.join("nowhere-either"), dir.0.join(BACKUP_NAME)).unwrap();
        assert_eq!(backup_path(&dir.0), Some(dir.0.join(format!("{BACKUP_NAME}2"))));
        let bytes = b"the pack".repeat(300);
        let fake = Fake::default().ok(URL, bytes.clone());
        let placed = download(&fake, &pack_of(&bytes), &dir.0, &NoProgress).unwrap();
        assert_eq!(placed.backup, Some(dir.0.join(format!("{BACKUP_NAME}2"))));
        assert_eq!(std::fs::read(&target).unwrap(), bytes);
        assert_eq!(std::fs::read_link(dir.0.join(format!("{BACKUP_NAME}2"))).unwrap(), dir.0.join("nowhere"));
        assert_eq!(std::fs::read_link(dir.0.join(BACKUP_NAME)).unwrap(), dir.0.join("nowhere-either"));
    }

    /// Why: when the old pack cannot be renamed aside, nothing changes: it stays as it was, no new file is put.
    #[test]
    fn a_pack_that_cannot_be_kept_changes_nothing() {
        let dir = scratch("hd-backup-refused");
        let target = old_pack(&dir.0, b"older");
        let bytes = b"the pack".repeat(300);
        let fake = Fake::default().ok(URL, bytes.clone());
        let held = |from: &Path, to: &Path| {
            if from.file_name() == Some(std::ffi::OsStr::new(FILE_NAME)) {
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "in use"));
            }
            std::fs::rename(from, to)
        };
        let err = download_with(&fake, &pack_of(&bytes), &dir.0, &NoProgress, &held).unwrap_err();
        assert!(err.contains("沒有放新檔"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"older");
        assert!(!dir.0.join(BACKUP_NAME).exists() && !dir.0.join(PART_NAME).exists());
    }

    #[test]
    fn the_on_demand_question_says_whose_it_is_how_big_and_where() {
        for yes_no in [false, true] {
            let text = demand_text(&OFFICIAL, Path::new("C:/Games/DBZ"), None, yes_no);
            for words in ["上游作者", "不是本繁中版的內容", "CC BY-NC-SA 4.0", "約 462 MB", "hd-pack.zip", "按下載之前不會連網", "上游抓不到時會改從備援下載", "核對檔案大小與 SHA-256"] {
                assert!(text.contains(words), "{words}: {text}");
            }
            assert_eq!(text.contains("「確定」"), yes_no);
        }
        let other = demand_text(&OFFICIAL, Path::new("C:/D"), Some(Path::new("C:/D/hd-pack-v0.3.8.zip")), false);
        assert!(other.contains("不是 v0.3.9") && other.contains("不會被改動"), "{other}");
        let dir = scratch("hd-demand-text");
        let replaced = demand_text(&OFFICIAL, &dir.0, Some(&dir.0.join(FILE_NAME)), false);
        assert!(replaced.contains("不是 v0.3.9") && replaced.contains("改名保留為 hd-pack.zip.old") && replaced.contains("不會自動刪除"), "{replaced}");
        let none = demand_text(&OFFICIAL, &dir.0, None, false);
        assert!(none.contains("若 exe 旁已有 hd-pack.zip，會先把它改名保留"), "{none}");
        assert_eq!(demand_buttons(), ["下載", "取消"]);
    }

    #[test]
    fn the_question_says_whose_it_is_how_big_and_where() {
        for yes_no in [false, true] {
            let text = prompt_text(&OFFICIAL, Path::new("C:/Games/DBZ"), yes_no);
            for words in ["上游作者", "不是本繁中版的內容", "CC BY-NC-SA 4.0", "約 462 MB", "hd-pack.zip", "不再詢問", "下次開啟再問", "上游抓不到時會改從備援下載", "核對檔案大小與 SHA-256"] {
                assert!(text.contains(words), "{words}: {text}");
            }
            assert_eq!(text.contains("「是」"), yes_no);
        }
    }
}
