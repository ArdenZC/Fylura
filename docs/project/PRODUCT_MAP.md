# Zen Canvas Product Map

Zen Canvas is a local-first file manager and safe file-lifecycle assistant. **Future product target:** users can choose Zen Canvas as their main everyday Windows/macOS file manager for supported browsing, file operations, search and preview, with optional AI help. This is a goal, not a claim that current functionality already matches Finder/File Explorer, nor an authorization to replace the operating system's default shell/file-handler registration.

This map describes product ownership. Durable implementation authority is defined in `ARCHITECTURE_MAP.md`. Current initiative/Track state remains owned by `STATUS.md`.

## Primary navigation workspaces

| Workspace | User purpose | Product boundary |
| --- | --- | --- |
| Overview | See coverage, health and work that needs attention | Summary/projection only; it does not invent counts or lifecycle truth |
| **File Library** (`Files` route/current internal name) | **Single top-level home for all everyday file management:** folders, tabs/panes, copy/move, Basket, compare, file details, tags/notes, filters and preview | Extend the **existing** `FileLibraryWorkspace` with internal Library and Browse modes; Managed Library Query V2 and Ephemeral Browse retain separate backends; no parallel File Manager/Files Workspace/Basket/Compare top-level product |
| Organize Files | Review organization proposals and decide what may proceed | Durable Organization Plan review; not a second filesystem executor |
| Storage Cleanup | Analyze storage findings and move confirmed findings through the safe cleanup path | Durable Analysis findings plus Safe Trash; no renderer-owned cleanup truth |
| History | Understand changes, restore recoverable work and resolve recovery cases | Operation/cleanup ledgers and identity revalidation |

## Advanced navigation workspaces

| Workspace | User purpose | Product boundary |
| --- | --- | --- |
| Automation | Save reusable file scopes, configure Manual/Schedule/Managed files changed, enable/pause and prepare plans for review | Schema 37 Automation is merged and Owner-reviewed: `organize_plan`, review-required and never-auto-execute remain fixed. Manual/Schedule/Managed-scope-changed prepare reviewable Organization Plans; existing Rule Repository / Rule Proposal remain explicit Advanced Policies compatibility surfaces, not semantic fallback for new Organize/Cleanup. Entry remains in Settings/Spotlight. |
| Settings | Configure app, search, indexing, AI/provider, lifecycle and diagnostics | Persisted settings/provider contracts; technical detail remains secondary to task language |

### File Library is the one everyday file-management module

**Owner product direction (2026-10-15):** all ordinary file-management features
live **inside the existing File Library module**, under the existing Files
route. "Files Workspace" may remain a source-code/internal shell name; it is
**not** a second user-facing file-manager product or sidebar entry.

```text
File Library (single top-level module; existing Files route)
├─ Library mode — managed files / saved views / tags
├─ Browse mode — normal directories and unmanaged locations
├─ Shared navigation — tabs, breadcrumbs, list/grid, sorting, later split panes
├─ File actions — select, clipboard, copy/move/rename, drag/drop, batch progress
├─ Basket & Compare — cross-directory intent and read-only differences
├─ Inspector & Notes — details, labels, favorites, metadata where authorized
└─ Preview — context/floating/pinned, backed by existing Preview/Read Gate
```

This is a **target feature hierarchy**; Tabs, Panes, Basket, Compare, rich
Notes and enhanced file operations are **not** declared implemented merely by
appearing here.

- **Library mode** remains the managed projection using File Library Query V2,
  `LibrarySelectionV1` and managed-file metadata authority.
- **Browse mode** remains session-scoped and ephemeral using existing
  BrowseService/WorkspaceSession. Opening an unmanaged directory never
  silently enrolls it for indexing, persistent tags or AI.
- Tabs and panes are presentation/navigation containers with independently
  scoped location, history and selection; **not** new durable file authorities.
- Basket holds file references and intended actions, not copied file bytes,
  permission to read content, or permission to mutate. Confirmed actions
  still use the existing Operation Preview/revalidation/journal/Restore chain.
  Compare starts read-only; comparison does not authorize sync or deletion.
- Tags, favorites and notes should reuse/extend the canonical managed
  metadata authority as appropriate. An unmanaged Browse reference does not
  magically inherit managed metadata or stable identity. Design its permitted
  persistence/identity behavior in the feature's bounded task, not as a new
  general file database.
- Quick Preview stays the existing shared provider/Read Gate, even when
  launched from a File Library item, Basket or Search handoff. Floating/pinned
  hosts remain contextual rather than separate file manager modules.
- Do **not** create parallel top-level `File Manager`, `File Workspace`,
  `Basket`, `Compare`, `Transfers`, or `Notes` workspaces. A contextual
  dialog/overlay within File Library is fine.

**Existing adjacent product modules remain distinct:** Global Search is a
cross-app overlay and passes identity/location into File Library for follow-on
actions; Organize Files reviews semantic plans; Storage Cleanup owns analysis
and Safe Trash; History owns recorded outcomes and Restore; Automation and
Settings retain their current roles. File Library may link/launch these
workflows without becoming their persistence or execution authority.

For future usability, Zen Canvas aims to replace *daily use* of Finder/Explorer
for supported workflows. **Optional OS-default interception/registration**
(e.g., Windows Win+E) is a separate late-stage choice; never a prerequisite
for a functional File Library or claimed for unsupported macOS shell surfaces.

## Cross-cutting and contextual surfaces

| Surface | User purpose | Product boundary |
| --- | --- | --- |
| Global Search | Find files and commands across configured global index sources | Cross-cutting global metadata search; separate from managed File Library search and Content Search |
| Zen Content Quick Preview | Inspect the current file/folder without entering a mutation workflow | Existing `ZenFloating` / `ZenPinned` content-preview experience over PreviewSession, Provider Registry, sourceVersion and bounded Read Gate contracts; not Operation Preview |
| macOS native-backed Zen Preview | Use stronger system-native rendering for reviewed strong-native formats inside the existing Zen Quick Preview experience | W4 adapter/presentation extension of `ZenFloating` / `ZenPinned`; preserves Managed/Ephemeral source identity, uses bounded Native Preview Access and host-bound `NativeOpaque`, and is not a Finder Preview Extension by default |
| Windows Explorer Preview Pane | Preview deliberately supported content from Explorer without launching the normal Zen UI | W4 shell-hosted read-only integration using request-scoped `HostProvided` ownership; not a second Preview/provider/read authority and not a Zen Floating window embedded in Explorer |
| Operation Preview & Execute | Review exact filesystem operations before execution | Contextual workflow/view over server-authoritative Operation Preview and revalidation; separate from content Quick Preview |
| Content Understanding | Extract/understand managed content under explicit policy and consent | Dedicated/contextual surface over Content Policy/Run/Artifact; not a sidebar primary workspace and not Global Search or filesystem mutation authority |

### Global Search versus File Library local search

They are separate products over separate authorities:

- Global Search answers “where is it?” over Global Index metadata and commands.
- File Library local search answers “which files match this current File Library mode?” through
  Query V2 in Library mode or the bounded current-folder Browse contract in
  Browse mode.
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

AI/provider results are advisory unless an existing durable contract says otherwise. AI must not silently authorize filesystem mutation, enable rules, execute rules, send cloud content, accept Organization Plan decisions or bypass confirmation.

## Platform experience

The same core Zen product concepts exist on Windows and supported Apple Silicon macOS, while platform adapters may select different safe filesystem and native-preview strategies. The backend/native boundary owns those platform decisions. The renderer must not infer mutation/read safety from paths, extensions or OS-name checks.

W4 native integration is explicitly capability/value driven: macOS may use stronger native rendering inside Zen while Windows exposes Explorer Preview Pane integration. Product quality is measured by truthful native behavior and preserved authority boundaries, not by equal feature counts.

## Product evolution

File Library 2.0 and the Zen Preview Platform are completed foundations. **The proposed daily-file-manager expansion is not yet implemented**: Tabs, split panes, Basket, Compare, additional manual file actions and richer notes should be delivered incrementally **within the existing File Library module**, reusing Preview, Read Gate, journal, Safe Trash and platform authorities rather than building replacements beside them.

Release/Hardening work remains a later governance step and must not be inferred as active merely because native product surfaces are recorded here; `STATUS.md` and `ROADMAP.md` own current sequencing truth.
