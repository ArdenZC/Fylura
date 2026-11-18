# Removing Fylura data on macOS

Dragging `Fylura.app` to Finder Trash is an ordinary app removal. It leaves the user's Fylura data in place so reinstalling the app can resume with the same settings, database, History, Journal, Restore records, API credentials, and Safe Trash.

To remove Fylura's own data as well, run `Fylura.app/Contents/Resources/Uninstall Fylura Data.command` before removing the app. The command file presents an interactive confirmation and stops if Fylura is running. The packaged application's `--fylura-uninstall-clean-user-data` path removes only the current user's `com.startlan.fylura` Application Support and cache directories, matching Preferences plist, matching Fylura LaunchAgent, and AI Keychain entries under the exact `com.startlan.fylura` and `com.startlan.fylura.ai-profile` services. Profile credentials are enumerated from Keychain attributes, so cleanup does not depend on a readable database or its current Profile list. It verifies that the targeted credentials and files are gone; an inspection or deletion error is reported as incomplete.

The cleanup preserves `safe-trash`, `.fylura-trash`, and legacy `.zen-canvas-trash` roots, including their files. Deleting the database also deletes History, Journal, and Restore metadata, so files left in Safe Trash may no longer have Fylura restore records. It does not clean old Zen Canvas application data or credentials, user documents, Finder Trash, or Safe Trash roots.

### Acceptance status

- Packaged application CLI cleanup and its Application Support, cache, Preferences, LaunchAgent, and Keychain checks: **PASS** in [CI run #44340028861](https://github.com/ArdenZC/Fylura/actions/runs/44340028861) on Apple Silicon at PR HEAD `78c90f10300581e8f2c344f5c6108a38dafb68f8`. The job uses a disposable CI account and does not exercise a graphical desktop.
- `Uninstall Fylura Data.command` interaction (confirmation dialog, cancel, confirm, running-app block, failure message, and Chinese/English rendering): **UNVERIFIED**. These GUI states have not been observed in an interactive Apple Silicon session. They remain for the #597 installer/uninstaller visual acceptance.
- Ordinary `.app` bundle removal in the headless packaged DMG lifecycle retaining Fylura data and preserving Finder Trash, Safe Trash, and user documents: **PASS**. The job removes its isolated copied bundle; Finder drag interaction itself is not exercised.
