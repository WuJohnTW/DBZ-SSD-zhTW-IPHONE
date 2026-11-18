//! The update manifest: which release is the newest, where its `.exe` is, its size and SHA-256, signed with
//! ed25519. Shared by the launcher (`update.rs`, which only verifies) and `dbz-update-sign` (which signs).
//!
//! UTF-8 text, every line ending in `\n`, the lines in exactly this order and nothing else:
//!
//! ```text
//! dbz-zhtw-update 1
//! version <the NN of vNN, a whole number>
//! size <bytes of the .exe>
//! sha256 <64 lower-case hexadecimal digits>
//! url <https:// URL of the .exe, ASCII, no spaces>
//! signature <128 lower-case hexadecimal digits>
//! ```
//!
//! The signature is ed25519 (`verify_strict`) over every byte before the `signature` line. It is checked before
//! any other line is read.

#![cfg_attr(not(test), allow(dead_code))]

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

pub const MAGIC_LINE: &str = "dbz-zhtw-update 1";
/// The largest `.exe` an update may download (the public `.exe` is about 60 MB).
pub const MAX_EXE_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    pub size: u64,
    pub sha256: [u8; 32],
    pub url: String,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `N` bytes from `2N` lower-case hexadecimal digits (upper case is refused: one spelling per manifest).
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    if text.len() != 2 * N || !text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// Where a GitHub release's file is served from: `github.com` answers `releases/download/...` with a 302 to
/// `release-assets.githubusercontent.com` (as measured); `objects.githubusercontent.com` is the older
/// one. Both are in the `domains.actions` list of https://api.github.com/meta. The launcher also allows its
/// manifest's own host.
pub const DOWNLOAD_HOSTS: &[&str] = &["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];

/// The host of an `https://` URL, in lower case. Anything else (another scheme, a user name, a port but 443, an
/// IP v6 literal) is refused.
pub fn https_host(url: &str) -> Result<String, String> {
    // guard: https only
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("不是 https 網址：{url}"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.strip_suffix(":443").unwrap_or(authority);
    if host.is_empty() || !host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
        return Err(format!("網址的主機不接受：{url}"));
    }
    Ok(host.to_ascii_lowercase())
}

/// Whether `url` may be written in a manifest: `https://`, then printable ASCII without spaces.
pub fn url_ok(url: &str) -> bool {
    url.starts_with("https://") && url.len() > "https://".len() && url.bytes().all(|b| b.is_ascii_graphic())
}

/// The signed bytes of `m`: every line but the signature.
pub fn body(m: &Manifest) -> String {
    format!("{MAGIC_LINE}\nversion {}\nsize {}\nsha256 {}\nurl {}\n", m.version, m.size, hex(&m.sha256), m.url)
}

/// The manifest file of `m`, signed with `key`.
pub fn sign(m: &Manifest, key: &SigningKey) -> String {
    let body = body(m);
    let signature = key.sign(body.as_bytes());
    format!("{body}signature {}\n", hex(&signature.to_bytes()))
}

/// The public key of `key`, as written in `DBZ_UPDATE_PUBLIC_KEY`.
pub fn public_key_hex(key: &SigningKey) -> String {
    hex(key.verifying_key().as_bytes())
}

/// The manifest in `text`, when its signature is `public_key`'s and every line is as the format says.
pub fn verify(text: &[u8], public_key: &[u8; 32]) -> Result<Manifest, String> {
    let text = std::str::from_utf8(text).map_err(|_| "清單不是 UTF-8".to_owned())?;
    let last = text
        .strip_suffix('\n')
        .ok_or("清單沒有以換行結尾")?
        .rfind('\n')
        .map(|i| i + 1)
        .ok_or("清單只有一行")?;
    let (body, last) = text.split_at(last);
    let signature = last
        .strip_prefix("signature ")
        .and_then(|s| s.strip_suffix('\n'))
        .and_then(unhex::<64>)
        .ok_or("清單最後一行不是簽章")?;
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| "公鑰無效".to_owned())?;
    // guard: signature
    key.verify_strict(body.as_bytes(), &Signature::from_bytes(&signature)).map_err(|_| "清單簽章不符".to_owned())?;
    parse(body)
}

/// The fields of a signed body (`verify` has checked the signature).
fn parse(body: &str) -> Result<Manifest, String> {
    let lines: Vec<&str> = body.strip_suffix('\n').unwrap_or(body).split('\n').collect();
    let [magic, version, size, sha256, url] = lines.as_slice() else {
        return Err(format!("清單應有 5 行內容，實際 {} 行", lines.len()));
    };
    if *magic != MAGIC_LINE {
        return Err(format!("清單格式不認得：{magic:?}"));
    }
    let field = |line: &str, key: &str| line.strip_prefix(key).and_then(|v| v.strip_prefix(' ')).map(str::to_owned);
    let number = |v: Option<String>| v.filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && !v.starts_with('0'));
    let version: u32 = number(field(version, "version")).and_then(|v| v.parse().ok()).ok_or("清單的 version 不對")?;
    let size: u64 = number(field(size, "size")).and_then(|v| v.parse().ok()).ok_or("清單的 size 不對")?;
    // guard: manifest size
    if size > MAX_EXE_SIZE {
        return Err(format!("清單的 size {size} 超過上限 {MAX_EXE_SIZE}"));
    }
    let sha256 = field(sha256, "sha256").and_then(|v| unhex::<32>(&v)).ok_or("清單的 sha256 不對")?;
    let url = field(url, "url").filter(|u| url_ok(u)).ok_or("清單的 url 不是 https:// 網址")?;
    Ok(Manifest { version, size, sha256, url })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn sample() -> Manifest {
        Manifest { version: 45, size: 1234, sha256: [0xab; 32], url: "https://github.com/o/r/releases/download/v45/a.exe".to_owned() }
    }

    #[test]
    fn signed_manifest_round_trips() {
        let k = key(7);
        let text = sign(&sample(), &k);
        assert_eq!(verify(text.as_bytes(), k.verifying_key().as_bytes()), Ok(sample()));
        assert!(text.starts_with("dbz-zhtw-update 1\nversion 45\nsize 1234\nsha256 abab"), "{text}");
        assert_eq!(public_key_hex(&k), hex(k.verifying_key().as_bytes()));
        assert_eq!(public_key_hex(&k).len(), 64);
    }

    /// Why: a manifest is trusted only for what its key signed; another key, or one changed byte, is refused.
    #[test]
    fn manifest_with_bad_signature_is_rejected() {
        let k = key(7);
        let text = sign(&sample(), &k);
        assert_eq!(verify(text.as_bytes(), key(8).verifying_key().as_bytes()), Err("清單簽章不符".to_owned()));
        let tampered = text.replace("version 45", "version 46");
        assert_eq!(verify(tampered.as_bytes(), k.verifying_key().as_bytes()), Err("清單簽章不符".to_owned()));
        let other_url = text.replace("a.exe", "b.exe");
        assert_eq!(verify(other_url.as_bytes(), k.verifying_key().as_bytes()), Err("清單簽章不符".to_owned()));
    }

    #[test]
    fn malformed_manifests_are_rejected_even_when_signed() {
        let k = key(3);
        let pk = *k.verifying_key().as_bytes();
        let signed = |body: &str| format!("{body}signature {}\n", hex(&k.sign(body.as_bytes()).to_bytes()));
        let good = body(&sample());
        for bad in [
            good.replace("dbz-zhtw-update 1", "dbz-zhtw-update 2"),
            good.replace("version 45", "version 045"),
            good.replace("version 45", "version -1"),
            good.replace("https://", "http://"),
            good.replace("a.exe", "a b.exe"),
            good.replace("sha256 abab", "sha256 ABAB"),
            format!("{good}extra 1\n"),
            good.replace("size 1234\n", ""),
        ] {
            assert!(verify(signed(&bad).as_bytes(), &pk).is_err(), "accepted: {bad}");
        }
        assert!(verify(good.as_bytes(), &pk).is_err(), "no signature line");
        assert!(verify(sign(&sample(), &k).trim_end().as_bytes(), &pk).is_err(), "no final newline");
    }

    /// Why: a signed manifest still cannot ask for more than MAX_EXE_SIZE bytes.
    #[test]
    fn manifest_size_over_the_limit_is_rejected() {
        let k = key(4);
        let mut m = sample();
        m.size = MAX_EXE_SIZE + 1;
        let err = verify(sign(&m, &k).as_bytes(), k.verifying_key().as_bytes()).unwrap_err();
        assert!(err.contains("超過上限"), "{err}");
        m.size = MAX_EXE_SIZE;
        assert!(verify(sign(&m, &k).as_bytes(), k.verifying_key().as_bytes()).is_ok());
    }
}
