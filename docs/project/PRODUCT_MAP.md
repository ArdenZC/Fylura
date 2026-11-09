# Fylura Product Map

Fylura is a local-first personal file manager and file lifecycle assistant. **Files aims to replace Finder/File Explorer for users' everyday file-management workflows** on supported Windows and Apple Silicon macOS, while preserving each OS's shell, file associations and mounting services. Managed indexing, content preview, safe operations/recovery and AI features build on that ordinary file-manager baseline; Fylura does not replace the OS shell process.

This map describes product ownership. Durable implementation authority is defined in `ARCHITECTURE_MAP.md`. Current initiative/Track state remains owned by `STATUS.md`.

**2026-11-01 Owner product direction:** [ADR-0016](DECISIONS/0016-file-first-four-core-jobs.md) replaces the older two-*visible*-mode product expectation. One ordinary Files/Browse experience is visible; existing Managed Library architecture, data and code remain preserved. The four main user jobs are Files, Global Search, AI Organize, and AI Cleanup. This is a **target product map**, not a claim that all flows are currently implemented or native-validated.

## Primary navigation workspaces

| Workspace | User purpose | Product boundary |
| --- | --- | --- |
| Overview | Optional summary of recent work, task state and issues | Secondary/supporting surface; first-run completion defaults to Files, not Overview |
| **Files / File Management** | Browse original system folders; use tabs, dual panes, File Basket, preview and ordinary safe file operations | **Only Browse is a primary visible mode**; Managed Library Query V2, tags, Saved Views and related durable data/code are preserved internally, not an initial setup prerequisite |
| **AI Organize** | Choose an arbitrary folder/selection/Basket, receive AI-explained grouped proposals, review exceptions and execute a batch | Must support one-off unmanaged scope without first admitting Managed Library; reuse durable Organization Plan / Dry Run / Journal / Restore and separate AI consent |
| **AI Cleanup** | Choose a cleanup scope, get an AI-explained grouped cleanup proposal, review risks, then Safe Trash eligible candidates | Durable detector facts + current AI semantic evidence + conservative eligibility; Safe Trash / history / recovery remain authoritative |
| History | Understand changes, restore recoverable work and resolve recovery cases | Operation/cleanup ledgers and identity revalidation |

## Advanced navigation workspaces

| Workspace | User purpose | Product boundary |
| --- | --- | --- |
| Automation | Save reusable file scopes, configure Manual/Schedule/Managed files changed, enable/pause and prepare plans for review | Schema 37 Automation is merged and Owner-reviewed: `organize_plan`, review-required and never-auto-execute remain fixed. Manual/Schedule/Managed-scope-changed prepare reviewable Organization Plans; existing Rule Repository / Rule Proposal remain explicit Advanced Policies compatibility surfaces, not semantic fallback for new Organize/Cleanup. Entry remains in Settings/Spotlight. |
| Settings | Configure app, search, indexing, AI/provider, lifecycle and diagnostics | Persisted settings/provider contracts; technical detail remains secondary to task language |

### Files workspace ownership

```text
Files (only one normal visible mode: Browse / File Management)
└─ Workspace tabs
   ├─ Single pane: physical folder / Places
   └─ Dual panes: two independent physical locations

Retained internal implementation:
Managed Library Query V2 / LibrarySelectionV1 / tags / Saved Views
(no default user-facing mode switch, no data deletion)
```

- **Browse** uses existing Browse/WorkspaceSession authority for real folder navigation and progressive enumeration, with no implicit managed-scan or AI admission.
- **Managed Library** retains Query V2, LibrarySelectionV1 and existing durable metadata, records and compatibility. Hiding its ordinary mode switch **does not authorize removing or rewriting those authorities or user data**.
- The user can run one-off AI Organize and AI Cleanup directly against a selected folder/file set; Organize accepts a selected folder’s direct contents (up to 100 entries) through versioned Plan snapshots and explicit metadata AI consent; subfolders stay in place. Cleanup’s ordinary-folder workflow is the next milestone. Neither requires Managed admission.
- Window/shell presentation can be shared, but durable Managed identity and session Browse identity remain separate.

## Files parity and connected storage (Owner direction 2026-11-09)

**Required product outcome:** a user can use Fylura Files as their primary everyday file manager without repeatedly returning to Explorer or Finder. The expectation is functional parity for common tasks, not shell-process replacement, takeover of system defaults or unsupported promises of complete OS API equivalence. Track delivery/qualification in [Files parity #553](https://github.com/ArdenZC/Fylura/issues/553) alongside [product inventory #512](https://github.com/ArdenZC/Fylura/issues/512) and [native E2E #487](https://github.com/ArdenZC/Fylura/issues/487). This section is a **target and acceptance contract, not a statement that the features are already complete**.

### Beta-blocking basic Files workflows (P0)

- Open files with the OS-associated default application through double-click/Enter; enter folders through double-click/Enter; offer context Open/Open with/Reveal. Space remains Zen Quick Preview, not an OS file launch.
- Navigate mounted drives and folders, show file types/metadata, create/rename/copy/cut/paste/move/delete/restore files and directories, use system clipboard and drag interoperability, and offer ordinary keyboard shortcuts.
- Browse list/grid and details columns must support name, type/extension, size, **creation time** and modified time, both sort directions and folders-first. Sorting must apply to the authoritative directory/result set, not only currently loaded rows. The current Browse sort fields lack creation time; hidden Managed Library support does not count as a visible Files implementation.
- Common operations should not require redundant preview-confirm clicks when no user decision is needed; retain existing backend Preview/Revalidation/Journal/History/Restore. Clarify OS Recycle Bin/Trash semantics versus the existing app-managed Safe Trash.

### Accessible source types and boundaries

| Source | Required everyday Files behavior | Boundary / qualification |
| --- | --- | --- |
| Local fixed drives | Browse, open, preview and common mutations | Respect filesystem/permission capabilities and platform-specific metadata |
| Removable USB/SSD/HDD/SD media | Discover mounted volumes; show device location; browse/open/copy/move/delete; report hot-plug, disconnect, capacity and read-only failures; safe eject where supported | Never trust a reused drive letter/mountpoint as the same physical source; cross-volume mutation must preserve existing operation and recovery authority |
| Mounted SMB/NAS | Browse and supported operations with clear offline, reconnection, credentials and permission states | Network provider semantics can be weaker than local filesystem; don't claim guaranteed Restore when not available |
| Mounted cloud clients (e.g. Google Drive Desktop, OneDrive, Dropbox) | Browse provider-exposed folders/virtual drives, open on explicit user action, transfer files where supported, show unavailable/online-only/download state | Respect on-demand hydration; no implicit mass downloads from thumbnail, preview or background search/indexing; no unverified offline guarantees |
| Direct native cloud accounts (P1 after mounted source P0) | User-opt-in login, browse, upload/download and supported file modifications | Dedicated provider OAuth and scopes, token revocation, pagination/rate-limit/conflict/transfer recovery; assess official APIs individually, no mandatory paid service |

**Discovery is not authorization.** Plugging in a device, mounting a share, or making a cloud client folder visible must not automatically enroll that source into Global Index, Managed Library or AI analysis; cloud-only entries must not be eagerly hydrated. Source identity, capabilities, authorization and stale-entry checks remain backend-owned. Native installed-app tests on Windows and supported Apple Silicon macOS are required before any complete parity/Beta claim.

## Cross-cutting and contextual surfaces

| Surface | User purpose | Product boundary |
| --- | --- | --- |
| Global Search | Find files and commands across explicitly authorized supported local sources | Metadata indexing is opt-in at first launch, defaults to Smart Lean and offers scope/status settings; separate from Managed Library and AI content analysis |
| Zen Content Quick Preview | Inspect the current file/folder without entering a mutation workflow | Existing `ZenFloating` / `ZenPinned` content-preview experience over PreviewSession, Provider Registry, sourceVersion and bounded Read Gate contracts; not Operation Preview |
| macOS native-backed Zen Preview | Use stronger system-native rendering for reviewed strong-native formats inside the existing Zen Quick Preview experience | W4 adapter/presentation extension of `ZenFloating` / `ZenPinned`; preserves Managed/Ephemeral source identity, uses bounded Native Preview Access and host-bound `NativeOpaque`, and is not a Finder Preview Extension by default |
| Windows Explorer Preview Pane | Preview deliberately supported content from Explorer without launching the normal Zen UI | W4 shell-hosted read-only integration using request-scoped `HostProvided` ownership; not a second Preview/provider/read authority and not a Zen Floating window embedded in Explorer |
| Operation Preview & Execute | Review exact filesystem operations before execution | Contextual workflow/view over server-authoritative Operation Preview and revalidation; separate from content Quick Preview |
| Content Understanding | Extract/understand managed content under explicit policy and consent | Dedicated/contextual surface over Content Policy/Run/Artifact; not a sidebar primary workspace and not Global Search or filesystem mutation authority |

### Search entry points

Global Search uses the top search bar and existing shortcut; it has no duplicate sidebar destination. Files keeps a bounded current-folder search beside its path. Index authorization/settings and the separate result window remain on existing authorities. Files places file rows before optional tools and comparison panels.

### Global Search versus Files local search

They are separate products over separate authorities:

- Global Search answers “where is it?” over Global Index metadata and commands.
- Files local search answers “which files match this current Files mode?” through
  Query V2 in Library mode or the bounded current-folder Browse contract in
  Browse Folder mode.
- Content Search operates only over managed Content Artifacts and must not silently merge into Global Search.

### Content Quick Preview versus Operation Preview

The word “preview” names two different product concepts and they must remain separate:

- **Content Quick Preview** answers “what is this item?” and is read-only. W3 owns the Zen Floating/Pinned experience; W4 may add native-backed rendering inside those hosts and a bounded Explorer Preview Pane integration.
- **Operation Preview** answers “what filesystem change will happen?” and remains part of the mutation/revalidation/journal chain.

Native rendering does not authorize mutation, and Operation Preview does not become a content renderer.

### Native preview ownership

W4 intentionally exposes different native value on each platform rather than forcing feature symmetry:

- on supported Apple Silicon macOS, the initial native product surface is stronger system-native rendering **inside the existing Zen Quick Preview** for reviewed formats; a Finder Quick Look Preview Extension remains conditional on separately reviewed ownership/value;
- on Windows, the native system surface is the normal **Explorer Preview Pane** through a Zen Preview Handler for a deliberately reviewed content matrix; `WindowsQuickPreview` remains a reserved contract, not a second global preview product.

Neither surface may create renderer path authority, implicit cloud hydration, a second provider stack or a second durable source/read authority.

### Preview and recovery

Any user-file mutation must preserve the product chain:

```text
intent
→ authoritative operation preview
→ explicit confirmation where required
→ backend revalidation
→ operation/cleanup journal
→ filesystem mutation
→ durable outcome
→ History / Restore
```

No product workspace may create an alternate executor or recovery ledger. Content Quick Preview, including native-backed W4 surfaces, remains outside this mutation chain and read-only.

### AI boundary

AI is the required semantic reasoning layer for new/refreshed AI Organize and newly executable AI Cleanup plans, **not** a prerequisite for ordinary Files/Global Search. Its grouped proposals are advisory until existing deterministic source, eligibility, identity, Preview, consent and execution authorities validate them. AI must not silently authorize filesystem mutation, enable/execute rules, send cloud content, accept Organization Plan decisions, promote cleanup risks, delete files or bypass confirmation. Explicit user review should primarily be **plan/group-first and exception-first**, not per-file click-through.

## Platform experience

The same core Zen product concepts exist on Windows and supported Apple Silicon macOS, while platform adapters may select different safe filesystem and native-preview strategies. The backend/native boundary owns those platform decisions. The renderer must not infer mutation/read safety from paths, extensions or OS-name checks.

W4 native integration is explicitly capability/value driven: macOS may use stronger native rendering inside Zen while Windows exposes Explorer Preview Pane integration. Product quality is measured by truthful native behavior and preserved authority boundaries, not by equal feature counts.

## Product evolution

File Library 2.0 and the Zen Preview Platform are completed **technical foundations**; their legacy two-visible-mode UI is superseded as a *product default* by ADR-0016. Native Integration extends those existing workspace, Preview, Read Gate, journal, Safe Trash and platform authorities rather than building replacements beside them.

Release/Hardening work remains a later governance step and must not be inferred as active merely because native product surfaces are recorded here; `STATUS.md` and `ROADMAP.md` own current sequencing truth.
