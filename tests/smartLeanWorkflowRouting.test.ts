import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Performance comparisons are opt-in. Rust CI owns routine correctness; this
// workflow keeps controlled writer pairs and master/candidate baselines apart.
const workflow = readFileSync(".github/workflows/smart-lean.yml", "utf8");
const filenameFirstQualification = readFileSync("scripts/qualifyWindowsGlobalIndexFilenameFirst.ps1", "utf8");
const mft = readFileSync("src-tauri/src/global_index/windows/mft.rs", "utf8");
const prBlock = workflow.split("\n  push:")[0];
const routes = [...prBlock.matchAll(/^      - '([^']+)'$/gmu)].map((match) => match[1]);
const writerJob = workflow.split("\n  master-candidate-baseline:")[0];
const baselineJob = workflow.split("\n  master-candidate-baseline:")[1]?.split("\n  macos-spotlight-capability:")[0] ?? "";

function triggersFor(file: string): boolean {
  return routes.some((route) => route.endsWith("/**")
    ? file.startsWith(route.slice(0, -2))
    : route === file);
}

describe("Global Index performance workflow routing", () => {
  it("offers explicit labeled performance runs for ingestion and native changes", () => {
    expect(prBlock).toContain("types: [labeled]");
    expect(routes).toEqual([
      "src-tauri/src/global_index/index_scope/**",
      "src-tauri/src/db/schema_index_coverage.rs",
      "src-tauri/src/global_index/repository.rs",
      "src-tauri/src/global_index/coordinator.rs",
      "src-tauri/src/global_index/models.rs",
      "src-tauri/src/global_index/windows/mod.rs",
      "src-tauri/src/global_index/windows/service.rs",
      "src-tauri/src/global_index/macos/mod.rs",
      "src-tauri/src/global_index/windows/mft.rs",
      "src-tauri/src/global_index/windows/usn.rs",
      "src-tauri/src/global_index/windows/fallback.rs",
      "src-tauri/src/global_index/windows/smart_lean_qa.rs",
      "src-tauri/src/global_index/macos/spotlight.rs",
      "src-tauri/src/global_index/macos/fsevents.rs",
      "src-tauri/src/global_index/macos/smart_lean_qa.rs",
      "scripts/qualifySmartLeanMacos.sh",
      ".github/workflows/smart-lean.yml",
    ]);
    for (const file of [
      "src-tauri/src/global_index/index_scope/native.rs",
      "src-tauri/src/global_index/index_scope/repository.rs",
      "src-tauri/src/global_index/index_scope/tests.rs",
      "src-tauri/src/db/schema_index_coverage.rs",
      "src-tauri/src/global_index/repository.rs",
      "src-tauri/src/global_index/coordinator.rs",
      "src-tauri/src/global_index/models.rs",
      "src-tauri/src/global_index/windows/mft.rs",
      "src-tauri/src/global_index/macos/spotlight.rs",
      "scripts/qualifySmartLeanMacos.sh",
    ]) {
      expect(triggersFor(file), file).toBe(true);
    }
  });

  it("keeps pure search, UI and activation edits out of the expensive workflow", () => {
    for (const file of [
      "src-tauri/src/global_index/search.rs",
      "src-tauri/src/global_index/filename_search.rs",
      "src-tauri/src/global_index/commands.rs",
      "src-tauri/src/global_index/windows/volumes.rs",
      "src-tauri/src/global_index/tests/search_source_authorization.rs",
      "src-tauri/src/global_index/tests/filename_search_candidate.rs",
      "src-tauri/src/global_index/tests/global_index_scope_authorization.rs",
      "src-tauri/src/app_control.rs",
      "src/views/settings/SettingsView.tsx",
      "tests/smartLeanWorkflowRouting.test.ts",
    ]) {
      expect(triggersFor(file), file).toBe(false);
    }
    expect(routes).not.toContain("src-tauri/src/global_index/**");
  });

  it("runs the writer pair only on an explicit mode, marked perf push or label", () => {
    expect(writerJob).toContain("inputs.mode == 'writer_pair'");
    expect(writerJob).toContain("[run-global-index-performance]");
    expect(writerJob).toContain("run-global-index-performance");
    expect(writerJob).toContain("ZEN_GLOBAL_WRITER_ENTRIES: '100000'");
    expect(writerJob).toContain("Run 100k writer pair in Release");
    expect(writerJob).toContain(
      "global_index_writer_ingestion_100k_pair -- --ignored --nocapture --test-threads=1",
    );
    expect(writerJob).toContain("Run trigger attribution after writer pair");
    expect(writerJob).toContain(
      "global_index_writer_trigger_contribution_25k -- --ignored --nocapture --test-threads=1",
    );
    expect(writerJob).toContain("cancel-in-progress: false");
    expect(writerJob).not.toContain("github.event_name == 'pull_request' &&");
  });

  it("keeps master/candidate release comparisons manual and preserves bounded sizes", () => {
    expect(workflow).toContain("  workflow_dispatch:");
    expect(workflow).toContain("options: [writer_pair, master_candidate, macos_spotlight, filename_first_pair]");
    expect(workflow).toContain("default: '100000'");
    expect(workflow).toContain("options: ['1000', '100000', '200000']");
    expect(baselineJob).toContain("if: github.event_name == 'workflow_dispatch' && inputs.mode == 'master_candidate'");
    expect(baselineJob).toContain("global_index::coverage::tests");
    expect(baselineJob).toContain("smart_lean_ingestion_cost -- --ignored --nocapture");
    expect(baselineJob).toContain("smart-lean-before.json");
    expect(baselineJob).toContain("smart-lean-after.json");
    expect(baselineJob).toContain("Build Windows native master and candidate binaries");
    expect(baselineJob).toContain("Measure master direct MFT qualification on task-owned NTFS");
    expect(baselineJob).toContain("Measure candidate direct MFT qualification on task-owned NTFS");
    expect(baselineJob).toContain("-DirectOnly");
  });

  it("reports hosted macOS Spotlight capability independently", () => {
    const spotlightJob = workflow.split("\n  macos-spotlight-capability:")[1] ?? "";
    expect(spotlightJob).toContain("inputs.mode == 'macos_spotlight'");
    expect(spotlightJob).toContain("UNVERIFIED");
    expect(spotlightJob).toContain("Synthetic writer results are reported in the separate writer-pair job");
  });

  it("mounts the owned NTFS volume before resolving its fixture path as a PowerShell drive", () => {
    const mount = filenameFirstQualification.indexOf('"assign letter=$driveLetter"');
    const volumeVisible = filenameFirstQualification.indexOf("Get-CimInstance Win32_LogicalDisk");
    const fixturePath = filenameFirstQualification.indexOf("$script:fixtureRoot = Join-Path $driveRoot");
    const fixtureCreate = filenameFirstQualification.indexOf("New-Item -ItemType Directory -Path $script:fixtureRoot");
    expect(mount).toBeGreaterThanOrEqual(0);
    expect(volumeVisible).toBeGreaterThan(mount);
    expect(fixturePath).toBeGreaterThan(volumeVisible);
    expect(fixtureCreate).toBeGreaterThan(fixturePath);
  });

  it("requires MFT and writer timings only for initial-index measurements", () => {
    expect(filenameFirstQualification).toContain("if (-not $run.stages.mft -or -not $run.stages.writer)");
    expect(filenameFirstQualification).toContain("initial run $repetition omitted MFT or repository stage timings");
    expect(filenameFirstQualification).not.toContain("$Label omitted MFT or repository stage timings");
  });

  it("lets a repeated native pair reuse prior trigger attribution", () => {
    expect(workflow).toContain("run_trigger_matrix:");
    expect(workflow).toContain("github.event_name == 'workflow_dispatch' && inputs.run_trigger_matrix");
    expect(workflow).toContain("github.event_name == 'push' && contains(github.event.head_commit.message, '[run-global-index-filename-first]')");
    expect(workflow).toContain("filename_file_count:");
    expect(workflow).toContain("filename_persist_batch_size:");
    expect(workflow).toContain("options: ['auto', '512', '1024', '2048', '4096', '8192']");
    expect(workflow).toContain('default: \'auto\'');
    expect(workflow).toContain("inputs.filename_file_count != '1000000'");
  });

  it("compares the filename-first path against SQLite using the same Release binary", () => {
    expect(workflow).toContain("Build one Release QA executable for both feature modes");
    expect(workflow).toContain("-BaselineProbeExe $probe -CandidateProbeExe $probe");
    expect(filenameFirstQualification).toContain("same_release_binary_sqlite_8192_vs_filename_first_catalog_$($script:candidatePersistBatch)");
    expect(workflow).toContain("same Release binary and same task-owned NTFS data; baseline maintains FTS in synchronous 8192-row repository batches, candidate exposes the completed transient filename catalog then persists with $env:FILENAME_FIRST_PERSIST_BATCH_SIZE transaction sizing");
    expect(workflow).toContain('Production auto sizing uses 4096 rows below 250k entries and 8192 rows at or above 250k.');
    expect(workflow).toContain('-CandidatePersistBatch $env:FILENAME_FIRST_PERSIST_BATCH_SIZE');
    expect(workflow).toContain("Existing-source and incremental FTS triggers stay enabled; no global rebuild runs.");
    expect(workflow).toContain("fills only its new-source FTS in restartable batches of at most 8192 rows");
    expect(filenameFirstQualification).toContain("$mode = if ($side -eq 'baseline') { '0' } else { '1' }");
    expect(filenameFirstQualification).toContain("$run.result.filenameFirstEnabled -ne $expectedFilenameFirstMode");
    expect(filenameFirstQualification).toContain("ZC_GLOBAL_INDEX_MFT_EMIT_BATCH'] = '8192'");
    expect(filenameFirstQualification).toContain("$script:candidatePersistBatch -eq 'auto'");
    expect(filenameFirstQualification).toContain("ZC_GLOBAL_INDEX_MFT_PERSIST_BATCH'] = $script:candidatePersistBatch");
    expect(filenameFirstQualification).toContain("$searchReadyTargetSeconds = if ($FileCount -ge 1000000) { 20 } else { 10 }");
    expect(filenameFirstQualification).toContain("$script:evidence.durableFilenameIndexTargetMet = $candidateDurableFilenameIndex -le $durableTargetMicroseconds -and $allCandidateDurableFilenameTargetsMet");
    expect(filenameFirstQualification).toContain("$script:evidence.completeFilenameSearchTargetMet = $candidateCompleteFilenameSearchable -le $searchReadyTargetMicroseconds -and $allCandidateCompleteCatalogsReady");
    expect(filenameFirstQualification).toContain("$normalWriteP95LimitUs = 1000000");
    expect(filenameFirstQualification).toContain("$script:evidence.concurrentWritesTarget = [ordered]@");
    expect(filenameFirstQualification).toContain("completeTransientSearchBeforeDurableFts");
    expect(filenameFirstQualification).toContain("collectionCompleteWhenTransientReady");
    expect(filenameFirstQualification).toContain("fullSourceCompletionTargetMet = $script:evidence.fullIndexTargetMet");
    expect(filenameFirstQualification).toContain("'result.durableFilenameIndexUs'");
    expect(filenameFirstQualification).toContain("sourceIndexStatus -ne 'ready' -or $run.result.coveragePending");
    expect(mft).toContain("let batch_size = mft_emit_batch_size();");
    expect(mft).toContain("fn mft_emit_batch_size_from(value: Option<&str>) -> usize");
  });

  it("reads native pair timing fields from the QA result and prints durable evidence", () => {
    expect(filenameFirstQualification).toContain("$baselineFirst = Get-Average $baselineRows.ToArray() 'result.firstSearchableUs'");
    expect(filenameFirstQualification).toContain("$candidateFull = Get-Average $candidateRows.ToArray() 'result.fullIndexUs'");
    expect(filenameFirstQualification).toContain("stages.writer.fts_bulk_build_us");
    expect(filenameFirstQualification).toContain("ftsIntegrityVerified");
    expect(workflow).toContain("per-source restartable FTS bulk build baseline/candidate:");
    expect(workflow).toContain("Average peak working set baseline/candidate:");
    expect(filenameFirstQualification).toContain("stages.writer.transient_staging_us");
    expect(workflow).toContain("Transient catalog staging baseline/candidate:");
    expect(workflow).toContain("Average complete filename search baseline/candidate:");
    expect(workflow).toContain("Peak DB+WAL during indexing baseline/candidate:");
    expect(workflow).toContain("Write-Output $summary");
  });
});
