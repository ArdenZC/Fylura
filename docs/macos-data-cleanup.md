# Removing Fylura data on macOS

Dragging `Fylura.app` to Finder Trash is an ordinary app removal. It leaves the user's Fylura data in place so reinstalling the app can resume with the same settings, database, History, Journal, Restore records, API credentials, and Safe Trash.

To remove Fylura's own data as well, run `Fylura.app/Contents/Resources/Uninstall Fylura Data.command` before removing the app. The helper asks for confirmation and stops if Fylura is running. It removes only the current user's `com.startlan.fylura` Application Support and cache directories, matching Preferences plist, matching Fylura LaunchAgent, and Fylura AI Keychain entries. It reads custom AI profile IDs from the database before deleting it, then verifies the targeted items are gone.

The helper preserves `safe-trash`, `.fylura-trash`, and legacy `.zen-canvas-trash` roots, including their files. Deleting the database also deletes History, Journal, and Restore metadata, so files left in Safe Trash may no longer have Fylura restore records. It does not clean old Zen Canvas application data or credentials, user documents, Finder Trash, or Safe Trash outside the app data roots.

Native macOS execution of the helper and LaunchAgent/Keychain cleanup remains `UNVERIFIED` until exercised on a disposable Apple Silicon account.
