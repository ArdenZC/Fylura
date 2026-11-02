# Fylura selected orb + glass-pane icon

The user-approved icon has the cyan/blue sphere in the upper left and a translucent rounded-square panel in the lower right. There is **one unified silhouette** for Windows and macOS.

**Source of truth:** `scripts/generateFyluraIcons.mjs` uses only built-in Node modules to generate:
- `build/icon.png` (1024px), `build/icon.ico` (16/24/32/48/64/128/256px), `build/icon.icns` (16–1024 + Retina slots), and the NSIS installer/uninstaller icon variants.
- `build/brand/small-sizes/Fylura_{16,24,32,48,64}.png` are direct native-size rasterizations, **not** 1024px downscales.
- The small variants suppress backdrop blur, increase foreground pane opacity and edge contrast, preserving recognition at 16px.

`npm run assets:icons` renders and verifies the icons. `npm run build` and Tauri `beforeBuildCommand` invoke it before bundling. On Windows, `npm run assets:brand` also generates NSIS sidebar/header bitmap and DMG background, after regenerating icons using the new source.

The ICO/ICNS binary files are committed with the optical small-size frames. The tracked 1024px PNG is the approved high-resolution source and is never regenerated. Build outputs use the newly generated resources, **not** the temporary-brand-source fixture. Do not manually overwrite these icon assets with images extracted from brand presentation boards.

The `.icns` is a normal macOS 13+ bundle icon; true macOS 26 native Icon Composer materials need a separate Mac-native pipeline. Windows NSIS and macOS DMG install/Dock/Taskbar visual acceptance still require native verification.
