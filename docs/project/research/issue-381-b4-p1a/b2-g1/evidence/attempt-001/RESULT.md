# Attempt 001 — hosted G1 result

**Disposition: `FAIL` (implementation defects; not an environment qualification).** This attempt is retained unchanged. It does not qualify Windows or macOS G1 and is not evidence of G2 feasibility.

## Exact run provenance

- Workflow run: [38561024108](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38561024108), attempt 1, `push`, created 2026-10-14T12:03:46Z.
- Branch: `research/issue-414-b2-g1-hosted-preflight`
- Source HEAD: `6f64227cc9569abd501d14904ec775998510f2dc`
- Source Tree: `481f08cfc0b13a4d19d69633f20b1ad97c0af24a`
- macOS job 115841826521: `failure`; Windows job 115841827504: `failure`.
- Both artifact uploads completed. GitHub artifact ZIP digests are recorded in [run-metadata.json](run-metadata.json) and were independently matched to the downloaded bytes.

## Platform observations captured before failure

- Windows runner image `win25-vs2026` version `20260925.250.1`; Windows Server 2025 build 26100, x64. Required provenance, OS, identity, host free-space and RUNNER_TEMP ACL probes exited 0. RUNNER_TEMP was on D: NTFS with 157,761,323,008 bytes free. Read-only inventory reported Hyper-V module 2.0 and `New-VHD`, `Mount-VHD`, and `Get-VHD` available. DiskPart printed its help text, but PowerShell observed `LASTEXITCODE=999999999`; the first script converted that into a wrapper error and then returned process exit 1.
- macOS runner image `macos-26-arm64` version `20260907.0351.1`; macOS 26.6.2 build 25G83, ARM64. The `df` output showed 98,906,112 KiB available for both the root and Data volumes. The required free-space probe failed because BSD `stat` rejected `%S`; the script then derived an incorrect filesystem value. `diskutil help` returned 1 because `help` is not a recognized verb. `diskutil info` against the RUNNER_TEMP directory returned 1 (`Could not find disk: /Users/runner/work/_temp`). The APFS device mapping was therefore not established by this attempt.

## Attempt defects and correction boundary

1. macOS free space and filesystem type must use supported read-only sources: `df` for available bytes and `diskutil info` for the device mapped from `df`.
2. The macOS help inventory must query the utility's actual usage interface, not an unsupported `diskutil help` verb.
3. The macOS artifact manifest must hash files from the report directory. Attempt 001 ran `shasum` from the repository directory, producing an empty uploaded `SHA256SUMS.txt`; all raw probe files and job logs remain present.
4. Windows must preserve DiskPart's observed process return value without converting an optional query into an uncaught exception, and must set an explicit workflow-step exit code after evidence generation. Attempt 001 recorded the DiskPart help text and its wrapper failure separately.

No disk image was created or allocated, no device was attached or mounted, no volume was formatted or repartitioned, no quota was changed, no test payload was written, and no privilege escalation was attempted. The only writes were small report files under runner temporary storage and the uploaded evidence artifacts. The run's job-level failures are implementation failures and are not reclassified as `ENVIRONMENT_UNAVAILABLE` or `CAPABILITY_PRESENT_UNVALIDATED`.
