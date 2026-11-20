# iOS build notes

The first native Xcode build reached the Xcode resource-copy phase after the Rust static library compiled successfully. Xcode reported duplicate font-license text resources because the initial project spec copied both the complete translation folder and the shared fonts folder.

The project spec now lists only the two Traditional Chinese TSV files and the six font files required by the UI, avoiding duplicate license-file destinations. The next CI run validates this correction.
