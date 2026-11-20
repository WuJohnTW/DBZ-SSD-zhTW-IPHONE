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

    // SDL_GetPrefPath creates and returns the app-specific writable directory in the iOS sandbox.
    // SAFETY: static NUL-terminated identifiers; SDL owns the returned pointer until SDL_free.
    let data_dir = unsafe {
        sdl3::sys::filesystem::SDL_GetPrefPath(
            c"WuJohnTW".as_ptr(),
            c"DBZ SSD".as_ptr(),
        )
    };
    if data_dir.is_null() {
        log::error!("SDL_GetPrefPath failed: {}", sdl3::get_error());
        return i32::from(EXIT_ROM_MISSING);
    }

    // SAFETY: SDL returned a valid NUL-terminated UTF-8 path.
    let path = unsafe { CStr::from_ptr(data_dir) }.to_string_lossy().into_owned();
    // SAFETY: free the allocation returned by SDL_GetPrefPath exactly once.
    unsafe { sdl3::sys::stdinc::SDL_free(data_dir.cast()) };
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

    i32::from(run(args))
}
