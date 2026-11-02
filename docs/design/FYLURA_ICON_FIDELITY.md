# Fylura small-icon clarity fix

**Existing issue:** The previous exporter rasterized a 1024px HTML icon containing a wide CSS `backdrop-filter`, translucent glass and shadows, then shrank it to 16–48px. Unsharp masking could not restore detail that was already blurred.

**Fix:** The new pure Node renderer calculates the original sphere/pane geometry at each native size. At 16,24,32,48,64px it removes backdrop blur, makes the glass pane more opaque, adds a one-device-pixel white edge and applies native-pixel antialiasing. Windows ICO contains PNG-backed individual frames. macOS ICNS contains all standard + Retina slots.

The identity remains one unified blue sphere + foreground pane on both platforms. No changes to SQL, file operations, Windows services, COM GUIDs, application ID or performance limits.

Validation: `npm run assets:icons`, then view the five `build/brand/small-sizes` samples at actual size and inspect the Windows taskbar / macOS Dock in genuine NSIS/DMG package runs. macOS 26 Icon Composer Liquid Glass is separate; standard `.icns` cannot claim that functionality.
