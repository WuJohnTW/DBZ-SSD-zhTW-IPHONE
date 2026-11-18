//! Embeds every file of `assets/` (but its README.md and .gitignore) into the launcher: writes
//! `$OUT_DIR/assets.rs` with one `Asset` per file (its path relative to `assets/`, its bytes by
//! `include_bytes!`, its SHA-256) and `BUNDLE_ID`, the first 12 hex digits of the SHA-256 of all of
//! them, which names the folder they are unpacked to.
//!
//! `dbz102.ips` is embedded only with the `bundle-ips` feature. A ROM (`.sfc`, `.smc`) in `assets/`
//! fails the build: the game's ROM is never shipped.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Files the launcher cannot work without.
const REQUIRED: &[&str] = &[
    "ssd-recompiled.exe",
    "zh-tw.tsv",
    "ui-zh-tw.tsv",
    // Hong Kong's tables: the launcher writes them into the data folder each run, and stops without them
    "zh-hk.tsv",
    "ui-zh-hk.tsv",
    "fonts/LXGWWenKaiTC-Regular.ttf",
    // the voice of Goku's first Super Saiyan transformation, installed into the data folder's `voice\`
    "voice/ssj-transform.wav",
];
const IPS: &str = "dbz102.ips";
/// The pictures the game draws in place of the HD art pack's (Super Vegeta without his shoulder pads), written once into the
/// data folder's `art-override\` by the launcher: at least one must be bundled.
const ART_OVERRIDE: &str = "art-override/";
const SKIPPED: &[&str] = &["README.md", ".gitignore"];
/// The exe's icon, outside `assets/` so it is not embedded as a file and does not change BUNDLE_ID:
/// Super Saiyan Goku's head over the installer's speed lines, in a rounded square (256 PNG + 128/64/48/32/16
/// 32-bit DIB): cut by `icon/make_icon.py` from the installer art's two sources (the Goku picture
/// `mode7_goku_ssj_t0_0_d.png` at 424,330 side 252, and `bg_vertical_v1.png`); the 32 and 16 px images take a tighter
/// cut, the face and the fringe (452,432 side 140). Nothing in it is drawn by the script.
/// Earlier builds used a four-star dragon ball.
const ICON: &str = "icon/app_icon.ico";

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Whether `path` (lower case) names the pixels of a keyed picture, `key-<16 hexadecimal digits>.txt`, in any folder.
fn is_keyed_pixels(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .and_then(|name| name.strip_prefix("key-"))
        .and_then(|rest| rest.strip_suffix(".txt"))
        .is_some_and(|key| key.len() == 16 && key.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets");
    println!("cargo:rerun-if-changed={}", root.display());
    println!("cargo:rerun-if-changed={ICON}");
    let bundle_ips = std::env::var_os("CARGO_FEATURE_BUNDLE_IPS").is_some();

    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut rows = Vec::new();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let rel = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        if SKIPPED.contains(&rel.as_str()) {
            continue;
        }
        let lower = rel.to_ascii_lowercase();
        assert!(
            !(lower.ends_with(".sfc") || lower.ends_with(".smc")),
            "{rel}: a ROM is in assets/; the ROM is never embedded"
        );
        // The pixels of a keyed picture's pose are the original sprite's: a release ships `key-<key>.pose`
        // (the game's `ssd_hd::pack::keyed_pose_text`) in their place
        assert!(
            !(lower.starts_with(ART_OVERRIDE) && is_keyed_pixels(&lower)),
            "{rel}: a keyed picture's pixels are in assets/; the original sprite's pixels are never embedded (ship its .pose)"
        );
        if rel == IPS && !bundle_ips {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        rows.push((rel, path, hex(&Sha256::digest(&bytes)), bytes.len()));
    }
    for name in REQUIRED.iter().copied().chain(bundle_ips.then_some(IPS)) {
        assert!(rows.iter().any(|r| r.0 == name), "assets/{name} is missing (see assets/README.md)");
    }
    assert!(
        rows.iter().any(|r| r.0.starts_with(ART_OVERRIDE) && r.0.ends_with(".png")),
        "assets/{ART_OVERRIDE}*.png is missing (see assets/README.md)"
    );

    let mut all = Sha256::new();
    let mut code = String::from("pub static ASSETS: &[Asset] = &[\n");
    for (rel, path, sha, len) in &rows {
        all.update(rel.as_bytes());
        all.update(sha.as_bytes());
        let _ = writeln!(
            code,
            "    Asset {{ path: {rel:?}, bytes: include_bytes!({:?}), sha256: {sha:?}, len: {len} }},",
            path.display().to_string()
        );
    }
    code.push_str("];\n");
    let id = hex(&all.finalize())[..12].to_owned();
    let _ = writeln!(code, "pub const BUNDLE_ID: &str = {id:?};");
    let _ = writeln!(code, "pub const HAS_IPS: bool = {bundle_ips};");
    update_source(&mut code);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("assets.rs");
    std::fs::write(out, code).unwrap();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        version_info();
    }
}

/// The public release number (the `NN` of `vNN`), in `RELEASE`: what the update check compares.
const RELEASE: &str = "RELEASE";
/// The release's name as the player sees it (for example `RC1`), in `RELEASE_NAME`: the `--help` title, the messages
/// and the exe's product version. Only `RELEASE` is compared.
const RELEASE_NAME: &str = "RELEASE_NAME";

/// The one line of `RELEASE_NAME`: letters, digits, `.` and `-`.
fn release_name() -> String {
    println!("cargo:rerun-if-changed={RELEASE_NAME}");
    let path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join(RELEASE_NAME);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let name = text.trim().to_owned();
    assert!(
        !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
        "{RELEASE_NAME}: one name of letters, digits, `.` and `-` (such as RC1), not {text:?}"
    );
    name
}
/// The build-time settings of the update check (`src/update.rs`). Both or neither: with either one
/// unset (or empty) the launcher is built without a source, and never connects.
const UPDATE_URL_VAR: &str = "DBZ_UPDATE_MANIFEST_URL";
const UPDATE_KEY_VAR: &str = "DBZ_UPDATE_PUBLIC_KEY";

/// Writes `RELEASE: u32`, `RELEASE_NAME: &str`, `UPDATE_MANIFEST_URL: Option<&str>` and
/// `UPDATE_PUBLIC_KEY: Option<[u8; 32]>`.
/// A setting that is there but malformed fails the build: a release must not lose its updates silently.
fn update_source(code: &mut String) {
    println!("cargo:rerun-if-changed={RELEASE}");
    println!("cargo:rerun-if-env-changed={UPDATE_URL_VAR}");
    println!("cargo:rerun-if-env-changed={UPDATE_KEY_VAR}");
    let path = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join(RELEASE);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let release: u32 = text.trim().parse().unwrap_or_else(|_| panic!("{RELEASE}: one whole number (the NN of vNN), not {text:?}"));
    assert!(release > 0, "{RELEASE}: 0 is not a release");
    let _ = writeln!(code, "pub const RELEASE: u32 = {release};");
    let _ = writeln!(code, "pub const RELEASE_NAME: &str = {:?};", release_name());

    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty()).map(|v| v.trim().to_owned());
    let (url, key) = (var(UPDATE_URL_VAR), var(UPDATE_KEY_VAR));
    if let Some(url) = &url {
        assert!(
            url.starts_with("https://") && url.len() > "https://".len() && url.bytes().all(|b| b.is_ascii_graphic()),
            "{UPDATE_URL_VAR}: an https:// URL in ASCII without spaces, not {url:?}"
        );
    }
    let key = key.map(|k| {
        assert!(
            k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit()),
            "{UPDATE_KEY_VAR}: the ed25519 public key as 64 hexadecimal digits (dbz-update-sign pubkey), not {k:?}"
        );
        (0..32).map(|i| u8::from_str_radix(&k[2 * i..2 * i + 2], 16).unwrap()).collect::<Vec<u8>>()
    });
    match (url, key) {
        (Some(url), Some(key)) => {
            let _ = writeln!(code, "pub const UPDATE_MANIFEST_URL: Option<&str> = Some({url:?});");
            let _ = writeln!(code, "pub const UPDATE_PUBLIC_KEY: Option<[u8; 32]> = Some({key:?});");
        }
        (url, key) => {
            if url.is_some() != key.is_some() {
                println!("cargo:warning=only one of {UPDATE_URL_VAR} and {UPDATE_KEY_VAR} is set: built without the update check");
            }
            code.push_str("pub const UPDATE_MANIFEST_URL: Option<&str> = None;\n");
            code.push_str("pub const UPDATE_PUBLIC_KEY: Option<[u8; 32]> = None;\n");
        }
    }
}

/// The application manifest: Common Controls 6 (Microsoft Learn, "Enabling Visual Styles") and asInvoker.
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

/// The exe's Windows version information (Properties > Details). The upstream's GPL-3 notice is
/// kept; the attribution of the translation is added beside it, never in its place. Also sets the
/// exe icon (ICON).
fn version_info() {
    let icon = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join(ICON);
    assert!(icon.is_file(), "{}: the exe icon is missing", icon.display());
    let mut res = winresource::WindowsResource::new();
    // forward slashes: the path goes into a .rc string, where a backslash would be an escape
    res.set_icon(&icon.to_string_lossy().replace('\\', "/"));
    res.set_language(0x0404) // Chinese (Taiwan)
        .set("ProductName", "DBZ 超賽亞傳說 繁中版")
        .set("FileDescription", "繁體中文化 By Ming Liu")
        .set(
            "LegalCopyright",
            "ssd-recompiled © JackRushante/DBZ-Super-Saiya-Densetsu-Rust 及貢獻者，GPL-3.0-or-later；繁中譯文 © Ming Liu",
        )
        .set("ProductVersion", &release_name())
        .set("OriginalFilename", "DBZ_超賽亞傳說_繁中.exe")
        .set("InternalName", "dbz-zhtw-launcher")
        .set("Comments", "本程式基於 JackRushante/DBZ-Super-Saiya-Densetsu-Rust（GPL-3）");
    // comctl32 v6 for the progress dialog and the questions' own buttons (src/progress.rs, which looks
    // TaskDialogIndirect up at run time: without this manifest the launcher still starts, with rfd's yes / no);
    // asInvoker: never elevated, whatever the exe is named
    res.set_manifest(MANIFEST);
    res.compile().expect("version resource (rc.exe of the Windows SDK)");
}
