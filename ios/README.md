# iOS app scaffold

The iOS app project is generated from `project.yml` with XcodeGen. The SDL3 iOS xcframework is supplied to the project at build time under `Vendor/SDL3.xcframework`; it is not committed to this repository.

## Local build on an Apple Silicon Mac

1. Install Xcode, Rust 1.99, XcodeGen, and CMake.
2. Download the SDL3 release DMG from the official SDL GitHub releases and copy `SDL3.xcframework` into `ios/Vendor/`.
3. Run `xcodegen generate --spec ios/project.yml`.
4. Build the Rust static library for `aarch64-apple-ios`.
5. Open `ios/DBZSSD.xcodeproj` and build the `DBZSSD` scheme for a physical iPhone.

The native Rust iOS target has passed a GitHub Actions `cargo check`. This project scaffold is not yet a verified installable app: final linking, SDL runtime startup, document import, and device testing remain to be completed.

ROM images and IPS patches are not included. Import only game files you are legally entitled to use.
