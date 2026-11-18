//! The ROM and IPS the installer copied for the first launch: `<install folder>\save\setup\rom.sfc` (or
//! `rom.smc`, the extension of the file the player chose) and, optionally, `save\setup\dbz102.ips`. Looked for
//! beside this `.exe` only (`save\setup\`, wherever the data folder resolved to: the installer puts them there).
//!
//! At the first launch (no `cache\rom\canonical.sfc` yet) they go through the same step as a ROM the player picks
//! ([`PresetEnv::cache`]: the Japanese Rev 1 check, then the game's one headless frame), without the dialogs.
//! Used or refused, the copies are removed (only these three names, then `setup\` if it is empty): the player's own
//! files are elsewhere, and a ROM that was refused is not refused again at the next start. A refused one is said
//! once ([`PresetEnv::report`]), and the dialogs follow as before. An IPS without a ROM is removed. A Japanese Rev 1
//! ROM without an IPS asks for the IPS only ([`PresetEnv::pick_ips`]). Once `canonical.sfc` exists, `setup\` is not
//! read: what is left there is removed.

use std::path::{Path, PathBuf};

/// The folder beside the `.exe`, in its `save\`.
pub const SETUP_DIR: &str = "setup";
/// The names the installer writes; nothing else in `setup\` is touched.
pub const ROM_NAMES: [&str; 2] = ["rom.sfc", "rom.smc"];
pub const IPS_NAME: &str = "dbz102.ips";

/// `<exe_dir>\save\setup`.
pub fn setup_dir(exe_dir: &Path) -> PathBuf {
    exe_dir.join(crate::SAVE_DIR).join(SETUP_DIR)
}

/// What the first launch does with a ROM: the real one runs the game (see `cache_rom` in main.rs).
pub trait PresetEnv {
    /// The same step as for a ROM the player picks: `Err` with what to tell the player when the ROM is refused.
    fn cache(&mut self, rom: &Path, ips: Option<&Path>) -> Result<(), String>;
    /// Tells the player why the installer's ROM was refused (once).
    fn report(&mut self, text: &str);
    /// The IPS dialog alone, for a Japanese Rev 1 ROM the installer was given without one.
    fn pick_ips(&mut self) -> Option<PathBuf>;
}

/// What happened.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `canonical.sfc` was there: `setup\` was not read (what was left there was removed).
    NotNeeded,
    /// No ROM in `setup\` (an IPS alone was removed): the dialogs as before.
    NoPreset,
    /// The ROM is cached: no dialog.
    Done,
    /// The ROM was refused and said; the copies are removed: the dialogs as before.
    Refused,
}

/// Removes the installer's copies in `setup` (only [`ROM_NAMES`] and [`IPS_NAME`]), then `setup` if it is empty.
/// Lines for `last-run.log`.
pub fn clear(setup: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    for name in ROM_NAMES.iter().chain([&IPS_NAME]) {
        let path = setup.join(name);
        if path.is_file() {
            lines.push(match std::fs::remove_file(&path) {
                Ok(()) => format!("setup: removed {}", path.display()),
                Err(e) => format!("setup: could not remove {}: {e}", path.display()),
            });
        }
    }
    // not recursive: anything else in it stays, and so does the folder
    if setup.is_dir() && std::fs::remove_dir(setup).is_ok() {
        lines.push(format!("setup: removed {}", setup.display()));
    }
    lines
}

/// The ROM in `setup` (`rom.sfc`, else `rom.smc`) and the IPS.
fn files(setup: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let rom = ROM_NAMES.iter().map(|n| setup.join(n)).find(|p| p.is_file());
    let ips = Some(setup.join(IPS_NAME)).filter(|p| p.is_file());
    (rom, ips)
}

/// The first launch's use of the installer's files. `canonical_exists`: the ROM is cached already. `fixed_ips`: an
/// IPS the launcher has without asking (the overlay's or the embedded one), used when `setup` has none.
/// `log`: lines for `last-run.log`.
pub fn run(
    canonical_exists: bool,
    setup: &Path,
    fixed_ips: Option<&Path>,
    env: &mut dyn PresetEnv,
    log: &mut Vec<String>,
) -> Outcome {
    // guard: canonical exists, setup not read
    if canonical_exists {
        log.extend(clear(setup));
        return Outcome::NotNeeded;
    }
    let (rom, ips) = files(setup);
    let Some(rom) = rom else {
        // guard: an IPS alone is removed
        log.extend(clear(setup));
        return Outcome::NoPreset;
    };
    let mut ips = ips.or_else(|| fixed_ips.map(Path::to_path_buf));
    // guard: a Japanese Rev 1 ROM without an IPS asks for the IPS only
    if ips.is_none() && std::fs::read(&rom).is_ok_and(|bytes| is_jp_rev1(&bytes)) {
        ips = env.pick_ips();
    }
    let result = env.cache(&rom, ips.as_deref());
    // guard: the copies are removed, used or refused
    log.extend(clear(setup));
    match result {
        Ok(()) => {
            log.push(format!("setup: the installer's ROM {} is cached", rom.display()));
            Outcome::Done
        }
        Err(e) => {
            log.push(format!("setup: the installer's ROM {} was refused", rom.display()));
            env.report(&format!("安裝時選的 ROM 不能用，已刪掉安裝程式複製的那一份，請重新選擇。\n\n{e}"));
            Outcome::Refused
        }
    }
}

/// Whether `bytes` is the Japanese Rev 1 ROM (with or without a copier header), which needs `dbz102.ips`: the
/// rule `set_up_rom` checks before running the game.
pub fn is_jp_rev1(bytes: &[u8]) -> bool {
    let image = if bytes.len() == crate::ROM_SIZE + 512 { bytes.get(512..).unwrap_or_default() } else { bytes };
    image.len() == crate::ROM_SIZE && crate::crc32(image) == crate::ROM_JP_REV1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::tests::scratch;

    /// Records what the first launch was asked to do.
    #[derive(Default)]
    struct Env {
        cached: Vec<(PathBuf, Option<PathBuf>)>,
        reports: Vec<String>,
        ips_asked: usize,
        refuse: Option<String>,
        ips_answer: Option<PathBuf>,
    }
    impl PresetEnv for Env {
        fn cache(&mut self, rom: &Path, ips: Option<&Path>) -> Result<(), String> {
            // what the game reads: the files must still be there
            assert!(rom.is_file(), "the ROM was removed before it was used");
            self.cached.push((rom.to_path_buf(), ips.map(Path::to_path_buf)));
            self.refuse.clone().map_or(Ok(()), Err)
        }
        fn report(&mut self, text: &str) {
            self.reports.push(text.to_owned());
        }
        fn pick_ips(&mut self) -> Option<PathBuf> {
            self.ips_asked += 1;
            self.ips_answer.clone()
        }
    }

    fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A 1 MiB image whose CRC-32 is `target`: zeros, and four bytes at the end chosen backwards through the
    /// CRC table (the CRC is linear).
    fn image_with_crc(target: u32) -> Vec<u8> {
        let table: Vec<u32> = (0..256u32)
            .map(|i| (0..8).fold(i, |c, _| if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 }))
            .collect();
        let mut image = vec![0u8; crate::ROM_SIZE - 4];
        let start = !crate::crc32(&image);
        let mut want = !target;
        let mut index = [0usize; 4];
        for k in (0..4).rev() {
            let j = (0..256).find(|&j| table[j] >> 24 == want >> 24).unwrap();
            index[k] = j;
            want = (want ^ table[j]) << 8;
        }
        let mut r = start;
        for j in index {
            image.push(((r ^ j as u32) & 0xFF) as u8);
            r = (r >> 8) ^ table[j];
        }
        assert_eq!(crate::crc32(&image), target);
        image
    }

    /// Why: a ROM (and IPS) the installer copied is used without a dialog, exactly as a picked one, and the
    /// copies are removed afterwards.
    #[test]
    fn the_installers_rom_and_ips_are_used_without_a_dialog_then_removed() {
        let dir = scratch("preset-ok");
        let setup = setup_dir(&dir.0);
        let rom = put(&setup, "rom.smc", b"a rom");
        let ips = put(&setup, IPS_NAME, b"an ips");
        let mut env = Env::default();
        let mut log = Vec::new();
        assert_eq!(run(false, &setup, None, &mut env, &mut log), Outcome::Done);
        assert_eq!(env.cached, vec![(rom, Some(ips))]);
        assert!(env.reports.is_empty() && env.ips_asked == 0);
        assert!(!setup.exists(), "{log:?}");
        assert!(dir.0.join(crate::SAVE_DIR).is_dir(), "save\\ itself stays");
    }

    /// Why: a refused ROM is said once, its copies are removed (the next start does not refuse it again) and the
    /// dialogs follow; files the installer did not write stay.
    #[test]
    fn a_refused_rom_is_said_once_removed_and_the_dialogs_follow() {
        let dir = scratch("preset-bad");
        let setup = setup_dir(&dir.0);
        put(&setup, "rom.sfc", b"not the game");
        put(&setup, IPS_NAME, b"an ips");
        let other = put(&setup, "readme.txt", b"not ours");
        let mut env = Env { refuse: Some("大小是 12 bytes，不是這個遊戲的 ROM".to_owned()), ..Env::default() };
        let mut log = Vec::new();
        assert_eq!(run(false, &setup, None, &mut env, &mut log), Outcome::Refused);
        assert_eq!(env.reports.len(), 1);
        assert!(env.reports[0].contains("安裝時選的 ROM 不能用") && env.reports[0].contains("不是這個遊戲的 ROM"), "{:?}", env.reports);
        assert!(!setup.join("rom.sfc").exists() && !setup.join(IPS_NAME).exists());
        assert!(other.exists() && setup.is_dir(), "a file not ours, or the folder holding it, was removed");
        // the next start: nothing to refuse again
        assert_eq!(run(false, &setup, None, &mut env, &mut log), Outcome::NoPreset);
        assert_eq!(env.reports.len(), 1);
        assert_eq!(env.cached.len(), 1);
    }

    /// Why: an IPS without a ROM is not used, and removed.
    #[test]
    fn an_ips_alone_is_removed() {
        let dir = scratch("preset-ips");
        let setup = setup_dir(&dir.0);
        put(&setup, IPS_NAME, b"an ips");
        let mut env = Env::default();
        assert_eq!(run(false, &setup, None, &mut env, &mut Vec::new()), Outcome::NoPreset);
        assert!(env.cached.is_empty());
        assert!(!setup.exists());
    }

    /// Why: once the ROM is cached, `setup\` is not read (the ROM there is never used), only cleared.
    #[test]
    fn with_the_rom_cached_setup_is_not_read_but_cleared() {
        let dir = scratch("preset-cached");
        let setup = setup_dir(&dir.0);
        put(&setup, "rom.sfc", b"a rom");
        let mut env = Env::default();
        assert_eq!(run(true, &setup, None, &mut env, &mut Vec::new()), Outcome::NotNeeded);
        assert!(env.cached.is_empty() && env.ips_asked == 0 && env.reports.is_empty());
        assert!(!setup.exists());
        // nothing there: nothing to do
        assert_eq!(run(true, &setup, None, &mut env, &mut Vec::new()), Outcome::NotNeeded);
    }

    /// Why: a Japanese Rev 1 ROM given without an IPS asks for the IPS only; an English ROM does not.
    #[test]
    fn a_japanese_rom_without_an_ips_asks_for_the_ips_only() {
        let dir = scratch("preset-jp");
        let setup = setup_dir(&dir.0);
        let jp = image_with_crc(crate::ROM_JP_REV1);
        assert!(is_jp_rev1(&jp));
        assert!(is_jp_rev1(&[vec![0u8; 512], jp.clone()].concat()), "with a copier header");
        let rom = put(&setup, "rom.sfc", &jp);
        let picked = put(&dir.0, "elsewhere.ips", b"ips");
        let mut env = Env { ips_answer: Some(picked.clone()), ..Env::default() };
        assert_eq!(run(false, &setup, None, &mut env, &mut Vec::new()), Outcome::Done);
        assert_eq!(env.ips_asked, 1);
        assert_eq!(env.cached, vec![(rom, Some(picked.clone()))]);
        assert!(picked.exists(), "the player's own IPS is not removed");

        // the launcher's own IPS (overlay or embedded) is used without asking
        let rom = put(&setup, "rom.sfc", &jp);
        let fixed = put(&dir.0, "fixed.ips", b"ips");
        let mut env = Env::default();
        run(false, &setup, Some(&fixed), &mut env, &mut Vec::new());
        assert_eq!(env.ips_asked, 0);
        assert_eq!(env.cached, vec![(rom, Some(fixed))]);

        // an English ROM: no IPS asked
        let en = image_with_crc(crate::ROM_CANONICAL);
        put(&setup, "rom.sfc", &en);
        let mut env = Env::default();
        run(false, &setup, None, &mut env, &mut Vec::new());
        assert_eq!(env.ips_asked, 0);
        assert_eq!(env.cached[0].1, None);
    }
}
