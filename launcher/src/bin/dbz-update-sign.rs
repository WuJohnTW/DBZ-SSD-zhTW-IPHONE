//! `dbz-update-sign`: writes the signed update manifest the launcher checks (`update_manifest.rs`). Built only with
//! the `signer` feature, never into the launcher:
//!
//! ```text
//! cargo run --features signer --bin dbz-update-sign -- pubkey <private key file>
//! cargo run --features signer --bin dbz-update-sign -- sign --exe <exe> --version <NN> --url <https://...> --key <private key file> --out <manifest>
//! ```
//!
//! The private key file holds the 32-byte ed25519 seed as 64 hexadecimal digits (`openssl rand -hex 32 > key.txt`).
//! It is only read, from the path given; keep it out of the repository. An `.exe` with an overlay (a private copy's
//! ROM and IPS) is refused: only the public `.exe` is published.

#[allow(dead_code)] // only `read` is used here
#[path = "../overlay.rs"]
mod overlay;
#[path = "../update_manifest.rs"]
mod update_manifest;

use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use std::io::Read as _;
use std::path::Path;
use std::process::ExitCode;
use update_manifest::Manifest;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn read_key(path: &Path) -> Result<SigningKey, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let seed = update_manifest::unhex::<32>(&text.trim().to_ascii_lowercase())
        .ok_or_else(|| format!("{}: not 64 hexadecimal digits", path.display()))?;
    Ok(SigningKey::from_bytes(&seed))
}

/// `url` with every byte outside printable ASCII (and the space) percent-encoded.
fn encode_url(url: &str) -> String {
    url.bytes()
        .map(|b| if b.is_ascii_graphic() { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn size_and_sha256(exe: &Path) -> Result<(u64, [u8; 32]), String> {
    let mut f = std::fs::File::open(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok((size, hasher.finalize().into()))
}

/// The manifest of `exe`, signed with the key in `key`, written to `out`; read back and checked.
/// Whether the launcher will download from `url`: its host is a GitHub release host
/// (`update_manifest::DOWNLOAD_HOSTS`) or the manifest's own (`manifest_url`, the build's
/// `DBZ_UPDATE_MANIFEST_URL`).
fn url_allowed(url: &str, manifest_url: Option<&str>) -> Result<(), String> {
    let host = update_manifest::https_host(url)?;
    let manifest_host = manifest_url.map(update_manifest::https_host).transpose()?;
    // guard: signer host allowlist
    if update_manifest::DOWNLOAD_HOSTS.contains(&host.as_str()) || manifest_host.as_deref() == Some(host.as_str()) {
        return Ok(());
    }
    Err(format!("--url: the launcher downloads only from {} or the manifest's host (--manifest-url), not {host}", update_manifest::DOWNLOAD_HOSTS.join(", ")))
}

fn sign(exe: &Path, version: u32, url: &str, manifest_url: Option<&str>, key: &Path, out: &Path) -> Result<String, String> {
    if version == 0 {
        return Err("--version: the NN of vNN, from 1".to_owned());
    }
    if overlay::read(exe)?.is_some_and(|s| !s.is_empty()) {
        return Err(format!("{}: has an overlay (a ROM or IPS): only the public .exe is published", exe.display()));
    }
    let url = encode_url(url);
    if !update_manifest::url_ok(&url) {
        return Err(format!("--url: an https:// URL, not {url:?}"));
    }
    url_allowed(&url, manifest_url)?;
    let key = read_key(key)?;
    let (size, sha256) = size_and_sha256(exe)?;
    if size > update_manifest::MAX_EXE_SIZE {
        return Err(format!("{}: {size} bytes, over the launcher's limit {}", exe.display(), update_manifest::MAX_EXE_SIZE));
    }
    let text = update_manifest::sign(&Manifest { version, size, sha256, url }, &key);
    update_manifest::verify(text.as_bytes(), key.verifying_key().as_bytes())?;
    std::fs::write(out, &text).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(format!("{}written {} (public key {})\n", text, out.display(), update_manifest::public_key_hex(&key)))
}

fn run(args: &[String]) -> Result<String, String> {
    let usage = "usage: dbz-update-sign pubkey <key file>\n       dbz-update-sign sign --exe <exe> --version <NN> --url <https://...> --key <key file> --out <manifest> [--manifest-url <DBZ_UPDATE_MANIFEST_URL>]";
    match args.first().map(String::as_str) {
        Some("pubkey") if args.len() == 2 => {
            Ok(format!("{}\n", update_manifest::public_key_hex(&read_key(Path::new(&args[1]))?)))
        }
        Some("sign") => {
            let opt = |name: &str| -> Result<&str, String> {
                let i = args.iter().position(|a| a == name).ok_or_else(|| format!("{name} is missing\n{usage}"))?;
                args.get(i + 1).map(String::as_str).ok_or_else(|| format!("{name} has no value"))
            };
            let version = opt("--version")?.trim_start_matches('v').parse().map_err(|_| "--version: a whole number".to_owned())?;
            let manifest_url = opt("--manifest-url").ok();
            sign(Path::new(opt("--exe")?), version, opt("--url")?, manifest_url, Path::new(opt("--key")?), Path::new(opt("--out")?))
        }
        _ => Err(usage.to_owned()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("dbz-update-sign: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod signer_tests {
    use super::*;

    /// Why: a manifest naming a host the launcher refuses would be signed for nothing; the signer says so first.
    #[test]
    fn a_url_off_the_launchers_list_is_refused() {
        assert!(url_allowed("https://github.com/o/r/releases/download/v45/a.exe", None).is_ok());
        assert!(url_allowed("https://release-assets.githubusercontent.com/x", None).is_ok());
        assert!(url_allowed("https://mirror.example.com/a.exe", None).unwrap_err().contains("mirror.example.com"));
        assert!(url_allowed("https://dl.example.org/a.exe", Some("https://dl.example.org/m.txt")).is_ok());
        assert!(url_allowed("https://dl.example.org/a.exe", Some("https://other.example.org/m.txt")).is_err());
        assert!(url_allowed("http://github.com/a.exe", None).is_err());
    }
}
