# Attempt 002 — hosted G1 inventory receipt

**Disposition: `T10_B2_G1_READY_FOR_OWNER_REVIEW`.** Independent Windows and macOS hosted inventory legs completed, preserved per-probe outputs and exit codes, produced verified SHA-256 manifests, and uploaded artifacts. Both platform summaries conservatively remain `ENVIRONMENT_UNAVAILABLE` because the optional top-level help queries returned nonzero codes; no environment status is being promoted to a capability PASS.

## Exact run provenance

- Workflow run: [38562031226](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38562031226), workflow run attempt 1, `push`, created 2026-10-14T12:12:01Z.
- Branch: `research/issue-414-b2-g1-hosted-preflight`
- Source HEAD: `5ccea76349aa5da2e31b9cb2203c2366d3d263ac`
- Source Tree: `4623041007610337e512e3f2784404cceeb29766`
- Windows job 115845213827: `success`; macOS job 115845214197: `success`.
- Each artifact's SHA-256 matched GitHub's artifact digest and every entry in both platform manifests verified against the archived bytes.
- Attempt 001 remains preserved as a separate failed attempt under `attempt-001/`; this receipt and its evidence do not overwrite it.

## Windows inventory

- Runner: Windows Server 2025, build 26100, X64; image `win25-vs2026`, revision `20260925.250.1`.
- `RUNNER_TEMP`: D: NTFS. Host query reported 235,992,932,352 free bytes; `Get-Volume` reported 235,992,928,256 free bytes.
- Read-only inventory found Hyper-V module 2.0 and `New-VHD`, `Mount-VHD`, `Get-VHD`, and `diskpart.exe` available. The `diskpart /?` stdout contains its help text, while the process returned `999999999`. This exact nonzero result is recorded in its exit file; the platform remains `ENVIRONMENT_UNAVAILABLE` for that optional query.

## macOS inventory

- Runner: macOS 26.6.2, build 25G83, ARM64; image `macos-26-arm64`, revision `20260907.0351.1`.
- `RUNNER_TEMP` mapped through `df` to `/dev/disk2s5`, the APFS Data volume. The read-only free-space query reported 101,845,848,064 bytes available.
- `hdiutil` and `diskutil` were present; `hdiutil help`, `diskutil list`, and `diskutil info /dev/disk2s5` returned exit 0. The actual filesystem personality query reported APFS. The separate no-argument `diskutil` usage query emitted its command list to stderr and returned exit 1, so the platform summary remains `ENVIRONMENT_UNAVAILABLE` for that optional help query.

## Scope and limits

The required source provenance, OS/image, identity, host free-space, and temporary-path probes exited 0 on both platforms. The exact source SHA and Tree are in each platform's provenance output. No test volume was created or allocated; no disk was attached or mounted; no volume was formatted, repartitioned, or filled; no quota changed; and no privilege escalation ran. Report files were written only under each ephemeral runner's `RUNNER_TEMP`.

These observations establish only what read-only commands and metadata were available on these runner images. They do not prove fixed-size disk image creation, attach/mount support, filesystem isolation, actual ENOSPC behavior, durability, sudden power-loss recovery, Windows OS5 behavior, or production F1 qualification. G2 and G3 remain unauthorized. Stop here for independent Owner review.
