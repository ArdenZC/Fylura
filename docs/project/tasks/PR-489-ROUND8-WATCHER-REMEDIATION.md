# PR #489 round 8 watcher resource fencing

Date: 2026-10-28. Scope: [Owner review 5682715051](https://github.com/ArdenZC/Zen-Canvas/pull/489#pullrequestreview-5682715051).
The PR body records final exact HEAD/tree, CI and the reconciled acceptance result.
PR stays OPEN / Draft; no merge or publication is authorized.

## Starting identity and boundaries

- Remote PR HEAD: `e5e6c36b8f5e4d7860618aa5adc141789b522759`.
- Starting tree: `563eb5d393b132deade15cdea1abd6a698847fb4`.
- Remote master/base: `df940915084b810101a8a3eb78796cdf8730d55b`.
- Original checkout: clean `test/issue-494-filename-search-ab-prototype` at
  `e2bab5018ca3fa93dc7385dc3d02f96e7afc51aa`; left on that branch.
- Isolated worktree: `F:/Coding/Zen-Canvas/pr489-round8`, branch `fix/pr489-round8`.
  Delivery updates only the existing PR branch, without force push.
- No real C/D/E/F/G volume was indexed. Native watcher tests open only unique
  task-owned fixture directories. MFT/USN work in those tests is injected through
  actual DatabaseIndexSink admission; it never enumerates a host disk.

## Root cause and synchronization proof

The old route installed a recursive watcher from a selected source snapshot
before Service/Direct database admission. A disable could commit and clear the
watcher map, then an old request could recreate a watcher and fail admission.
Discovery UPSERT could revoke consent without destroying the registered handle.

The final route creates no watcher until Service/Direct work succeeds. Under the
existing watcher map mutex, DatabaseIndexSink then proves the current operation
owner ID/revision/running state and the current enabled/authorized/native NTFS
fixed-volume identity and mount. Only then can a watcher be reused or installed.
The database query is not independently sufficient: Coordinator enable/disable,
authorization and discovery mutations use `update_source_lifecycle`, taking the
same resource mutex before changing authority and disposing invalid resources.
The native qualification wrapper forwards this boundary to its Windows provider.

Lock order:

1. Watcher map/resource lifecycle mutex.
2. Existing source watcher callback publication mutex, when changing authority.
3. Short pooled SQLite read/write, releasing connection/transaction on return.
4. Release publication mutex before native watcher destruction/join.

Registration takes (1), performs its short database owner check and installs the
native resource without a database write lock. MFT/USN, scheduler admission,
IPC and process ownership checks execute outside these locks. Notify callbacks
take only (2), then the bounded wake-slot mutex, and never call SQLite or acquire
the watcher map. There is no inverse lock edge.

If disable wins (1), it commits and destroys the resource; a later old request
cannot pass the current operation fence. If registration wins (1), disable waits
for registration, then commits and destroys that handle before returning.
Publication freezes before the authority mutation, so a callback cannot publish
between the revocation commit and handle destruction. Drop invalidates the same
publication gate before destroying the native watcher, covering queued callbacks.
An error from a lifecycle mutation also discards the resource, including an error
after a committed write. A failed owner-validation read discards the resource.
Each watcher also records the request ID of its most recent successful resource
admission. Reuse transfers that lease to the new admitted operation. Old failure,
cancellation or owner-loss cleanup removes only its unchanged predecessor lease;
it cannot delete a newer request's watcher. Rejected admission creates nothing
and leaves the active owner's healthy resource intact.

Pause/shutdown clear the map and increment one resource cancellation generation.
A request captures that generation before native work and checks it under (1)
afterward, together with the existing cancellation flag. This also covers Pause
invoked directly from a native service admission callback without Coordinator
cancellation. The generation is not an indexing state machine or authority.

Ordinary unchanged discovery preserves watchers. Capability/mount changes use the
existing durable operation revision to invalidate them. A missing source drops
its watcher and invalidates an active owner; a new identity at the same mount
also disables and revokes the old identity, retaining all historical data.
Returning capability/identity cannot silently grant consent. An ordinary unplug
without replacement retains consent, while dropping native resources.

## Failure-first evidence

Test-only commit: `bc1feecef99e6fd6915962b25272bd3503d2d474`.
`disable_completed_before_stale_request_cannot_leave_native_watcher` uses two
Barriers: the old task reads the source, disable commits and clears resources,
then the task continues. It installs a real `notify` recursive directory watcher
on the old implementation; database admission rejects the native indexing closure.

```text
stale request leaked a real recursive watcher after disable returned
test result: FAILED. 0 passed; 1 failed; 0 ignored
```

The same assertion passes after the repair. Neither a sleep nor a fake resource
is used to produce this failure. Raw logs remain in the task evidence directory.

## Resource lifecycle acceptance matrix

| Boundary | Evidence |
| --- | --- |
| Selected snapshot -> disable completes -> old request continues | Failure-first Barrier test; real watcher leak reproduced, then absent; native work closure cannot run |
| Admitted work -> disable -> registration | Barrier test with real sink/checkpoint and real preexisting watcher; late registration rejected |
| NTFS -> unsupported; USN disappears; GUID distrust | Production classifier + Coordinator rediscovery with real handles; watcher removed, source disabled/unapproved |
| Native capability returns | No watcher/no index work until explicit new confirmation; then real watcher and Ready |
| Different identity reuses mount; original identity returns | Old handle removed; history retained; both new and returning identity require confirmation |
| Callback queued after revoke commit, before disposal | Controlled callback uses the real watcher's publication gate; no notification after revocation |
| Native callback after Drop | Real filesystem event verified before Drop; retained callback cannot publish afterward; directory removable |
| Initial -> Incremental -> Rebuild -> Incremental | Real resource fixture plus previous DatabaseIndexSink lifecycle regressions; timestamps/checkpoint and rebuild completion preserved |
| Pause/Resume; provider-only Pause | Cancellation/generation fences; resources removed and later healthy cycles register again |
| Disable/Enable; multiple sources | One source's handle is destroyed, another remains healthy and continues indexing; re-enable registers again |
| Owner loss; failed/unknown service result | Post-work Barrier tests; no late resource; existing owner/Service/Direct protocol regressions retained |
| Rejected admission; obsolete error or owner loss after a successor completes | Barrier tests retain the successor's real watcher and registration request ID |
| Watcher startup failure -> bounded USN recovery -> Ready | Existing Coordinator/worker route tests remain active; no fatal watcher failure and production polling stays 300 seconds |

New resource tests use real directory watchers and injected authoritative work.
Existing protocol tests and hosted disposable-VHD qualification cover Service and
Direct separately. These claims do not imply installed GUI/native failure E2E.

## Validation and limitations

Local task-scoped TEMP/TMP and Cargo outputs are on F:, after a capacity check.
No thresholds, schema, migrations, frontend product code, AI state, history,
release outputs or user files are changed. #371 remains NOT IMPLEMENTED.

Initial local Global Index run: 201 passed / 0 failed / 13 existing ignored;
later resource-lifecycle run: 202 passed / 0 failed / 13 existing ignored.
The callback boundary test also passes independently; the later candidate
Global Index run is 203 passed / 0 failed / 13 existing ignored. Final exact-candidate
results and counts are reconciled in the PR body.

Local broad Rust attempt retained four failures: three Automation readiness
assertions observed Ready/completed rather than unavailable/blocked, and one
scheduler queue-deadline assertion failed. Each Automation assertion also fails
on unchanged same-host `master@df940915`, with the same Ready/completed result;
those fixtures expect an unavailable provider on this host. No credential is
deleted or changed to satisfy a test. The scheduler single-test comparison passes
on master; its original full-suite failure is retained without a baseline-cause
claim. The PR body records candidate revalidation and hosted final outcomes.

Frontend verification initially exposed CRLF hashbang parsing and missing frozen
research Git objects in this shallow local checkout. The required historical
object was fetched; research checkout bytes were reconstructed from exact HEAD
blobs for the frozen-byte checks. No research source/test is changed in the PR.
Independent npm installation and original failure logs are preserved.

Still UNVERIFIED: Owner Windows 11 installed GUI; installed v3->v4 upgrade;
native Win32 5/1314 Service->IPC->GUI; native watcher failure/recovery GUI E2E.
Hosted Windows service/VHD tests are distinct from those acceptance claims.
Historical SQLite ReadOnly, W2-11 blob/resource failures and macOS read-deadline
evidence remain unresolved with original reports retained; later passes do not
establish historical root causes. Review/merge remain Owner decisions.
