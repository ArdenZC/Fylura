# Issue #328 Windows Global Index Diagnostics

## Scope and disposition

This is a bounded test and diagnostics follow-up for Issue #328, based on
`master@df37531f785193a948409e2f8cb36369561c6a6f` (tree
`96230e46cdc1529d95314506d6971a8b40f6e1a3`). It does not change index
scheduling, retry policy, timeouts, IPC v3, schema, security boundaries, or
user data. The Service admission check was extracted into a pure response
helper so the existing busy branch can be exercised without a named pipe; the
response and admission behavior are unchanged.

Owner's 2026-10-11 static triage remains the research authority. It distinguishes
the historical `sqlite error: database is locked` from the newer
`index_service_busy` response. The native lock holder remains unknown. The
reported C: progress and D:/E:/F: errors with zero rows are separate source
observations whose native typed errors and count meaning were not available.

## Reproduction findings

| Path | Test evidence | Finding |
| --- | --- | --- |
| Windows Service admission | `same_source_start_resume_and_rebuild_contenders_get_busy_without_service_fault_state` | With a first request holding the service slot, same-source Start, Resume and Rebuild contenders preserve each request ID and return `index_service_busy` with Service status `indexing`. The Service lifecycle error remains unset. |
| Service error decoding | `service_busy_decodes_as_service_provider_contention` | The busy response decodes to the generic `GlobalIndexError::Provider` category. |
| Coordinator persistence | `multivolume_busy_and_source_failures_keep_independent_durable_statuses` | A provider-level busy result received while the source is in `syncing` is persisted as source status `error` with the busy message. This is the concrete false source-fault candidate. |
| Multi-volume active state | `multivolume_busy_and_source_failures_keep_independent_durable_statuses` | While source A is held in `syncing`, source B retains its previous busy `error` and source C retains `permission_required`. The aggregate remains `error` while exposing `pending_volumes=1`, `total_entries=0`, and `collection_complete=false`. This synthetic snapshot shows the active work and durable peer failures separately; zero entries alone does not identify a cause. |
| Multi-volume completed state | Same multi-volume test | After the active operation is released, A is `ready`, B becomes `error` from its busy result, and C remains `permission_required`. The aggregate reports `partial`, one of three volumes ready, and `collection_complete=false`; source failures do not stop later volumes in the cycle. |
| Retry lifecycle | `busy_source_waits_for_a_new_wake_before_an_explicit_retry` | The worker makes one busy attempt and blocks without a self-generated retry. A later explicit wake causes a second attempt, which reaches `ready`. The test does not add a retry mechanism. |
| Duplicate lifecycle requests | `repeated_same_source_lifecycle_requests_are_serialized_and_rebuild_wakes_coalesce` | While one Start is blocked, repeated Start/Resume signals and two Rebuild calls produce one serialized follow-up Rebuild for that source. The maximum concurrent provider operations is one. This does not establish native request frequency. |
| SQLite classification | Existing `global_index_state_write_reproduces_busy_and_recovers_after_holder_release` | A real SQLite writer conflict retains primary/extended code 5 and is wrapped as `GlobalIndexError::Database(DbError::Sqlite(_))`; this remains distinct from the Service's provider-level busy result. |

## Bounded, sanitized evidence

Test evidence is capped at 8 events. It records the active multi-volume
snapshot, per-source results, and the later explicit retry. Each event carries
`source_id`, a distinct test-fixture `operation_id`, `request_id`, optional
Service status, request result, error category, and persisted source status.
IDs are capped at 128 bytes. Evidence uses fixture IDs and omits mount paths,
file names, file contents, Service error text, and SQLite database paths.

The current IPC v3 request has a `request_id` but no production `operation_id`.
The diagnostic `operation_id` is therefore explicitly a test-fixture scenario
ID; production operation-ID correlation remains **UNVERIFIED**. No IPC field was
added.

## Fix candidate (not implemented)

The narrow candidate is to preserve the Service busy category through decoding
and handle it separately from source/provider failures in the Coordinator. A
busy request should remain a request-level result and should not overwrite the
last committed source health with `error`. The smallest likely code boundary is
`windows/service_errors.rs` plus the per-source result match in
`coordinator.rs`. Owner approval is required before implementing this product
behavior. No retry or scheduling change is proposed by this test branch.

## Validation and limits

- `git diff --check`: **PASS**.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` with Rust 1.97.1:
  **PASS**.
- Local Coordinator Rust tests: **BLOCKED**; compilation requires the missing
  system development package `glib-2.0 >= 2.70`.
- Hosted source validation on `e501630179cf59662ae806db052992ed4dd5c43b` in
  run `38116710256`: both Windows Rust quality lanes and the macOS
  merge-integration Rust quality lane **PASS**, including tests and Clippy.
  Windows Release compiles, macOS head Release compile, and both performance
  search lanes **PASS**. The disposable Windows Global Index Service
  qualification **PASS**. The macOS head-validation Rust test step was
  cancelled when a later documentation-only commit changed the PR head; it
  had not reported a failure. No source code changed after `e501630`.
- Previous run `38116172965` on `01dd163a2ada651101d40adbd28395f99c9e5185`:
  Windows/macOS Rust format and tests **PASS**. macOS Clippy identified two
  test-only issues; both were fixed in `e501630`, and that earlier run was
  cancelled before its remaining lanes completed.
- Windows native Service / installed application reproduction: **UNVERIFIED**.
- Historical SQLite lock owner and exact historical native SQLite result code:
  **UNVERIFIED**.
- Whether C: processing and D:/E:/F: errors share any cause: **UNVERIFIED**.

No Codex Review was run. No production defect is claimed as fixed by this PR.
