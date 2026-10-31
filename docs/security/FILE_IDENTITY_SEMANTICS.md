# Fylura file identity semantics

This document defines the separate identity facts used by operation journals,
filesystem mutation and Safe Trash recovery. It prevents the legacy
`quick_hash` name from being mistaken for a complete content hash and prevents
content verification from being confused with namespace rebinding proof.

## Identity layers

| Layer | Meaning | Safety role |
| --- | --- | --- |
| `NamespaceIdentity` | Physical identity plus type, size and modification time; provider URL evidence is an additional operation-scoped fact when a generic provider path is involved | Prepared journal, same-volume rename/move, Safe Trash, Restore, Replace and namespace delete/rebind checks |
| `ContentVerificationIdentity` | Optional bounded sample hash or complete BLAKE3 content hash | Copy, Duplicate, cross-volume copy verification and recovery where byte equivalence is required |
| `ProviderIdentityEvidence` | Decision B coordinated user-visible URL fingerprint plus physical identity; native item/domain identity is extension-scoped diagnostic data only | Provider-aware revalidation; a CloudStorage path, resource ID or dev/ino tuple alone is only a routing hint |

## Fields

| Field | Meaning | Safety role |
| --- | --- | --- |
| `size` | Byte size for files, or the deterministic recursive byte-size sum for directories | Required in every identity comparison |
| `modified_ns` | Filesystem modification time when available | Fallback metadata signal when a platform file ID is unavailable |
| `platform_volume_id` | Platform volume/device identity when available | Prevents a journal from silently crossing volumes |
| `platform_file_id` | Platform inode/file identity when available | Detects replacement at the same path |
| `quick_hash` / `sample_hash` | BLAKE3 sample hash. Small files hash all bytes; larger files hash the first and last 1 MiB with a domain and size prefix | Fast change detection; never sufficient by itself for high-risk execution |
| `full_hash` | BLAKE3 complete-content hash with a domain and size prefix | Required where the operation policy promises byte equivalence, including Copy/Duplicate, cross-volume verification and recovery that needs content proof; not required for same-volume namespace mutation |

The database keeps `quick_hash` for compatibility, while Rust domain models
expose it as a sample hash. New code requests `full_hash` only when the
operation promises byte equivalence (Copy, Duplicate, cross-volume staging or
content-dependent recovery). Same-volume Rename/Move, Safe Trash and
namespace-only Restore/Delete bind the retained physical object without a
content read.

Schema 35 cleanup rows persist `source_platform_volume_id` and
`source_platform_file_id` as separate physical identity components. Trash
rows persist their existing volume/file components, and Claim rows persist the
raw file identity while the current source-side volume invariant owns the
Claim volume. The schema-34 `macos-dev-ino:<volume>:<file>` value is now a
historical migration input only; runtime code neither generates nor parses
that encoding. New macOS cleanup rows take the source volume from the same
physical capture as the source file identity; an unavailable source volume is
unverifiable and cannot authorize automatic source or Claim recovery. Non-macOS
cleanup rows retain optional physical-ID behavior: a missing optional volume
or file ID is not a blanket failure when the operation's required content,
size, type and time evidence remains valid. A tagged source combined with an
untagged Trash or Claim file ID, or an untagged source combined with tagged
Trash, is retained as legacy evidence and cannot promote a source volume or
trusted identity.

## Directory identity

Directories use a stable recursive manifest when content verification is part
of the operation policy. Entries are sorted by filename; each manifest record
includes the filename, entry type, byte-size contribution and child content
hash. Same-volume directory namespace operations use the root namespace
identity and do not recursively hash the tree. Symlinks, Windows reparse
points and unsupported special entries are rejected instead of being followed.

## Fail-closed comparison

An expected identity field that is present must match an actual field. A missing
actual field cannot satisfy a present expected field. Missing expected optional
content fields are valid for namespace-only operations. On macOS, cleanup
physical matching requires the explicit volume/file components required by the
operation; on non-macOS, optional physical IDs do not fail an operation by
themselves. A proven same-volume Restore target still requires physical file
identity, while a cross-volume or unknown-volume target may rely on complete
content identity and object type. Legacy rows without the physical identity
required by their operation policy are never treated as verified.

The sample-hash regression deliberately changes the middle of a large file:
the sample hash remains equal while the full hash changes. This proves that a
sample collision cannot authorize a high-risk operation.
