# UX-04 — File-first onboarding and single Files mode

**Issue:** [#527](https://github.com/ArdenZC/Fylura/issues/527)
**Decision:** [ADR-0016](../DECISIONS/0016-file-first-four-core-jobs.md)
**Scope owner:** product information architecture, first launch, Files entry,
Global Search consent and navigation priority.
**Out of scope:** AI Organize/Cleanup backends, Global Search 1M/FTS performance,
Managed Scan internals, Safe Trash/Preview/Journal/Rename behavior, schema.

## Delivered behavior

### First launch

`src/components/OnboardingDialog.tsx` is now a four-scene product introduction
(Files, AI organize, AI cleanup, Global Search consent). Every exit — Start,
Next…Done, Skip, Close, Escape, and the AI Settings link — completes the intro
durably and lands on the ordinary Files workspace. The first scene exposes
**Start using Fylura** directly, so the intro is never a gate.

The intro performs no configuration. It does not call `setDefaultScanFolders`,
`addManagedScope`, `saveSettings`, `setGlobalIndexSourceEnabled`,
`authorizeGlobalIndexSource`, `startGlobalIndex` or `startManagedScan` unless the
user explicitly selects volumes on the Global Search scene and presses
**Enable Global Search**.

`tests/onboardingDialog.test.tsx` and `tests/onboardingSettingsPersistence.test.tsx`
assert these negative side effects, clean-profile restart durability and that an
existing Managed Library profile keeps its configured roots unchanged.

Re-entry is voluntary: Settings → About → **Review the introduction**
(`src/components/onboardingReopen.ts`), never an automatic modal.

### Files is one user-visible mode

`FileLibraryExperienceController` now starts in Browse with no Managed Library
target, so the ordinary file manager opens with `currentTarget === null` and
never adopts, scans or authorizes anything on entry.

The Library/Browse tablist was removed from `WorkspaceCommandBar`. Managed
Library keeps its implementation: `LibraryMode`, Query V2, `LibrarySelectionV1`,
tags, Saved Views, saved places, scan roots, watchers and every existing command
and API. `FileLibraryExperienceController.switchMode` and
`LEGACY_LIBRARY_MIGRATION_TARGET` remain for compatibility callers and tests.

### Places before any folder is open

`src/views/fileLibrary/browse/useBrowsePlaces.ts` combines two read-only
projections:

- `file_workspace_known_locations` (new, read-only) — home, desktop, downloads,
  documents and mounted volumes, with an availability flag;
- `file_workspace_places` — existing favorites and recent Browse locations.

Selecting one re-admits the path through the existing user-intent
`file_workspace_browse_open` admission, exactly like the OS folder picker.
Unavailable locations are listed with an explanation instead of being dropped.

`file_workspace_known_locations` is registered in `build.rs`, `main.rs`,
`capabilities/default.json` and `docs/security/TAURI_COMMAND_PERMISSION_MATRIX.md`
as `read_only` with the main-window guard. It creates no Browse session, scan
root, index source, stored preference or directory enumeration.

### Global Search consent

The fourth scene lists the real volumes returned by `list_global_index_sources`
and requires an explicit per-volume selection. `src/components/onboardingGlobalSearch.ts`
then applies the existing contract: Windows fixed NTFS volumes receive
`authorize_global_index_source(id, true)` before `set_global_index_source_enabled`,
and `start_global_index` runs only after at least one source was enabled.
Smart Lean remains the default coverage policy (#509); no second index system,
scope policy or schema is introduced.

When the source projection fails, the scene states that Files stays usable and
points at Settings instead of blocking.

### Navigation

`navGroups` leads with the four core jobs — Files, Global Search (opens the
existing command launcher), AI organize, AI cleanup — then History/Restore, then
Overview (Managed Library setup) and Settings. `initialViewFromSearch` defaults
to `library`; `?view=scanner` and every other canonical route still resolve.

The Files toolbar "More" panel exposes **Organize this folder** and
**Clean up this folder**; both use the existing views, whose AI readiness notices
already explain how to configure a provider instead of reporting a technical
error.

## Validation

- `npm run typecheck`
- `npx vitest run` (full frontend suite)
- `npm run build:frontend`
- `cargo check --features desktop-runtime`, `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`
- focused Rust tests for the known-locations projection
- browser interaction check of the intro and the no-folder Files entry

## Known limitations

- Windows and Apple Silicon macOS native GUI acceptance is **UNVERIFIED** in this
  environment; browser evidence is not native evidence.
- AI organize and AI cleanup backends were not changed; the intro describes the
  currently implemented plan/review/execute and Safe Trash flows.
- Managed Library remains reachable through internal navigation and Settings, but
  has no first-class user-visible switch until a future ADR reopens it.
