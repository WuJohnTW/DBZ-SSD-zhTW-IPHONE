//! iOS entry point for the native Xcode host.
//!
//! SDL3 owns the UIKit application entry point in the native host (SDL_main.h). The host calls
//! `ssd_ios_main` from that SDL-managed entry point. This crate is a static library, not an
//! executable or a dynamically loaded iOS library.
#![cfg(target_os = "ios")]
#![allow(dead_code)]

include!("main.rs");

use std::ffi::{CStr, OsString, c_char, c_int};

/// The SDL/Xcode host calls this once the UIKit application has entered SDL's run loop.
///
/// The game stores saves and settings under SDL's app-specific preferences directory. The bundle
/// resource directory is read-only on iOS and must never be used for player data.
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

    if run(args) == std::process::ExitCode::SUCCESS { 0 } else { 1 }
}
