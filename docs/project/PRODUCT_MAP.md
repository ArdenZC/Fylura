# Fylura Product Map

Fylura is a local-first personal file lifecycle assistant. It does not replace Finder or File Explorer as the operating system's canonical filesystem browser. It adds managed indexing, review, content preview, safe execution and recovery around user files.

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
- The user can run one-off AI Organize and AI Cleanup directly against a selected folder/file set; the current Organize selection-to-Plan coupling is an implementation gap, not a reason to force Managed admission.
- Window/shell presentation can be shared, but durable Managed identity and session Browse identity remain separate.

## Cross-cutting and contextual surfaces

| Surface | User purpose | Product boundary |
| --- | --- | --- |
| Global Search | Find files and commands across explicitly authorized supported local sources | Metadata indexing is opt-in at first launch, defaults to Smart Lean and offers scope/status settings; separate from Managed Library and AI content analysis |
| Zen Content Quick Preview | Inspect the current file/folder without entering a mutation workflow | Existing `ZenFloating` / `ZenPinned` content-preview experience over PreviewSession, Provider Registry, sourceVersion and bounded Read Gate contracts; not Operation Preview |
| macOS native-backed Zen Preview | Use stronger system-native rendering for reviewed strong-native formats inside the existing Zen Quick Preview experience | W4 adapter/presentation extension of `ZenFloating` / `ZenPinned`; preserves Managed/Ephemeral source identity, uses bounded Native Preview Access and host-bound `NativeOpaque`, and is not a Finder Preview Extension by default |
| Windows Explorer Preview Pane | Preview deliberately supported content from Explorer without launching the normal Zen UI | W4 shell-hosted read-only integration using request-scoped `HostProvided` ownership; not a second Preview/provider/read authority and not a Zen Floating window embedded in Explorer |
| Operation Preview & Execute | Review exact filesystem operations before execution | Contextual workflow/view over server-authoritative Operation Preview and revalidation; separate from content Quick Preview |
| Content Understanding | Extract/understand managed content under explicit policy and consent | Dedicated/contextual surface over Content Policy/Run/Artifact; not a sidebar primary workspace and not Global Search or filesystem mutation authority |

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
