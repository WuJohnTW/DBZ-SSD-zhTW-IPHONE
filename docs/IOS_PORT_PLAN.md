# iOS port plan

Status: investigation only; no iOS app target or IPA exists yet.

## Verified project structure

- The game is a Rust workspace. The desktop entry point is `crates/ssd-app/src/main.rs`.
- Android reuses the desktop app modules by including `main.rs` from `crates/ssd-app/src/android.rs`, then exposes SDL's `SDL_main` from a `cdylib` built by `ssd-android`.
- The Android setup flow is implemented in `crates/ssd-app/src/android_setup.rs` and the Android lifecycle glue is in `crates/ssd-app/src/android_lifecycle.rs`.
- The app already has touch controls in `crates/ssd-app/src/touch.rs`; touch controls are enabled automatically only on Android, or by `--touch-ui` elsewhere.
- A separate Apple Silicon macOS launcher exists under `macos/`. It is not an iOS app and cannot be reused as an iOS bundle unchanged.
- The existing workspace has no iOS crate/member or Xcode project.

## Proposed iOS architecture

1. Add an iOS app wrapper (Xcode project or generated project) whose native entry point uses SDL3's iOS `SDL_main` integration. SDL3 documents that iOS supplies the app delegate/run loop and that `SDL_main.h` provides the entry point.
2. Build the Rust game as an iOS static library for `aarch64-apple-ios`, with the iOS wrapper providing the platform entry point. Do not attempt to use an iOS dynamic library as the app's main executable.
3. Reuse the Android app setup and bundled-translation logic where it is platform-neutral, but implement iOS-specific document import using the iOS document picker / Files app.
4. Store ROM cache, saves, settings, and optional HD pack under the iOS app sandbox. Never assume desktop paths such as `LOCALAPPDATA` or Android's internal-storage API exist on iOS.
5. Bundle the Traditional Chinese translation tables and font as app resources. Keep the user's ROM, IPS patch, and optional HD pack outside the repository; the user imports files they lawfully possess.
6. Add a macOS GitHub Actions workflow to build the unsigned app bundle/IPA artifact. Signing with a free Apple ID is a separate installation/signing step and should not be conflated with compilation.

## Required engineering work

- Inspect and adapt `ssd-app`'s SDL initialization, window creation, event loop, filesystem paths, audio, and app lifecycle handling for iOS.
- Add a touch-first UI. Existing Android touch controls are a useful starting point, but must be tested for iPhone safe areas, rotation, scaling, and iOS lifecycle events.
- Integrate SDL3 as an iOS-compatible framework/xcframework (or a reproducible source build) and link required Apple system frameworks.
- Add Xcode project settings, Info.plist, launch assets, bundle identifier, deployment target, and resource-copy phases.
- Add simulator/device build checks where available, plus runtime tests for ROM import, Chinese font rendering, audio, saves, background/foreground, and memory pressure.

## Acceptance criteria

- A macOS CI job produces an installable iOS app bundle for a real iPhone device architecture.
- The app launches to a Traditional Chinese setup flow, imports a user-provided compatible ROM and optional IPS patch, and starts in Remastered/Enhanced mode.
- Touch input, audio, saves, and app suspension/resumption work on a physical iPhone.
- CI publishes an unsigned artifact; signing and installation instructions explicitly state the free Apple ID provisioning and refresh limitations.

## Current limitation

This document records the investigation and intended approach only. It does not claim that the iOS wrapper, Rust target, Xcode project, signing, or IPA build has been implemented or tested.
