# B04 R1 source-only checks — attempt 002

**Scope:** static validation and in-memory policy tests only. No native
Windows executable or probe was launched.

## Source identity

- Existing branch: `test/issue-411-t10-b04-ntfs-probe`.
- Starting HEAD/tree: `89b1fad5a47397c6409c014fe2fb329a5410530d` /
  `92ca5a6d5e7c6a0728eb233c030d69f7da0b3498`.
- Source file SHA-256 used by the final checks:
  `fa3af9eae967fdf9828529d433d2582ef38e6f9bb8e3749a6f1cd70833115308`.
- Host: Linux 6.18.44 x86_64; `rustc 1.97.1`, `cargo 1.97.1`.
- Installed targets: `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-gnu`.

## Final command receipts

Raw stdout/stderr are stored byte-for-byte in deterministic gzip files
(`.log.gz`); exact exit codes are stored in matching `.exit` files. The
authoritative final receipts below were run against the source SHA recorded
above. Decompress a log with `gzip -cd <file>.log.gz`.

| Check | Command | Exit | Evidence |
| --- | --- | ---: | --- |
| Formatting | `rustfmt --edition 2024 --check tests/issue381_b4_p1a_f1/src/bin/t10_b04_ntfs_probe.rs` | 0 | `final-rustfmt.log.gz`, `final-rustfmt.exit` |
| Focused policy tests | `cargo test --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml --bin t10_b04_ntfs_probe` | 0 | `final-focused-tests.log.gz`, `final-focused-tests.exit` |
| Linux source check | `cargo check --locked --manifest-path tests/issue381_b4_p1a_f1/Cargo.toml --bin t10_b04_ntfs_probe` | 0 | `final-linux-check.log.gz`, `final-linux-check.exit` |
| Windows-target source type-check | `cargo check --offline --manifest-path /tmp/b04-windows-check/Cargo.toml --target x86_64-pc-windows-gnu --bin t10_b04_ntfs_probe` | 0 | `final-windows-source-typecheck.log.gz`, `final-windows-source-typecheck.exit` |

The focused test run passed **16/16** in-memory tests. These tests do not call
the probe's `main`, access the filesystem, or invoke Windows APIs. The Windows
target command is `cargo check` with a temporary manifest outside the
repository and the same pinned direct `serde_json` and `sha2` versions; it
type-checks the Windows-only Rust and FFI source but does not link or execute a
Windows program. Repository Cargo manifests and lockfiles were not changed.

The initial non-`final-` logs in this attempt directory are retained as
append-only preliminary receipts from before the last source-only identity
gate/test-name adjustment. They are not the final validation claim; the
`final-*` receipts are authoritative for the source SHA above. All log files
are stored as byte-preserving gzip copies so terminal blank lines do not make
the repository whitespace check report false positives.

## R1 outcome and limits

- Pre-rename identity must be nonzero/non-placeholder and equal to the
  captured identity; missing, invalid, or changed identity prevents the rename
  call. Post-rename identity mismatch prevents the content-read step.
- Cleanup verifies the captured identity through read-only handles before
  removing either generated name. Unknown or different objects are left
  untouched and produce `CLEANUP_INCOMPLETE`.
- Required phases drive the aggregate status. Directory open/sync API errors
  remain observations; safety/environment failures, skips, missing phases, and
  cleanup failures do not produce an overall observed-success state. Enabled
  blocked outcomes have nonzero exit codes and are also explicit in JSON.
- The binary now verifies its own SHA-256 against
  `T10_B04_EXPECTED_BINARY_SHA256`, in addition to exact clean Git HEAD/tree.
  The future approved runner still must build and set that expected hash in
  the same job.
- Standard `windows-latest` execution is expected to fail the existing
  non-elevated-token safety gate. No runner/workflow or timeout wrapper was
  added. A runner-level 30-second process timeout and safe timeout evidence/
  cleanup plan remain prerequisites for any later native authorization.

`T10_B04_NATIVE_EXECUTION_NO_GO` remains in force. Windows native execution,
full CI, real ENOSPC, and physical power loss were not run.
