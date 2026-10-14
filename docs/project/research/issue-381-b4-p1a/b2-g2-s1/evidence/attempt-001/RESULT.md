# G2-S1 static model — attempt 001

## Scope and source identity

- Authorized branch: `research/issue-414-b2-g2-s1-static-safety`.
- Frozen base HEAD: `26e4255f487b61a4f985491f757e65631739646d`.
- Frozen base Tree: `956f49af8a9aad3d5ac88eb42d4c72be137165cf`.
- Owner authorization: Issue #414 comment `6169345320`.
- Validation scope: only `tests/issue381_b4_p1a_g2_s1/staticSafetyModel.test.ts` and its pure in-memory model/fixtures. No product tests, CI, workflows, or native host operations ran.

## Local validation

Environment: Linux development workspace, Vitest `v4.1.9`.

| Check | Command | Result |
| --- | --- | --- |
| Focused static model suite | `node_modules/.bin/vitest run tests/issue381_b4_p1a_g2_s1/staticSafetyModel.test.ts --reporter=verbose` | Exit 0; 1 file passed; **71 tests passed, 0 failed**. Full output is preserved in `vitest.log`. |
| Focused TypeScript typecheck | `node_modules/.bin/tsc --noEmit --target ES2022 --module ESNext --moduleResolution Bundler --strict --skipLibCheck --types vitest/globals tests/issue381_b4_p1a_g2_s1/staticSafetyModel.ts tests/issue381_b4_p1a_g2_s1/fixtures.ts tests/issue381_b4_p1a_g2_s1/staticSafetyModel.test.ts` | Exit 0; no diagnostics. Exact stdout/stderr capture is `typecheck.log` (empty on success). |
| Test-source prohibited-API scan | `rg -n 'node:(fs|child_process)|child_process|spawn\(|execFile\(|execSync\(|spawnSync\(|New-VHD|Mount-VHD|Initialize-Disk|Format-Volume|hdiutil|diskutil' tests/issue381_b4_p1a_g2_s1` | No matches (exit 1). |

The verbose Vitest log reports start time `13:55:14` and duration `470ms`. The source and test code were unchanged after this run; subsequent changes were limited to the allowed research documentation/evidence paths.

The captured Vitest stdout ends in a blank line, as written by the runner. `git diff --cached --check` reports that raw-log line as a blank-at-EOF warning; the log was retained without trimming or reformatting. The same check excluding that raw log passes for all source, specification, matrix, and receipt files.

## Evidence interpretation

- The positive cases describe safe **synthetic plans** and produce only structured intent records tagged `NEVER_EXECUTED`.
- Negative cases verify rejection of source/path/capacity/identity/host-device/mount/format/cleanup failures, with no intent emitted. Outcomes include `SAFETY_REJECTION`, `ENVIRONMENT_UNAVAILABLE`, `FAIL`, and `CLEANUP_INCOMPLETE`.
- G1 Windows system and data disk controls use synthetic fingerprints derived from the archived G1 inventory; G1 did not publish immutable device serial fingerprints. The macOS deny-list is a synthetic composite of G1's captured `/dev/disk2s5` Data/APFS mount tuple.
- The Microsoft reference pages were checked at the pinned docs-repository commit in `SOURCE-CHECK.md`. The `hdiutil` text was cross-read from a dated third-party Xcode man-page mirror, not from a canonical Apple documentation endpoint. Owner review must validate the mirror against the current Apple-cited page and target macOS version before any S2 proposal.
- This is static model evidence only. It does not demonstrate actual disk API behavior, supported-host capability, real symlink/reparse resolution, safe cleanup on a runner, power-loss safety, or real ENOSPC/durability.

No physical disk or image was created, attached, partitioned, formatted, filled, or mounted. No external disk command, workflow, or CI job was run. Existing blockers remain: `T10_NATIVE_GATE_BLOCKED_ENVIRONMENT`, `P1A_FULL_NO_GO`, `P1B_PRODUCT_NO_GO`, and `ISSUE_381_PRODUCT_FIX_NO_GO`.

`SHA256SUMS.txt` contains digests for the model, fixtures, test source, specification, source crosscheck, test matrix, and raw validation logs. Its independently calculated digest and this receipt's digest are reported in the task closeout.
