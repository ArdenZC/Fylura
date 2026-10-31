# macOS mutation threat model

## Scope

This contract covers macOS 13 or later on Apple Silicon
(`aarch64-apple-darwin`). Intel Macs, Rosetta, Linux, signing, notarization,
stapling, certificates, and Endpoint Security hardened mode are outside this
mutation implementation.

The macOS filesystem adapter is enabled through the existing Operation Preview,
operation journal, Safe Trash, restore, and recovery authorities. It does not
create a second journal, queue, trash namespace authority, or recovery store.

## Safety levels

macOS does not provide the Windows source-handle rename primitive used by the
existing Windows path. Fylura therefore distinguishes the guarantees:

- Level A, descriptor-bound copy/clone: implemented for regular files and
  directory staging; descriptor-bound destructive rename/delete is not claimed
  because Darwin has no portable source-FD rename primitive.
- Level B, recoverable namespace transaction: implemented for the supported
  local APFS path and the existing Safe Trash, restore, replacement,
  package-root, symlink, hardlink and permanent-delete authorities. Portable,
  external and network paths use target-first copy/verification; source
  retirement is committed only after an implementation-backed exclusive claim
  probe. If that proof is unavailable, source and verified target remain
  recorded as `source_cleanup_pending` rather than entering a check-then-
  `unlinkat` fallback.
- Level C, coordinated provider transaction: iCloud uses native accessors.
  Under ADR-0003 Decision B, generic File Provider coordination uses
  `NSFileCoordinator` with the provider/user-visible URL and physical identity;
  the public `NSFileProviderManager` item/domain/download APIs are not generic
  authority for another extension. A bounded explicit content-access proof is
  not treated as full materialization: byte operations reopen, consume and
  revalidate their own source. URL, physical-identity, coordination or
  materialization failures remain runtime errors rather than being inferred
  from a path. Non-local content is never downloaded implicitly.

Level B never silently upgrades itself to Level A. The source is claimed under a
private exclusive name, verified again, and only then published to an exclusive
destination. A race or rollback conflict retains the object and the durable
journal enters recovery; it never falls back to an unverified overwrite or
unlink of the original user pathname.

## Unified identity

`MacPhysicalIdentity` is the platform physical identity used by claims, copy,
move, Safe Trash, restore, delete, and post-commit checks. It captures device,
inode, object type, link count, size, timestamps, and an optional generation
field from `fstat`, `lstat`, or `fstatat(..., AT_SYMLINK_NOFOLLOW)`.

Physical identity and content identity remain separate facts. Content hashes are
used for copy and recovery verification; physical identity is used to prove that
the namespace entry still refers to the claimed object. A symlink is mutated as
the symlink object and is never followed. A hardlink entry is a namespace
operation; permanently deleting one entry only decrements its link count. A
package root is one logical object and is never partially mutated.

## Transaction boundary

The backend sequence is:

1. Validate absolute paths, protected locations, volume writability, package
   boundaries, object kind, and provider state.
2. Open source and verified parent directories with `O_NOFOLLOW | O_CLOEXEC`.
3. Capture `MacPhysicalIdentity` and only the content identity requested by the
   operation policy.
4. Move the source into a private claim using exclusive `renameatx_np` namespace
   publication, then verify both physical and optional content identity. The
   unsupported Darwin `linkat` plus pathname `unlinkat` rename substitute is
   never used; portable source-retirement failure leaves source and the
   journaled retirement slot for recovery.
5. For copy or cross-volume work, select `PhysicalClone`, `StreamingHash` or
   `FullPostVerify`. Stage with `fclonefileat` when available; the regular-file
   fallback reads once while writing and computing BLAKE3. Preserve metadata
   and symlinks, report metadata degradation explicitly, verify the destination
   contract, and publish with exclusive rename.
6. For replacement, claim the old target into a deterministic private backup,
   publish the new source, and retain the old target for restore.
7. For Safe Trash, move into the existing durable Safe Trash ledger. For
   permanent delete, quarantine first, recheck identity, and delete only the
   quarantined object. Physical SSD erasure is not claimed.
8. Persist the existing journal phase and revalidate after restart.

The macOS name-based primitive is a known Level B boundary: Darwin has no
portable source-FD rename equivalent. The high-entropy private claim,
exclusive destination, immediate identity checks, and manual recovery state
bound the residual namespace race without presenting it as a kernel guarantee.

## Strategy matrix

| Observed source/target | Backend strategy | User outcome |
| --- | --- | --- |
| Local writable APFS | `local_apfs` | Same-volume namespace transaction |
| Local writable non-APFS | `local_portable` | Target-first copy/verify; source retirement may remain pending until an exclusive claim is proven |
| Writable external/removable volume | `local_portable` or `cross_volume_copy_verify` | Probe-backed source retirement; metadata/volume limitations preserve source and target for recovery |
| Different devices/volumes | `cross_volume_copy_verify` | Copy, verify, then retire source |
| Network volume | `network_portable` | Target-first route is represented; identity/rename/no-replace/durable source-retirement and disconnect/reconnect evidence are not claimed without the real fixture |
| iCloud item | `icloud_coordinated` | Coordinate metadata operations; copy/duplicate requires explicit materialization |
| Known File Provider domain | `file_provider_coordinated` | CloudStorage path is a hint; Decision B uses `NSFileCoordinator` plus the user-visible URL and physical identity, while byte operations require explicit coordinated content access plus operation-time consumption |
| Read-only, offline, unknown, or ambiguous provider | runtime refusal | Stable error; object and journal remain recoverable |

Known File Provider domains are observed conservatively from the macOS
`~/Library/CloudStorage` namespace. Under ADR-0003 Decision B, Zen does not
call `getIdentifierForUserVisibleFileAtURL:`, `managerForDomain:` or
`requestDownloadForItemWithIdentifier:` as generic authority: Apple documents
those item/domain operations around the caller’s File Provider extension and
domain. Generic metadata operations use `NSFileCoordinator` with the
coordinator-supplied user-visible URL and a physical identity captured and
revalidated at the operation boundary. Native item/domain IDs, NSURL resource
identifiers and path hints are not substitutes for that evidence. The recent
explicit-content proof cache is bounded, expires after five minutes and is
cleared on provider lifecycle invalidation; it records only a bounded
`BoundaryReadable` observation. Real provider, external-volume and
network-volume fixture results remain separate from the platform capability
advertisement and are **NOT VERIFIED — fixture unavailable** when the
corresponding fixture is absent.

## Race and recovery guarantees

The operation journal records `source_claimed`, `copying`,
`target_committed`, `source_cleanup_pending`, and `completed`. On restart the
existing reconciliation code distinguishes pre-commit rollback, completed
publication, source-cleanup pending, replacement restore, and ambiguous states.
Ambiguous identity, source reappearance, target replacement, unreadable
provider content, or failed rollback becomes manual review with the claim or
backup retained.

Staging cleanup is also identity-bound: a cleanup path may delete only an
object whose physical identity was captured before the failure. If that proof
is unavailable or the staging name has been rebound, the staging object is
retained rather than deleting by a newly observed pathname.

The permanent-delete invariant is especially strict: after quarantine, the
original source pathname is irrelevant. Delete is attempted only against the
revalidated private quarantine object. If that object cannot be revalidated or
removed, it is retained for manual recovery.

## Capability and UI contract

Runtime capabilities are fine-grained: copy, duplicate, rename, same-volume
move, cross-volume move, replace, Safe Trash, restore, permanent delete,
secure removal, package, iCloud, File Provider, external-volume, and
network-volume mutation. Each provider/external/network capability reports
three separate layers: `PlatformFeatureAvailability`,
`RuntimeEnvironmentCapability`, and `OperationEligibility`. A platform API
existing is not an execution claim; current volume/provider eligibility is
resolved during Preview and again at confirmation. File Provider,
removable/external-volume, and network-volume operations can return stable
runtime refusals when their mounted filesystem or provider cannot satisfy the
operation contract.

File Library exposes the existing Operation Preview route for file operations,
including an explicit permanent-delete review. Preview displays the backend
strategy, materialization requirement, cross-volume copy requirement, metadata
degradation possibility, source-retirement capability, provider coordination
and conflict policy. For a required download, the user explicitly confirms
`Download and continue`; the backend owns bounded progress/cancellation and
revalidates the preview, source namespace identity and coordinated provider URL
evidence before
retry. Normal History/Restore remains the recovery UI; internal claim paths and
error details stay behind technical disclosures.

## Verification status

Windows checks in this task preserve the existing handle-bound implementation.
The Windows host can run shared Rust/frontend checks and contract tests, but it
cannot produce Apple Silicon evidence. Native Apple Silicon compile, provider
fixtures, real external/network volumes, sleep/wake, mount/unmount, and the
configured 10k/100k race stress are exact-head remote evidence gates. The
V2.1 initiative is not release-complete until the macOS workflow reports the
exact pushed production head green.

## Explicitly deferred

- Endpoint Security/System Extension hardened mode;
- signing, notarization, stapling, certificates, and signed DMG delivery;
- advanced `QLPreviewPanel` integration;
- physical SSD secure erase guarantees.
