# #380-P1 IPC v3 / Service Admission candidate

Production is **ISSUE_380_PRODUCT_FIX_NO_GO**. This candidate repairs Windows IPC admission/identity/completion and typed errors; it does not repair Coordinator durable source ownership.

Owner scope: [6145267380](https://github.com/ArdenZC/Zen-Canvas/issues/380#issuecomment-6145267380), [signed independent review](https://github.com/ArdenZC/Zen-Canvas/pull/398#issuecomment-6145257628). Existing [Draft PR #398](https://github.com/ArdenZC/Zen-Canvas/pull/398), same branch. No merge/cherry-pick of #386, no other candidate baseline.

## Identity and reproducible evidence

- Starting/current checked master: `3275ac7c72df8ae4139a30f16c7f649971324e97`; Tree `009576241cd4e8e2ba12cc644a368314f599faa6`.
- Accepted diagnostic HEAD: `5144c44b34dbe0039c59feebf52399510c0c0629`; Tree `4b3cee18a28499669737b7e017df1fabe6c2e6ec`.
- Locally tested production source commit: `46519713a2bed941415f39d3f42ecb839b0874a3`; Tree `be42569eebecdee2f05390bab98ec3ffbe956d92`. The delivery commit adds report/raw evidence only. Final delivery HEAD/Tree, complete Diff and exact-HEAD CI results are recorded in PR #398's description and handoff; CI results must not be predicted from this local run.
- [Historical scope report](HISTORICAL-SCOPE-BLOCKER.md), [old characterization](evidence/characterization.json), [three real failing required-contract probes](evidence/required-contract.json), [old source/command provenance](evidence/probe.json) are retained. They establish erased error semantics, not genuine UnknownOutcome or repaired-runtime acceptance. Original runner remains historical: its expected exit 101 contract no longer describes repaired source.
- [Green-after actual-source raw output](evidence/repaired-client.json): 30 passed, 0 failed on Linux, Rust 1.97.1, rusqlite 0.39 / SQLite 3.51.3. Full output and SHA-256 source/harness hashes included, with exact execution commit. Source files are imported whole for client/decoder; model/error/trait and ServiceRuntime/PipeSink core are extracted verbatim. An **unused default Direct constructor stub** permits Linux compilation; tests immediately install controlled providers. This is not Windows native API qualification.
- Reproduce this limited Linux experiment: `python3 docs/research/issue-380-p1/run_repaired_contract.py --fixture .tmp-tests/p1/repaired-fixture --output .tmp-tests/p1/repaired-results`. The full desktop local build remains environment-blocked by missing glib >=2.70 ([original raw failure](evidence/full-project-local.json)).
- Local `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` and `git diff --check` passed. Reduced-fixture Clippy passed with `-D warnings -A dead_code`; dead-code suppression is confined to the fixture command because Windows constructors/SCM APIs are excluded. Actual Windows/macOS CI must run full production Clippy without suppression.
- First production push `bfa31184be92b69fc6c74ae06ed6dad27d9e7aef`: [CI 38399155013](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38399155013) ([preserved raw Windows job log](evidence/first-production-ci.json)) had genuine Windows compilation failures (private helper exports; connection trait missing Send). Both corrected; no CI rerun/waiver. Final normal PR CI is separate and required; cancelled/skipped/failed jobs remain visible in the original run.

## Changed behavior and error mapping

Only `coordinator.rs` lines added inside `GlobalIndexError` define four variants. Each pair of lines is a derive display annotation and its public variant: ServiceBusy, PermissionRequired, ServiceProtocol, ServiceOutcomeUnknown. No Coordinator method, scheduler, match algorithm, SQL/repository or database ownership code changed. No non-Windows production caller required a new match arm.

| Trusted v3 error_code / local condition | Public error | Meaning |
| --- | --- | --- |
| `index_service_busy` | ServiceBusy | Service slot rejected request; no admitted Sink event |
| `index_permission_required` | PermissionRequired | Explicit remote permission diagnosis |
| `index_provider_error` | Provider | Ordinary provider fault; not permission |
| `index_failed` | Database(DbError::Validation) | Remote database-class failure; does **not** invent a remote SQLite numeric code |
| `index_windows_io:<canonical u32>` | WindowsIo(original code) | Original OS identity; 5 ACCESS_DENIED and 1314 PRIVILEGE_NOT_HELD have permission status, other codes ERROR |
| `index_paused` / `index_rebuild_required` | Paused / RebuildRequired | Existing recovery semantics retained |
| Missing/unknown/noncanonical code, malformed JSON/frame, version or request mismatch | ServiceProtocol | Untrusted remote/protocol input, never fabricated remote UnknownOutcome |
| Actual write attempt or read/lookup-reply transport failure after dispatch | ServiceOutcomeUnknown | Completion may be ambiguous; no Direct replay |
| Local Sink error | Original GlobalIndexError, including Database(Sqlite BUSY 5) | Preserved without serialization/text reclassification |
| Valid request, pipe connect fails before dispatch | Private NotSent | Only this branch can consider Direct; source's prior uncertainty fence must also be clear |

`classify` never encodes ordinary Provider/arbitrary WindowsIo as permission. Message text does not decide semantics. No fabricated `index_outcome_unknown` wire code. Existing v3 six response fields and event variants retained; `EntryStale` still has only entry_id, `journal_cursor` remains Option<String>. Unknown fields are rejected on requests and inbound responses/events/lookup frames. No permission/transport authentication weakening.

Direct producer completion error-state mapping uses the typed classifier rather than marking every fault PermissionRequired. Existing recovery test is deliberately updated to expect ERROR for ordinary Provider; original historical evidence remains. Paused and actual journal discontinuity/rebuild handling remain typed.

## Admission, ownership checks and completion

Actual runtime: reserve busy slot → operation mutex → native-discovered source revalidation + durable snapshot merge → Sources marker with the selected source → Direct provider/PipeSink events → response. Reservation failure sends correlated busy response and emits no marker/event. Failure to send marker prevents provider invocation; operation exits and releases slot.

Actual named-pipe client uses the same `exchange` function tested with deterministic channels. It validates request v3/source ID before connect; every event request ID before callback; every response version, active request ID and ok/error_code consistency before decoding. Send/framing errors are structured: serialization/size/JSON errors are Protocol; actual Win32 write/read failure is transport UnknownOutcome. A callback Sink failure retains its original typed error. Server-side same-executable/security/SCM restrictions are unchanged.

Admission marker is mandatory and unique. It must bind source ID, stable physical volume identity, mount/filesystem (Windows case-insensitive), platform, native MFT/USN provider, enabled and journal snapshot. Native display/count/timestamps may differ from the durable desktop projection and are not mistaken for authority. Sources acceptance itself performs **no Sink writes**.

Before each callback effect, validate source ownership and order. Every Entries element must belong to the bound source/provider. All volume-bearing variants must match. Legacy EntryStale is allowed only for an entry ID returned by this request's validated FindEntryByIdentity lookup; its returned volume/file/parent/name identity must match the query. Duplicate lookup IDs and old-request frame replay are rejected before a reply. No new EntryStale field or Sink trait/database method.

Full/Initial/Rebuild success requires admitted source + trusted SourceProvider + full-volume stale boundary + actual canonical numeric-string journal checkpoint. Incremental success requires admitted source + trusted SourceProvider + checkpoint, with unchanged saved journal identity and no cursor rewind. No fabricated Entries is required for an empty stream. Terminal OK alone never qualifies. Native MFT's existing empty-baseline integrity rejection remains unchanged: controlled empty-provider tests do not claim an actual empty MFT baseline is accepted. Source faults must be terminal-compatible; post-completion entries/lookup/checkpoints are rejected. Existing incremental directory-rename stale/rebuild flow remains an error/reconciliation flow, not success.

No Direct for remote busy/rejection, bad protocol, partial stream, failed completion evidence or local Sink failure. A lifetime source fence also blocks a later NotSent request after previous uncertainty. It is deliberately fail closed and has no automatic retry/reset. It is **not** durable intent, global arbitration or process-restart isolation.

## P1 contracts and actual execution scope

| Contract | Production regression evidence |
| --- | --- |
| 01 Busy before writes | p1_t01 plus barrier-controlled actual Runtime competitors |
| 02 Version/request identity | p1_t02 validates before callback |
| 03 Foreign entries/stale/state | p1_t03 plus owned/foreign lookup and nonempty Entries test |
| 04 Duplicate/missing/order | p1_t04; no completion without admission |
| 05 Permission/Rebuild/Provider fault | p1_t05 plus actual Runtime encoded-error test |
| 06 Initial/Rebuild completion | p1_t06 plus actual Runtime/PipeSink/client roundtrip |
| 07 Empty full vs incremental | p1_t07, no fake Entries, existing native MFT integrity guard preserved |
| 08 Partial/disconnect | p1_t08, actual exchange returns transport UnknownOutcome |
| 09 Local SQLite vs ServiceBusy | p1_t09 actual file lock BUSY(5) through Sink callback; original two-connection test |
| 10 Lookup ownership/order | p1_t10, source/result ownership test, actual Runtime PipeSink lookup roundtrip |
| 11 No unsafe Direct replay | Windows provider p1_direct_fallback_only_for_proven_not_sent_and_never_after_uncertain_history |
| 12 Legitimate Direct | Same provider test proves fresh NotSent invokes Direct once; partial-send p1_t12 refuses NotSent |
| 13 Before any DB effect | p1_t13 and callback/write-count assertions; Sources writes nothing |
| 14 Normal v3 / recovery | p1_t14, retained wire-shape/recovery tests, Runtime Initial/Resume/Rebuild tests |

30 Linux checks passed. Complete Windows CI additionally compiles/runs the actual Windows provider fallback test and **original complete ServiceRuntime**, without the Linux constructor stub. All P1-controlled producers/transport fault injectors are test doubles; Native Service/API acceptance depends on the separate normal Windows Service qualification job and its raw evidence, not ordinary CI success alone. Tests use channels/barriers and checked counters; recv_timeout only prevents hangs, no sleep-based correctness argument.

## Mandatory P2/P3 dependencies and NO-GO

- P2: public Coordinator Rebuild still writes REBUILD_REQUIRED, clears cause and marks volume stale **before** Service Admission. Initial/resume still prewrite INDEXING/SYNCING. Generic error handling can still persist ERROR for typed busy. No whole-source preservation claim.
- P2: Source Revision/CAS, request/operation-owned conditional writes and persistent intent are absent. Admission validates IPC eligibility, not DB ownership. Already authorized effects cannot be rolled back when a later stream fails.
- P2: lifetime Direct fence does not survive desktop restart; it does not isolate another desktop/process/Direct writer or prove Service quiescence after transport loss. No unified arbitration added.
- P3: Global/Managed/AI publication remains multiple non-atomic repository operations; no production publication or schema change here.
- #328 remains true SQLite contention. This patch preserves local SQLITE_BUSY(5) identity and adds no busy_timeout/retry/workers/queue; it does not cure database locking.
- Windows Owner native adversarial acceptance (service restart/late requests/real process loss/IPC fault timing against durable state) remains NOT_RUN on Owner hardware. Hosted normal Service qualification cannot certify these P2/P3 properties.

Only report P1 candidate READY after final exact-HEAD native-capable Rust tests/format/Clippy and applicable normal CI pass. Product remains NO_GO. Keep #398 OPEN/Draft for Owner review; no master/Schema/CI/installer/release modification or Codex Review.
