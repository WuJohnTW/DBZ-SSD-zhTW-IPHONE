//! iOS entry point for the native Xcode host.
//!
//! SDL3 owns the UIKit application entry point in the native host (SDL_main.h). The host calls
//! `ssd_ios_main` from that SDL-managed entry point. This crate is a static library, not an
//! executable or a dynamically loaded iOS library.
#![cfg(target_os = "ios")]
#![allow(dead_code)]

include!("main.rs");

use std::ffi::{CStr, OsString, c_char, c_int};

/// Find ROM files copied into the app's Documents folder through the iOS Files app.
/// Only auto-select a ROM when exactly one candidate exists; never guess between multiple dumps.
fn documents_rom_args() -> Vec<OsString> {
    let Some(home) = std::env::var_os("HOME") else {
        return Vec::new();
    };
    let documents = PathBuf::from(home).join("Documents");
    let Ok(entries) = std::fs::read_dir(&documents) else {
        return Vec::new();
    };

    let mut roms: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("sfc") || ext.eq_ignore_ascii_case("smc")
                })
        })
        .collect();
    roms.sort();

    if roms.len() != 1 {
        if roms.len() > 1 {
            log::warn!(
                "more than one ROM is in Documents; keep only the ROM you want to import"
            );
        }
        return Vec::new();
    }

    let mut args = vec![OsString::from("--rom"), roms.remove(0).into_os_string()];
    let rom_path = PathBuf::from(args[1].clone());
    if let Ok(metadata) = std::fs::metadata(&rom_path) {
        log::info!("iOS ROM candidate: {} ({} bytes)", rom_path.display(), metadata.len());
    }
    // Files may preserve the patch name's case or append a duplicate suffix; accept only the
    // exact base name case-insensitively and report the selected path for diagnostics.
    let patch = std::fs::read_dir(&documents).ok().and_then(|entries| {
        entries.filter_map(Result::ok).map(|entry| entry.path()).find(|path| {
            path.is_file()
                && path.file_name().and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("dbz102.ips"))
        })
    });
    if let Some(patch) = patch {
        log::info!("iOS IPS patch found: {}", patch.display());
        args.push(OsString::from("--ips"));
        args.push(patch.into_os_string());
    } else {
        log::warn!("iOS dbz102.ips patch not found in {}", documents.display());
    }
    args
}

/// The SDL/Xcode host calls this once the UIKit application has entered SDL's run loop.
///
/// The game stores saves and settings under SDL's app-specific preferences directory. The bundle
/// resource directory is read-only on iOS and must never be used for player data. ROM files can be
/// copied into the app's Documents folder from the iOS Files app; the first launch imports the one
/// unambiguous .sfc/.smc file it finds there into the game's validated ROM cache.
#[unsafe(no_mangle)]
pub extern "C" fn ssd_ios_main(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();

    // SDL creates this app-specific writable directory inside the iOS sandbox.
    let path = match sdl3::filesystem::get_pref_path("WuJohnTW", "DBZ SSD") {
        Ok(path) => path,
        Err(error) => {
            log::error!("could not create the iOS app data directory: {error}");
            return i32::from(EXIT_ROM_MISSING);
        }
    };
    // SAFETY: this entry runs before the game starts worker threads.
    unsafe { std::env::set_var("SSD_DATA_DIR", path) };

    let mut args = vec![OsString::from("ssd-recompiled")];
    if argc > 0 && !argv.is_null() {
        // SAFETY: argv is supplied by SDL's native host and contains argc valid C strings.
        let values = unsafe { std::slice::from_raw_parts(argv, argc as usize) };
        for value in values.iter().skip(1) {
            if !value.is_null() {
                // SAFETY: each non-null argv item is NUL-terminated for the duration of this call.
                args.push(OsString::from(unsafe { CStr::from_ptr(*value) }.to_string_lossy().into_owned()));
            }
        }
    }

    // Prefer explicit launch arguments; otherwise import the single ROM copied via Files.
    if !args.iter().any(|arg| arg == "--rom") {
        args.extend(documents_rom_args());
    }

    // This iPhone build is the Traditional Chinese Remastered edition. Classic mode does not
    // render translation tables, so select Enhanced explicitly and enable the existing SDL touch UI.
    args.extend([
        OsString::from("--mode"),
        OsString::from("enhanced"),
        OsString::from("--lang"),
        OsString::from("zh-tw"),
        OsString::from("--touch-ui"),
    ]);

    if run(args) == std::process::ExitCode::SUCCESS { 0 } else { 1 }
}
