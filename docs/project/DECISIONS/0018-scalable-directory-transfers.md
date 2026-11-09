# ADR-0018 — Scalable, recoverable directory transfers

**Status:** Accepted by the task owner’s explicit implementation brief (2026-11-09).
**Scope:** Directory Copy, cross-volume Move, Restore, content identity, and the macOS transfer adapter.

## Context

Directory transfer currently refuses trees above 5,000 descendant entries. The
native macOS full-copy profile already exercises 100,000 entries, so this
guard prevents the product path from reaching the requested validation scale.
The transfer path also materializes sorted directory entries and, on macOS,
streams every regular file even when APFS can clone its extents.

Raising the count to the test size would leave the same scaling behavior and
would make that test size the new product ceiling.

## Decision

- Remove the fixed descendant-entry ceiling from directory identity, Preview,
  Copy, Move, and Restore. Retain the 64-level depth ceiling and fail closed
  when the operation’s shared directory-name memory budget reaches 64 MiB or
  its buffers cannot grow.
- Keep directory work serial and descriptor/handle scoped. Copy and retirement
  process entries one at a time; deterministic manifest passes retain only
  sorted names for active directories, within the shared memory budget.
  Recovery-compatible transfer fingerprints retain globally sorted records
  under that same budget. Existing cancellation, source/target verification,
  journal, commit, and recovery boundaries remain authoritative. Retained
  hardlink paths share the same directory-name memory budget.
- On local same-volume APFS, directory Copy may attempt `fclonefileat` for each
  regular file. It preserves source hardlink groups, copies and checks supported
  metadata, and still runs the complete staged-tree and source verification.
  An unavailable clone falls back to the existing streamed copy.
- On Windows, keep the existing 1 MiB copy, source claim, streaming hash,
  staging verification, publication, and recovery contracts. Consolidate the
  pre-publication flushes to the existing final `sync_all()` after data and
  metadata are written. Do not use a path-based native copy API that would
  bypass the retained source claim.
- The 100,000-entry fixture is a validation workload, not a new item-count
  limit. This decision adds no user setting, journal, schema, or operation
  authority. Transfer fingerprint serialization remains byte-compatible with
  existing journal rows so an upgrade does not invalidate pending recovery
  proofs.

## Consequences

Wide directories use bounded name-buffer memory instead of a fixed item count;
depth, cancellation, and allocation failures still stop safely. APFS cloning
does not waive content verification or hardlink preservation. Windows retains
the same content checks and durability boundary with fewer redundant flushes.
Native performance claims remain tied to the matching Windows and Apple
Silicon measurements.
