//! Data appended to the end of the launcher's own `.exe` (an "overlay": Windows loads only what
//! the PE headers name, so bytes after the image change nothing). Written by
//! a tool outside this repository; lets a private copy carry the ROM and the IPS without them ever being built
//! into the program.
//!
//! Layout, at the very end of the file (all integers little-endian):
//!
//! ```text
//! [segment 0 bytes][segment 1 bytes]...          data, in table order
//! [name: 8 bytes, ASCII, NUL-padded][len: u64][sha256: 32 bytes]   one 48-byte entry per segment
//! [count: u32][body_len: u64][magic "DBZZHTW1"]                    20-byte footer
//! ```
//!
//! `body_len` is the length of the data plus the table, so the overlay starts at
//! `file_len - 20 - body_len`. Segment names: `rom`, `ips`.

use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"DBZZHTW1";
const FOOTER: u64 = 20;
const ENTRY: u64 = 48;

pub struct Segment {
    pub name: String,
    pub bytes: Vec<u8>,
    /// SHA-256 of `bytes` (hex), checked against the table when read.
    pub sha256: String,
}

/// The segments of the overlay of `exe`: `Ok(None)` when it has none, `Err` when the trailer is
/// there but damaged (a length out of range, a SHA-256 that does not match).
pub fn read(exe: &Path) -> Result<Option<Vec<Segment>>, String> {
    let mut f = std::fs::File::open(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len < FOOTER {
        return Ok(None);
    }
    let mut footer = [0u8; FOOTER as usize];
    f.seek(SeekFrom::Start(len - FOOTER)).map_err(|e| e.to_string())?;
    f.read_exact(&mut footer).map_err(|e| e.to_string())?;
    if &footer[12..20] != MAGIC {
        return Ok(None);
    }
    let count = u64::from(u32::from_le_bytes(footer[0..4].try_into().unwrap()));
    let body_len = u64::from_le_bytes(footer[4..12].try_into().unwrap());
    let table_len = count.checked_mul(ENTRY).ok_or("overlay: bad count")?;
    if body_len < table_len || body_len > len - FOOTER {
        return Err("overlay: 長度表損壞".to_owned());
    }
    let start = len - FOOTER - body_len;
    let mut table = vec![0u8; table_len as usize];
    f.seek(SeekFrom::Start(len - FOOTER - table_len)).map_err(|e| e.to_string())?;
    f.read_exact(&mut table).map_err(|e| e.to_string())?;
    let data_len: u64 = table
        .chunks(ENTRY as usize)
        .map(|e| u64::from_le_bytes(e[8..16].try_into().unwrap()))
        .try_fold(0u64, u64::checked_add)
        .ok_or("overlay: 長度溢位")?;
    if data_len != body_len - table_len {
        return Err("overlay: 段長度與總長不符".to_owned());
    }
    f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
    let mut segments = Vec::new();
    for e in table.chunks(ENTRY as usize) {
        let name = String::from_utf8_lossy(&e[0..8]).trim_end_matches('\0').to_owned();
        let seg_len = u64::from_le_bytes(e[8..16].try_into().unwrap());
        let want = super::hex(&e[16..48]);
        let mut bytes = vec![0u8; seg_len as usize];
        f.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        let got = super::hex(&Sha256::digest(&bytes));
        if got != want {
            return Err(format!("overlay: 段 {name} 的 sha256 不符"));
        }
        segments.push(Segment { name, bytes, sha256: got });
    }
    Ok(Some(segments))
}

/// The overlay of `exe` byte for byte (its data, table and footer), checked as [`read`] checks it: what the
/// update appends to the new `.exe`, so a private copy keeps its ROM and IPS. `Ok(None)` when it has none.
pub fn raw(exe: &Path) -> Result<Option<Vec<u8>>, String> {
    let Some(segments) = read(exe)? else {
        return Ok(None);
    };
    let body: u64 = segments.iter().map(|s| s.bytes.len() as u64 + ENTRY).sum();
    let mut f = std::fs::File::open(exe).map_err(|e| format!("{}: {e}", exe.display()))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    f.seek(SeekFrom::Start(len - FOOTER - body)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity((body + FOOTER) as usize);
    f.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(Some(bytes))
}
