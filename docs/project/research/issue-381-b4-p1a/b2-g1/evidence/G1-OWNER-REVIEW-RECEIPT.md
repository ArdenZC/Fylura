# Issue #414 T10-B2-G1 hosted read-only preflight

**Status: `T10_B2_G1_READY_FOR_OWNER_REVIEW`.** The Windows and macOS hosted jobs completed the authorized metadata-only inventory and uploaded artifacts whose downloaded bytes and in-run file manifests were verified. Per-platform classifications remain `ENVIRONMENT_UNAVAILABLE` because each runner returned a nonzero result for an optional utility help query. No capability is marked validated. Stop here for independent Owner review; G2 and G3 have not run and remain unauthorized.

## Scope and source

- Repository: `ArdenZC/Zen-Canvas`
- Branch: `research/issue-414-b2-g1-hosted-preflight`
- Frozen baseline HEAD: `26e4255f487b61a4f985491f757e65631739646d`
- Baseline Tree: `956f49af8a9aad3d5ac88eb42d4c72be137165cf`
- Hosted workflow path: `.github/workflows/issue-414-b2-g1-hosted-preflight.yml`
- Source commit for the successful inventory: `5ccea76349aa5da2e31b9cb2203c2366d3d263ac`
- Source Tree: `4623041007610337e512e3f2784404cceeb29766`
- `attempt-002/workflow-source.yml` preserves the exact workflow source used by that run. This receipt and its enclosing evidence commit were added afterward; they were not rerun.

## Attempts

### Attempt 001 — implementation failure, preserved

- Run [38561024108](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38561024108), source HEAD `6f64227cc9569abd501d14904ec775998510f2dc`, Tree `481f08cfc0b13a4d19d69633f20b1ad97c0af24a`.
- Both jobs failed due to probe/reporting defects. The exact logs, artifacts, metadata, workflow source, and original SHA manifest remain in `attempt-001/`; none were overwritten.

### Attempt 002 — both read-only inventories completed

- Run [38562031226](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38562031226), attempt 1, source HEAD `5ccea76349aa5da2e31b9cb2203c2366d3d263ac`, Tree `4623041007610337e512e3f2784404cceeb29766`.
- Windows job `115845213827`: success; artifact `11893365960`, 9,673 bytes, SHA-256 `135a8ba5d716ffd1321fa44286063ef839b47b1dc8bc8bde260b4889480d391b`.
- macOS job `115845214197`: success; artifact `11892191907`, 10,443 bytes, SHA-256 `0987b59d87452dd6993ae37ddd6cc3e95dc7d0ba9ffa6162e03ecc0daab40cfb`.
- Full raw job logs, original artifact ZIPs, extracted per-probe stdout/stderr/exit codes, in-run manifests, run metadata, and exact workflow source are preserved in `attempt-002/`. The per-platform in-run manifests contain 29 Windows and 30 macOS entries; all entries verified against the extracted artifact bytes. Downloaded ZIP digests match the GitHub artifact digests.

## Observations and classification

- Windows: Windows Server 2025 build 26100, X64; image `win25-vs2026` revision `20260925.250.1`; `RUNNER_TEMP` on D: NTFS, with 235,992,932,352 free bytes by Win32 query and 235,992,928,256 by `Get-Volume`. Hyper-V module 2.0, `New-VHD`, `Mount-VHD`, `Get-VHD`, and `diskpart.exe` were reported present. `diskpart /?` emitted help text but returned the exact unexpected exit code `999999999`, so classification is `ENVIRONMENT_UNAVAILABLE`.
- macOS: macOS 26.6.2 build 25G83, ARM64; image `macos-26-arm64` revision `20260907.0351.1`; `RUNNER_TEMP` mapped to APFS Data device `/dev/disk2s5`, with 101,845,848,064 free bytes by `df`. `hdiutil` and `diskutil` were reported present; `hdiutil help`, `diskutil list`, and `diskutil info` for the mapped device returned 0. The no-argument `diskutil` usage query emitted its command list to stderr but returned 1, so classification is `ENVIRONMENT_UNAVAILABLE`.
- All required provenance, OS/image, identity, temporary-path, and free-space queries returned 0 on both platforms. The optional nonzero results remain visible in their raw probe files and are not suppressed or converted into capability success.

## Safety boundary and interpretation

Only read-only metadata, device/volume inventory, and help queries ran. No disk image or test volume was created or allocated; no device was attached or mounted; no filesystem was formatted or filled; no partition, quota, or privileges were changed. Report files were written under each ephemeral runner's `RUNNER_TEMP` and uploaded as workflow artifacts.

This inventory does not prove that bounded image creation or attachment works, filesystem isolation, actual disk-full behavior, durability, sudden power-loss recovery, Windows OS5 behavior, or production qualification. In particular, it is not evidence of real `ENOSPC` or power-loss behavior. G2/G3 were not run. Handoff is for independent Owner review only.
