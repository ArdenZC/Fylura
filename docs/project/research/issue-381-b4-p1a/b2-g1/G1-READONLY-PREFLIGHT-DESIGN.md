# Issue #414 B2-G1 hosted read-only preflight

**Authorization:** Owner comment [#6167568523](https://github.com/ArdenZC/Zen-Canvas/issues/414#issuecomment-6167568523) authorizes only G1 read-only hosted environment inventory. G2 volume creation, G3 physical ENOSPC, B3 power reset, production changes, PR, merge, and Codex Review remain unauthorized.

## Provenance and scope

- Repository: `ArdenZC/Zen-Canvas`
- Existing branch: `research/issue-414-b2-g1-hosted-preflight`
- Frozen starting HEAD: `26e4255f487b61a4f985491f757e65631739646d`
- Starting Tree: `956f49af8a9aad3d5ac88eb42d4c72be137165cf`
- Branch was checked out from the existing remote ref; no other remote branch was created.
- Exactly one new workflow is permitted: `.github/workflows/issue-414-b2-g1-hosted-preflight.yml`.
- Evidence additions are confined to this `docs/project/research/issue-381-b4-p1a/b2-g1/**` directory. Existing evidence remains untouched.

## Hosted inventory

The workflow runs a two-leg, fail-independent matrix on `windows-latest` and `macos-latest`. It triggers only on a push to the exact research branch when the new workflow file itself is in the changed paths. It uses the repository's existing pinned `actions/checkout` and `actions/upload-artifact` SHAs, sets `contents: read`, checks out `github.sha`, and disables checkout credential persistence.

Each leg records source SHA and tree, run ID and attempt, runner image/OS/build/architecture, identity and read-only permission hints, filesystem and host free-space metadata, `RUNNER_TEMP` path details, and current read-only device/volume metadata. Windows queries Hyper-V VHD command availability and DiskPart help. macOS queries `hdiutil` and `diskutil` help and current volume information. Each probe has separate stdout, stderr, and exit-code files. A platform-local SHA-256 manifest covers the generated evidence files.

Only small sanitized reports are written under the ephemeral runner's `RUNNER_TEMP` for artifact upload. The probe does not create or allocate disk images, attach or mount volumes, format or repartition devices, change quotas, write test payloads, elevate privileges, or try to fill a filesystem. Device reports omit volume UUIDs and disk serial identifiers.

## Result semantics

- `CAPABILITY_PRESENT_UNVALIDATED` means read-only command and filesystem inventory is present; it does not prove that G2 image creation/attachment or filesystem behavior will work.
- `ENVIRONMENT_UNAVAILABLE` records a missing/failed optional primitive, unknown/insufficient host free capacity, or unsupported runner filesystem with the exact probe output and exit code.
- `FAIL` is reserved for a required provenance/OS/identity/free-space/path probe or the evidence-manifest step failing.
- A successful GitHub Actions job means G1 inventory completed. It is not B2 qualification, native durability evidence, ENOSPC evidence, or a power-loss result.

The G2 planning threshold is at least 4 GiB free on the host volume containing `RUNNER_TEMP`; the read-only probe only measures and reports it. Regardless of G1 results, execution stops for Owner review. No G2/G3 action is chained or authorized by this workflow.
