# Zen Canvas Current Architecture Map

This document records the current architecture and ownership model. It is intentionally shorter than historical remediation/design taskbooks and should change only when real architecture ownership changes.

## System shape

```text
React / TypeScript UI
        │
        ├─ App Shell and workspace components
        ├─ Zustand projection / interaction stores
        └─ domain API facades
                 │
              Tauri IPC
                 │
             Rust backend
        ┌────────┼─────────┐
        │        │         │
      SQLite   domain    platform
      ledgers  engines    adapters
        │        │         │
        └──── durable authorities ────┘
```

The normal direction is **durable backend authority → API → replaceable frontend projection**. UI state may own interaction state, request epochs, local selection presentation, focus, dialogs and disposable Preview host presentation; it must not become a second durable fact source.

## Durable authority table

| Domain | Current authority | Frontend role |
| --- | --- | --- |
| Global Search | Global Index / Global Search repository and backend ordering | Query interaction, command grouping, result presentation |
| Managed file browsing | File Library Query V2 | Query projection, cursor/snapshot handling, inspector and interaction |
| Cross-page selection | `LibrarySelectionV1` plus backend resolution | Selection intent/projection |
| Ephemeral filesystem browsing | W1 `BrowseService` session/request/enumeration/opaque refs | Current target/page presentation, loaded-entry interaction and cancellation projection |
| Workspace navigation/presentation | Per-pane `WorkspaceSession` disposable navigation/history/presentation contract plus source owners | File Library tabs/dual panes, Back/Forward, presentation, focus and transient workspace interaction |
| Preview lifecycle / provider publication | W1 Rust `PreviewSession`, SourceResolver/sourceVersion, Provider Registry/fallback and typed `PreviewRepresentation` contracts | Floating/Pinned host presentation, command context, request epoch/focus restoration and representation rendering only |
| Preview byte-read eligibility/access | W1 `MaterializationReadGate` plus existing authoritative platform/content open/revalidation boundary | Eligibility/materialization state projection; no general renderer byte-read lease/path authority |
| Expensive Preview/Thumbnail/Foundation work | existing global `WorkScheduler` resource admission | Work priority/request intent projection only |
| Scan | durable scan roots/sessions/runs | Scan controls and durable event projection |
| Watcher health | backend watcher reconciliation and root revisions | Health projection and refresh coordination |
| Duplicate detection | durable Dedupe runs/groups/members | Run/group projection |
| Storage analysis | durable Analysis Run/Finding/Evidence/Decision | Review, local selection interaction and preview requests |
| Organization | durable Organization Plan/Plan Item ledger; current Managed AI semantics enter only through a backend resolver as proposal inputs | Review projection, pagination and revision-aware interaction |
| Rules | Rule Repository V2 and catalog revision | Replaceable rule-library projection |
| Natural-language rules | durable Rule Proposal | Proposal/review projection |
| Content | Content Scope Policy, Content Run and Content Artifact | Policy/run/artifact projection and interaction |
| File operations | authoritative Operation Preview and operation journal | Preview selection and progress projection |
| Cleanup mutation | Safe Trash and cleanup journal | Selection/confirmation/progress projection |
| Restore | operation/cleanup ledgers plus identity revalidation | Restore intent, confirmation and outcome projection |
| App settings | persisted versioned settings | Editing/reconciliation projection |
| AI readiness / consent | existing AI settings + credential store, backend-owned Managed Scope policy, Content Scope Policy and distinct Cleanup local/cloud sharing policy composed by `crate::ai::readiness`; no separate readiness store | Settings/status presentation only; no renderer-created authority |
| Automation | PM-02B Schema 37 Intent/Run + trigger state; scanner/watcher root change clock; one wake/deadline coordinator admitted by existing WorkScheduler Background; atomic Run + existing Plan | Trigger editor, manual Run now, backend next due/source/deferred/skip projections and existing Organize handoff; no provider loop or filesystem executor |
| Managed AI | existing durable managed-AI queue and provider policy; canonical per-file `SemanticAssessmentV1` lives in `ai_analysis_state.classification_json` | Configuration/progress projection |

Preview in the table means **content Quick Preview**, not Operation Preview. File-operation planning remains owned by the existing mutation/operation authorities and is not merged into W3 Preview Platform.

AI readiness is a derived backend projection, not a new durable authority. `crate::ai::readiness` composes current AI settings and keyring credential truth with backend-owned scope/policy identity. Provider readiness distinguishes disabled, invalid provider/model configuration and missing cloud credential without fabricating live-connectivity status. Provider blockers are evaluated before feature-specific consent. Managed metadata AI applies the current Managed Scope `allow_local_ai` / `allow_cloud_ai` policy; Content Understanding separately applies Content Scope Policy `enabled/local_allowed/cloud_allowed` and still requires the existing per-run confirmation; Cleanup AI separately applies `cleanup_local_ai_allowed` / `cleanup_cloud_ai_allowed` from the existing versioned AI settings. These three consent domains are not merged or interchangeable.

Readiness bindings are fail-closed snapshots, not persisted `ai_ready` flags. Provider settings revision plus credential-presence truth feed the provider binding; Managed Scope policy/identity and Content root/policy revisions feed their context bindings. Cleanup readiness additionally binds Cleanup feature enablement, local/cloud sharing permission and parent/full-path disclosure settings. Consumers that depend on currentness must re-evaluate or use backend current-binding helpers rather than trusting a stale renderer snapshot. Managed metadata disclosure reports filename metadata plus current parent/full-path settings and no file content. Cleanup disclosure likewise includes candidate name and deterministic candidate metadata, with parent/full path controlled by the existing privacy switches and no file content. Content disclosure remains separate: bounded extracted content may be included only through the authorized Content provider path, while path/filename/secrets remain excluded from that provider payload. No readiness API grants filesystem mutation, Operation Preview, confirmation, journal or Restore authority.

Managed AI semantic persistence reuses the existing `ai_analysis_state.classification_json` slot. The backend canonicalizes provider output into `SemanticAssessmentV1`, explicitly decodes valid V0 classification rows, and binds each assessment to its Global Index entry, Managed Scope, provider and input fingerprint. The Organization Plan resolver rechecks current File Library/Global Index identity, enabled scope/provider policy, completed queue state and user-correction state before deriving deterministic proposal inputs. `targetTemplate` remains a relative hint; target paths and Operation Preview still come from existing backend builders and operation authorities. Cleanup retains its Analysis Finding lifecycle, and Rule Proposal remains policy-authoring assistance.

Cleanup AI provider access is additionally gated by a distinct feature-level data-sharing policy in existing versioned AI settings. Legacy settings with no Cleanup consent fields deserialize fail-closed. The configured Cleanup command reloads current settings plus credential truth from the backend, derives Cleanup readiness, and rejects the request before provider construction/work unless the active local/cloud provider has matching Cleanup permission. Managed Scope or Content Understanding permission cannot satisfy this gate; frontend switches edit persisted policy but do not become authority. No schema migration, second readiness store or second provider queue is introduced.

Cleanup AI enrichment remains attached to the existing Analysis Finding authority. A provider request is bound to the exact backend candidate IDs, Finding revisions, Analysis Run/source snapshot, detector revisions and persisted AI settings fingerprint. After provider work, live identity and policy are revalidated; publication then checks those same preconditions again in one immediate database transaction and updates only Findings whose IDs were returned exactly once. Missing, duplicated, malformed, unknown or stale responses do not create current AI assessment evidence. Cleanup `size` remains a reclaim estimate; when it differs from a file's logical length (including macOS allocated-size behavior), the identity snapshot records `logicalSize` separately and live revalidation compares that value to filesystem metadata. Analysis Finding decisions, Operation Preview, explicit confirmation, Safe Trash journals and Restore remain the mutation/recovery chain; no schema migration or second AI queue is introduced.

`ai_assessment` evidence exists != current AI assessment. The backend-only `crate::ai::cleanup::has_current_ai_assessment(db, finding_id)` predicate reloads the durable Finding and returns false unless the current Finding is active, its live source identity still matches, the candidate appears exactly once in a unique requested set with matching fingerprints, the Finding revision relation and `published` / `compareAndSwap: succeeded` markers match, the completed Cleanup Analysis Run is still at the captured pre-publication revision plus its single transactional aggregate refresh with unchanged source-snapshot and detector-set hashes, the detector remains completed at its recorded revision, persisted provider settings still match, and exactly one durable `ai_assessment` evidence row matches `evidence_summary.aiAssessment`. PM-01 Cleanup gating must call this predicate or a richer backend status API; renderer code must not infer currentness from raw evidence JSON. This predicate is not a Tauri renderer command and grants no filesystem-mutation authority.

U2's `FileWorkbenchModel` owns only Main-window UI instances and their cleanup.
`createFileLibraryProjection` instantiates the existing Query V2/selection/inspector
projection per pane; the default export remains the Main compatibility projection.
It does not create another backend query, file identity or permission authority.
Hidden panes pause observation while retaining their current opaque Browse refs.
History with complete existing restore locators may release inactive sessions;
Back/Forward obtains fresh backend admission instead of reviving released refs.
U1 transfer tickets, clipboard/basket, Operation Preview, confirmation, journal and
History/Restore remain shared through their existing APIs. Only the focused pane
owns keyboard intent; mutation refresh is routed to affected pane projections.

## W1-to-W2 consumer boundary

W1 runtime contracts are not consumer-ready merely because a type or backend command exists. Before W2 shared presentation work, each public producer and consumer proved that it could carry the owning authority and lifetime:

- Thumbnail requests carry truthful Browse source-generation/session/stale/Read Gate checks;
- LocationDescriptor remains a projection and backend-owned admission/navigation owns live Browse authority;
- Library all-matching membership retains exact Query V2 collection context;
- Browse presentation preserves sessionId, requestId and enumerationId and keeps BrowsePathRef source-specific/session-paired;
- CI evidence binds checked-out source, diff head and reported artifact identity to the exact validated commit.

R1/R2/R3/R4 closed these consumer-boundary prerequisites before the completed W2 experience.

## W3 Preview consumer boundary

W3 starts from a real W1 Preview foundation, but the product-facing consumption seam is intentionally incomplete and must be made consumer-ready before rich providers are added.

### Existing authoritative foundation

- `src-tauri/src/file_workspace/preview.rs` owns PreviewSession lifecycle, sourceVersion stale protection, provider contracts/priority/fallback, representation families, capability intersection and deterministic cancellation/disposal semantics.
- `src-tauri/src/file_workspace/integration/preview.rs` resolves managed sources through existing managed File Library detail authority and ephemeral sources through the same BrowseService that issued their refs.
- the integration uses MaterializationReadGate for read eligibility/source version and injects its opaque bounded content-read consumer into Preview providers.
- Preview Tauri commands are main-window-authorized and expose create/snapshot/start/cancel/dispose/switch-source lifecycle without exposing arbitrary paths or general byte-read leases to React.
- WorkScheduler remains the global expensive-work admission authority.

### W3-01 consumer-readiness record

W3-01 closes the previously intentional production-consumption seams without
moving durable authority:

- `preview_policy::production_preview_provider_registry()` is the single
  production composition owner. Its bounded rich-provider set is intentionally
  empty until later built-in provider Tracks are reviewed; Metadata fallback
  remains the truthful integrated behavior.
- `preview_policy` owns explicit activated Zen Floating/Pinned host matrices
  and backend source capability projection. Native W4 host kinds remain
  contract-only and fail closed.
- Rust and TypeScript use the same exhaustive strict representation and warning
  wire. Unknown families/fields, host-mismatched native opaque values and
  path-like asset tokens fail closed at the consumer boundary.
- Preview Core owns a direct bounded progressive publication callback. Each
  update is bound to the session/request/sourceVersion token, ordered by a
  monotonic sequence, and rejected after switch/cancel/dispose or provider
  timeout. The callback retains only the current projection; it is not an
  app-wide event bus or an unbounded queue.
- `preview_asset::PreviewAssetRegistry` owns ephemeral bounded asset bytes.
  Tokens are process-local, request/sourceVersion-bound, TTL-limited and
  revoked with Preview lifecycle; the only retrieval command is
  main-window-authorized and Preview-specific.
- No general renderer-callable materialization/download action is assumed.
  `materialization_required` remains an explicit state unless an authoritative
  user-initiated materialization action is separately reviewed.

### W3 host/projection rule

W3 frontend code may own only disposable host/interaction coordination:

- Floating/Pinned visibility;
- frontend request epochs;
- mapping current W2 entry identity into PreviewSourceRef;
- command-context gating;
- shell/render state;
- focus restoration;
- bounded sibling-navigation projection;
- cancel/dispose/switch-source orchestration and frontend late-result rejection.

It must not own:

- provider selection truth;
- sourceVersion;
- filesystem resolution;
- byte-read/materialization authorization;
- durable query/selection truth;
- file mutation/recovery authority.

Providers produce typed representations; Hosts render them. A Host does not infer provider/read capability from a filename/path, and a Provider does not import React host state.

## W4 native-host boundary

W4 adds native host adapters without moving the durable Preview authorities above. ADR-0005 is the binding architecture decision.

### Native request ownership

`PreviewSourceRef::HostProvided { hostToken }` is the reserved seam for an OS-owned/native request. W4-01 may make that seam usable through one bounded backend/native request registry, but the token remains opaque, request-scoped, non-durable and revocable. It must never be a disguised filesystem path.

A platform-supplied stream/file handle may back one native request for its bounded lifetime. That request-scoped authority does not become a second generic renderer ReadGate or durable file identity source.

### Zen-owned native access ownership

The initial macOS native-backed path is still a normal Zen Preview request: `ManagedFile` / `EphemeralBrowse` plus sourceVersion remain source authority and `ZenFloating` / `ZenPinned` remain host authority.

Quick Look's asynchronous URL open does **not** weaken the W1 Read Gate rule that previous eligibility is not durable authorization. W4-01 may add a process-local **Native Preview Access registry/lease**, but that component is only a bounded consumer/staging owner:

- it is bound to Preview session/request/sourceVersion/host;
- it obtains source data through the existing `MaterializationReadGate` and authoritative identity-checked platform open/read boundary;
- it creates only complete Zen-owned private staging snapshots and performs final sourceVersion/freshness revalidation before allowing `NativeOpaque` publication;
- it never turns `MaterializationRequired`, `Downloading`, `MetadataOnly`, permission, availability or identity failures into hydration;
- it never treats a checked-once original source URL or path-based copy-after-preflight as equivalent to authoritative access;
- it is bounded by explicit staging byte/disk/deadline/concurrency limits and cleans staging on switch/cancel/dispose/failure/expiry;
- its staged URL is backend/native-only presentation data, not durable file identity, not a generic renderer path and not a second byte-read authority.

A future direct native-access mechanism may replace staging only after architecture review proves equivalent actual-open identity and no-hydration semantics.

### Platform host ownership

- macOS initial W4 scope is a Zen-internal native Quick Look-backed host/fallback for strong-native standard formats. Existing Quick Look thumbnail infrastructure remains separate. A Finder Quick Look Preview Extension is conditional on a separately reviewed custom-UTI/native-gap ownership case rather than being broadly registered for standard formats.
- Windows W4 system scope prioritizes `WindowsPreviewHandler` / Explorer Preview Pane integration. `WindowsQuickPreview` remains a reserved inactive contract unless a separate product review proves distinct value beyond W3 Floating Preview.
- native host selection, provider selection and source eligibility remain backend/capability-driven; native code must not infer authority from extension/path alone.

### Cross-process/process-local work

A native extension/COM preview host may need process-local handles, streams, rendering resources and admission limits because it is a real OS process boundary. Those resources may be locally owned for lifecycle correctness, but they are not a second product-level Provider Registry, MaterializationReadGate, WorkScheduler policy or mutation authority.

If sharing W3 provider/representation code across a native process requires extraction, the result must preserve one provider contract and one authoritative composition policy rather than fork platform copies.

### Native cleanup boundary

Native cancel/unload/close must revoke host/native-access request ownership and release platform streams/handles/staging/renderers/assets. In particular, Windows Preview Handler `Unload` is a hard cleanup boundary and the source must not remain locked after unload where the platform permits subsequent mutation.

## Runtime ownership

`App.tsx` is a small composition boundary. `AppRuntimeProviders` currently coordinates several lifecycle concerns including settings, capabilities, scan/watcher/background indexing, search-window integration and other startup/runtime effects.

That concentration is a **hardening target**, not evidence of a second durable authority. Future work may split runtime ownership into focused providers/controllers while keeping one application composition root.

### Resident process and on-demand windows

- The Tauri process may remain resident while no app WebView is open. Main and standalone Search WebViews are created when needed; Main-to-background and Search-close paths destroy their WebView and release its window-owned runtime.
- `ExitIntentState` is a process-local, non-persistent lifecycle signal. Only internal teardown that removes the last WebView can arm a one-shot stay-resident decision. Explicit Quit records exit intent; an unmarked native/system exit shuts down resident owners.
- `FileWorkspaceRuntimeOwner` lazily creates one runtime for a Main-window generation and disposes it at Main teardown. Reopening Main acquires a fresh generation. Browse sessions, refs and page cursors remain owned by the existing BrowseService and are not promoted into process-wide durable state.

### Idle worker and platform wake models

- The Global Index coordinator waits on one bounded process-local wake slot. Startup, provider changes, source-topology changes, explicit commands, lifecycle resume and recovery notifications coalesce into wake hints; durable Global Index providers/repository remain the row and status authorities. The existing cheap source-topology safety audit is separate from incremental work and may wake a normal cycle when topology changes.
- Windows Global Index uses MFT for the fixed-NTFS baseline and USN Journal for incremental changes. Desktop filesystem notifications wake the coordinator; they do not write durable rows.
- macOS Spotlight query/update results supply Global Index rows. FSEvents supplies a reconcile/checkpoint signal and wakes the coordinator; it does not supply durable row truth.
- The macOS lifecycle observer registers native notifications and owns a dormant default-mode CFRunLoopSource that keeps its run loop blocking while idle, without a periodic timer. Stop signals the source to cover pre-entry races and sends native stop/wake; the worker clears the stop handle, removes/invalidates the source, removes observers and joins. An unexpected run-loop return records a reconciliation-required diagnostic. Global Index Spotlight/FSEvents watchers use the same macOS run-loop stop utility.
- FileWorkspace ephemeral change monitoring uses a bounded `Notify`/`Stop` channel. Its idle worker blocks on receive; only active event coalescing uses a bounded timeout. Overflow becomes `Uncertain` and routes through the existing Browse invalidation/re-enumeration authority.
- OS notifications and ephemeral change events are hints. They can wake, invalidate or request reconciliation, but they do not become durable per-row truth.

W3 Preview frontend orchestration should be a bounded `PreviewExperienceController`/provider rather than another responsibility appended independently to LibraryMode, BrowseMode, List, Grid and Context Panel. The exact file/module name may differ, but one consumer-facing Preview lifecycle coordinator is the preferred ownership shape.

## Pre-Rebaseline authority freezes

Two proposed ADRs are under Owner review before the next Product / Architecture Rebaseline:

- [ADR-0012 — Files Authority and Compatibility Retirement Boundary](DECISIONS/0012-files-authority-and-compatibility-retirement.md), tracked by [#332](https://github.com/ArdenZC/Zen-Canvas/issues/332). It freezes new dependency on TD-001 `useFileLibraryStore` and TD-015 Vault/File Library compatibility for future File Workspace work while preserving Query V2, LibrarySelectionV1, BrowseService, WorkspaceSession, Preview and mutation authority separation.
- [ADR-0013 — Managed AI Eligibility and Queue Authority Boundary](DECISIONS/0013-managed-ai-eligibility-and-queue-authority.md), tracked by [#333](https://github.com/ArdenZC/Zen-Canvas/issues/333). It preserves one `ai_jobs` queue, one ManagedAiWorker and existing WorkScheduler while separating Index, Managed Scope, AI Eligibility, current-assessment reuse, analysis and mutation eligibility.

Both ADRs are **Owner-accepted for PR #334 and become effective only when that PR merges**. They do not authorize File Workspace or AI Eligibility implementation and do not change current runtime truth before merge.

## Compatibility bridges

Compatibility code is allowed only when it translates into a single current authority and has a deletion condition.

Current known bridges include:

- `src/store/useFileLibraryStore.ts` — legacy scope/stats/scan/AI compatibility umbrella while Query V2 is the managed-library query authority.
- renderer watcher legacy adapter — fallback when backend watcher reconciliation capability is unavailable.
- `src/store/useOrganizeDecisionStore.ts` — edited-name continuity bridge in older operation-preview wiring; not Organization Plan authority.
- `src-tauri/src/global_index/legacy_queue.rs` — compatibility adapter into the existing managed-AI durable queue.
- legacy design-token aliases — migration layer for older production surfaces.
- File Library Preview compatibility — current Library Mode still reaches `FileLibraryPreviewDialog` / Vault Inspector compatibility, and the Inspector may use the existing macOS Quick Look thumbnail path. These are migration inputs while W3 activates the shared Preview Core/Host path; they are not a second W3 Preview authority.

Detailed exit conditions remain recorded in `docs/remediation/LEGACY_RETIREMENT_PLAN.md` and `TECH_DEBT.md` until migrated into accepted debt-retirement changes.

`useOperationQueueStore.syncPreviews(files)` has no known production caller at the G0 audit baseline and remains a retirement candidate; Operation Preview is unrelated to W3 Quick Preview despite the shared word “preview.”

For TD-015, W3 may retire a preview-specific compatibility caller only after the owning W3 replacement is active and behavioral/real-browser equivalence is proven. Broader Vault/File Library compatibility retirement remains independently gated.

## Platform boundary

Platform filesystem strategy is backend-owned.

- Windows retains its existing source-handle and verified-directory authority.
- macOS 13+ Apple Silicon uses the dedicated macOS identity, mutation, provider, Finder, lifecycle, copy/package and Quick Look adapters. Mutation correctness is split into namespace identity, optional content-verification identity and coordinated provider-URL evidence; a provider path hint and provider-internal item/domain ID are not generic provider authority.
- macOS name-based mutation requires verified parent identity, current leaf-entry identity obtained through the retained parent descriptor, and retained object identity. Same-volume namespace operations do not require complete content hashing; copy/cross-volume/recovery policies may require it.
- Provider coordination is operation-aware and treats accessor-supplied URLs as authoritative. Safe Trash uses a source/actual-target pair while Permanent Delete uses a single deleting source. Under ADR-0003 Decision B, generic File Provider paths use `NSFileCoordinator` plus user-visible URL and physical-identity revalidation; the public item/domain manager APIs remain extension-scoped diagnostics, not a prerequisite for arbitrary third-party providers.
- Source retirement is an explicit capability decision (`ExclusiveClaim`, `ProviderCoordinated` or `PortableNamespaceRetirement`). Portable claims use the Zen-owned mode-0700 `.zen-canvas-retirement/<session>/` namespace; unknown/read-only/disconnect-unverified volumes fail closed and target-first cleanup remains recoverable through the existing journal and History actions.
- existing macOS Quick Look thumbnail capability is a Thumbnail/placeholder asset that may be adapted safely; W3 does not reinterpret it as Finder Quick Look extension authority.
- W4 owns reviewed native Preview host integration while preserving the existing identity/read/mutation boundaries; Zen-owned native staging is an ephemeral presentation artifact created through the existing Read Gate, not another filesystem/read authority.
- Linux is not a product target.

Shared product code must depend on capability/strategy results rather than reimplement platform safety in the renderer.

## Non-negotiable invariants

Do not create:

- a second Global Index;
- a second managed File Library query authority;
- a second Browse identity/session authority;
- a second Preview lifecycle/provider/publication authority;
- a second Materialization/Read Gate or renderer byte-read engine;
- a second global WorkScheduler/resource-policy executor;
- a second durable AI queue;
- a second Organization Plan ledger;
- a second Rule repository or Rule execution authority;
- a second operation journal, Safe Trash or restore ledger;
- renderer-authoritative filesystem paths, totals or completion facts;
- a generic Agent/shell/MCP/tool runtime without a separately approved architecture decision;
- arbitrary third-party Preview DLL/dylib/plugin loading in W3/W4;
- schema changes merely to simplify UI/Preview implementation;
- native platform integration that bypasses ADR-0005 host/source/native-access lifecycle ownership.

Global Index, managed File Library and managed Content Search remain separate data domains.

## Architecture-change rule

A change that moves durable authority, persistence ownership, command permission ownership, platform mutation ownership, supported-platform truth or recovery ownership requires:

1. an accepted initiative scope;
2. an ADR under `docs/project/DECISIONS/`;
3. updates to this map and `STATUS.md`;
4. focused contract tests and applicable full validation.

W4 activation records the native host/process/native-access boundary in ADR-0005 without moving existing durable Preview/read/identity/mutation ownership. If a W4 implementation Track discovers that its proposed solution would move one of those authorities, it must stop and return to architecture review before coding further.


## PM-02A manual Automation boundary

The main-window-only API resolves reusable File Query V2 semantics against healthy enabled roots and a fresh backend library revision on every new request. `current_scan`, Browse paths, renderer-selected IDs/counts and saved snapshot revisions are rejected as Intent authority. Current Managed AI semantics remain owned by Organize; its builder is extracted unchanged into `db/queries/organization/materialize.rs` so one immediate transaction can publish a Plan and Run receipt.

Missing/stale analysis is admitted only after existing backend readiness, provider/credential and managed-scope consent checks, in batches of at most 100 through `analyze_organization_plan_items`. Current valid assessments bypass fresh provider readiness. The existing queue/governor owns subsequent analysis. Automation never accepts decisions, requests Dry Run or executes files. The PM-02A manual service starts no timer/worker; the authorized PM-02B coordinator is documented below. Crashes after Plan publication retain a terminal blocked receipt and reviewable Plan; retry returns the existing receipt and cannot enqueue again. Organize owns explicit analysis/refresh and all later review/execution.

[Schema 36 contract](SCHEMA_36_AUTOMATION_INTENTS.md) and [ADR-0010](DECISIONS/0010-manual-automation-intent-boundary.md). PM-02A is merged / Owner accepted. Its manual contract remains shared with PM-02B; PM-03 was inactive at that historical closeout and was authorized separately later.


## PM-02B Event / Schedule Trigger boundary

Activation PR #316 is merged; PM-02B implementation PR #317 is **COMPLETE / MERGED** as `master@fc433f305aafbc2326a15a39eb93476ebd4da20d` / tree `f7ce37786816b1af31a43f70e351430067790f6d`, with Owner Review PASS and merge-after master CI 37265536740 SUCCESS. Accepted production candidate remains 823edc2b87c958f4bfdf7f6ba4211328140ccca2 / tree 63918653f3791386cb86f7208b01deda96ea7f89. [Schema 37](SCHEMA_37_AUTOMATION_TRIGGERS.md), [ADR-0011](DECISIONS/0011-automation-trigger-boundary.md), and [result](tasks/AI-ONLY-PM-02B-EVENT-SCHEDULE-TRIGGER-RESULT.md) record the bounded extension. The PM-02B closeout did not activate PM-03; activation PR #320 did so separately.

| Module | Responsibility |
| --- | --- |
| `db/schema_automation_triggers.rs` | Atomic 36→37 migration / preserved history / bounded publication and recovery fields |
| `db/automation/types.rs`, `repository.rs` | Strict Trigger V2 wire types, CAS Intent state/reset and truthful runtime projection |
| `db/automation/calendar.rs` | Explicit IANA minute/day recurrence, fold/gap and bounded newest-due catch-up through Jiff |
| `db/automation/trigger_state.rs` | Root map baselines/coalescing, exact revision claim, deterministic key and receipt-before-cursor recovery |
| `db/automation/runtime.rs` | One condition-variable worker, nearest deadline, hints, cancellation/join and existing Background admission |
| `db/queries/scan.rs` | Existing scanner/watcher transaction publication; filesystem-only root clock + read-only physical identity observation; postcommit wake |
| `db/automation/service.rs` | Shared manual/automatic fresh Query V2→existing Plan→existing Managed AI admission→Run; review-pending suppression |
| `main.rs` / existing macOS lifecycle | Startup recovery precedes coordinator start; sleep/unmount pause; recovery precedes resume; teardown joins Automation before AI shutdown |
| `platform/windows/automation_resume.rs` | Native suspend/resume hint adapter only; no timer, policy or durable authority |
| `AutomationTriggerEditor.tsx`, `useAutomationIntents.ts` | Trigger editing and event-driven receipt/status reload; no UI polling |

There are no new renderer commands, watchers, AI queues, provider clients, execution paths or scheduler fairness/governor ownership changes. Owner accepted the core Windows native behavior. Pending-at-exit restart recovery and real suspend/resume remain accepted UNVERIFIED evidence limits; the Windows host never entered suspend. No PM-03 activation followed from this PM-02B closeout.

## PM-03 product hierarchy route boundary — merged

Activation PR #320 authorized the final frontend/product hierarchy migration. Implementation [PR #322](https://github.com/ArdenZC/Zen-Canvas/pull/322) is now **COMPLETE / MERGED** as `master@4848e51dc9cfd1b87b6f4281aac7717452840cd8` / tree `e5a225a0e2c35d2a509c3e540b29862840c023aa`, with merge-after master CI 37561509211 SUCCESS. The accepted implementation started from authorized baseline `master@c72bbce173d53660a68d797b3e0ac2a32cfeb7c5` / tree `8fc2b93b9c64abbc8e0fd892d44fe363a0468f5f`; this section is now the merged production route contract.

The candidate makes `automation` the only canonical `View` identity. A single frontend normalization seam accepts the historical `rules` identifier at URL and cross-window input boundaries and resolves it to `automation`; unknown startup routes retain the existing scanner fallback. The transient `automationSurface` keeps Intents as the ordinary-entry default and Advanced Policies as an explicit in-workspace choice. It is not persisted and does not change Intent or Rule records.

Rule Repository V2 and Rule Proposal remain intact and are rendered only through the explicit Advanced Policies surface. PM-03 changes no Managed AI semantic authority, Automation Trigger/Run/WorkScheduler behavior, Rule execution or persistence semantics, Operation Preview, confirmation, mutation journal, Safe Trash or Restore authority. The [PM-03 result](tasks/AI-ONLY-PM-03-PRODUCT-HIERARCHY-MIGRATION-CLOSEOUT-RESULT.md) and [browser-mock evidence](tasks/evidence/PM-03/measurements.json) record candidate validation; Windows Owner native requalification is PASS on accepted production candidate `843693ea7e3612a662a5331daaedef557d40fee1` / tree `72b1c263650951be118a5f23e820d5dca8e4e621`; native product behavior PASS + deterministic ordering evidence accepted, without a captured native ACK trace. PR #322 is merged; merge-after master CI 37561509211 is SUCCESS.

Owner review of candidate `47ef02922d58329e035eaf8391feaaa4d9523cbf` found that native `SearchView` still serialized the former `Rules` variant as `rules`, so the existing activation/background commands could not deserialize the canonical frontend value `automation`; resident Main restoration also returned to `?view=rules`. The Owner-authorized remediation changes only this transport enum: `Automation` serializes as lowercase `automation`, legacy inbound `rules` deserializes to the same variant, and Main restoration emits `view=automation`. It adds no command, permission, durable authority or runtime behavior. The earlier candidate installer is superseded and is not eligible for Owner qualification; see the [PM-03 result](tasks/AI-ONLY-PM-03-PRODUCT-HIERARCHY-MIGRATION-CLOSEOUT-RESULT.md).


### Accepted standalone Search handoff boundary — PM-03 closeout

**PM-03 COMPLETE / MERGED / OWNER REVIEW PASSED / MERGE-AFTER MASTER CI SUCCESS**. The route-only transport correction above was followed by the separately authorized transient handoff remediation: a Main-only fixed navigation ACK after successful `applySearchNavigation` and `flushSync` mounted commit. Rust waits for the matching positive ACK, revalidates original Search session/revision and only then hides that original Search under scoped CAS. File-less commands ignore unrelated file-selection projection churn; non-null file activations retain strict ID-only continuity. Search write permissions are unchanged. This corrects handoff success semantics without moving PM-02, Rule, semantic or filesystem execution authority.

Ordering is **ACCEPTED DETERMINISTIC ORDERING EVIDENCE**; native route behavior PASS is distinct from the unrecorded native transient ACK trace. Historical failed candidates remain in the result/native record. Independent Global Index #323 remains open/unfixed. Schema 37 / package 0.1.40 unchanged; #273 is eligible for final Owner closure after this reconciliation. No post-PM initiative or release activation follows.


## Windows Global Index recovery-state preservation — #323 review candidate

[Implementation result](tasks/WINDOWS-GLOBAL-INDEX-RECOVERY-323-RESULT.md), branch `fix/issue-323-windows-global-index-recovery`: v3 response error_code carries index_paused/index_rebuild_required/index_permission_required/index_failed. Desktop reconstructs typed Paused/RebuildRequired; text remains diagnostic only. USN discontinuities and MFT integrity failures preserve global_volumes rebuild_required. The existing RecoveryRequired wake and Background WorkScheduler/RuntimeResourceGovernor admission select MFT rebuild on the next eligible cycle; no new loop/store/service/coordinator. Successful rebuild refreshes full-index checkpoint truth.

This is an unmerged Draft implementation candidate, not native Owner acceptance. Schema 37 / package 0.1.40 / wire shape unchanged; SCM and service security ownership unchanged. User-file mutation authority added = 0. PM-03 and completed/closed #273 are not reopened.
