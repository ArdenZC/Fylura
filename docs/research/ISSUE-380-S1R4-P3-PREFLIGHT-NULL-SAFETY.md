# #380 S1R4-P3 Preflight null-safety remediation

## Status

`S1R4-P3 PRECHECK_NULL_SAFETY_REMEDIATION_READY_FOR_OWNER_REVIEW`

This is an offline research package based on the frozen activation commit. It does not change the consumed activation branch, its Workflow, the frozen P3 Runner, S0 Probe, P1 path guard, production code, Schema, or PR #386. No Hosted run, rerun, workflow dispatch, PowerShell runtime, NSIS, installer, Service, IPC, database, full CI, or Codex Review was started. `ONE_SHOT_CONSUMED`, `RERUN_NO_GO`, and `U2_PRODUCTION_NO_GO` remain in force.

The preflight failure has a source-supported root cause and a narrow inert Workflow remediation. A separate, unchanged clean-status null-output hazard remains in the frozen P3 Runner. This report does **not** claim that a future Hosted NSIS run is ready or authorized.

## Frozen identity and scope

| Item | Identity |
|---|---|
| Frozen activation HEAD | `c474517ff2ae9fe91932c61468fbdf6e2baf28a2` |
| Frozen activation Tree | `b5ce99e3809b78555c1273fe5f984bbd7fb32c11` |
| Frozen P3-P0 parent | `0aa8be6d58fc0994b3be6aec2af8dc02318fcefb` |
| Working branch for this report | `research/issue-380-s1r4-p3-preflight-null-safety-remediation` |
| Original activated Workflow | [source at frozen activation HEAD](https://github.com/ArdenZC/Zen-Canvas/blob/c474517ff2ae9fe91932c61468fbdf6e2baf28a2/.github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml) (unchanged) |
| Frozen Workflow Git blob / raw SHA-256 | `05bcc48e531f662d7aa52e4887f329d2b9710739` / `19bdbce52a890fd340fb9d0d84d45666298080fc2934a0ff1d551d553eaab022` |
| Inert full Workflow draft | [ISSUE-380-S1R4-P3-PREFLIGHT-NULL-SAFETY-WORKFLOW.md](ISSUE-380-S1R4-P3-PREFLIGHT-NULL-SAFETY-WORKFLOW.md) |

The inert Workflow draft changes only the `byte_policy` and `preflight` step bodies. Its push trigger, sole designated path, read-only permissions, Windows image, one-shot attempt guard, direct-child activation proof, all frozen identity checks, Runner process/observer contract, safe evidence upload, and final fail-closed result gate are checked against the original activation Workflow by the accompanying Validator.

## Historical Hosted evidence — immutable and one-shot consumed

| Evidence | Value |
|---|---|
| Run | [38276002263](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38276002263) |
| Job | [114901522647](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38276002263/job/114901522647) — Frozen S0 synthetic NSIS trace probe |
| Attempt | `1` |
| Trigger HEAD / Tree | `c474517ff2ae9fe91932c61468fbdf6e2baf28a2` / `b5ce99e3809b78555c1273fe5f984bbd7fb32c11` |
| Artifact | [11758978213](https://github.com/ArdenZC/Zen-Canvas/actions/runs/38276002263/artifacts/11758978213) |
| Artifact ZIP size / SHA-256 | `2712` bytes / `5e30fdfe11113444ef8663bbb78127ab4d1e1ca221bc2b0eda84aa206ff31d2d` |
| Actions log size / SHA-256 | `136798` bytes / `88429ae1d8e603aff87a0a803758f13d70a7e4558c99ee7519e24c78526fc5ed` |
| Windows / PowerShell | Windows Server 2022, build `10.0.20348.0`; PowerShell `7.6.6`; image `20261004.326.1` |

The artifact's four original JSON records were independently checked against its `SHA256SUMS`. Their hashes are:

| Artifact member | SHA-256 |
|---|---|
| `assessment.json` | `7373d13cba452351f091148b60515e1bc310af76e864bb37b12958ff09e0941f` |
| `git-byte-policy.json` | `caa78d428deda8c039bd1c132cfd0df59a4c3303fc451afe4e31ccf956030058` |
| `preflight.json` | `f79ed7efab147060a74ba6ae31ac2ec5a09371fe60700e9b668f34fe056f1ee8` |
| `workflow-run.json` | `01baa6aea56714e87f6dee9d6ecf96e078cbff5572064883cd0989fec905192d` |

### Failure timeline

Times below are the original Actions log timestamps in UTC.

1. `16:14:35.592` — Windows Server 2022 job starts on `windows-2022`; GitHub reports read-only `contents` permission.
2. `16:14:36.714` — bootstrap creates the private evidence root and records Run ID, attempt, SHA, host and PowerShell identity.
3. `16:14:37.263` — raw byte policy step succeeds and records `core.autocrlf=false`, `core.eol=lf` before checkout.
4. `16:14:38.135` — checkout of activation HEAD starts; checkout completes successfully.
5. `16:14:47.306` — preflight's `finally` persists `preflight.json` with the activation parent and the single expected Workflow addition. `current_p2_tree`, P1 hashes, Probe blob and Runner identities remain null/empty.
6. `16:14:48.491` — preflight throws `You cannot call a method on a null-valued expression`; the logged source is the final error wrapper line, not a PowerShell source line for the original expression.
7. `16:14:48.539` — preflight process exits `1`. The Runner step is skipped by its `steps.preflight.outcome == 'success'` gate.
8. `16:14:49.191` — independent evidence-upload eligibility succeeds.
9. `16:14:50.947` — original raw artifact is finalized and uploaded as Artifact `11758978213`.
10. `16:14:51.446` — final one-shot result gate fails closed because the assessment is not a synthetic NSIS pass. Job ends failed. There is no later Run attempt.

No NSIS download, compiler invocation, Probe compilation, stage, installer/uninstaller process, expected exit code 37/42 assertion, or observer execution occurred. The artifact confirms `runner_outcome: skipped`, `runner_exit_code: null`, no observed processes, and `process_observer_status: NOT_AVAILABLE`.

## Root cause review

The activated Workflow preflight sequence is:

1. It resolves and records the activation parent and the one-file activation diff.
2. It immediately executes this expression ([frozen Workflow line 166](https://github.com/ArdenZC/Zen-Canvas/blob/c474517ff2ae9fe91932c61468fbdf6e2baf28a2/.github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml#L166)):

   ```powershell
   $checkoutStatus = (& git -C $env:GITHUB_WORKSPACE status --porcelain --untracked-files all).Trim()
   ```

3. Only after that does it enumerate controlled files, verify Git attributes and identities, resolve the P2/P1 trees, check the S0 Probe, and inspect the synthetic HKCU key.

For a clean checkout, `git status --porcelain --untracked-files all` has no stdout. A no-output native invocation supplies `$null` to the direct expression in PowerShell, so `.Trim()` is invoked on `$null`. PowerShell raises the exact null-valued method-call error recorded in the hosted failure. In this checkout, the equivalent Git command also produces zero output bytes. The hosted Windows image and PowerShell version are independently recorded as Windows Server 2022 / PowerShell 7.6.6.

The hosted artifact does not record a separate stdout-byte count for the failing `git status` call, and the generic stack trace omits its original source line. Therefore the exact expression is identified by the source order, persisted fields, and runtime exception, but the empty stdout itself was not separately captured in that consumed run. The local zero-byte check plus PowerShell no-output semantics support the diagnosis; an independent PowerShell runtime reproduction was not run because `pwsh` is unavailable in this Linux environment and no new Windows job is authorized.

## Other output and exit-code hazards found

The audit covered all native Git calls in the active Workflow's byte-policy and preflight steps, plus empty collections and the following Runner gate.

| Existing path | Finding | Required contract in inert draft |
|---|---|---|
| Byte-policy `core.autocrlf` and `core.eol` reads | Both directly call `.Trim()` on Git output; the exit code is checked only after both reads, so the first status can be overwritten. | Capture each call's lines and exit code immediately; require exactly one non-empty line for each setting. |
| Preflight effective `core.autocrlf` and `core.eol` reads | Same direct `.Trim()` and paired `$LASTEXITCODE` ambiguity. These values are required after the preceding global set. | Strict scalar helper; a missing setting or nonzero exit fails before any identity gate can pass. |
| HEAD, Tree, parent, frozen trees, object/blob IDs and raw-file Git hashes | These are required outputs; direct `.Trim()` can mask an empty result with a null method error, and some paired commands only check the last exit code. | Required scalar helper captures each exit independently and rejects nonzero exit, zero/multiple lines, or whitespace-only output before identity comparisons. Existing SHA/Tree/Blob comparisons remain exact. |
| `git status --porcelain` | Empty output is the normal clean result, not a missing required identity. | Accept only exit `0` and exactly zero output lines. Any output, including diagnostics, rejects; any nonzero exit rejects. |
| `git config --show-origin --get-regexp` for filters | No configured filter is an allowed `git config` result: exit `1` with no output. | Encode that one optional-missing case explicitly. Exit `0` requires non-empty rows; every other exit/output combination fails closed. |
| `git ls-files` and `.gitattributes` discovery | A missing tracked `.gitattributes` is allowed and is recorded as `NO_TRACKED_GITATTRIBUTES`; however, the tracked-file inventory itself must be available. | Capture exit immediately, require a successful non-empty tracked file list, then allow the filtered `.gitattributes` set to be empty. |
| `git check-attr` | The old preflight checked exit code and validated returned rows, but did not assert that all requested rows were returned. | Require exactly four attribute rows per controlled file before examining transform attributes. |
| `git merge-base --is-ancestor` | This command succeeds with no output; its result is carried by exit code. | Require exit `0` and zero output. Nonzero exit or unexpected output fails. |

### Separate frozen Runner blocker

The frozen P3 Runner still has a separate clean-status null-output hazard and repeats the same pattern at [line 665](https://github.com/ArdenZC/Zen-Canvas/blob/0aa8be6d58fc0994b3be6aec2af8dc02318fcefb/scripts/issue380S1R4P3SyntheticNsisRunner.ps1#L665):

```powershell
$status = (& git status --porcelain --untracked-files all).Trim()
```

The Runner's current approved Git blob is `1d11c3add3c0d1f45e7d7ac082d09c616cca51a1`, raw SHA-256 `c47686d36f19a0732c8aafc2ad502f96e20e6ce16e9d443e67428f76568292cd`. It is deliberately unchanged because the activated Workflow requires this exact identity. If only the Workflow preflight is corrected, then a clean checkout can pass the new preflight and the frozen Runner can fail at its own clean-status `.Trim()` before NSIS. That downstream defect is recorded as an outstanding blocker, not silently patched or bypassed here. It requires a separate Owner decision and, if changed, a newly reviewed Runner identity contract; it cannot revive or rerun the consumed one-shot Run.

## Minimal fail-closed remediation prepared

The inert full Workflow draft introduces a small `Invoke-Issue380GitCapture` helper in the byte-policy and preflight steps. It locally disables `PSNativeCommandUseErrorActionPreference` so `$ErrorActionPreference = 'Stop'` cannot turn a native diagnostic into a terminating PowerShell error before the native exit code is captured. Immediately after the native invocation it captures `$LASTEXITCODE`, merges output/error text for fail-closed inspection, stores output lines in a typed array, records the output count, and builds a non-null text string. Required scalar calls then demand exit `0` and exactly one non-empty line before trimming or comparing. Git command status and output emptiness are separate values.

For worktree cleanliness, the draft uses `git status` result `{ exit code, line count, text }`: clean means exit `0` plus `OutputCount == 0`; any emitted line rejects. It never calls `.Trim()` on a possibly null native result. For required identities, empty output is an error before equality checks. The optional filter config recognizes only exit `1` with no lines as “no filters configured.” `git check-attr` additionally requires a complete result set.

The draft retains the original direct-child activation check, exact one-file activation diff, core Git byte policy, P2/P1 ancestry and Tree checks, frozen P3 Runner/P1/S0 identities, path and HKCU checks, Run attempt `1`, process argument and observer checks, secure independent artifact eligibility, and final PASS criteria. A failing preflight keeps `PREFLIGHT_OUTCOME` unsuccessful; the Runner step remains gated on preflight success and cannot start after any preflight failure.

No production or frozen Runner admission is weakened. The known Runner null-output hazard means this preflight draft is a reviewable repair proposal, not a Hosted-ready or NSIS-ready test plan.

## Offline validation and requested contract matrix

The Validator parses the complete draft YAML with PyYAML and all embedded PowerShell steps with tree-sitter-powershell. It compares the draft against the frozen activated Workflow and requires that only `byte_policy` and `preflight` bodies differ. Its eight offline contract tests use typed `{ exit_code, output_lines }` fixtures; they are deliberately labeled a contract model, not PowerShell execution.

| # | Contract | Expected result | Offline result |
|---:|---|---|---|
| 1 | Legacy direct method call on no-output data fails in the contract model; `git status`: exit `0`, no output (`$null` normalized to zero lines) | Legacy failure reproduced in model; normalized result is clean | PASS |
| 2 | `git status`: exit `0`, porcelain modification row | Reject | PASS |
| 3 | `git status`: nonzero exit, no output | Fail closed | PASS |
| 4 | Required SHA command: exit `0`, empty output | Fail closed | PASS |
| 5 | Optional filter config: exit `1`, empty output | Accept as absent | PASS |
| 6 | Required core config: exit `1`, empty output | Fail closed | PASS |
| 7 | Any failed preflight outcome | Runner step remains skipped | PASS (static Workflow gate + contract test) |
| 8 | One-shot attempt, activation identity, read-only permission, safe evidence upload, final fail-closed gate | Preserved | PASS (static comparison) |

Static parsing validates syntax shape only. The Python fixtures do not execute PowerShell or native Git through PowerShell. The recorded command is:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=/tmp/issue380-p3p0r-python-deps python3 scripts/validateIssue380S1R4P3PreflightNullSafety.py
```

The offline parser versions are PyYAML `6.0.3`, tree-sitter `0.25.2`, and tree-sitter-powershell `0.26.4`.

The Validator also creates and removes a temporary local Git repository, runs `git status --porcelain --untracked-files all` on its clean worktree, and verifies exit `0` with exactly zero stdout/stderr bytes. This confirms Git's empty-output side of the scenario on the current Linux host; it does not reproduce PowerShell 7's null conversion.

## Unverified and not run

- PowerShell reproduction of empty native stdout and the proposed helper: **NOT RUN**.
- Windows hosted validation: **NOT RUN; rerun prohibited**.
- P3 Runner and NSIS: **NOT EXECUTED** in Run `38276002263`; no later attempt is authorized.
- Frozen Runner clean-status null-output repair: **NOT IMPLEMENTED**; awaiting Owner design/identity decision.
- NSIS download/compiler/Probe/stage/installer/uninstaller and exit-code `37/42` evidence: **NOT RUN**.
- Service, IPC, UAC, databases, Schema, production installer, U2 production changes: **NOT RUN / NO-GO**.
- Whether the PowerShell native stderr merge behavior in the proposed helper has any runner-image-specific edge case: **UNVERIFIED** until a separately authorized PowerShell runtime validation.

## Original Artifact JSON records

The following are exact JSON bodies read from Artifact `11758978213`; the artifact's internal `SHA256SUMS` matched every file hash listed above.

### `workflow-run.json`

```json
{
  "repository": "ArdenZC/Zen-Canvas",
  "workflow_ref": "ArdenZC/Zen-Canvas/.github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml@refs/heads/research/issue-380-s1r4-p3-p0-nsis-hosted-plan",
  "triggering_ref": "refs/heads/research/issue-380-s1r4-p3-p0-nsis-hosted-plan",
  "triggering_head": "c474517ff2ae9fe91932c61468fbdf6e2baf28a2",
  "run_id": "38276002263",
  "run_attempt": "1",
  "bootstrap_owner_token": "ArdenZC/Zen-Canvas|refs/heads/research/issue-380-s1r4-p3-p0-nsis-hosted-plan|38276002263|1|c474517ff2ae9fe91932c61468fbdf6e2baf28a2",
  "runner_os": "Windows",
  "runner_arch": "X64",
  "image_os": "win22",
  "image_version": "20261004.326.1",
  "powershell": "7.6.6",
  "windows_version": "Microsoft Windows NT 10.0.20348.0"
}
```

### `git-byte-policy.json`

```json
{
  "status": "PASS",
  "core_autocrlf_global": "false",
  "core_eol_global": "lf",
  "set_before_checkout": true,
  "recorded_utc": "2026-10-12T16:14:38.0425887Z"
}
```

### `assessment.json`

```json
{
  "status": "FAIL_CLOSED",
  "preflight_outcome": "failure",
  "runner_outcome": "skipped",
  "workflow_sha": "c474517ff2ae9fe91932c61468fbdf6e2baf28a2",
  "run_id": "38276002263",
  "run_attempt": "1",
  "runner_exit_code": null,
  "runner_timed_out": null,
  "runner_process_exited": false,
  "runner_wait_for_exit_succeeded": false,
  "runner_wrapper_failure": null,
  "runner_result_status": null,
  "required_primary_assertions_pass": false,
  "hkcu_absent_after_run": true,
  "task_root_absent_after_run": true,
  "task_root_cleanup": "ALREADY_ABSENT",
  "task_owned_processes": [],
  "process_observer_status": "NOT_AVAILABLE",
  "observed_descendant_processes": [],
  "suspected_out_of_root_children": [],
  "direct_process_identity_map": [],
  "pre_cleanup_task_process_snapshot": [],
  "post_cleanup_task_process_snapshot": [],
  "failure": "System.Management.Automation.RuntimeException: One or more P3 fail-closed evidence or cleanup conditions failed."
}
```

### `preflight.json`

```json
{
  "status": "FAIL",
  "expected_p2_head": "86c21c14dd65bd2bbdb3056513489e0d4139bb47",
  "expected_p2_tree": "883a851cf8e75b4f39a4a46baea15ab109feb312",
  "expected_p1_head": "7a6fa69e5521d9759702c1923a9ade60a75819c3",
  "expected_p1_tree": "c5465ef93eae268e2c683876d959db90fac5da08",
  "triggering_ref": "refs/heads/research/issue-380-s1r4-p3-p0-nsis-hosted-plan",
  "triggering_head": "c474517ff2ae9fe91932c61468fbdf6e2baf28a2",
  "checked_out_head": "c474517ff2ae9fe91932c61468fbdf6e2baf28a2",
  "checked_out_tree": "b5ce99e3809b78555c1273fe5f984bbd7fb32c11",
  "current_p2_tree": null,
  "p1_path_guard_sha256": null,
  "p1_path_test_sha256": null,
  "probe_blob": null,
  "runner_temp": "D:\\a\\_temp",
  "task_root": "D:\\a\\_temp\\issue380-s1r4-p3-task",
  "output_root": "D:\\a\\_temp\\issue380-s1r4-p3-evidence",
  "p3p0_head": "0aa8be6d58fc0994b3be6aec2af8dc02318fcefb",
  "p3p0_tree": "688bd53c3c44040f3ab86fe4cf71a7a3aa879274",
  "activation_parent": "0aa8be6d58fc0994b3be6aec2af8dc02318fcefb",
  "activation_diff": [
    "A\t.github/workflows/issue-380-s1r4-p3-synthetic-nsis.yml"
  ],
  "p3_runner_git_blob_sha": null,
  "p3_runner_file_sha256": null,
  "controlled_file_identities": [],
  "git_byte_policy": {
    "core_autocrlf_global": "false",
    "core_eol_global": "lf",
    "core_autocrlf_effective": "false",
    "core_eol_effective": "lf",
    "set_before_checkout": true
  },
  "gitattributes_inventory": [],
  "effective_transform_attributes": [],
  "filter_configuration": [],
  "hkcu_key_initially_absent": false,
  "failure": "System.Management.Automation.RuntimeException: You cannot call a method on a null-valued expression.\r\n   at System.Management.Automation.ExceptionHandlingOps.CheckActionPreference(FunctionContext funcContext, Exception exception)\r\n   at System.Management.Automation.Interpreter.ActionCallInstruction`2.Run(InterpretedFrame frame)\r\n   at System.Management.Automation.Interpreter.EnterTryCatchFinallyInstruction.Run(InterpretedFrame frame)\r\n   at System.Management.Automation.Interpreter.EnterTryCatchFinallyInstruction.Run(InterpretedFrame frame)"
}
```

## SHA-256 manifest

The accompanying `docs/research/evidence/issue-380-s1r4-p3-preflight-null-safety/SHA256SUMS` covers the report, full inert Workflow draft, and offline Validator. The frozen activation commit and GitHub Artifact/log hashes above remain the source identities for the historical run.
