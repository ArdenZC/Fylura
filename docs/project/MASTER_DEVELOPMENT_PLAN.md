# Zen Canvas — Master Development Plan

Status: canonical long-horizon development plan

Purpose: preserve the product, architecture and sequencing conclusions produced by the File Library 2.0 / Preview Platform research and keep later implementation Tracks aligned with those conclusions.

This document is intentionally more stable than `ROADMAP.md`, initiative files and Codex taskbooks. It defines the long-horizon product direction, architectural invariants, Wave boundaries and stop/escalate rules. Current progress belongs in `STATUS.md` / `ROADMAP.md`; implementation authorization belongs in the active initiative; detailed execution belongs in per-Track taskbooks.

The external-project evidence and reconstructed Round 1–4 synthesis behind this plan are preserved under [`research/file-library-preview/`](research/file-library-preview/). Those notes explain **why** the program reached these conclusions; this Master Plan remains the higher-level long-horizon direction. The exact contemporaneous W-1 working notes/source revisions were not preserved; the research directory records the reconstruction and audit provenance explicitly.

## 1. Product north star

Zen Canvas is a calm, local-first, safety-oriented **everyday file manager with file-lifecycle and optional AI intelligence**. Its long-horizon goal is to be the user's preferred daily Windows/macOS file-management workspace for supported tasks, not to forcibly replace an operating-system shell.

Its job is to help users:

- find and understand files quickly;
- work in **one File Library module** for both managed files and ordinary filesystem directories;
- navigate, search, preview, select, copy/move, compare and manage files efficiently, even when AI is disabled;
- use optional semantic organization and durable knowledge when a file is admitted to the managed Library;
- preview content quickly and safely;
- organize, deduplicate, clean up and recover files with explicit user control;
- preserve platform-native expectations on macOS and Windows.

Zen Canvas is **not** intended to become:

- an unsafe or compulsory Finder/Explorer system-shell replacement (ordinary daily-use substitution is the goal; system-level default takeover is optional and separate);
- a cloud drive;
- a general document editor or media suite;
- a universal OCR toolbox;
- an autonomous Agent shell;
- a generic MCP/tool execution host;
- a RAG/vector-database product;
- a second operating-system search engine;
- a feature bundle that hides platform limitations behind false parity.

The product should make normal file management good enough to become a daily default while preserving access to the real filesystem, platform expectations and the user's ability to keep using Finder/Explorer. No system-level interception or migration is needed to achieve this.

### 1.1 2026 product-direction refinement

**2026-10-15 Owner direction — Pulse × Oil Find × AI-only integration:**
Pulse informs complete file-manager workflows; Oil Find informs responsive
search and low-value indexing scope; the existing Zen semantic pipeline
provides optional AI suggestions; Zen Core owns safe deterministic
operations. **All ordinary file-management features belong to the single
File Library module**, not a new File Manager/Files Workspace route. This
direction refines the intended product breadth but does not imply that
unimplemented features exist today or launch a new production initiative.

The daily manager must work without an API key, offline model or AI
analysis. Existing AI-only Organize/eligible Cleanup semantic constraints
remain active when those specific AI workflows are requested. Index source
coverage, content permission and AI admission remain distinct; do not
blindly persist or analyze every system/dependency file.



The owner direction now refines that north star further:

> **Zen Canvas is an AI-native safe file workspace that stays nearly invisible when idle.**

“AI-native” means AI is the preferred semantic reasoning layer where human-like judgment is actually needed. It does **not** mean an always-on Agent, default whole-machine content ingestion, autonomous filesystem mutation, or a requirement that ordinary search/browse/filter/tag workflows invoke AI.

The five product-level rules are:

> **Search everything lightly.**
>
> **Understand only what is needed.**
>
> **Run AI only when asked or explicitly scheduled.**
>
> **Consume resources only when useful.**
>
> **Touch files only after approval.**

The architectural consequence is intentional separation:

```text
fast local metadata search
        ≠
managed File Library
        ≠
authorized content understanding
        ≠
AI semantic reasoning
        ≠
filesystem mutation authority
```

AI may propose meaning and plans. Existing deterministic safety, policy, preview, journal, Safe Trash, Restore and identity authorities decide whether and how a proposal may become a filesystem change.

## 2. File Library 2.0 product model

File Library 2.0 is **the single top-level daily file-management product**
and has two first-class working modes that share the same higher-level
workspace shell. The user-facing Chinese module is **文件库**; the existing
`Files` route/`FileLibraryWorkspace` and internal source folder names
can be preserved to avoid unnecessary migration. New file-manager
functionality **extends this module** and must not introduce a separate
sidebar-level manager, Basket, Compare or transfer product.

**Target integrated features** (not current implementation claims):
folder navigation, breadcrumbs, tabs, list/grid/sorting, later split
panes, normal user-controlled copy/cut/paste/move/rename, drag/drop,
operation progress/conflicts, cross-folder File Basket, folder/file
comparison, preview, labels, notes, favorites and Inspector. Other
top-level areas remain for their specialized purposes: cross-app Global
Search, semantic Organize, Storage Cleanup, History/Restore, Automation
and Settings.

Underneath this single experience, **Library remains managed and Browse
remains ephemeral**. Panes/Basket are references and UI intents, not new
indexes, durable file truth or execution permission. Mutation continues
through the existing authoritative Operation Preview, identity
revalidation, journal and recovery chain. Managed notes/tags do not
silently attach to arbitrary unmanaged Browse paths.

### 2.1 Library Mode

Library Mode is the managed/semantic view.

It is backed by existing managed authorities such as File Library Query V2 and managed selection. It is appropriate for:

- managed files;
- semantic/query-driven organization;
- saved views and filters;
- durable metadata and classification;
- higher-level workflows that rely on indexed/managed truth.

### 2.2 Browse Mode

Browse Mode is the familiar filesystem view.

It exists because many users want Finder/Explorer-like direct navigation and should not be forced to adopt a semantic Library workflow for ordinary file browsing.

Browse Mode is:

- filesystem/path oriented at the UX level;
- session-scoped and ephemeral at the authority level;
- progressive and cancellable;
- able to work on unmanaged locations;
- deliberately separate from durable managed-library/query truth.

Browse Mode must not silently create scan roots, index unmanaged trees, or turn ephemeral path/session state into durable database authority.

### 2.3 Shared dimensions

Library vs Browse is independent from presentation dimensions such as:

- List vs Grid;
- Inspector / Context Panel state;
- Preview state;
- per-target presentation preferences.

Do not build separate duplicated products for every combination. Share the workspace shell while keeping underlying authorities separate.

### 2.4 Platform fidelity

macOS users should encounter Finder-like concepts where appropriate; Windows users should encounter Explorer-like concepts where appropriate.

The product should feel native to each platform without pretending the platforms are identical. Platform capability, permission, provider, volume and native-preview differences must remain explicit.

Intel macOS is outside the current supported macOS target; Apple Silicon is the supported macOS architecture for this plan.

### 2.5 Fast delivery slices for the next File Library expansion

This is **future product sequencing**, not an implementation authorization
or a precondition for unrelated active fixes.

1. **U1 daily browsing polish:** existing File Library shell, truthful
   Browse/Library handoff, current-folder navigation, sorting, list/grid,
   selection and one coherent tab implementation; use existing UI/backend.
2. **U2 workspace actions:** deliver dual panes, a virtual File Basket and
   safe manual Copy/Move/Compare in small usable slices. No new executor.
3. **U3 search and index scope:** make Search→File Library handoff feel native;
   add high-value query/pinyin or coverage controls only when supported by
   existing source and measured need. Low-value indexing and AI admission
   remain distinct policies.
4. **U4 metadata/preview:** enrich the same File Library Inspector with
   notes/favorites and extend valuable preview formats incrementally.
5. **Parallel AI experience:** optional selected-file/contextual semantics,
   reusing current SemanticAssessment/Managed Scope, AI consent and queue
   contracts; no per-file model call merely for browsing.

Ship one small user-visible behavior per coherent PR, with focused
regression coverage and the existing required CI. Do not resurrect
F1/T10 optional fault-verification campaigns as a prerequisite for
ordinary File Library work. The independent safety obligations of real
user-file mutations and destructive migrations remain in force.

## 3. Core architecture invariants

These invariants are binding across Waves unless changed by an explicit architecture/governance decision.

### 3.1 Identity is not path

`FileIdentity` / backend source identity is not the same thing as `PhysicalPath`.

Paths are resolution/routing inputs. Durable identity, cache identity, operation identity and publication authority must not be inferred from a pathname when a stronger backend-verified identity exists.

Rename/move should preserve higher-level identity when the verified physical/content identity survives the operation.

### 3.2 Managed and ephemeral authorities stay distinct

Managed Library data and Ephemeral Browse data may project into common UI contracts, but their authority/lifecycle remains different.

Do not persist session-scoped Browse refs, cursors, path refs, handles or temporary provider tokens as durable truth.

### 3.3 Availability is not deletion

Offline, unavailable, disconnected, provider-remote, downloading, permission-blocked, stale and unknown states must not be interpreted as file deletion.

Managed truth is reconciled by its existing watcher/reconciliation authorities.

### 3.4 Watchers are hints, reconciliation is truth

Filesystem/provider notifications are change hints. They may invalidate cached/enumerated results and schedule bounded refresh, but must not be treated as perfectly ordered row-level truth.

Overflow or ambiguity fails toward bounded reconciliation/refresh, not fabricated completeness.

### 3.5 Progressive work by default

Large locations and large result sets must not require full enumeration before useful UI appears.

Prefer:

- progressive pages;
- bounded batches;
- cancellable requests;
- stale-publication rejection;
- visible/foreground work before speculative/offscreen enrichment.

### 3.6 Shell-first responsiveness

Interactive shell/navigation must not wait for:

- network/provider probes;
- external-volume checks;
- indexing/reconciliation;
- thumbnail generation;
- Preview restoration;
- deep folder analysis;
- Git/project enrichment.

### 3.7 Global resource budgets

Expensive work participates in `WorkScheduler` resource admission/backpressure.

No subsystem may silently create an unbounded executor or pretend to hold one resource lease while actually using unrestricted CPU/IO/native-helper concurrency.

Foreground has priority; lower-priority work still needs bounded fairness and must not starve forever.

### 3.8 Explicit materialization; no implicit hydration

Cloud/File Provider content must not be downloaded merely because Zen wants a thumbnail, Preview, hash, analysis result or deeper metadata.

Materialization/content availability is entry/source scoped, not a Location-wide truth.

Metadata-only operations should stay metadata-only when possible.

All byte-dependent consumers use the existing authoritative content-read/materialization boundary and revalidate at the actual open boundary.

### 3.9 Safety authorities are preserved

Existing authorities remain authoritative unless an explicit initiative changes them:

- File Library Query V2;
- `LibrarySelectionV1`;
- Global Index;
- managed scan-root/watcher/reconciliation truth;
- content/platform byte-read eligibility and safe open/revalidation;
- filesystem physical-identity validation;
- Operation Preview / journal / Safe Trash / cleanup / Restore;
- Rule / Analysis / Content authorities already defined by the project.

A feature Track must adapt to these authorities rather than quietly replace them.

## 4. Preview Platform north star

Preview is a platform, not a single modal component.

### 4.1 Core model

Preview is built from separable concerns:

- Preview command/context;
- `PreviewSession` lifecycle;
- source resolution + `sourceVersion`;
- authoritative content-read/materialization access;
- provider registry;
- preview representation;
- host rendering/lifecycle.

The shell/session exists before slow source/provider work so cancellation, timeout and shell-first behavior are always available.

### 4.2 Providers vs hosts

A Preview Provider understands a content/source type.

A Preview Host renders/owns the lifecycle in a specific environment.

Examples of future hosts include:

- Zen floating Preview;
- Zen pinned Preview;
- macOS native Quick Look integration;
- Windows Quick Preview/native integration.

Do not conflate provider logic with native host integration.

### 4.3 Read-only and safe by default

Preview must remain read-only and fail closed.

Default rules:

- no arbitrary code execution;
- no macro execution;
- no implicit network/remote-resource loading;
- sanitize HTML/Markdown output;
- no implicit AI/content-understanding side effects;
- archives are indexed, not silently extracted;
- folder analysis is bounded/progressive;
- generic renderer/provider code does not receive renderer-authorized arbitrary raw paths.

### 4.4 Native capability strategy

Use strong native platform capability where it provides safe, high-quality preview/thumbnail coverage, but keep native integration behind platform adapters and lifecycle boundaries.

Existing macOS Quick Look thumbnail capability is an asset to adapt, not something to rewrite for architectural symmetry.

Windows should receive equally deliberate adaptation, not macOS assumptions translated by filename/path checks.

## 5. Thumbnail infrastructure principles

Thumbnail is shared infrastructure for List, Grid, Inspector, Preview placeholders and Folder Preview.

It is not a UI-specific image-loading hack.

Binding principles:

- small / medium / large semantic variants;
- platform-scale mapping in one place;
- bounded/deduplicated/cancellable work;
- viewport/interactive work receives appropriate priority;
- byte-reading generation uses the authoritative Read Gate;
- no implicit provider hydration;
- cache identity uses stable backend-verified identity + source version + variant + renderer/version when durable reuse is allowed;
- path is not persistent logical cache identity;
- ephemeral/session-only sources use bounded session/memory caching rather than guessed cross-session identity;
- existing `MacThumbnailService` is preserved/adapted rather than replaced.

## 6. Long-horizon Wave plan

### W-1 — Research / reference discovery

Goal: understand existing platform behavior and comparable open-source/native approaches before architecture is frozen.

The detailed reference-project matrix and reconstructed Round 1–4 reasoning are preserved in [`research/file-library-preview/README.md`](research/file-library-preview/README.md), including the limits of the surviving historical evidence and the pinned re-verification snapshots.

Key conclusions carried forward:

- users value instant Space/Quick-Look-style preview because it preserves browsing flow;
- Finder/Explorer familiarity is an asset, not something Zen should intentionally erase;
- strong implementations separate filesystem enumeration, preview generation, caching, cancellation and native platform behavior rather than building one monolithic view;
- local-first and native-first approaches are preferable for the core browsing/preview experience;
- reference projects are inputs, not authorities — Zen keeps its own safety, identity and lifecycle contracts.

W-1 research is complete for the current File Library/Preview program. New research may be added when a later Wave reaches a genuinely new platform boundary.

### W0 — Specification / architecture freeze

Goal: convert research into reviewed product and architecture contracts before production implementation.

W0 freezes:

- Library/Browse IA;
- identity/location/session contracts;
- Preview architecture;
- WorkScheduler/Thumbnail/change infrastructure boundaries;
- materialization/read semantics;
- performance/QA expectations;
- W1 dependency and merge plan.

W0 does not authorize feature implementation.

### W1 — Foundation

Goal: build the shared contracts and infrastructure required before the user-facing File Library 2.0/Preview experiences.

Tracks:

- W1-00 governance/initiative activation;
- W1-01 Contract Spine;
- W1-02 Workspace Navigation / WorkspaceSession;
- W1-03 Ephemeral Browse Core;
- W1-04 Location Core;
- W1-05 WorkScheduler;
- W1-06 Preview Contract Core;
- W1-07 Materialization / Read Gate;
- W1-08 Thumbnail Infrastructure;
- W1-09 Ephemeral Change / Refresh;
- W1-10 Integration Surface;
- W1-11 Performance / QA;
- W1-12 closeout.

W1 exit gate: Foundation behavior, cancellation, stale-publication protection, resource cleanup, read/materialization safety, cross-platform compile/fixture evidence and performance baselines are proven. W1 does **not** ship the full new File Library UX.

### W2 — File Library 2.0 Experience

Goal: build the user-facing File Library workspace on top of W1 rather than smuggling UI/product decisions into infrastructure Tracks.

Planned scope:

- shared Library/Browse workspace shell;
- familiar platform-adaptive navigation;
- mode switching without forcing one mental model;
- List / Grid presentation;
- Context Panel / Inspector;
- per-target presentation preferences;
- managed Library search/filter and bounded Browse current-folder search/filter;
- selection/focus/navigation behavior shared with Preview.

Explicit restraint:

- unmanaged recursive whole-location/global filesystem search is not implicitly authorized;
- Query V2 is not replaced by a new Query V3 just to simplify UI implementation.

### W3 — Preview Platform

Goal: turn the W1 Preview Core into the user-facing Zen Quick Preview experience.

Planned scope:

- Space/toggle Quick Preview command behavior;
- floating and pinned Preview hosts;
- metadata fallback;
- rich built-in providers such as Text/Code, Markdown, JSON/YAML/XML, CSV/TSV, Folder, ZIP and Image;
- bounded sibling navigation;
- rapid-switch/cancellation/cleanup behavior;
- corrupt/unsupported/provider-failure fallback;
- large-folder Preview performance gates.

PDF/Office/iWork/audio/video and similar strong native formats should prefer safe native capabilities where appropriate rather than automatically receiving a duplicate Zen renderer.

W3 does not itself authorize Finder/Explorer system extension integration.

### W4 — Native Integration

Goal: integrate the stable Preview/File Library foundations with native platform surfaces where that integration provides real user value.

Planned macOS scope:

- Apple Silicon native Quick Look extension/host integration where appropriate;
- native lifecycle/provider/file-provider correctness;
- real fixture and signing/packaging validation.

Planned Windows scope:

- Zen Quick Preview integration aligned with Windows conventions;
- evaluate Preview Handler/Explorer integration separately rather than assuming macOS APIs/UX map 1:1;
- DPI/display/shell lifecycle/native failure QA.

Native integration remains an adapter/host problem; it must not create a second content-read, mutation or identity authority.

### W5 — Release / hardening gate

Goal: stabilize and polish the complete supported product rather than add another feature wave.

Focus:

- performance and resource steady state;
- long-session stability;
- cancellation/leak/handle audits;
- cross-platform behavior and fixtures;
- accessibility and keyboard behavior;
- security/materialization/provider hardening;
- packaging/signing/update behavior;
- visual and interaction polish;
- technical-debt deletion only where replacement/equivalence is proven.

No major feature expansion belongs in W5.

## 7. Gate and dependency rules

Wave numbering is meaningful. A later Wave must not be implemented inside an earlier Track simply because the code location is convenient.

Current critical dependency examples:

- W1 Contract Spine precedes parallel Foundation Tracks;
- Thumbnail byte-reading work requires W1 Read Gate;
- W1 Integration Surface follows the underlying Foundation contracts;
- W1 Performance/QA follows integration;
- W2 does not start until W1/F4 closes;
- W4 native integration does not start until the core Preview platform is stable enough to host it safely.

When a dependency is unavailable, stop or build only the explicitly allowed contract/fake seam. Do not bypass the dependency.

## 8. Cross-platform policy

Zen supports Windows and macOS as first-class product platforms, while preserving truthful platform differences.

Rules:

- avoid generic `isMac ? yes : assume Windows same` behavior;
- capability checks are layered: build/platform implementation, runtime environment, source/provider state and operation/read eligibility;
- unsupported native capability must be reported as unsupported, not emulated unsafely;
- network/external/provider/cloud behavior fails closed where identity/readability cannot be established safely;
- platform UX may differ when native expectations differ, while core concepts remain coherent.

## 9. Performance and scale expectations

The File Library/Preview program is designed for large real libraries, not demo-sized folders.

Preserve existing Query V2 100k/1M gates.

Foundation/experience work must be designed around:

- 100k-entry Browse progressive behavior;
- bounded first-page work;
- no full-list React render assumptions;
- cancellation on target/session switch;
- stale page/cursor/result rejection;
- bounded thumbnail generation and caches;
- scheduler interference checks with real heavy authorities;
- resource/handle/temp-file steady state;
- unavailable/offline locations without false deletion;
- platform release-build measurements before hard absolute RSS/FD ceilings are frozen.

## 10. UX principles carried from research

### 10.1 Familiarity without stagnation

Users who like Finder/Explorer should be able to browse in a familiar way. Zen should improve the workflow around that model rather than force semantic Library behavior everywhere.

### 10.2 Complexity is earned

Do not expose implementation telemetry, provider internals, architecture metadata or low-value controls in the main interface merely because the backend knows them.

Prefer progressive disclosure and a calm default surface.

### 10.3 Quick actions must preserve flow

Space/Quick Preview, navigation, open/reveal and lightweight context actions should feel immediate and non-destructive. The Preview shell appears before expensive content work and remains independently cancellable.

### 10.4 Failure is a product state

Offline, materialization-required, unsupported, stale, corrupt, permission-denied and provider-failed states receive explicit UI behavior. They are not collapsed into generic errors or hidden retries that perform surprising work.

## 11. Explicitly deferred / not authorized by this plan

Unless a new reviewed initiative changes the plan, do not silently pull in:

- OCR as a general always-on or whole-machine product module;
- whole-disk semantic/vector indexing or a RAG database over all personal files;
- automatic background AI content understanding merely because files exist or change;
- a generic always-on Agent runtime;
- shell/MCP/tool execution;
- third-party Preview plugin SDK;
- generic full-volume recursive crawling when a supported native metadata provider is unavailable;
- Query V3;
- a second watcher truth source or speculative managed-watcher rewrite;
- a second content-read/materialization engine;
- a second filesystem mutation/recovery system;
- automatic cloud hydration;
- automatic crash/usage telemetry that runs or uploads without explicit product review and consent;
- Intel macOS support;
- Linux support;
- new schema merely to make a local Track easier to implement.

Default **native metadata-only global search** is no longer considered an unmanaged-crawler feature. It remains separate from managed File Library admission, Content Understanding and AI scope, and must use platform-native metadata providers plus bounded explicit fallback rather than a hidden whole-machine recursive scan.

These ideas may be evaluated in future initiatives, but they are not silently authorized by a local implementation Track.

## 12. Development governance

### 12.1 Source-of-truth precedence

Conflict resolution is governed by [`README.md`](README.md), especially its **Source-of-truth precedence** section. This Master Plan must not redefine that ordering differently.

In short:

1. **Normative safety/privacy/correctness authority** defines what Zen is allowed to do. Security/privacy contracts, filesystem mutation/recovery contracts, permission boundaries and accepted durable safety invariants remain authoritative even when current code violates them.
2. **Descriptive implementation truth** — current production code, executable tests and the actual schema — describes what the repository does now. It is evidence of behavior, not permission to weaken a normative safety contract.
3. **This Master Development Plan** defines long-horizon product/architecture direction and Wave boundaries.
4. **Reviewed specifications and accepted ADRs** define narrower current architecture/product contracts.
5. **The active initiative** authorizes what may change now.
6. **STATUS / ROADMAP** record current repository state and authorized sequencing.
7. **Per-Track taskbooks** provide bounded execution detail.
8. **Research evidence** explains rationale/history but does not authorize implementation by itself.

If normative safety authority and current implementation truth disagree, stop and treat the mismatch as an implementation defect or governance conflict; do not use current code to silently weaken the safety constraint.

A lower-level document can narrow a higher-level authorization but cannot silently expand or contradict it.

### 12.2 Codex / agent rule

Every new implementation Track must require agents to read this document before coding.

Agents must not broaden scope because a related implementation is convenient. If implementation appears to require:

- a schema change;
- a new durable authority;
- a cross-Wave feature;
- a safety-authority rewrite;
- a performance-threshold reduction;
- platform support that the current Wave does not authorize;

stop and escalate to architecture/initiative review.

### 12.3 PR discipline

Prefer short-lived, bounded PRs with explicit authority and DoD.

Do not create a multi-week mega branch that mixes Foundation, UI, Preview providers and native integration.

Independent architecture/code review is required before merging significant Tracks even when CI is green.

### 12.4 Test-artifact hygiene

Follow `AGENTS.md` / `DEVELOPMENT_WORKFLOW.md`:

- task-owned test fixtures/staging/cache should live on the worktree/repository drive where practical;
- Windows tasks must not default large task-owned temp data to system `C:` when the worktree is elsewhere;
- task closeout removes owned temporary artifacts and reports exact residual paths if cleanup cannot complete;
- shared dependency/build caches are not deleted merely to claim cleanup.

## 13. Change control for this plan

This document is deliberately stable.

Update it only when the long-horizon product/architecture plan genuinely changes, not when a Track merely finishes or a PR number changes.

Changes that alter Wave boundaries, product positioning, core authority, platform strategy or explicit non-goals should use a dedicated architecture/governance PR with rationale.

Routine progress updates belong in `STATUS.md` / `ROADMAP.md` / initiative closeout, not here.

## 14. Current canonical supporting documents

- `docs/project/research/file-library-preview/` — reconstructed external-project research evidence, source snapshots and Round 1–4 synthesis behind the File Library/Preview program.
- `docs/project/ROADMAP.md` — current sequencing/progress.
- `docs/project/STATUS.md` — current repository truth.
- `docs/project/DEVELOPMENT_WORKFLOW.md` — execution and closeout workflow.
- `docs/project/initiatives/W1-file-library-foundation.md` — current W1 implementation authority.
- `docs/project/specs/file-library-preview/00-MASTER-SPEC.md` — W0 master specification.
- `docs/project/specs/file-library-preview/01-PRODUCT-IA.md` — File Library product/IA contract.
- `docs/project/specs/file-library-preview/02-CORE-DOMAIN-CONTRACTS.md` — identity/domain contracts.
- `docs/project/specs/file-library-preview/03-PREVIEW-ARCHITECTURE.md` — Preview architecture.
- `docs/project/specs/file-library-preview/04-INFRASTRUCTURE-CONTRACTS.md` — scheduler/thumbnail/watcher/read infrastructure.
- `docs/project/specs/file-library-preview/05-PERFORMANCE-QA.md` — QA/performance contract.
- `docs/project/specs/file-library-preview/06-W1-IMPLEMENTATION-PLAN.md` — W1 dependency/Track plan.

This Master Plan explains **why the whole program is shaped this way**. The research evidence preserves **how those conclusions were reconstructed and re-verified**; the supporting specifications define **what the currently frozen contracts mean**; initiatives/tasks define **what is authorized to change now**.

## 15. Six-layer product architecture

Long-horizon product work should preserve six conceptual layers. These are responsibility layers, not a demand for six processes or six databases.

```text
EXPERIENCE
Overview · Files · Organize · Cleanup · History · Settings

INTELLIGENCE
AI Planner · AI Cleanup reasoning · Preference Memory · Automation intent

CONTENT
bounded extraction · current/stale artifacts · summary · keywords · content identity

FILE PLATFORM
Global Search · File Library · Browse · Preview · Tags · Saved Views

SAFETY
Policy · Operation Preview · identity revalidation · journal · Safe Trash · Restore

ZERO-BURDEN RUNTIME
Resident Core · WorkScheduler · ResourceGovernor · native events · power/QoS · lifecycle
```

The lower layers constrain the upper layers:

- Intelligence must not bypass Safety.
- Experience must not become a durable fact source.
- Content must not become a prerequisite for metadata-only Global Search.
- Zero-Burden runtime policy may change **when/how much** work runs, not **what is true**.
- Safety/recovery correctness outranks power/performance optimization.

## 16. Three-level file access model

Zen must distinguish searchable, managed and AI-readable state.

### Level 1 — Searchable

Default local metadata search may include supported internal/local volumes through native platform metadata providers.

Allowed facts include bounded metadata such as name, path, kind, extension, size, timestamps and native identity.

Searchable does **not** imply:

- File Library membership;
- content extraction;
- hashing;
- thumbnail generation;
- duplicate analysis;
- AI access;
- cloud upload;
- mutation permission.

### Level 2 — Managed

A file enters managed File Library semantics only through the existing managed authorities and user/product admission paths.

Managed state enables durable Library workflows such as tags, saved views, history context and reviewed organization/cleanup workflows.

### Level 3 — AI-readable

Content understanding and AI access require their own scope/policy/consent boundary.

A file can therefore validly be:

```text
Searchable = YES
Managed = NO
AI-readable = NO
```

No implementation may collapse these levels into one “Zen has access” flag.

## 17. Zero-Burden runtime direction

Zen should behave like a native utility when idle and a richer workspace only when invoked.

The detailed runtime/lifecycle contract is frozen separately in:

- `docs/project/tasks/ZB-00-RUNTIME-RESOURCE-LIFECYCLE-FREEZE.md`

The long-horizon shape is:

```text
DORMANT_RESIDENT
≈ native utility

SEARCH_ACTIVE
≈ lightweight launcher

MAIN_FOREGROUND
≈ interactive file workspace

USER_TASK_ACTIVE
≈ bounded compute/IO service

CONSTRAINED
≈ interactive + safety-essential work only
```

Binding direction:

- no expensive work merely because the app exists;
- resident state is allowed, periodic idle work is not automatically allowed;
- Search and Main WebViews should be demand-owned rather than permanently hidden full-app surfaces;
- expensive work participates in the existing `WorkScheduler`;
- a transient RuntimeResourceGovernor may adapt admission/concurrency/QoS but must not become a second scheduler or durable job authority;
- Windows and Apple Silicon may use different native search, power and lifecycle mechanisms to deliver the same product contract;
- completed work should release resources and converge back toward the resident baseline.

## 18. Native-first default Global Search direction

Global Search should ultimately answer “where is it?” without requiring the user to first import, scan or manage the file.

That product contract is achieved through lightweight metadata, not universal deep indexing.

### Windows

Preferred supported-volume route:

```text
NTFS metadata
→ MFT initial collection
→ USN incremental changes
→ Global Index
```

The existing privileged Global Index Windows service remains a metadata sensor/provider. It must not absorb general settings, AI, mutation, Preview, File Library, cleanup or restore authority.

A native-provider failure on a full fixed/local volume must fail/degrade truthfully rather than silently becoming an expensive recursive whole-volume crawler.

### macOS

Preferred route:

```text
Spotlight / NSMetadataQuery
+ native metadata update notifications
+ bounded FSEvents reconciliation where required
→ Zen Global Search
```

Zen should consume the system metadata authority instead of rebuilding a second full-machine crawler.

### External and network storage

Internal/local fixed storage may participate by default when the supported native contract is available.

Removable/external and network/NAS sources should remain explicit/configurable and may use scheduled/manual refresh rather than permanent realtime observation when that is the lighter truthful behavior.

## 19. AI semantic authority direction

Future semantic Organize/Cleanup work should converge on:

> **AI decides meaning. Deterministic systems provide facts, policy, validation and safe execution.**

This does not move current durable authorities merely by being recorded here.

### Organize

Preferred future flow:

```text
user scope + intent
+ metadata/content facts
+ relationships
+ preference memory
+ policy context
→ AI Planner
→ structured proposal with reason/evidence/confidence
→ deterministic validation
→ existing durable Organization Plan / Operation Preview
→ user review
→ existing safe executor
```

The existing Organization Plan/journal/preview chain should be extended, not replaced.

### Cleanup

Preferred future split:

```text
deterministic detector
→ fact/signal/evidence

AI semantic reasoner
→ what those facts likely mean for this user

policy + review
→ Safe Trash / Restore authority
```

A detector may prove “same hash”, “large”, “old”, “similar” or “temporary pattern”; that does not automatically mean “delete this”.

### Rules and learning

The long-horizon Rules role should move away from semantic destination/deletion authority and toward:

- Policy;
- Trigger;
- Signal;
- Smart View.

User corrections should preferentially become **Preference Memory** supplied as advisory AI context rather than invisible auto-executing semantic rules.

No old persisted setting/rule is silently reinterpreted into broader permission. Compatibility/migration must be explicit.

### AI availability

AI unavailability must not silently hand semantic authority back to legacy rules.

Ordinary Search, Files, Browse, Preview, tags, filters and other deterministic workflows remain usable without AI.

## 20. Long-horizon program phases

These names are conceptual dependency phases, not current initiative activation. `STATUS.md` and `ROADMAP.md` remain the only project-level sequencing/authorization truth.

### Phase A — Foundation

- Zero-Burden resident/runtime lifecycle;
- native-first default Global Search;
- resource/power/QoS governance;
- background-work admission and long-run performance.

Goal: Zen becomes an extremely light desktop foundation before intelligence breadth increases.

### Phase B — Intelligence

- Content pipeline integration;
- AI semantic authority separation;
- AI Planner;
- AI Organize;
- AI Cleanup;
- Preference Memory;
- Rules-to-Policy/Trigger/Signal/View migration;
- plan-generating Automation.

Goal: AI performs the semantic work that deterministic rules cannot do well, while existing safety authorities remain deterministic.

### Phase C — Experience

- first-run/onboarding contract;
- Overview reframed around attention/recent/next action rather than technical runtime status;
- Settings simplification/progressive disclosure;
- unified failure/recovery UX;
- product workflow polish.

Goal: ordinary users understand tasks and outcomes rather than Zen internals.

### Phase D — Qualification

- resident/long-run resource qualification;
- battery/thermal/power behavior;
- native platform lifecycle;
- accessibility;
- migration/upgrade compatibility;
- installation/release evidence;
- security and recovery regression.

Goal: prove Zen can remain installed and resident on a real user's computer without becoming a burden or weakening safety.

## 21. Cross-cutting tracks

Every substantial future feature should be evaluated across these six tracks:

1. **Privacy / Consent** — what metadata/content may be read, processed locally or sent to cloud providers?
2. **Security / Recovery** — what existing identity, preview, journal, Safe Trash or Restore authority constrains the feature?
3. **Resource / Performance** — when does it exist, wake, run, pause, release and return to idle?
4. **Diagnostics** — how can failure be inspected locally without creating always-on telemetry or exposing sensitive content by default?
5. **Accessibility** — keyboard/focus/screen-reader/reduced-motion/native-display behavior throughout its lifecycle.
6. **Migration / Compatibility** — how do existing settings, rules, plans, caches and durable state survive the change?

A feature is not product-complete merely because its happy-path UI works.
