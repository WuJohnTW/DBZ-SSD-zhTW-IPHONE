//! The launcher updates itself: at start a background thread reads the signed manifest
//! (`update_manifest.rs`), at most once a day; when it names a newer release, the player is asked before the game
//! starts — if the answer came in time, else at the next start (the manifest is kept in [`OFFER_FILE`] and checked
//! again then). The game never waits for the check. "Update now" downloads the new `.exe` with a progress dialog
//! (`progress.rs`), checks its size and SHA-256 while it comes and again from the disk, appends this `.exe`'s
//! overlay (a private copy's ROM and IPS), puts it in this `.exe`'s place and starts it.
//!
//! Built without a source (`DBZ_UPDATE_MANIFEST_URL` and `DBZ_UPDATE_PUBLIC_KEY`, see `build.rs`) — or with
//! `check_updates = false` in [`SETTINGS_FILE`] — nothing here runs: no thread, no connection, no dialog.
//!
//! What it trusts: only `https://`, only the manifest's host and [`DOWNLOAD_HOSTS`] (every redirect is followed by
//! hand and checked); a manifest only when its ed25519 signature is the built-in key's; a release only when it is
//! newer than this one ([`crate::RELEASE`]); a download only when its size and SHA-256 are the manifest's. Anything
//! else leaves this `.exe` as it is and removes the partial file. Requests are plain GETs with a fixed User-Agent:
//! nothing about the player, and never the overlay. Nothing is run but the new `.exe` itself.
//!
//! The launcher is built with `panic = "abort"`: a panic on the check's or the download's thread would end the
//! launcher. So nothing on those paths unwraps, indexes or slices unchecked; every I/O and HTTP error is a
//! `Result`. The one panic ureq has on this path (an https request with a TLS provider whose feature is off) is
//! ruled out by [`agent_config`] choosing native-tls, which a test checks.

pub use crate::update_manifest::{https_host, DOWNLOAD_HOSTS};
use crate::progress::{Phase, ProgressSink, CANCELLED};
use crate::update_manifest::{self, Manifest};
use sha2::{Digest, Sha256};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

/// The whole check (resolve, connect, the manifest).
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(4);
/// The whole of a download the player asked for.
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
/// A download that receives nothing this long fails (ureq's per-read timeout, restarted by every read).
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// The largest manifest read.
pub const MANIFEST_MAX: u64 = 256 * 1024;
const MAX_REDIRECTS: usize = 5;
/// The manifest is read at most once in this long.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// In the launcher's folder (`%LOCALAPPDATA%\dbz-ssd-zhtw\`): `check_updates`, `skipped_version`.
pub const SETTINGS_FILE: &str = "update-settings.txt";
/// In the launcher's folder: the signed manifest of a newer release, kept to be offered at the next start.
pub const OFFER_FILE: &str = "update-offer.txt";
/// In the launcher's folder: when the manifest was last read (seconds since 1970).
pub const CHECKED_FILE: &str = "update-checked.txt";
/// What a download says when another launcher is writing the same file.
pub const BUSY: &str = "另一個視窗正在下載同一個檔，請等它完成";
/// A partial file not written to for this long belongs to no running download (on Windows the open handle tells).
#[cfg(not(windows))]
const STALE_AFTER: Duration = Duration::from_secs(60);
/// How often, and how far apart, a rename is tried (an antivirus may hold a new `.exe` for a moment).
const RENAME_TRIES: u32 = 5;
pub(crate) const RENAME_PAUSE: Duration = Duration::from_millis(300);

/// The User-Agent of every request: the program and its release, nothing about the player or the machine.
pub fn user_agent() -> String {
    format!("dbz-zhtw-updater/{}", crate::RELEASE)
}

/// Where the manifest is and the key it must be signed with: both set at build time.
#[derive(Debug, Clone)]
pub struct Source {
    pub manifest_url: String,
    pub public_key: [u8; 32],
}

/// The build's source; `None` (the update check never runs) unless both settings were given to the build.
pub fn source() -> Option<Source> {
    Some(Source { manifest_url: crate::UPDATE_MANIFEST_URL?.to_owned(), public_key: crate::UPDATE_PUBLIC_KEY? })
}

/// One answer: its status, its `Location`, its body.
pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub body: Box<dyn Read + Send>,
}

/// A GET of `url`, redirects not followed. Its only input is the URL: a request has nothing else to carry.
pub trait Transport: Send {
    fn get(&self, url: &str) -> Result<Reply, String>;
}

/// The agent's settings: native-tls (the only TLS built in: SChannel on Windows) with the system's root
/// certificates, no redirects (they are followed by [`open`], which checks every hop), statuses as answers,
/// `total` for everything, `idle` for each read, the User-Agent.
pub fn agent_config(total: Duration, idle: Option<Duration>) -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(total))
        .timeout_per_read(idle)
        .user_agent(user_agent())
        .tls_config(tls)
}

/// The real transport.
pub struct Http(pub ureq::Agent);

impl Http {
    /// For the manifest: [`CHECK_TIMEOUT`] in all.
    pub fn check() -> Self {
        Http(agent_config(CHECK_TIMEOUT, None).build().into())
    }

    /// For a download: [`DOWNLOAD_TIMEOUT`] in all, [`IDLE_TIMEOUT`] without data.
    pub fn download() -> Self {
        Http(agent_config(DOWNLOAD_TIMEOUT, Some(IDLE_TIMEOUT)).build().into())
    }
}

impl Transport for Http {
    fn get(&self, url: &str) -> Result<Reply, String> {
        let reply = self.0.get(url).call().map_err(|e| e.to_string())?;
        let location = reply.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_owned);
        Ok(Reply { status: reply.status().as_u16(), location, body: Box::new(reply.into_body().into_reader()) })
    }
}

/// Where a redirect goes: an absolute `Location`, or a path on the same host.
fn redirect(from: &str, location: Option<String>) -> Result<String, String> {
    let location = location.ok_or("轉址沒有 Location")?;
    if location.starts_with('/') && !location.starts_with("//") {
        let rest = from.strip_prefix("https://").ok_or("轉址來源不是 https")?;
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        return Ok(format!("https://{host}{location}"));
    }
    Ok(location)
}

/// The hosts a request of `source`'s may go to: the manifest's and [`DOWNLOAD_HOSTS`].
pub fn allowed_hosts(source: &Source) -> Result<Vec<String>, String> {
    let mut hosts = vec![https_host(&source.manifest_url)?];
    hosts.extend(DOWNLOAD_HOSTS.iter().map(|h| (*h).to_owned()));
    Ok(hosts)
}

/// The body at `url`, following at most [`MAX_REDIRECTS`] redirects, every hop https and on an allowed host.
fn open(transport: &dyn Transport, url: &str, allowed: &[String]) -> Result<Box<dyn Read + Send>, String> {
    let mut url = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        let host = https_host(&url)?;
        // guard: host allowlist
        if !allowed.contains(&host) {
            return Err(format!("主機不在允許清單：{host}"));
        }
        let reply = transport.get(&url)?;
        match reply.status {
            200 => return Ok(reply.body),
            301 | 302 | 303 | 307 | 308 => url = redirect(&url, reply.location)?,
            status => return Err(format!("HTTP {status}：{host}")),
        }
    }
    Err("轉址次數過多".to_owned())
}

/// All of `body`, when it is at most `max` bytes; reads no more than `max + 1`.
fn read_capped(body: Box<dyn Read + Send>, max: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    // guard: manifest read cap
    body.take(max + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    // guard: manifest size limit
    if bytes.len() as u64 > max {
        return Err(format!("清單超過上限 {max} 位元組"));
    }
    Ok(bytes)
}

/// A newer release, from a manifest whose signature was checked: only [`accept`] makes one.
#[derive(Debug)]
pub struct Offer {
    manifest: Manifest,
    /// The signed manifest as read, kept in [`OFFER_FILE`].
    text: Vec<u8>,
}

impl Offer {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
}

/// The offer in a manifest `text`: `Ok(None)` when it is not newer than `current`; `Err` when it is not signed
/// with the source's key or names a download host not allowed.
pub fn accept(text: &[u8], source: &Source, current: u32) -> Result<Option<Offer>, String> {
    let manifest = update_manifest::verify(text, &source.public_key)?;
    // guard: no downgrade
    if manifest.version <= current {
        return Ok(None);
    }
    // said now rather than after the player said yes; the download checks every hop again
    let host = https_host(&manifest.url)?;
    if !allowed_hosts(source)?.contains(&host) {
        return Err(format!("下載網址的主機不在允許清單：{host}"));
    }
    Ok(Some(Offer { manifest, text: text.to_vec() }))
}

/// Reads and checks the manifest: `Ok(Some)` when it names a release newer than `current`.
pub fn check(transport: &dyn Transport, source: &Source, current: u32) -> Result<Option<Offer>, String> {
    let allowed = allowed_hosts(source)?;
    let text = read_capped(open(transport, &source.manifest_url, &allowed)?, MANIFEST_MAX)?;
    accept(&text, source, current)
}

/// What the player set in [`SETTINGS_FILE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// `false`: the launcher never checks.
    pub check_updates: bool,
    /// The release the player answered "later" to: it is not offered again; a newer one is.
    pub skipped_version: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { check_updates: true, skipped_version: 0 }
    }
}

/// The settings in `path`: the default for a missing file, a line not understood, or a value not understood.
pub fn read_settings(path: &Path) -> Settings {
    let mut settings = Settings::default();
    let text = std::fs::read_to_string(path).unwrap_or_default();
    for line in text.lines().map(str::trim).filter(|l| !l.starts_with('#')) {
        let Some((key, value)) = line.split_once('=') else { continue };
        match (key.trim(), value.trim()) {
            ("check_updates", "false") => settings.check_updates = false,
            ("check_updates", "true") => settings.check_updates = true,
            ("skipped_version", v) => settings.skipped_version = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    settings
}

pub fn write_settings(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        path,
        format!(
            "# 七龍珠Z 超賽亞傳說 繁中版：自動更新的設定\n\
             # check_updates：true＝開啟時檢查有沒有新版（一天最多一次）；false＝不檢查、不連網\n\
             check_updates = {}\n\
             # skipped_version：選了「稍後」的版本，不再提示這一版；更新的版本仍會提示\n\
             skipped_version = {}\n",
            settings.check_updates, settings.skipped_version
        ),
    )
}

fn seconds(t: SystemTime) -> u64 {
    t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether the manifest is due to be read again: never read, a file not understood, [`CHECK_INTERVAL`] gone by, or
/// a time in the future (the clock was set back).
pub fn due(checked: &Path, now: SystemTime) -> bool {
    let Some(last) = std::fs::read_to_string(checked).ok().and_then(|t| t.trim().parse::<u64>().ok()) else {
        return true;
    };
    let now = seconds(now);
    // guard: once a day
    now < last || now - last >= CHECK_INTERVAL.as_secs()
}

/// The offer kept in `path` by an earlier check, checked again (signature, version, host); the file is removed
/// when it no longer offers anything.
pub fn stored_offer(path: &Path, source: &Source, current: u32) -> Option<Offer> {
    let file = std::fs::File::open(path).ok()?;
    // read as the manifest is read: at most MANIFEST_MAX bytes
    let read = read_capped(Box::new(file), MANIFEST_MAX);
    match read.and_then(|text| accept(&text, source, current)) {
        Ok(Some(offer)) => Some(offer),
        _ => {
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

pub type Check = std::thread::JoinHandle<Result<Option<Offer>, String>>;

/// What [`start`] found: an offer kept from an earlier check, and the check running now.
#[derive(Default)]
pub struct Startup {
    pub stored: Option<Offer>,
    pub check: Option<Check>,
}

/// At start: nothing at all (no file, no thread, no connection) without a source or with `check_updates = false`.
/// Else the settings file is written with its defaults when there is none, an offer kept from an earlier check is
/// read back, and the manifest is read in the background when [`due`]. The thread keeps what it found for the
/// next start ([`OFFER_FILE`], [`CHECKED_FILE`]); a failed check is tried again at the next start.
pub fn start(
    source: Option<Source>,
    root: &Path,
    now: SystemTime,
    transport: impl FnOnce() -> Box<dyn Transport> + Send + 'static,
) -> Startup {
    let Some(source) = source else { return Startup::default() };
    let settings_path = root.join(SETTINGS_FILE);
    if !settings_path.exists() {
        let _ = write_settings(&settings_path, &Settings::default());
    }
    if !read_settings(&settings_path).check_updates {
        return Startup::default();
    }
    let (offer_path, checked_path) = (root.join(OFFER_FILE), root.join(CHECKED_FILE));
    let stored = stored_offer(&offer_path, &source, crate::RELEASE);
    if !due(&checked_path, now) {
        return Startup { stored, check: None };
    }
    let check = std::thread::Builder::new().name("update-check".to_owned()).spawn(move || {
        let result = check(&*transport(), &source, crate::RELEASE);
        match &result {
            Ok(Some(offer)) => {
                let _ = std::fs::write(&offer_path, &offer.text);
            }
            Ok(None) => {
                let _ = std::fs::remove_file(&offer_path);
            }
            Err(_) => return result,
        }
        let _ = std::fs::write(&checked_path, seconds(now).to_string());
        result
    });
    // no thread (out of resources): no check this time, rather than a panic
    Startup { stored, check: check.ok() }
}

/// The check's answer if it is there already; never waits.
pub fn poll(check: Option<Check>) -> Option<Result<Option<Offer>, String>> {
    let check = check?;
    if check.is_finished() { check.join().ok() } else { None }
}

/// What to ask (if anything) and the lines for `last-run.log`: this start's answer first, else the offer kept
/// from an earlier start; a release the player answered "later" to is not asked about.
pub fn decide(fresh: Option<Result<Option<Offer>, String>>, stored: Option<Offer>, settings: &Settings) -> (Option<Offer>, Vec<String>) {
    let mut lines = Vec::new();
    let offer = match fresh {
        Some(Ok(Some(offer))) => Some(offer),
        Some(Ok(None)) => {
            lines.push(format!("update check: v{} is the newest", crate::RELEASE));
            None
        }
        Some(Err(e)) => {
            lines.push(format!("update check failed: {e}"));
            stored
        }
        None => {
            lines.push("update check: not answered yet or not due (kept for the next start)".to_owned());
            stored
        }
    };
    match offer {
        Some(offer) if offer.manifest.version <= settings.skipped_version => {
            lines.push(format!("update check: v{} was answered later", offer.manifest.version));
            (None, lines)
        }
        offer => (offer, lines),
    }
}

/// The question: [`prompt_buttons`] in a task dialog, or (`yes_no`) rfd's yes / no.
pub fn prompt_text(m: &Manifest, settings_path: &Path, yes_no: bool) -> String {
    let buttons = if yes_no {
        "「是」：立即更新。「否」：稍後，這一版不再提示，有更新的版本時才會再提示。\n\n"
    } else {
        "選「稍後」：這一版不再提示，有更新的版本時才會再提示。\n\n"
    };
    format!(
        "有新版本：v{}（目前是 {}）。\n\n\
         立即更新會下載新版（約 {:.1} MB），驗證無誤後換掉這個程式並重新開啟；自備 ROM 的版本會保留附加的 ROM。\n\n\
         {buttons}不想檢查更新：把 {} 裡的 check_updates 改成 false。",
        m.version,
        crate::RELEASE_NAME,
        m.size as f64 / (1024.0 * 1024.0),
        settings_path.display()
    )
}

/// The task dialog's buttons for the question: "update now" first.
pub fn prompt_buttons() -> [&'static str; 2] {
    ["立即更新", "稍後"]
}

/// Beside the `.exe`: the download while it is checked, and the previous release, kept until the new one has started.
#[derive(Debug, Clone)]
pub struct Files {
    pub exe: PathBuf,
    pub staging: PathBuf,
    pub backup: PathBuf,
}

pub fn files(exe: &Path) -> Files {
    let dir = exe.parent().unwrap_or(Path::new("."));
    let stem = exe.file_stem().map_or_else(|| "launcher".into(), |s| s.to_string_lossy().into_owned());
    Files {
        exe: exe.to_path_buf(),
        staging: dir.join(format!("{stem}.update.tmp")),
        backup: dir.join(format!("{stem}.previous.exe")),
    }
}

/// Whether another launcher is writing `path` now. Windows: it holds the file open without sharing its writes
/// (see [`claim`]); else: it was written to in the last [`STALE_AFTER`].
fn in_use(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // share mode 0: refused (ERROR_SHARING_VIOLATION) while any other handle has the file open
        match std::fs::OpenOptions::new().read(true).share_mode(0).open(path) {
            Ok(_) => false,
            Err(e) => e.kind() != std::io::ErrorKind::NotFound,
        }
    }
    #[cfg(not(windows))]
    {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().map_or(true, |age| age < STALE_AFTER))
    }
}

/// Creates the partial file `path` for this launcher alone: a new file (`create_new`), shared for reading only
/// (Windows). One left by a launcher that was killed is removed first; one another launcher is writing is
/// [`BUSY`].
pub fn claim(path: &Path) -> Result<std::fs::File, String> {
    let create = || {
        let mut options = std::fs::OpenOptions::new();
        // guard: exclusive partial file
        options.read(true).write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            options.share_mode(1); // FILE_SHARE_READ
        }
        options.open(path)
    };
    match create() {
        Ok(file) => Ok(file),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            claim_left_over(path, &|p| std::fs::remove_file(p), &create)
        }
        Err(e) => Err(format!("{}：{e}", path.display())),
    }
}

/// Takes over the partial file `path` left by a launcher that was killed: removes it and creates it again. Two
/// launchers may do this at once; the one that loses is [`BUSY`] (not the system's error).
fn claim_left_over(
    path: &Path,
    remove: &dyn Fn(&Path) -> std::io::Result<()>,
    create: &dyn Fn() -> std::io::Result<std::fs::File>,
) -> Result<std::fs::File, String> {
    if in_use(path) {
        return Err(BUSY.to_owned());
    }
    match remove(path) {
        // removed here, or already by the other launcher
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(BUSY.to_owned()),
    }
    match create() {
        Ok(file) => Ok(file),
        // guard: the launcher that lost the race says BUSY
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(BUSY.to_owned()),
        Err(e) => Err(format!("{}：{e}", path.display())),
    }
}

/// Removes `path` when it is a partial file nobody is writing (left by a launcher that was killed). A line for
/// `last-run.log` when it did.
pub fn remove_stale(path: &Path) -> Option<String> {
    if !path.exists() || in_use(path) {
        return None;
    }
    Some(match std::fs::remove_file(path) {
        Ok(()) => format!("removed the partial download {}", path.display()),
        Err(e) => format!("could not remove {}: {e}", path.display()),
    })
}

/// Downloads `url` (every hop checked as [`open`] does) into `out` (at `path`), when it is `size` bytes whose
/// SHA-256 is `sha256`: reads no more than `size + 1` bytes, says how far it is to `sink` and stops when `sink`
/// is cancelled ([`CANCELLED`]). The caller removes `path` on an error.
#[allow(clippy::too_many_arguments)]
pub fn fetch_verified(
    transport: &dyn Transport,
    url: &str,
    allowed: &[String],
    size: u64,
    sha256: &[u8; 32],
    out: &mut std::fs::File,
    path: &Path,
    sink: &dyn ProgressSink,
) -> Result<(), String> {
    sink.phase(Phase::Connecting);
    sink.bytes(0, size);
    // a connection that fails after the player cancelled is the cancel, not an error to show
    let as_cancel = |e: String| if sink.cancelled() { CANCELLED.to_owned() } else { e };
    let body = open(transport, url, allowed).map_err(as_cancel)?;
    sink.phase(Phase::Downloading);
    let io = |e: std::io::Error| format!("{}：{e}", path.display());
    let mut hasher = Sha256::new();
    let mut got = 0u64;
    // guard: download read cap
    let mut body = body.take(size + 1);
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        // guard: cancel
        if sink.cancelled() {
            return Err(CANCELLED.to_owned());
        }
        // guard: a read that fails after a cancel is the cancel
        let n = body.read(&mut buf).map_err(|e| as_cancel(format!("下載中斷：{e}")))?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
        got += n as u64;
        hasher.update(chunk);
        out.write_all(chunk).map_err(io)?;
        sink.bytes(got.min(size), size);
    }
    // guard: download size
    if got != size {
        return Err(format!("下載大小不符：{got} 位元組，應為 {size}"));
    }
    // guard: download sha256
    if hasher.finalize()[..] != sha256[..] {
        return Err("下載檔的 sha256 不符".to_owned());
    }
    Ok(())
}

/// Whether the first `size` bytes of `path`, read back from the disk now, have the SHA-256 `sha256`. The file is
/// not held between this and a later rename, so what is renamed is checked again where it lands ([`install`]).
pub fn verify_on_disk(path: &Path, size: u64, sha256: &[u8; 32]) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}：{e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut taken = 0u64;
    let mut reader = file.take(size);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("{}：{e}", path.display()))?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else { break };
        taken += n as u64;
        hasher.update(chunk);
    }
    // guard: disk sha256
    if taken != size || hasher.finalize()[..] != sha256[..] {
        return Err("磁碟上的檔與驗證過的內容不同".to_owned());
    }
    Ok(())
}

/// Downloads the offered `.exe` beside `exe`, checks it and appends `exe`'s overlay: the path of the result. On
/// any error (or a cancel) the partial file is removed and `exe` is untouched; a download another launcher is
/// making is left to it ([`BUSY`]).
pub fn stage(transport: &dyn Transport, source: &Source, offer: &Offer, exe: &Path, sink: &dyn ProgressSink) -> Result<PathBuf, String> {
    let staging = files(exe).staging;
    let mut out = claim(&staging)?;
    let result = download_into(transport, source, &offer.manifest, exe, &staging, &mut out, sink);
    drop(out);
    if result.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    result.map(|()| staging)
}

fn download_into(
    transport: &dyn Transport,
    source: &Source,
    m: &Manifest,
    exe: &Path,
    staging: &Path,
    out: &mut std::fs::File,
    sink: &dyn ProgressSink,
) -> Result<(), String> {
    let allowed = allowed_hosts(source)?;
    let io = |e: std::io::Error| format!("{}：{e}", staging.display());
    fetch_verified(transport, &m.url, &allowed, m.size, &m.sha256, out, staging, sink)?;
    sink.phase(Phase::Verifying);
    // A private copy's ROM and IPS, as they are, read back before anything is swapped
    if let Some(raw) = crate::overlay::raw(exe)? {
        out.write_all(&raw).map_err(io)?;
    }
    out.sync_all().map_err(io)?;
    verify_on_disk(staging, m.size, &m.sha256)?;
    if segments(exe)? != segments(staging)? {
        return Err("新版附加的 ROM／IPS 讀回與目前的不一致".to_owned());
    }
    Ok(())
}

/// `op`, tried [`RENAME_TRIES`] times `pause` apart until it works.
pub(crate) fn retry(pause: Duration, mut op: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let mut last = op();
    // guard: rename retry
    for _ in 1..RENAME_TRIES {
        if last.is_ok() {
            break;
        }
        std::thread::sleep(pause);
        last = op();
    }
    last
}

/// Puts the staged `.exe` in `exe`'s place, the old one becoming the backup. Windows lets a running `.exe` be
/// renamed (not replaced or removed), so the old one is renamed aside first, then the new one renamed in (tried a
/// few times: an antivirus may hold it for a moment). When that fails the old one is renamed back; between the
/// two renames the backup is the old `.exe`, whole.
fn swap_with(f: &Files, pause: Duration, rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>) -> Result<(), String> {
    if f.backup.exists() {
        std::fs::remove_file(&f.backup).map_err(|e| format!("{}：{e}", f.backup.display()))?;
    }
    retry(pause, || rename(&f.exe, &f.backup)).map_err(|e| format!("無法把 {} 改名：{e}", f.exe.display()))?;
    if let Err(e) = retry(pause, || rename(&f.staging, &f.exe)) {
        let _ = retry(pause, || rename(&f.backup, &f.exe));
        let _ = std::fs::remove_file(&f.staging);
        return Err(format!("無法放入新版：{e}"));
    }
    Ok(())
}

/// The overlay segments of `path` (name, SHA-256).
fn segments(path: &Path) -> Result<Vec<(String, String)>, String> {
    Ok(crate::overlay::read(path)?.unwrap_or_default().into_iter().map(|s| (s.name, s.sha256)).collect())
}

/// Swaps the staged `.exe` in ([`swap_with`]), then checks the `.exe` now in place, read back from the disk: the
/// manifest's bytes, and the old `.exe`'s overlay (now the backup's). The staged file is not held between its
/// check and the rename (another launcher could replace it); when the result is not what was checked the old
/// `.exe` is put back.
pub fn install(f: &Files, m: &Manifest) -> Result<(), String> {
    install_with(f, m, RENAME_PAUSE, &|from, to| std::fs::rename(from, to))
}

fn install_with(f: &Files, m: &Manifest, pause: Duration, rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>) -> Result<(), String> {
    swap_with(f, pause, rename)?;
    // guard: the swapped exe checked again
    let checked = verify_on_disk(&f.exe, m.size, &m.sha256).and_then(|()| {
        if segments(&f.exe)? == segments(&f.backup)? {
            Ok(())
        } else {
            Err("新版附加的 ROM／IPS 與原本的不一致".to_owned())
        }
    });
    if let Err(e) = checked {
        // the old .exe back in its place; what was swapped in goes
        let _ = retry(pause, || rename(&f.exe, &f.staging));
        let _ = retry(pause, || rename(&f.backup, &f.exe));
        let _ = std::fs::remove_file(&f.staging);
        return Err(format!("換上的新版與驗證過的內容不同，已換回原本的版本：{e}"));
    }
    Ok(())
}

/// The whole update after "update now": download and check ([`stage`]), then swap and check again ([`install`]).
/// The player can cancel until the download is checked; the swap is never cancelled halfway.
pub fn apply(transport: &dyn Transport, source: &Source, offer: &Offer, exe: &Path, sink: &dyn ProgressSink) -> Result<(), String> {
    stage(transport, source, offer, exe, sink)?;
    sink.phase(Phase::Installing);
    install(&files(exe), &offer.manifest)
}

/// Starts the new `.exe` (no shell, no arguments). When it cannot be started the old one is put back.
pub fn restart(f: &Files) -> Result<(), String> {
    let mut cmd = Command::new(&f.exe);
    if let Some(dir) = f.exe.parent() {
        cmd.current_dir(dir);
    }
    match cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(_) => Ok(()),
        Err(e) => {
            let _ = std::fs::rename(&f.exe, &f.staging);
            let _ = std::fs::rename(&f.backup, &f.exe);
            let _ = std::fs::remove_file(&f.staging);
            Err(format!("新版無法啟動，已換回原本的版本：{e}"))
        }
    }
}

/// Once this `.exe` has started: the previous release is removed (not when this `.exe` is it), and a download
/// left by a launcher that was killed. Lines for `last-run.log`.
pub fn clean_after_update(exe: &Path) -> Vec<String> {
    let f = files(exe);
    let mut lines = Vec::new();
    if f.backup.as_path() != exe && f.backup.exists() {
        lines.push(match std::fs::remove_file(&f.backup) {
            Ok(()) => format!("update: removed {}", f.backup.display()),
            Err(e) => format!("update: could not remove {}: {e}", f.backup.display()),
        });
    }
    if f.staging.as_path() != exe {
        lines.extend(remove_stale(&f.staging));
    }
    lines
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The swap with the real rename, no pause.
    fn swap(f: &Files) -> Result<(), String> {
        swap_with(f, Duration::ZERO, &|from, to| std::fs::rename(from, to))
    }
    use crate::progress::{NoProgress, Progress};
    use crate::update_manifest::{sign, Manifest};
    use ed25519_dalek::SigningKey;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    const MANIFEST_URL: &str = "https://updates.example.org/dbz/manifest.txt";
    const EXE_URL: &str = "https://github.com/o/r/releases/download/v45/new.exe";
    const ASSET_URL: &str = "https://release-assets.githubusercontent.com/asset/1?sig=x";

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    pub(crate) fn source() -> Source {
        Source { manifest_url: MANIFEST_URL.to_owned(), public_key: *key().verifying_key().as_bytes() }
    }

    fn manifest_for(exe: &[u8], version: u32, url: &str) -> Manifest {
        Manifest { version, size: exe.len() as u64, sha256: Sha256::digest(exe).into(), url: url.to_owned() }
    }

    /// Counts the bytes taken from it.
    struct Counting<R> {
        inner: R,
        pulled: Arc<AtomicU64>,
    }
    impl<R: Read> Read for Counting<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.pulled.fetch_add(n as u64, Ordering::SeqCst);
            Ok(n)
        }
    }

    enum Body {
        Bytes(Vec<u8>),
        Endless,
        /// This many bytes, then a read error (a connection that broke), after which `broken` is set.
        Breaks(Vec<u8>, Arc<std::sync::atomic::AtomicBool>),
    }

    /// Gives its bytes, then fails once and says so.
    struct Breaking(std::io::Cursor<Vec<u8>>, Arc<std::sync::atomic::AtomicBool>);
    impl Read for Breaking {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.read(buf)?;
            if n == 0 {
                self.1.store(true, Ordering::SeqCst);
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"));
            }
            Ok(n)
        }
    }

    /// Answers from a table, records each URL asked for and every byte read.
    #[derive(Default)]
    pub(crate) struct Fake {
        routes: HashMap<String, (u16, Option<String>, Arc<Body>)>,
        asked: Arc<Mutex<Vec<String>>>,
        pub(crate) pulled: Arc<AtomicU64>,
    }
    impl Fake {
        pub(crate) fn ok(mut self, url: &str, body: Vec<u8>) -> Self {
            self.routes.insert(url.to_owned(), (200, None, Arc::new(Body::Bytes(body))));
            self
        }
        /// `url` answers `status` (a 404, a 5xx), with no body.
        pub(crate) fn status(mut self, url: &str, status: u16) -> Self {
            self.routes.insert(url.to_owned(), (status, None, Arc::new(Body::Bytes(vec![]))));
            self
        }
        pub(crate) fn breaks(mut self, url: &str, body: Vec<u8>, broken: Arc<std::sync::atomic::AtomicBool>) -> Self {
            self.routes.insert(url.to_owned(), (200, None, Arc::new(Body::Breaks(body, broken))));
            self
        }
        fn endless(mut self, url: &str) -> Self {
            self.routes.insert(url.to_owned(), (200, None, Arc::new(Body::Endless)));
            self
        }
        pub(crate) fn moved(mut self, url: &str, to: &str) -> Self {
            self.routes.insert(url.to_owned(), (302, Some(to.to_owned()), Arc::new(Body::Bytes(vec![]))));
            self
        }
        pub(crate) fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }
    impl Transport for Fake {
        fn get(&self, url: &str) -> Result<Reply, String> {
            self.asked.lock().unwrap().push(url.to_owned());
            let (status, location, body) = self.routes.get(url).cloned().ok_or_else(|| format!("no route {url}"))?;
            let pulled = self.pulled.clone();
            let body: Box<dyn Read + Send> = match &*body {
                Body::Bytes(b) => Box::new(Counting { inner: std::io::Cursor::new(b.clone()), pulled }),
                Body::Endless => Box::new(Counting { inner: std::io::repeat(b'x'), pulled }),
                Body::Breaks(b, broken) => Box::new(Breaking(std::io::Cursor::new(b.clone()), broken.clone())),
            };
            Ok(Reply { status, location, body })
        }
    }

    /// Records every report; cancels when `cancel_at` bytes are reached; may write over the partial file then.
    #[derive(Default)]
    pub(crate) struct Recorder {
        pub(crate) phases: Mutex<Vec<Phase>>,
        pub(crate) bytes: Mutex<Vec<(u64, u64)>>,
        pub(crate) cancel_at: Option<u64>,
        /// Written over, as another program would, once the last byte is reported (the launcher still holds the
        /// partial file then). Windows: that write must be refused, as [`claim`] shares the file for reading only;
        /// [`Recorder::tamper_refused`] then says so.
        pub(crate) tamper: Option<PathBuf>,
        #[cfg_attr(not(windows), allow(dead_code))] // read by the tests on Windows
        pub(crate) tamper_refused: std::sync::atomic::AtomicBool,
        /// Written over when the phase becomes Installing (after the disk check, before the rename).
        pub(crate) tamper_when_installing: Option<PathBuf>,
        /// Created (path, bytes) when the phase becomes Installing: a file the player puts there meanwhile.
        pub(crate) create_when_installing: Option<(PathBuf, Vec<u8>)>,
    }
    impl ProgressSink for Recorder {
        fn phase(&self, phase: Phase) {
            self.phases.lock().unwrap().push(phase);
            if let Some(path) = self.tamper_when_installing.as_ref().filter(|_| phase == Phase::Installing) {
                let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
                f.write_all(b"XX").unwrap();
            }
            if let Some((path, bytes)) = self.create_when_installing.as_ref().filter(|_| phase == Phase::Installing) {
                std::fs::write(path, bytes).unwrap();
            }
        }
        fn bytes(&self, done: u64, total: u64) {
            self.bytes.lock().unwrap().push((done, total));
            if let Some(path) = self.tamper.as_ref().filter(|_| done == total && done > 0) {
                let opened = std::fs::OpenOptions::new().write(true).open(path);
                #[cfg(windows)]
                {
                    // ERROR_SHARING_VIOLATION: the launcher's handle shares only reads
                    assert_eq!(opened.map(|_| ()).map_err(|e| e.raw_os_error()), Err(Some(32)));
                    self.tamper_refused.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                #[cfg(not(windows))]
                opened.unwrap().write_all(b"XX").unwrap();
            }
        }
        fn cancelled(&self) -> bool {
            let done = self.bytes.lock().unwrap().last().map_or(0, |b| b.0);
            self.cancel_at.is_some_and(|at| done >= at)
        }
    }
    impl Recorder {
        /// Reports never go back, and the last is the whole size.
        pub(crate) fn assert_monotonic_to(&self, total: u64) {
            let bytes = self.bytes.lock().unwrap().clone();
            assert!(bytes.windows(2).all(|w| w[0].0 <= w[1].0), "{bytes:?}");
            assert!(bytes.iter().all(|b| b.1 == total), "{bytes:?}");
            assert_eq!(bytes.last().map(|b| b.0), Some(total));
        }
    }

    pub(crate) struct Scratch(pub(crate) PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    pub(crate) fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("dbz-update-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    /// An overlay in the format `overlay.rs` reads.
    fn overlay(segments: &[(&str, &[u8])]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut table = Vec::new();
        for (name, bytes) in segments {
            data.extend_from_slice(bytes);
            let mut n = [0u8; 8];
            n[..name.len()].copy_from_slice(name.as_bytes());
            table.extend_from_slice(&n);
            table.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            table.extend_from_slice(&Sha256::digest(bytes));
        }
        let body_len = (data.len() + table.len()) as u64;
        data.extend(table);
        data.extend_from_slice(&(segments.len() as u32).to_le_bytes());
        data.extend_from_slice(&body_len.to_le_bytes());
        data.extend_from_slice(crate::overlay::MAGIC);
        data
    }

    fn offered(fake: &Fake) -> Offer {
        check(fake, &source(), crate::RELEASE).unwrap().expect("an offer")
    }

    fn now() -> SystemTime {
        SystemTime::now()
    }

    #[test]
    fn check_offers_a_newer_signed_release() {
        let exe = b"new exe".to_vec();
        let m = manifest_for(&exe, crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes());
        assert_eq!(offered(&fake).manifest(), &m);
        assert_eq!(fake.asked(), vec![MANIFEST_URL.to_owned()]);
    }

    /// Why: an old manifest, still validly signed, must not take the player back to an older release.
    #[test]
    fn older_or_same_version_is_not_offered() {
        for version in [1, crate::RELEASE - 1, crate::RELEASE] {
            let m = manifest_for(b"x", version, EXE_URL);
            let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes());
            assert!(check(&fake, &source(), crate::RELEASE).unwrap().is_none(), "v{version} offered");
        }
        let m = manifest_for(b"x", crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes());
        assert!(check(&fake, &source(), crate::RELEASE).unwrap().is_some());
    }

    /// Why: a manifest signed with any other key (or not signed) is no offer, and nothing is downloaded.
    #[test]
    fn manifest_signed_with_another_key_is_refused() {
        let m = manifest_for(b"x", crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &SigningKey::from_bytes(&[1; 32])).into_bytes());
        assert_eq!(check(&fake, &source(), crate::RELEASE).unwrap_err(), "清單簽章不符");
        assert_eq!(fake.asked(), vec![MANIFEST_URL.to_owned()]);
    }

    /// Why: every hop of a redirect must be on the manifest's host or a GitHub release host.
    #[test]
    fn redirect_to_an_unlisted_host_is_refused() {
        let evil = "https://evil.example.com/m.txt";
        let fake = Fake::default().moved(MANIFEST_URL, evil);
        assert_eq!(check(&fake, &source(), crate::RELEASE).unwrap_err(), "主機不在允許清單：evil.example.com");
        assert_eq!(fake.asked(), vec![MANIFEST_URL.to_owned()], "the unlisted host was asked");

        // a manifest naming an unlisted download host is no offer
        let m = manifest_for(b"x", crate::RELEASE + 1, "https://evil.example.com/new.exe");
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes());
        assert!(check(&fake, &source(), crate::RELEASE).unwrap_err().contains("evil.example.com"));

        // the download redirected off the list: refused, nothing left behind
        let dir = scratch("evil-hop");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let m = manifest_for(b"new", crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).moved(EXE_URL, "https://evil.example.com/x");
        let offer = offered(&fake);
        assert!(stage(&fake, &source(), &offer, &exe_path, &NoProgress).unwrap_err().contains("evil.example.com"));
        assert!(!files(&exe_path).staging.exists());
        assert_eq!(std::fs::read(&exe_path).unwrap(), b"old");
    }

    /// Why: http (or anything but https) is never followed, even on an allowed host.
    #[test]
    fn only_https_is_followed() {
        let fake = Fake::default().moved(MANIFEST_URL, "http://updates.example.org/dbz/manifest.txt");
        assert!(check(&fake, &source(), crate::RELEASE).unwrap_err().starts_with("不是 https 網址"));
        let plain = Source { manifest_url: "http://updates.example.org/m.txt".to_owned(), ..source() };
        assert!(check(&Fake::default(), &plain, crate::RELEASE).is_err());
        for bad in ["https://user@github.com/x", "https://github.com:8443/x", "https://[::1]/x", "ftp://github.com/x"] {
            assert!(https_host(bad).is_err(), "{bad}");
        }
        assert_eq!(https_host("https://GitHub.com:443/x?y").unwrap(), "github.com");
        assert_eq!(redirect("https://github.com/a/b", Some("/c?d".into())).unwrap(), "https://github.com/c?d");
    }

    /// Why: an endless or huge answer is cut at MANIFEST_MAX + 1 bytes and refused.
    #[test]
    fn manifest_response_over_the_limit_is_refused() {
        let fake = Fake::default().endless(MANIFEST_URL);
        let err = check(&fake, &source(), crate::RELEASE).unwrap_err();
        assert!(err.contains("上限"), "{err}");
        assert!(fake.pulled.load(Ordering::SeqCst) <= MANIFEST_MAX + 1, "read {} bytes", fake.pulled.load(Ordering::SeqCst));
    }

    /// Why: the download stops one byte past the manifest's size and is refused for its size; the partial file
    /// is removed and the launcher is untouched.
    #[test]
    fn download_larger_than_the_manifest_size_is_discarded() {
        let dir = scratch("too-big");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let new = vec![7u8; 1000];
        let m = manifest_for(&new[..400], crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).ok(EXE_URL, new);
        let offer = offered(&fake);
        let before = fake.pulled.load(Ordering::SeqCst);
        let err = stage(&fake, &source(), &offer, &exe_path, &NoProgress).unwrap_err();
        assert!(err.contains("大小"), "{err}");
        assert!(fake.pulled.load(Ordering::SeqCst) - before <= 401, "read {} bytes", fake.pulled.load(Ordering::SeqCst) - before);
        assert!(!files(&exe_path).staging.exists());
        assert_eq!(std::fs::read(&exe_path).unwrap(), b"old");
    }

    /// Why: bytes that are not the signed SHA-256 are never put in place.
    #[test]
    fn download_with_wrong_sha256_is_discarded() {
        let dir = scratch("bad-sha");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let m = manifest_for(b"the signed exe", crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).ok(EXE_URL, b"the served exe".to_vec());
        let offer = offered(&fake);
        assert_eq!(stage(&fake, &source(), &offer, &exe_path, &NoProgress).unwrap_err(), "下載檔的 sha256 不符");
        assert!(!files(&exe_path).staging.exists());
        assert_eq!(std::fs::read(&exe_path).unwrap(), b"old");
    }

    /// Why: what is swapped in is what is on the disk, read back; a file changed after its bytes were checked
    /// (another program, another launcher) is refused. Windows: no other program can change it while the launcher
    /// holds it (from the download to its check), so the write is refused and the checked bytes are staged.
    #[test]
    fn a_staged_exe_changed_on_disk_is_refused() {
        let dir = scratch("tamper");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let new = b"new exe image".repeat(10);
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).ok(EXE_URL, new.clone());
        let offer = offered(&fake);
        let sink = Recorder { tamper: Some(files(&exe_path).staging), ..Recorder::default() };
        let staged = stage(&fake, &source(), &offer, &exe_path, &sink);
        #[cfg(not(windows))]
        {
            assert_eq!(staged.unwrap_err(), "磁碟上的檔與驗證過的內容不同");
            assert!(!files(&exe_path).staging.exists());
        }
        #[cfg(windows)]
        {
            assert!(sink.tamper_refused.load(Ordering::SeqCst), "the partial file was never written over");
            assert_eq!(std::fs::read(staged.unwrap()).unwrap(), new);
        }
        assert_eq!(std::fs::read(&exe_path).unwrap(), b"old");
    }

    /// Why: the staged file is not held between its check and the rename; when it was replaced meanwhile (another
    /// launcher), the .exe swapped in is checked again and the old one, with its ROM and IPS, is put back.
    #[test]
    fn a_staged_exe_replaced_before_the_swap_leaves_the_old_exe_and_its_overlay() {
        let dir = scratch("race");
        let exe_path = dir.0.join("launcher.exe");
        let tail = overlay(&[("rom", b"MY-ROM".as_slice())]);
        let old = [b"old exe".as_slice(), &tail].concat();
        std::fs::write(&exe_path, &old).unwrap();
        let new = b"new exe image".repeat(10);
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).ok(EXE_URL, new);
        let offer = offered(&fake);
        let f = files(&exe_path);
        let sink = Recorder { tamper_when_installing: Some(f.staging.clone()), ..Recorder::default() };
        let err = apply(&fake, &source(), &offer, &exe_path, &sink).unwrap_err();
        assert!(err.contains("已換回原本的版本"), "{err}");
        assert_eq!(std::fs::read(&exe_path).unwrap(), old);
        assert_eq!(crate::overlay::read(&exe_path).unwrap().unwrap()[0].bytes, b"MY-ROM");
        assert!(!f.staging.exists() && !f.backup.exists());
    }

    /// Why: a private copy keeps its ROM and IPS; and they never leave the machine (no request carries them).
    #[test]
    fn overlay_is_carried_to_the_new_exe_and_never_sent() {
        let dir = scratch("overlay");
        let exe_path = dir.0.join("launcher.exe");
        let rom = b"ROM-BYTES-0123456789".repeat(50);
        let ips = b"PATCH-IPS-BYTES".to_vec();
        let tail = overlay(&[("rom", &rom), ("ips", &ips)]);
        std::fs::write(&exe_path, [b"old exe".as_slice(), &tail].concat()).unwrap();
        let new = b"new exe image".to_vec();
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).moved(EXE_URL, ASSET_URL).ok(ASSET_URL, new.clone());
        let offer = offered(&fake);
        let staged = stage(&fake, &source(), &offer, &exe_path, &NoProgress).unwrap();
        assert_eq!(std::fs::read(&staged).unwrap(), [new.as_slice(), &tail].concat());
        let segments = crate::overlay::read(&staged).unwrap().unwrap();
        assert_eq!(segments.iter().map(|s| (s.name.as_str(), s.bytes.as_slice())).collect::<Vec<_>>(), vec![("rom", rom.as_slice()), ("ips", ips.as_slice())]);
        assert_eq!(fake.asked(), vec![MANIFEST_URL.to_owned(), EXE_URL.to_owned(), ASSET_URL.to_owned()]);
        assert!(fake.asked().iter().all(|u| !u.contains("ROM-BYTES") && !u.contains("PATCH-IPS")));
    }

    /// Why: the progress the player sees only goes forward, ends at the file's size, and the phases come in order.
    #[test]
    fn progress_goes_forward_to_the_size_through_the_phases() {
        let dir = scratch("progress");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let new = vec![3u8; 300_000];
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).ok(EXE_URL, new.clone());
        let offer = offered(&fake);
        let sink = Recorder::default();
        apply(&fake, &source(), &offer, &exe_path, &sink).unwrap();
        sink.assert_monotonic_to(new.len() as u64);
        assert!(sink.bytes.lock().unwrap().len() >= 5, "reported in chunks");
        assert_eq!(*sink.phases.lock().unwrap(), vec![Phase::Connecting, Phase::Downloading, Phase::Verifying, Phase::Installing]);
        assert_eq!(std::fs::read(&exe_path).unwrap(), new);
    }

    /// Why: a cancel during the download stops it, leaves no partial file, and the old .exe stays as it was.
    #[test]
    fn a_cancelled_update_leaves_nothing_and_the_old_exe() {
        let dir = scratch("cancel");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let new = vec![4u8; 500_000];
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).ok(EXE_URL, new);
        let offer = offered(&fake);
        let sink = Recorder { cancel_at: Some(100_000), ..Recorder::default() };
        assert_eq!(apply(&fake, &source(), &offer, &exe_path, &sink).unwrap_err(), CANCELLED);
        let f = files(&exe_path);
        assert!(!f.staging.exists() && !f.backup.exists());
        assert_eq!(std::fs::read(&exe_path).unwrap(), b"old");
        assert!(sink.bytes.lock().unwrap().last().unwrap().0 < 500_000);
    }

    /// Why: a cancel while a read is stuck ends as the cancel (no error box), not as "download broken".
    #[test]
    fn a_read_that_fails_after_a_cancel_is_the_cancel() {
        let dir = scratch("cancel-stuck");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let new = vec![6u8; 300_000];
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let broken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).breaks(EXE_URL, new[..100_000].to_vec(), broken.clone());
        let offer = offered(&fake);
        /// The player clicks cancel while the read is stuck (here: the read fails after the click).
        struct CancelledWhenBroken(Arc<std::sync::atomic::AtomicBool>);
        impl ProgressSink for CancelledWhenBroken {
            fn phase(&self, _: Phase) {}
            fn bytes(&self, _: u64, _: u64) {}
            fn cancelled(&self) -> bool {
                self.0.load(Ordering::SeqCst)
            }
        }
        assert_eq!(apply(&fake, &source(), &offer, &exe_path, &CancelledWhenBroken(broken)).unwrap_err(), CANCELLED);
        assert!(!files(&exe_path).staging.exists());
        // without a cancel the same break is an error to show
        let broken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).breaks(EXE_URL, new[..100_000].to_vec(), broken);
        let offer = offered(&fake);
        assert!(apply(&fake, &source(), &offer, &exe_path, &NoProgress).unwrap_err().starts_with("下載中斷"));
    }

    /// Why: a cancel asked for once the download is checked changes nothing: the update is finished, not left
    /// half swapped.
    #[test]
    fn a_cancel_while_installing_is_ignored() {
        let dir = scratch("cancel-late");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let new = b"new".repeat(1000);
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).ok(EXE_URL, new.clone());
        let offer = offered(&fake);
        /// Asks to cancel as soon as the download is checked.
        struct LateCancel(Progress);
        impl ProgressSink for LateCancel {
            fn phase(&self, phase: Phase) {
                self.0.phase(phase);
                if phase >= Phase::Verifying {
                    self.0.request_cancel();
                }
            }
            fn bytes(&self, done: u64, total: u64) {
                self.0.bytes(done, total);
            }
            fn cancelled(&self) -> bool {
                self.0.cancelled()
            }
        }
        let sink = LateCancel(Progress::default());
        apply(&fake, &source(), &offer, &exe_path, &sink).unwrap();
        assert!(sink.0.cancel_requested());
        assert_eq!(std::fs::read(&exe_path).unwrap(), new);
        assert_eq!(std::fs::read(files(&exe_path).backup).unwrap(), b"old");
    }

    /// Why: two launchers never write the same partial file: the second is told another one is downloading,
    /// and does not touch the first one's file. A file left by a killed launcher is taken over.
    #[test]
    fn a_partial_file_is_claimed_by_one_launcher_only() {
        let dir = scratch("claim");
        let part = dir.0.join("x.part");
        let mut first = claim(&part).unwrap();
        first.write_all(b"first").unwrap();
        assert_eq!(claim(&part).unwrap_err(), BUSY);
        assert_eq!(std::fs::read(&part).unwrap(), b"first");
        assert_eq!(remove_stale(&part), None, "a file being written is not removed");
        drop(first);
        // left by a launcher that was killed long ago
        std::fs::File::options().write(true).open(&part).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        assert!(remove_stale(&part).is_some());
        assert!(!part.exists());
        std::fs::write(&part, b"stale").unwrap();
        std::fs::File::options().write(true).open(&part).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        let mut taken = claim(&part).unwrap();
        taken.write_all(b"new").unwrap();
        drop(taken);
        assert_eq!(std::fs::read(&part).unwrap(), b"new");
    }

    /// Why: two launchers taking over the same left-over file at once: the one that loses says BUSY, in words,
    /// not the system's "file exists".
    #[test]
    fn the_launcher_that_loses_the_race_for_a_left_over_file_says_busy() {
        let dir = scratch("race-claim");
        let part = dir.0.join("x.part");
        std::fs::write(&part, b"stale").unwrap();
        std::fs::File::options().write(true).open(&part).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        // the other launcher removes it and creates it again between this one's remove and create
        let remove_then_other_wins = |p: &Path| {
            std::fs::remove_file(p)?;
            std::fs::write(p, b"theirs")
        };
        let create = || std::fs::OpenOptions::new().write(true).create_new(true).open(&part);
        assert_eq!(claim_left_over(&part, &remove_then_other_wins, &create).unwrap_err(), BUSY);
        assert_eq!(std::fs::read(&part).unwrap(), b"theirs");
        // the other launcher removed it first: this one goes on
        std::fs::remove_file(&part).unwrap();
        let already_gone = |_: &Path| Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(claim_left_over(&part, &already_gone, &create).is_ok());
    }

    /// Why: a second launcher updating at the same time leaves the first one's download alone.
    #[test]
    fn a_second_update_at_the_same_time_is_busy() {
        let dir = scratch("busy");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let new = b"new".repeat(100);
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes()).ok(EXE_URL, new);
        let offer = offered(&fake);
        let other = claim(&files(&exe_path).staging).unwrap();
        assert_eq!(stage(&fake, &source(), &offer, &exe_path, &NoProgress).unwrap_err(), BUSY);
        assert!(files(&exe_path).staging.exists(), "the other launcher's file was removed");
        drop(other);
    }

    /// Why: without a source, or with check_updates = false, nothing connects and nothing is asked.
    #[test]
    fn not_configured_or_switched_off_never_connects() {
        let dir = scratch("off");
        let called = Arc::new(AtomicU64::new(0));
        let factory = |c: Arc<AtomicU64>| {
            move || -> Box<dyn Transport> {
                c.fetch_add(1, Ordering::SeqCst);
                Box::new(Fake::default())
            }
        };
        let s = start(None, &dir.0, now(), factory(called.clone()));
        assert!(s.check.is_none() && s.stored.is_none());
        assert!(!dir.0.join(SETTINGS_FILE).exists(), "a build without a source writes no settings");
        write_settings(&dir.0.join(SETTINGS_FILE), &Settings { check_updates: false, skipped_version: 0 }).unwrap();
        let s = start(Some(source()), &dir.0, now(), factory(called.clone()));
        assert!(s.check.is_none() && s.stored.is_none());
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(called.load(Ordering::SeqCst), 0);
        // and this build's own source: none unless both settings were given to the build
        assert_eq!(super::source().is_some(), crate::UPDATE_MANIFEST_URL.is_some() && crate::UPDATE_PUBLIC_KEY.is_some());
    }

    /// Why: the manifest is read at most once a day; a check that failed is tried again at the next start.
    #[test]
    fn the_manifest_is_read_at_most_once_a_day() {
        let dir = scratch("daily");
        let checked = dir.0.join(CHECKED_FILE);
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert!(due(&checked, t0));
        std::fs::write(&checked, seconds(t0).to_string()).unwrap();
        assert!(!due(&checked, t0 + Duration::from_secs(3600)));
        assert!(due(&checked, t0 + CHECK_INTERVAL));
        assert!(due(&checked, t0 - Duration::from_secs(10)), "clock set back");

        let m = manifest_for(b"x", crate::RELEASE + 1, EXE_URL);
        let fake = Fake::default().ok(MANIFEST_URL, sign(&m, &key()).into_bytes());
        let asked = fake.asked.clone();
        std::fs::remove_file(&checked).unwrap();
        let s = start(Some(source()), &dir.0, t0, move || Box::new(fake));
        assert!(s.check.unwrap().join().unwrap().unwrap().is_some());
        assert_eq!(std::fs::read_to_string(&checked).unwrap(), seconds(t0).to_string());
        // an hour later: no request; the offer kept from the check is there
        let s = start(Some(source()), &dir.0, t0 + Duration::from_secs(3600), || panic!("connected again"));
        assert!(s.check.is_none());
        assert_eq!(s.stored.unwrap().manifest(), &m);
        assert_eq!(asked.lock().unwrap().len(), 1);

        // a failed check leaves no mark: tried again next time
        let failing = scratch("daily-fail");
        let s = start(Some(source()), &failing.0, t0, || Box::new(Fake::default()));
        assert!(s.check.unwrap().join().unwrap().is_err());
        assert!(!failing.0.join(CHECKED_FILE).exists());
    }

    /// Why: an answer that comes after the game started is offered at the next start, from the kept manifest
    /// checked again; a kept manifest that is not newer any more (or not signed) is dropped.
    #[test]
    fn a_late_answer_is_offered_next_time() {
        let dir = scratch("late");
        let m = manifest_for(b"x", crate::RELEASE + 1, EXE_URL);
        let text = sign(&m, &key());
        std::fs::write(dir.0.join(OFFER_FILE), &text).unwrap();
        let (offer, lines) = decide(None, stored_offer(&dir.0.join(OFFER_FILE), &source(), crate::RELEASE), &Settings::default());
        assert_eq!(offer.unwrap().manifest(), &m);
        assert!(lines[0].contains("not answered yet"), "{lines:?}");
        // after updating to it: nothing, the file is removed
        assert!(stored_offer(&dir.0.join(OFFER_FILE), &source(), crate::RELEASE + 1).is_none());
        assert!(!dir.0.join(OFFER_FILE).exists());
        std::fs::write(dir.0.join(OFFER_FILE), text.replace("version", "version ")).unwrap();
        assert!(stored_offer(&dir.0.join(OFFER_FILE), &source(), crate::RELEASE).is_none());
        // this start's answer wins over the kept one
        let fresh = Offer { manifest: manifest_for(b"y", crate::RELEASE + 2, EXE_URL), text: vec![] };
        let kept = Offer { manifest: m.clone(), text: vec![] };
        let (offer, _) = decide(Some(Ok(Some(fresh))), Some(kept), &Settings::default());
        assert_eq!(offer.unwrap().manifest().version, crate::RELEASE + 2);
        let kept = Offer { manifest: m, text: vec![] };
        assert!(decide(Some(Ok(None)), Some(kept), &Settings::default()).0.is_none());
    }

    /// Why: the kept manifest is read as the manifest is (at most MANIFEST_MAX bytes): a huge file is dropped
    /// without being read whole.
    #[test]
    fn a_huge_kept_offer_is_dropped_without_reading_it() {
        let dir = scratch("huge-offer");
        let path = dir.0.join(OFFER_FILE);
        std::fs::write(&path, vec![b'a'; 300 * 1024]).unwrap();
        assert!(stored_offer(&path, &source(), crate::RELEASE).is_none());
        assert!(!path.exists());
        // 2 GiB (sparse): read whole it would take seconds and the memory
        std::fs::File::create(&path).unwrap().set_len(2 << 30).unwrap();
        let t = std::time::Instant::now();
        assert!(stored_offer(&path, &source(), crate::RELEASE).is_none());
        assert!(t.elapsed() < Duration::from_millis(500), "read for {:?}", t.elapsed());
        assert!(!path.exists());
    }

    /// Why: the game never waits for the check: an answer not there yet is not waited for.
    #[test]
    fn the_answer_is_never_waited_for() {
        let slow = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(500));
            Ok(None)
        });
        let t = std::time::Instant::now();
        assert!(poll(Some(slow)).is_none());
        assert!(t.elapsed() < Duration::from_millis(100));
        let quick = std::thread::spawn(|| Ok(None));
        std::thread::sleep(Duration::from_millis(50));
        assert!(matches!(poll(Some(quick)), Some(Ok(None))));
    }

    #[test]
    fn settings_round_trip_and_tolerate_junk() {
        let dir = scratch("settings");
        let path = dir.0.join(SETTINGS_FILE);
        assert_eq!(read_settings(&path), Settings::default());
        let s = Settings { check_updates: false, skipped_version: 47 };
        write_settings(&path, &s).unwrap();
        assert_eq!(read_settings(&path), s);
        std::fs::write(&path, "junk\ncheck_updates = maybe\nskipped_version = x\n").unwrap();
        assert_eq!(read_settings(&path), Settings::default());
    }

    /// Why: the release the player answered "later" to is not asked about again; a newer one is.
    #[test]
    fn a_skipped_version_is_not_asked_again() {
        let offer = |v: u32| Some(Ok(Some(Offer { manifest: manifest_for(b"x", v, EXE_URL), text: vec![] })));
        let skipped = Settings { check_updates: true, skipped_version: crate::RELEASE + 1 };
        assert!(decide(offer(crate::RELEASE + 1), None, &skipped).0.is_none());
        assert!(decide(offer(crate::RELEASE + 2), None, &skipped).0.is_some());
        assert!(decide(offer(crate::RELEASE + 1), None, &Settings::default()).0.is_some());
        assert!(decide(None, None, &Settings::default()).0.is_none());
        assert!(decide(Some(Err("x".into())), None, &Settings::default()).0.is_none());
    }

    /// Why: the old .exe stays startable: as the backup once moved aside, and back in its place when the new one
    /// cannot be put there.
    #[test]
    fn swap_keeps_the_old_exe_until_the_new_one_is_in_place() {
        let dir = scratch("swap");
        let f = files(&dir.0.join("launcher.exe"));
        std::fs::write(&f.exe, b"old").unwrap();
        std::fs::write(&f.staging, b"new").unwrap();
        swap(&f).unwrap();
        assert_eq!(std::fs::read(&f.exe).unwrap(), b"new");
        assert_eq!(std::fs::read(&f.backup).unwrap(), b"old");

        // a second update replaces the backup
        std::fs::write(&f.staging, b"newer").unwrap();
        swap(&f).unwrap();
        assert_eq!(std::fs::read(&f.exe).unwrap(), b"newer");
        assert_eq!(std::fs::read(&f.backup).unwrap(), b"new");

        // the new one cannot be put in place (here: it is gone): the old one is back, nothing else changed
        std::fs::remove_file(&f.backup).unwrap();
        assert!(swap_with(&f, Duration::ZERO, &|a, b| std::fs::rename(a, b)).is_err());
        assert_eq!(std::fs::read(&f.exe).unwrap(), b"newer");
        assert!(!f.backup.exists() && !f.staging.exists());
        assert_eq!(f.backup.file_name().unwrap(), "launcher.previous.exe");
    }

    /// Why: a new .exe held for a moment (an antivirus scanning it) is put in place after a few tries, not
    /// given up on at once.
    #[test]
    fn a_rename_refused_a_few_times_is_tried_again() {
        let dir = scratch("retry");
        let f = files(&dir.0.join("launcher.exe"));
        std::fs::write(&f.exe, b"old").unwrap();
        std::fs::write(&f.staging, b"new").unwrap();
        let refusals = std::cell::Cell::new(0);
        let held = |from: &Path, to: &Path| {
            if from == f.staging && refusals.get() < 2 {
                refusals.set(refusals.get() + 1);
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "in use"));
            }
            std::fs::rename(from, to)
        };
        swap_with(&f, Duration::ZERO, &held).unwrap();
        assert_eq!(refusals.get(), 2);
        assert_eq!(std::fs::read(&f.exe).unwrap(), b"new");
        assert_eq!(std::fs::read(&f.backup).unwrap(), b"old");
    }

    #[test]
    fn restart_that_fails_puts_the_old_exe_back() {
        let dir = scratch("restart");
        let f = files(&dir.0.join("launcher.exe"));
        std::fs::write(&f.exe, b"not a program").unwrap();
        std::fs::write(&f.backup, b"old").unwrap();
        assert!(restart(&f).is_err());
        assert_eq!(std::fs::read(&f.exe).unwrap(), b"old");
        assert!(!f.backup.exists() && !f.staging.exists());
    }

    #[test]
    fn the_backup_is_removed_once_the_new_exe_runs_but_not_by_itself() {
        let dir = scratch("clean");
        let f = files(&dir.0.join("launcher.exe"));
        std::fs::write(&f.exe, b"new").unwrap();
        std::fs::write(&f.backup, b"old").unwrap();
        std::fs::write(&f.staging, b"partial").unwrap();
        std::fs::File::options().write(true).open(&f.staging).unwrap().set_modified(SystemTime::now() - Duration::from_secs(3600)).unwrap();
        // started as the backup: removes nothing of the other
        assert!(clean_after_update(&f.backup).is_empty());
        assert!(f.backup.exists());
        assert_eq!(clean_after_update(&f.exe).len(), 2);
        assert!(!f.backup.exists() && !f.staging.exists() && f.exe.exists());
        assert!(clean_after_update(&f.exe).is_empty());
    }

    /// Why: the build has native-tls only; an agent asking for another provider panics on its first https request,
    /// and panic = "abort" takes the launcher with it.
    #[test]
    fn the_production_agent_uses_native_tls() {
        for http in [Http::check(), Http::download()] {
            assert_eq!(http.0.config().tls_config().provider(), ureq::tls::TlsProvider::NativeTls);
        }
    }

    /// Why: an https request of the production agent ends in an Err, never a panic (here: a server that does
    /// not speak TLS).
    #[test]
    fn an_https_request_fails_without_panicking() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 512];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nno");
            }
        });
        let url = format!("https://127.0.0.1:{port}/manifest.txt");
        for http in [Http::check(), Http::download()] {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| http.get(&url).map(|r| r.status)));
            assert!(matches!(outcome, Ok(Err(_))), "{:?}", outcome.as_ref().map_err(|_| "panicked"));
        }
    }

    #[test]
    fn the_production_agents_are_https_only_without_redirects() {
        let check = Http::check();
        let config = check.0.config();
        assert!(config.https_only());
        assert_eq!(config.max_redirects(), 0);
        assert!(!config.http_status_as_error());
        assert_eq!(config.timeouts().global, Some(CHECK_TIMEOUT));
        let download = Http::download();
        assert_eq!(download.0.config().timeouts().global, Some(DOWNLOAD_TIMEOUT));
        assert_eq!(download.0.config().timeout_per_read(), Some(IDLE_TIMEOUT));
        assert!(download.0.config().https_only());
        assert_eq!(user_agent(), format!("dbz-zhtw-updater/{}", crate::RELEASE));
    }

    #[test]
    fn the_prompt_names_the_versions_and_the_setting() {
        let m = manifest_for(&vec![0u8; 3 * 1024 * 1024], crate::RELEASE + 1, EXE_URL);
        for yes_no in [false, true] {
            let text = prompt_text(&m, Path::new("C:/x/update-settings.txt"), yes_no);
            assert!(text.contains(&format!("v{}（目前是 {}）", crate::RELEASE + 1, crate::RELEASE_NAME)), "{text}");
            assert!(text.contains("約 3.0 MB") && text.contains("check_updates") && text.contains("稍後"), "{text}");
            assert_eq!(text.contains("「是」"), yes_no);
        }
        assert_eq!(prompt_buttons(), ["立即更新", "稍後"]);
    }

    /// A loopback HTTP server: `/<host>/<path>` answers what `routes` says for `https://<host>/<path>`; every
    /// request head is recorded. A route's `stall` sends that many bytes of the body, then nothing.
    pub(crate) fn serve(routes: Vec<(String, u16, Option<String>, Vec<u8>)>, stall: Option<usize>) -> (u16, Arc<Mutex<Vec<String>>>) {
        use std::io::BufRead as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let heads = Arc::new(Mutex::new(Vec::new()));
        let log = heads.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                let path = head.split(' ').nth(1).unwrap_or("").to_owned();
                log.lock().unwrap().push(head);
                let url = format!("https:/{path}");
                let (status, location, body) = routes
                    .iter()
                    .find(|r| r.0 == url)
                    .map(|r| (r.1, r.2.clone(), r.3.clone()))
                    .unwrap_or((404, None, vec![]));
                let mut answer = format!("HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                if let Some(l) = location {
                    answer.push_str(&format!("Location: {l}\r\n"));
                }
                answer.push_str("\r\n");
                let _ = stream.write_all(answer.as_bytes());
                match stall {
                    Some(n) if status == 200 => {
                        let _ = stream.write_all(&body[..n.min(body.len())]);
                        let _ = stream.flush();
                        std::thread::sleep(Duration::from_secs(5));
                    }
                    _ => {
                        let _ = stream.write_all(&body);
                    }
                }
            }
        });
        (port, heads)
    }

    /// The real agent (ureq, the production settings but https_only and the environment's proxy), sent to the
    /// loopback server instead of the URL's host: TLS is the only part not run.
    pub(crate) struct Loopback(pub(crate) ureq::Agent, pub(crate) u16);
    impl Transport for Loopback {
        fn get(&self, url: &str) -> Result<Reply, String> {
            let local = format!("http://127.0.0.1:{}/{}", self.1, url.strip_prefix("https://").unwrap());
            Http(self.0.clone()).get(&local)
        }
    }

    pub(crate) fn loopback_agent(total: Duration, idle: Option<Duration>) -> ureq::Agent {
        agent_config(total, idle).https_only(false).proxy(None).build().into()
    }

    /// Why: a download that stops sending fails after the idle timeout (not after the whole 2 hours), and its
    /// partial file is removed.
    #[test]
    fn a_stalled_download_fails_after_the_idle_timeout() {
        let dir = scratch("stall");
        let exe_path = dir.0.join("launcher.exe");
        std::fs::write(&exe_path, b"old").unwrap();
        let new = vec![5u8; 200_000];
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let offer = Offer { manifest: m, text: vec![] };
        let (port, _) = serve(vec![(EXE_URL.to_owned(), 200, None, new)], Some(50_000));
        let agent = loopback_agent(Duration::from_secs(60), Some(Duration::from_millis(400)));
        let t = std::time::Instant::now();
        let err = stage(&Loopback(agent, port), &source(), &offer, &exe_path, &NoProgress).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(4), "waited {:?}: {err}", t.elapsed());
        assert!(err.contains("下載中斷"), "{err}");
        assert!(!files(&exe_path).staging.exists());
        assert_eq!(std::fs::read(&exe_path).unwrap(), b"old");
    }

    /// End to end on this machine: a source like a configured build's, a loopback server playing the manifest's
    /// host and a GitHub release (with its 302), the real HTTP client. Check, download with progress, verify,
    /// append the overlay, swap.
    #[test]
    fn end_to_end_over_loopback_http() {
        let dir = scratch("e2e");
        let root = dir.0.join("root");
        let exe_path = dir.0.join("DBZ_launcher.exe");
        let tail = overlay(&[("rom", b"SECRET-ROM-CONTENT".as_slice())]);
        std::fs::write(&exe_path, [b"MZ old launcher".as_slice(), &tail].concat()).unwrap();
        let new = b"MZ new launcher image ".repeat(5000);
        let m = manifest_for(&new, crate::RELEASE + 1, EXE_URL);
        let manifest = sign(&m, &key());
        let (port, heads) = serve(
            vec![
                (MANIFEST_URL.to_owned(), 200, None, manifest.clone().into_bytes()),
                (EXE_URL.to_owned(), 302, Some(ASSET_URL.to_owned()), vec![]),
                (ASSET_URL.to_owned(), 200, None, new.clone()),
            ],
            None,
        );
        let agent = loopback_agent(CHECK_TIMEOUT, None);
        let a = agent.clone();
        let started = start(Some(source()), &root, now(), move || Box::new(Loopback(a, port)));
        let check = started.check.expect("check started");
        while !check.is_finished() {
            std::thread::sleep(Duration::from_millis(10));
        }
        let (offer, lines) = decide(poll(Some(check)), started.stored, &read_settings(&root.join(SETTINGS_FILE)));
        let offer = offer.expect("an offer");
        println!("e2e manifest served:\n{manifest}");
        println!("e2e offer: v{} size {} url {} {lines:?}", offer.manifest().version, offer.manifest().size, offer.manifest().url);
        let sink = Recorder::default();
        apply(&Loopback(loopback_agent(DOWNLOAD_TIMEOUT, Some(IDLE_TIMEOUT)), port), &source(), &offer, &exe_path, &sink).unwrap();
        let bytes = sink.bytes.lock().unwrap().clone();
        println!("e2e progress: {} reports, first {:?}, last {:?}; phases {:?}", bytes.len(), bytes.first(), bytes.last(), sink.phases.lock().unwrap());
        sink.assert_monotonic_to(new.len() as u64);
        let f = files(&exe_path);
        println!(
            "e2e swapped: {} now {} bytes (exe {} + overlay {}); backup {} {} bytes",
            f.exe.display(),
            std::fs::metadata(&f.exe).unwrap().len(),
            new.len(),
            tail.len(),
            f.backup.display(),
            std::fs::metadata(&f.backup).unwrap().len()
        );
        assert_eq!(std::fs::read(&f.exe).unwrap(), [new.as_slice(), &tail].concat());
        assert_eq!(std::fs::read(root.join(OFFER_FILE)).unwrap(), manifest.as_bytes(), "the offer is kept");
        assert!(root.join(CHECKED_FILE).exists());
        let heads = heads.lock().unwrap().clone();
        for h in &heads {
            println!("e2e request:\n{}", h.trim_end());
        }
        assert_eq!(heads.len(), 3);
        for h in &heads {
            assert!(h.starts_with("GET /"), "{h}");
            let lower = h.to_ascii_lowercase();
            assert!(lower.contains(&format!("user-agent: dbz-zhtw-updater/{}\r\n", crate::RELEASE)), "{h}");
            for header in ["cookie", "authorization", "content-length", "referer", "x-"] {
                assert!(!lower.lines().any(|l| l.starts_with(header)), "{header} in {h}");
            }
            assert!(!h.contains("SECRET-ROM"), "{h}");
        }
        assert_eq!(read_settings(&root.join(SETTINGS_FILE)), Settings::default(), "the settings file is written with its defaults");
    }
}
