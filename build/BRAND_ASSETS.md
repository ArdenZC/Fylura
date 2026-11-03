# Fylura selected orb + glass-pane icon

The approved symbol is a blue/cyan sphere upper-left, overlapped by a translucent rounded pane lower-right. Windows and macOS use the **same silhouette**.

The Tauri bundle configuration already reads the committed files `build/icon.png`, `build/icon.ico`, `build/icon.icns`, and `build/installerIcon.ico`. The binaries in this PR are production inputs; **no Tauri config change is necessary**.

- Windows ICO: separate hand-optimized frames 16, 24, 32, 48, 64, 128 and 256px.
- macOS ICNS: standard 16–1024px icon slots and Retina sizes (macOS 13+).
- 16–64px frames are optically rendered at native sizes, not blurry downscales.
- `build/icon.png` remains the original 1024px approved source, unmodified.

To regenerate or verify with Node alone, run `node scripts/generateFyluraIcons.mjs` and then `node scripts/verifyFyluraIcons.mjs`. On Windows, the existing `npm run assets:brand` command calls the new generator and also creates the NSIS header/sidebar and DMG background; it no longer regenerates the old temporary HTML artwork.

We deliberately do not modify `package.json`, its dependency lockfile, npm lifecycle hooks, or Tauri configuration. Existing packaged app icons use the committed images directly; when intentionally changing the master image, rerun the generator and commit its outputs.

Standard `.icns` does not implement native macOS 26 Icon Composer / Liquid Glass dynamic materials. Windows NSIS and macOS DMG installation and taskbar/Dock presentation remain subject to native smoke testing.
