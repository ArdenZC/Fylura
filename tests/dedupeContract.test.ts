import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(path, "utf8");

const DEDUPE_SCHEMA_MINIMUM_VERSION = 29;
const ANALYSIS_SCHEMA_MINIMUM_VERSION = 30;

const readSchemaVersionAtLeast = (
  schema: string,
  minimum: number,
  feature: string
): number => {
  const declarations = [
    ...schema.matchAll(
      /^[ \t]*pub\(crate\)[ \t]+const[ \t]+CURRENT_SCHEMA_VERSION:[ \t]*i32[ \t]*=[ \t]*([^;\r\n]*)[ \t]*;[ \t]*$/gm
    )
  ];
  if (declarations.length !== 1) {
    throw new Error("Expected exactly one CURRENT_SCHEMA_VERSION i32 declaration");
  }

  const literal = declarations[0]?.[1];
  if (literal === undefined || !/^[+-]?\d(?:_?\d)*$/.test(literal)) {
    throw new Error("CURRENT_SCHEMA_VERSION must be an integer literal");
  }

  const version = Number(literal.replaceAll("_", ""));
  if (
    !Number.isSafeInteger(version) ||
    version < -2_147_483_648 ||
    version > 2_147_483_647
  ) {
    throw new Error("CURRENT_SCHEMA_VERSION must fit in an i32");
  }
  if (version < minimum) {
    throw new Error(`${feature} requires schema version ${minimum} or later`);
  }

  return version;
};

describe("Task 02 durable dedupe contract", () => {
  it("keeps the durable schema authoritative for fingerprints and duplicate membership", () => {
    const schema = read("src-tauri/src/db/schema.rs");
    const queries = read("src-tauri/src/db/queries/dedupe/mod.rs");

    expect(
      readSchemaVersionAtLeast(schema, DEDUPE_SCHEMA_MINIMUM_VERSION, "Dedupe")
    ).toBeGreaterThanOrEqual(DEDUPE_SCHEMA_MINIMUM_VERSION);
    expect(schema).toContain("CREATE TABLE IF NOT EXISTS file_fingerprints");
    expect(schema).toContain("CREATE TABLE IF NOT EXISTS dedupe_runs");
    expect(schema).toContain("CREATE TABLE IF NOT EXISTS duplicate_groups");
    expect(schema).toContain("CREATE TABLE IF NOT EXISTS duplicate_group_members");
    expect(schema).toContain("CREATE VIEW active_duplicate_membership AS");
    expect(schema).toContain("idx_dedupe_runs_one_active_scope");
    expect(queries).toContain("request_attempt");
    expect(queries).toContain("retry_dedupe_run");
    expect(queries).toContain("scope_snapshot_hash");
    expect(queries).toContain("candidate_sizes");
    expect(queries).toContain("parse_group_cursor");
  });

  it("requires identity checks and bounded hashing before group publication", () => {
    const dedupe = read("src-tauri/src/dedupe.rs");
    const queries = read("src-tauri/src/db/queries/dedupe/mod.rs");
    const physical = read("src-tauri/src/fs_safety/physical.rs");

    expect(dedupe).toContain("sync_channel");
    expect(dedupe).toContain("file_changed_before_hash");
    expect(dedupe).toContain("file_changed_during_hash");
    expect(dedupe).toContain("expected_identity");
    expect(dedupe).toContain("hash_subject_with_identity");
    expect(dedupe).toContain("publish_dedupe_groups");
    expect(queries).toContain("scope_snapshot_hash");
    expect(physical).toContain("capture_physical_identity");
    expect(physical).toContain("UnsupportedLink");
    expect(physical).toContain("PhysicalFileIdentity");
  });

  it("hydrates durable state and rejects stale or gapped renderer events", () => {
    const store = read("src/store/useDedupeStore.ts");
    const api = read("src/api/dedupeApi.ts");

    expect(store).toContain("listDedupeRuns(50)");
    expect(store).toContain("getActiveDedupeRun()");
    expect(store).toContain("next.revision < known.revision");
    expect(store).toContain("next.revision > known.revision + 1");
    expect(store).toContain("void current.hydrate()");
    expect(api).toContain('invokeCommand<DedupeRun>("start_dedupe_run"');
    expect(api).toContain('invokeCommand<DedupeRun>("retry_dedupe_run"');
    expect(api).toContain('invokeCommand<DedupeGroupPage>("list_duplicate_groups"');
    expect(api).toContain('listenTo("dedupe-run-updated"');
  });

  it("keeps the duplicate panel read-only and exposes the command permission surface", () => {
    const panel = read("src/views/vault/components/DuplicateGroupsPanel.tsx");
    const matrix = read("docs/security/TAURI_COMMAND_PERMISSION_MATRIX.md");

    expect(panel).toContain("listDuplicateGroupMembers");
    expect(panel).toContain("revealInFolder");
    expect(panel).not.toContain("executeMoves");
    expect(panel).not.toContain("moveCleanupCandidates");
    for (const command of [
      "start_dedupe_run",
      "retry_dedupe_run",
      "cancel_dedupe_run",
      "get_dedupe_run",
      "list_dedupe_runs",
      "get_active_dedupe_run",
      "list_duplicate_groups",
      "get_duplicate_group",
      "list_duplicate_group_members",
      "get_file_duplicate_membership"
    ]) {
      expect(matrix).toContain(`| \`${command}\` |`);
    }
  });

  it("preserves the dedupe migration floor and accepts later schema versions", () => {
    const schemaWithVersion = (version: string) =>
      `pub(crate) const CURRENT_SCHEMA_VERSION: i32 = ${version};`;

    expect(
      readSchemaVersionAtLeast(schemaWithVersion("39"), DEDUPE_SCHEMA_MINIMUM_VERSION, "Dedupe")
    ).toBe(39);
    expect(() =>
      readSchemaVersionAtLeast(schemaWithVersion("28"), DEDUPE_SCHEMA_MINIMUM_VERSION, "Dedupe")
    ).toThrow(/schema version 29 or later/);
    expect(() => readSchemaVersionAtLeast("", DEDUPE_SCHEMA_MINIMUM_VERSION, "Dedupe")).toThrow(
      /exactly one/
    );
    expect(() =>
      readSchemaVersionAtLeast(
        `${schemaWithVersion("38")}\n${schemaWithVersion("39")}`,
        DEDUPE_SCHEMA_MINIMUM_VERSION,
        "Dedupe"
      )
    ).toThrow(/exactly one/);
    for (const invalid of ["not-a-number", "2147483648", "-2147483649"]) {
      expect(() =>
        readSchemaVersionAtLeast(schemaWithVersion(invalid), DEDUPE_SCHEMA_MINIMUM_VERSION, "Dedupe")
      ).toThrow();
    }
  });
});

describe("Task 03 durable analysis contract", () => {
  it("uses a fixed detector registry and durable run/finding ledger", () => {
    const schema = read("src-tauri/src/db/schema.rs");
    const analysis = read("src-tauri/src/analysis.rs");
    const queries = read("src-tauri/src/db/queries/analysis/mod.rs");

    expect(
      readSchemaVersionAtLeast(schema, ANALYSIS_SCHEMA_MINIMUM_VERSION, "Analysis")
    ).toBeGreaterThanOrEqual(ANALYSIS_SCHEMA_MINIMUM_VERSION);
    for (const table of [
      "analysis_runs",
      "analysis_run_detectors",
      "analysis_findings",
      "analysis_finding_evidence",
      "analysis_finding_decisions",
      "dedupe_authority_state"
    ]) {
      expect(schema).toContain(`CREATE TABLE IF NOT EXISTS ${table}`);
    }
    expect(schema).toContain("ON DELETE SET NULL");
    for (const detector of [
      "duplicate_reclaimable_v1",
      "large_file_v1",
      "large_directory_v1",
      "cleanup_heuristics_v1"
    ]) {
      expect(analysis).toContain(detector);
    }
    expect(analysis).toContain("resolve_detector_ids");
    expect(analysis).toContain("stage_analysis_findings");
    expect(queries).toContain("publish_analysis_run");
    expect(queries).toContain("source_changed_during_run");
    expect(queries).toContain("deterministic_finding_id");
    expect(queries).toContain("analysis_finding_decisions");
  });

  it("keeps analysis findings advisory and preserves the existing mutation boundaries", () => {
    const analysis = read("src-tauri/src/analysis.rs");
    const cleanup = read("src-tauri/src/storage_analyzer.rs");
    const aiCleanup = read("src-tauri/src/ai/cleanup.rs");
    const aiCleanupPublication = read("src-tauri/src/ai/cleanup/publication.rs");
    const api = read("src/api/analysisApi.ts");

    expect(analysis).toContain("becomes an authority for a run");
    expect(analysis).not.toContain("std::process::Command");
    expect(analysis).not.toContain("execute_moves");
    expect(cleanup).toContain("resolve_analysis_candidates_for_cleanup");
    expect(cleanup).toContain("Storage cleanup finding identity changed");
    expect(cleanup).toContain("move_cleanup_candidates_to_safe_trash");
    expect(aiCleanupPublication).toContain("publish_analysis_ai_assessments_cas");
    expect(aiCleanup).not.toContain("execute_moves");
    expect(aiCleanupPublication).not.toContain("execute_moves");
    expect(api).toContain("listAnalysisFindings");
    expect(api).toContain("setAnalysisFindingDecision");
  });

  it("requires durable identity dispatch, detector-owned review contracts, and full selection CAS", () => {
    const analysis = read("src-tauri/src/analysis.rs");
    const queries = read("src-tauri/src/db/queries/analysis/mod.rs");
    const cleanup = read("src-tauri/src/storage_analyzer.rs");
    const aiCleanup = read("src-tauri/src/ai/cleanup.rs");
    const api = read("src/api/analysisApi.ts");
    const cleanupApi = read("src/api/cleanupApi.ts");

    expect(analysis).toContain('"managed_file" | "file"');
    expect(analysis).toContain('"duplicate_group" => duplicate_group_identity_matches');
    expect(analysis).toContain('"directory" => directory_identity_matches');
    expect(analysis).toContain('"approved_path" => approved_path_identity_matches');
    expect(analysis).toContain('"detectorContract": "review_reveal"');
    expect(analysis).toContain("CLEANUP_HEURISTICS_DETECTOR.to_string()");
    expect(queries).toContain("ANALYSIS_PRUNE_ROW_BUDGET: usize = 1000");
    expect(queries).toContain("refresh_analysis_run_aggregate_tx");
    expect(queries).toContain("revision = revision + 1");
    expect(cleanup).toContain("expected_revision: i64");
    expect(cleanup).toContain("ReviewFindingConfirmation");
    expect(cleanup).toContain("authorize_cleanup_candidate");
    expect(cleanup).not.toContain("move_path_to_system_trash_with_safety");
    expect(aiCleanup).toContain("ANALYSIS_RUN_UPDATED_EVENT");
    expect(api).toContain("expectedRevision: number");
    expect(cleanupApi).toContain("selections: CleanupFindingSelection[]");
  });

  it("hydrates cleanup from durable revisions and starts only Rule-free durable analysis runs", () => {
    const analysisApi = read("src/api/analysisApi.ts");
    const view = [
      "src/views/cleanup/StorageCleanupView.tsx",
      "src/views/cleanup/useCleanupAnalysisController.ts"
    ].map(read).join("\n");

    expect(analysisApi).toContain("getActiveAnalysisRun");
    expect(analysisApi).toContain("listAnalysisRuns");
    expect(analysisApi).toContain("getAnalysisRun");
    expect(view).toContain("onAnalysisRunUpdated");
    expect(view).toContain("onAnalysisFindingsPublished");
    expect(view).toContain("runDetectors");
    expect(view).toContain("nextDetectors");
    expect(view).toContain("loadRunDetails(candidate.id)");
    expect(view).toContain("startAnalysisRun");
    expect(view).toContain('scope: { kind: "approvedCleanupPaths"');
    expect(view).not.toContain("useStorageCleanupStore");
    expect(view).not.toContain("StorageAnalysis");
    expect(view).toContain("setAnalysisFindingDecision");
  });

  it("preserves the analysis migration floor and accepts later schema versions", () => {
    const schemaWithVersion = (version: string) =>
      `pub(crate) const CURRENT_SCHEMA_VERSION: i32 = ${version};`;

    expect(
      readSchemaVersionAtLeast(schemaWithVersion("39"), ANALYSIS_SCHEMA_MINIMUM_VERSION, "Analysis")
    ).toBe(39);
    expect(() =>
      readSchemaVersionAtLeast(schemaWithVersion("29"), ANALYSIS_SCHEMA_MINIMUM_VERSION, "Analysis")
    ).toThrow(/schema version 30 or later/);
    expect(() => readSchemaVersionAtLeast("", ANALYSIS_SCHEMA_MINIMUM_VERSION, "Analysis")).toThrow(
      /exactly one/
    );
    expect(() =>
      readSchemaVersionAtLeast(
        `${schemaWithVersion("38")}\n${schemaWithVersion("39")}`,
        ANALYSIS_SCHEMA_MINIMUM_VERSION,
        "Analysis"
      )
    ).toThrow(/exactly one/);
    for (const invalid of ["not-a-number", "2147483648", "-2147483649"]) {
      expect(() =>
        readSchemaVersionAtLeast(schemaWithVersion(invalid), ANALYSIS_SCHEMA_MINIMUM_VERSION, "Analysis")
      ).toThrow();
    }
  });
});
