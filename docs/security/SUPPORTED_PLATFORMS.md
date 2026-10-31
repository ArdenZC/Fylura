# Supported platforms

## Product targets

Fylura supports:

- Windows, with the existing source-handle and verified-directory mutation
  authority. Same-volume directory Move uses the existing no-replace atomic
  source claim and operation journal. Cross-volume directory Move stages on
  the target volume, verifies content and owner/group/DACL security
  descriptors, publishes without replacement, then retires the claimed source
  only after revalidation;
- macOS 13 or later on Apple Silicon (`aarch64-apple-darwin`), with the
  recoverable namespace and provider-coordination strategies described in
  `MACOS_MUTATION_THREAT_MODEL.md`.

Intel Macs, Universal binaries, Rosetta, and Linux are not product targets.

## macOS operation support

macOS runtime capability is intentionally fine-grained. Copy, duplicate,
rename, same-volume move, replacement, Safe Trash, restore, package-root
mutation, iCloud coordination and permanent-delete strategies exist in the
backend. Cross-volume, external-volume, network-volume and generic File
Provider operations remain runtime/fixture dependent; they are not a blanket
platform guarantee. Individual operations fail closed with a stable
capability, materialization, identity or coordination error. Secure physical
SSD erasure is not available.

The backend chooses the strategy; the renderer does not infer it from a path.
Operation Preview shows the chosen strategy and conflict policy, and the
backend resolves it again after confirmation. Read-only volumes, offline or
ambiguous providers, permission failures, identity races, and target collisions
are runtime refusals, not platform-wide feature deferrals.

Symlinks are operated on as link objects and are never followed. A hardlink
directory entry can be moved or renamed; permanent deletion removes only that
entry and does not claim to erase all links. Package roots move as whole logical
objects. Package-internal mutation is rejected.

## Recovery and cleanup

macOS Safe Trash is Fylura's durable recoverable namespace, recorded by the
existing cleanup ledger and restored through History. Replacement retains the
old target in a private backup and restores both source and destination only
after identity verification. Permanent delete first uses a private quarantine;
failed verification or deletion retains that object for manual review. No
operation bypasses Operation Preview, journals, Safe Trash, or restore
revalidation.

## Cloud and provider behavior

iCloud content is not implicitly downloaded during indexing, Preview, or
mutation. Preview marks non-local content as an explicit materialization
precondition; the current backend does not silently start that download. A
user-confirmed download reports progress, supports cancellation, then
revalidates the original preview before retry. Generic File Provider paths use
`~/Library/CloudStorage` only as a routing hint. Under ADR-0003 Decision B,
ordinary metadata operations use `NSFileCoordinator` with the provider's
user-visible URL and filesystem physical identity; the public item/domain
translation and provider-manager download APIs are not treated as authority
for arbitrary third-party extensions. Byte operations require explicit
user-consented coordinated content access and a bounded open/read proof;
`BoundaryReadable` is not full materialization, so the byte operation still
reopens and consumes the source once. Provider-internal IDs, path text and
POSIX metadata are not interchangeable. Real iCloud, generic File Provider,
external APFS, exFAT and network-volume fixtures are **NOT VERIFIED — fixture unavailable**
when absent; the CI fixture tests print
`NOT VERIFIED — REAL FIXTURE NOT PROVIDED` and do not convert a skip into a
pass claim.

Portable source retirement uses an exclusive APFS claim or the Zen-owned
mode-0700 `.zen-canvas-retirement/<random-session>/` namespace after a
runtime capability probe. The Darwin `linkat` plus pathname `unlinkat` fallback
is not used because a rebinding race could delete an unexpected object. A
verified target-first copy may therefore preserve both source and target as
`mac_source_retirement_pending`; the existing journal records the unique
retirement slot and recovery retries only after revalidating the source
identity. Unknown/read-only volumes and network mounts without
disconnect/reconnect durability evidence return
`mac_filesystem_capability_insufficient` as a retirement-capability result;
when a target-first copy has already been verified, the source remains
preserved and the journal records `mac_source_retirement_pending` instead of
attempting source cleanup.

## Windows and Linux

Windows source-handle and verified-directory primitives remain the filesystem
authority. Cross-volume directory Move is
bounded to 5,000 entries and depth 64; reparse points, named streams,
unsupported attributes, security-descriptor access failures, and source or
target identity changes fail closed. Owner, group, and DACL changes are
included in the cross-volume verification digest. The legacy system Recycle
Bin path continues to fail closed where source binding cannot be proven; Safe
Trash is the durable cleanup route. This Draft still requires Windows native
GUI acceptance and active-transfer resource measurements before release
qualification. Windows audit SACLs are not copied by this path, so SACL-bearing
trees remain outside its qualified semantics until the backend can preserve or
explicitly reject them.

During cross-volume staging, each target entry is protected from destination
parent DACL inheritance while retaining the source's current owner, group, and
DACL ACEs. This keeps a different destination-parent ACL from being merged
silently into the moved tree. DACL control flags and the per-ACE inherited
marker are excluded from the content comparison because staging materializes
inherited ACEs as explicit ACEs. ACE type, rights, SIDs, conditions, and all
other ACE flags remain verified at staging, publication, and source retirement.
Future changes to the source parent's inherited ACL will not propagate to the
staged target.

Linux is not a supported product platform. It is outside product support,
build, release, and mutation-safety scope.
The fact that shared Rust code can be parsed on Linux is not a support or
security guarantee.

## Verification boundary

The supported-platform gate is Windows Quality, macOS Apple Silicon Quality,
and Dependency Audit. A macOS feature is not reported as release-verified from
Windows cross-compilation: native Apple Silicon compile/test, provider and
external-volume fixtures, race stress, and packaging evidence must come from
the remote macOS workflow at the exact pushed commit.
