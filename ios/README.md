# iOS app scaffold

The iOS app project is generated from `project.yml` with XcodeGen. The SDL3 iOS xcframework is supplied to the project at build time under `ios/Vendor/SDL3.xcframework`; it is not committed to this repository.

## Import your own ROM on iPhone

1. In the iPhone Files app, open **On My iPhone** and find the **七龍珠Z 超賽亞傳說** app folder.
2. Copy exactly one compatible `.sfc` or `.smc` ROM into that app's Documents folder.
3. If you are importing the Japanese Rev 1 dump, also copy the required Klepto v1.02 patch and name it exactly `dbz102.ips` in the same folder.
4. Launch the game. On first launch, the iOS wrapper detects the single ROM, passes it through the existing ROM validator, and caches the accepted canonical image in the app's private data directory.
5. Once the ROM has been accepted and cached, it can remain in Documents or be removed. The app will use its validated cache on later launches.

If more than one `.sfc`/`.smc` file is present, the app deliberately does not guess which one to use. Remove the extra candidates and relaunch. ROM images and IPS patches are not included; import only files you are legally entitled to use.

## Local build on an Apple Silicon Mac

1. Install Xcode, Rust 1.99, XcodeGen, and CMake.
2. Download the SDL3 release DMG from the official SDL GitHub releases and copy `SDL3.xcframework` into `ios/Vendor/`.
3. Run `xcodegen generate --spec ios/project.yml`.
4. Build the Rust static library for `aarch64-apple-ios`.
5. Open `ios/DBZSSD.xcodeproj` and build the `DBZSSD` scheme for a physical iPhone.

The GitHub Actions workflow builds an unsigned IPA. Signing, installation, final SDL runtime validation, and physical-device testing are still separate steps; a successful CI build alone does not prove that the app runs correctly on an iPhone.
