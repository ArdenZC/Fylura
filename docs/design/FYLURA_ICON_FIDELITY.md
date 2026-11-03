# Fylura native-size icon clarity

Previous icon assets were derived by shrinking a 1024px HTML render with `backdrop-filter` glass and wide glow shadows. The pane and sphere merged under 64px; sharpening cannot restore that missing boundary.

This PR keeps the approved high-resolution image and updates only embedded 16/24/32/48/64px frames: clear white pane outline, stronger opacity, no backdrop blur, native-scale antialiasing. Large ICO and ICNS frames preserve the original artwork.

To reproduce: `node scripts/generateFyluraIcons.mjs` then `node scripts/verifyFyluraIcons.mjs`. Inspect `build/brand/small-sizes/Fylura_16.png` through `Fylura_64.png` at actual pixel sizes. The build already uses committed `build/icon.ico`, `build/icon.icns`, and `build/icon.png` via the unchanged Tauri configuration.

No change to Rust source, dependency manifests, app identity, file operations, indexing, search SQL, Windows services, or platform signing. Native installed icon appearance on Windows taskbar and macOS Dock remains to be verified on each OS.
