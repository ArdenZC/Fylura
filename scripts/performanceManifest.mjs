const CARGO_TEST_ARGS = ["--release", "--locked", "--manifest-path", "src-tauri/Cargo.toml"];

export const PERFORMANCE_BUILD_FEATURES = "performance-test-tauri";

export const PERFORMANCE_TARGETS = Object.freeze({
  lib: Object.freeze({
    id: "lib",
    cargoArgs: ["--lib", "--features", PERFORMANCE_BUILD_FEATURES],
    executableStem: "zen_canvas_tauri",
    shardTarget: true,
  }),
  fts: Object.freeze({
    id: "fts",
    cargoArgs: ["--test", "fts_benchmark"],
    executableStem: "fts_benchmark",
    shardTarget: true,
  }),
  migrations: Object.freeze({
    id: "migrations",
    cargoArgs: ["--test", "migrations"],
    executableStem: "migrations",
    shardTarget: true,
  }),
  fileLibrary: Object.freeze({
    id: "file-library-performance",
    cargoArgs: ["--test", "file_library_performance"],
    executableStem: "file_library_performance",
    shardTarget: true,
  }),
  fixtureBuilder: Object.freeze({
    id: "performance-fixture-builder",
    cargoArgs: ["--test", "performance_fixture_builder"],
    executableStem: "performance_fixture_builder",
    shardTarget: false,
  }),
});

// Phase A keeps the frozen W3 targets and fixture vocabulary in the existing
// performance manifest. Rust/browser helpers consume the same logical IDs;
// this is measurement metadata, not a second benchmark authority.
export const PREVIEW_PERFORMANCE_CONTRACT = Object.freeze({
  metricDefinition: "w3-10-phase-a-v1",
  phaseBMetricDefinition: "w3-10-phase-b-v1",
  fixtureManifest: "w3-10-preview-fixtures-v1",
  shellFirstVisibleTargetP95Ms: 100,
  usefulRepresentationTargetP95Ms: 300,
  nativeUsefulRepresentationTargetP95Ms: 1000,
  rapidSwitchEntries: 100,
  warmupSamples: 3,
  timingSamples: 20,
});

export const PREVIEW_FIXTURES = Object.freeze([
  ["text-normal", "preview-text.txt", "builtin.text", "text", "normal"],
  ["source-normal", "preview-source.rs", "builtin.source-code", "text", "normal"],
  ["markdown-normal", "preview-markdown.md", "builtin.markdown", "safe_html", "normal"],
  ["json-normal", "preview-structured.json", "builtin.structured-json", "structured_tree", "normal"],
  ["yaml-normal", "preview-config.yaml", "builtin.structured-yaml", "structured_tree", "normal"],
  ["xml-normal", "preview-markup.xml", "builtin.structured-xml", "structured_tree", "normal"],
  ["csv-normal", "preview-records.csv", "builtin.table-csv", "table", "normal"],
  ["tsv-normal", "preview-records.tsv", "builtin.table-tsv", "table", "normal"],
  ["png-normal", "preview-image.png", "builtin.image", "image", "normal"],
  ["jpeg-normal", "preview-image.jpg", "builtin.image", "image", "normal"],
  ["folder-normal", "preview-folder", "builtin.folder", "folder_summary", "normal"],
  ["archive-normal", "preview-archive.zip", "builtin.archive-zip", "archive_tree", "normal"],
  ["text-large-bounded", "preview-large.txt", "builtin.text", "text", "large-bounded"],
  ["malformed-json", "preview-malformed.json", "metadata-fallback", "metadata", "corrupt-malformed"],
  ["corrupt-image", "preview-corrupt.png", "metadata-fallback", "metadata", "corrupt-malformed"],
  ["unavailable-source", "preview-unavailable.txt", "terminal-source", "metadata", "permission-unavailable"],
  ["cancel-during-load", "preview-cancel.txt", "builtin.text", "text", "cancel"],
].map(([id, fileName, providerId, representationFamily, fixtureClass]) => Object.freeze({
  id,
  fileName,
  providerId,
  representationFamily,
  fixtureClass,
})));

function benchmark({
  id,
  label,
  targetKey,
  targetArgs,
  testName,
  ignored = true,
  testThreads,
  env = {},
}) {
  return Object.freeze({
    id,
    label,
    targetKey,
    targetArgs: Object.freeze([...targetArgs]),
    testName,
    ignored,
    testThreads,
    env: Object.freeze({ ...env }),
  });
}

function precompile(targetKey) {
  const target = PERFORMANCE_TARGETS[targetKey];
  if (!target) throw new Error(`Unsupported performance target: ${targetKey}`);
  return Object.freeze({
    id: target.id,
    targetKey,
    targetArgs: Object.freeze([...target.cargoArgs]),
  });
}

export const PERFORMANCE_SUITES = Object.freeze({
  search: Object.freeze({
    label: "Performance / Search",
    precompile: Object.freeze([
      precompile("lib"),
      precompile("fts"),
    ]),
    extended: Object.freeze([
      benchmark({
        id: "fts_100k",
        label: "SQLite/FTS 100k",
        targetKey: "fts",
        targetArgs: PERFORMANCE_TARGETS.fts.cargoArgs,
        testName: "fts_benchmark_100k",
      }),
      benchmark({
        id: "global_search_100k",
        label: "Global Search 100k",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "global_index::tests::global_search_performance_100k_synthetic_entries",
      }),
    ]),
    fullOnly: Object.freeze([
      benchmark({
        id: "global_search_1m",
        label: "Global Search 1M",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "global_index::tests::global_search_performance_one_million_synthetic_entries",
      }),
    ]),
    fixtureKeys: Object.freeze([]),
  }),
  "scan-schema": Object.freeze({
    label: "Performance / Scan & Schema",
    precompile: Object.freeze([
      precompile("lib"),
      precompile("migrations"),
    ]),
    extended: Object.freeze([
      benchmark({
        id: "scan_100k",
        label: "Managed Scan 100k",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::queries::scan::tests::performance_100k_scan_seen_missing_reconcile_and_wal_reader",
      }),
      benchmark({
        id: "schema_28_to_29_100k",
        label: "Schema 28->29 100k migration",
        targetKey: "migrations",
        targetArgs: PERFORMANCE_TARGETS.migrations.cargoArgs,
        testName: "performance_100k_files_schema_28_to_29_and_wal_reader",
      }),
      benchmark({
        id: "schema_29_to_30_100k",
        label: "Schema 29->30 100k migration",
        targetKey: "migrations",
        targetArgs: PERFORMANCE_TARGETS.migrations.cargoArgs,
        testName: "performance_100k_files_schema_29_to_30_analysis_and_wal_reader",
      }),
    ]),
    fullOnly: Object.freeze([]),
    fixtureKeys: Object.freeze([]),
  }),
  "library-content": Object.freeze({
    label: "Performance / Library & Content",
    precompile: Object.freeze([
      precompile("fileLibrary"),
      precompile("fixtureBuilder"),
    ]),
    extended: Object.freeze([
      benchmark({
        id: "file_library_100k",
        label: "File Library 100k query matrix",
        targetKey: "fileLibrary",
        targetArgs: PERFORMANCE_TARGETS.fileLibrary.cargoArgs,
        testName: "performance_100k_file_library_query_matrix",
      }),
      benchmark({
        id: "file_library_migration_100k",
        label: "File Library 100k migration",
        targetKey: "fileLibrary",
        targetArgs: PERFORMANCE_TARGETS.fileLibrary.cargoArgs,
        testName: "performance_100k_schema_30_to_31_file_library_migration",
      }),
      benchmark({
        id: "content_migration_100k",
        label: "Content migration 100k",
        targetKey: "fileLibrary",
        targetArgs: PERFORMANCE_TARGETS.fileLibrary.cargoArgs,
        testName: "performance_100k_schema_32_to_33_rule_proposal_migration",
      }),
    ]),
    fullOnly: Object.freeze([
      benchmark({
        id: "file_library_1m",
        label: "File Library 1M query matrix",
        targetKey: "fileLibrary",
        targetArgs: PERFORMANCE_TARGETS.fileLibrary.cargoArgs,
        testName: "performance_1m_file_library_query_matrix",
      }),
      benchmark({
        id: "file_library_migration_1m",
        label: "File Library 1M migration",
        targetKey: "fileLibrary",
        targetArgs: PERFORMANCE_TARGETS.fileLibrary.cargoArgs,
        testName: "performance_1m_schema_30_to_31_file_library_migration",
      }),
      benchmark({
        id: "content_migration_1m",
        label: "Content migration 1M",
        targetKey: "fileLibrary",
        targetArgs: PERFORMANCE_TARGETS.fileLibrary.cargoArgs,
        testName: "performance_1m_schema_32_to_33_rule_proposal_migration",
      }),
    ]),
    fixtureKeys: Object.freeze(["file-library-100k", "file-library-1m"]),
  }),
  intelligence: Object.freeze({
    label: "Performance / Intelligence",
    precompile: Object.freeze([precompile("lib")]),
    extended: Object.freeze([
      benchmark({
        id: "analysis_100k",
        label: "Analysis 100k",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::tests::performance_task03_analysis_100k_findings_and_wal_reader",
      }),
      benchmark({
        id: "analysis_publication_10k",
        label: "Analysis publication 10k",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::tests::performance_task03_10k_finding_publication_transaction",
      }),
      benchmark({
        id: "analysis_prune",
        label: "Analysis prune",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::tests::analysis_prune_uses_one_global_child_first_row_budget_and_wal_reader",
        ignored: false,
      }),
      benchmark({
        id: "dedupe_repository_100k",
        label: "Dedupe repository 100k",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::queries::dedupe::tests::performance_task02_repository_100k_and_group_pages",
      }),
      benchmark({
        id: "dedupe_hash_io_bounded",
        label: "Dedupe bounded hash IO",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "dedupe::job_manager_tests::performance_task02_hash_io_1000x16mib_1_worker_and_default_workers",
        env: { ZC_TASK02_IO_FILES: "16", ZC_TASK02_IO_BYTES: "1048576" },
      }),
      benchmark({
        id: "organization_100_1k_10k",
        label: "Organization Plan 100/1k/10k",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::queries::organization::tests::performance_task06_plan_100_1k_10k_repository",
      }),
      benchmark({
        id: "rule_proposal_100k",
        label: "Rule Proposal 100k impact",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::queries::rule_proposals::tests::performance_task07_rule_proposal_repository_and_impact",
        testThreads: 1,
      }),
    ]),
    fullOnly: Object.freeze([
      benchmark({
        id: "rule_proposal_1m",
        label: "Rule Proposal 1M impact",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "performance_task07_rule_proposal_repository_and_impact",
        testThreads: 1,
      }),
    ]),
    fixtureKeys: Object.freeze([]),
  }),
  "workspace-foundation": Object.freeze({
    label: "Performance / Workspace Foundation",
    // File Workspace performance tests live in the library test binary so they
    // can exercise the real process-local BrowseService/Runtime ownership
    // without creating a second Cargo target or a second runtime authority.
    precompile: Object.freeze([precompile("lib")]),
    extended: Object.freeze([
      benchmark({
        id: "workspace_foundation_harness_smoke",
        label: "File Workspace/Foundation harness smoke",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::harness::harness_smoke",
      }),
      benchmark({
        id: "workspace_foundation_browse_100k",
        label: "File Workspace Browse 100k",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::browse::browse_100k_progressive_bounded_ownership",
      }),
      benchmark({
        id: "workspace_foundation_browse_session_capacity",
        label: "File Workspace Browse session capacity",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::browse::browse_session_capacity_remains_bounded",
      }),
      benchmark({
        id: "workspace_foundation_scheduler_pressure",
        label: "File Workspace managed-scan scheduler pressure",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::scheduler::managed_scan_pressure_preserves_foreground_browse_and_releases",
      }),
      benchmark({
        id: "workspace_foundation_browse_stage_observation_contract",
        label: "Browse stage observation scope and percentile contract",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::scheduler::browse_stage_observation_scope_and_percentile_contract",
        ignored: false,
        testThreads: 1,
      }),
      benchmark({
        id: "workspace_foundation_sqlite_admission_wait",
        label: "Issue 345 managed-scan admission waits for writer release",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::queries::scan::tests::managed_scan_admission_waits_for_a_short_real_writer_lock",
        ignored: true,
        testThreads: 1,
      }),
      benchmark({
        id: "workspace_foundation_sqlite_admission_timeout",
        label: "Issue 345 managed-scan admission fails closed at busy timeout",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::queries::scan::tests::managed_scan_admission_fails_closed_after_busy_timeout_without_partial_authority",
        ignored: true,
        testThreads: 1,
      }),
      benchmark({
        id: "workspace_foundation_sqlite_batch_busy",
        label: "Issue 345 managed-scan batch write is atomic under writer contention",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "db::queries::scan::tests::managed_scan_batch_write_returns_busy_atomically_and_succeeds_after_release",
        ignored: true,
        testThreads: 1,
      }),
      benchmark({
        id: "workspace_foundation_scan_writer_contention",
        label: "Issue 345 real managed-scan SQLite writer overlap",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "scanner::tests::issue345_concurrent_real_managed_scan_writers_hold_and_release_authority",
        testThreads: 1,
      }),
      benchmark({
        id: "workspace_foundation_resource_steady_state",
        label: "File Workspace resource and registry steady state",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::steady_state::resource_and_registry_steady_state_after_browse_preview_switches",
      }),
      benchmark({
        id: "workspace_foundation_windows_private_usage_detector",
        label: "File Workspace Windows PrivateUsage detector correctness",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::steady_state::windows_private_usage_detector_catches_sustained_retention",
      }),
    ]),
    fullOnly: Object.freeze([]),
    fixtureKeys: Object.freeze([]),
  }),
  "preview-platform": Object.freeze({
    label: "Performance / Preview Platform",
    // Preview tests stay in the existing lib test binary so they exercise the
    // real FileWorkspaceRuntime, Read Gate, PreviewSession and global
    // WorkScheduler ownership used by Workspace Foundation.
    precompile: Object.freeze([precompile("lib")]),
    extended: Object.freeze([
      benchmark({
        id: "preview_shell_first_visible",
        label: "Preview shell first-visible measurement preparation",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_shell_first_visible",
      }),
      benchmark({
        id: "preview_provider_useful_representation",
        label: "Preview provider useful representation",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_provider_useful_representation",
      }),
      benchmark({
        id: "preview_folder_scale",
        label: "Preview Folder 1k/10k/100k/>100k scale",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_folder_scale",
      }),
      benchmark({
        id: "preview_zip_scale",
        label: "Preview ZIP bounded metadata scale/security",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_zip_scale",
      }),
      benchmark({
        id: "preview_close_mutate_open_hard_gate",
        label: "Preview close-to-mutate/open filesystem lifecycle gate",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_close_mutate_open_hard_gate",
      }),
      benchmark({
        id: "preview_rapid_switch_100",
        label: "Preview 100-entry rapid switch runtime evidence",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_rapid_switch_100",
      }),
      benchmark({
        id: "preview_rapid_switch_100_mixed_provider_families",
        label: "Preview 100-entry mixed-provider rapid switch runtime evidence",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_rapid_switch_100_mixed_provider_families",
      }),
      benchmark({
        id: "preview_rapid_switch_100_deferred_correctness",
        label: "Preview 100-entry deferred latest-wins correctness",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_rapid_switch_100_deferred_correctness",
        ignored: false,
      }),
      benchmark({
        id: "preview_resource_steady_state",
        label: "Preview repeated-cycle resource steady state",
        targetKey: "lib",
        targetArgs: PERFORMANCE_TARGETS.lib.cargoArgs,
        testName: "file_workspace::integration::performance::preview::preview_repeated_cycle_steady_state",
      }),
    ]),
    fullOnly: Object.freeze([]),
    fixtureKeys: Object.freeze([]),
    previewFixtures: PREVIEW_FIXTURES,
  }),
});

export const PERFORMANCE_SUITE_NAMES = Object.freeze(Object.keys(PERFORMANCE_SUITES));

export function resolvePerformanceSuite(argv = []) {
  const suiteArguments = argv.filter(
    (argument) => argument === "--suite" || argument.startsWith("--suite="),
  );
  if (suiteArguments.length !== 1 || suiteArguments[0] === "--suite") {
    throw new Error("Specify exactly one performance suite with --suite=<name>.");
  }
  const suite = suiteArguments[0].slice("--suite=".length);
  if (!Object.hasOwn(PERFORMANCE_SUITES, suite)) {
    throw new Error(`Unsupported performance suite: ${suite}`);
  }
  return suite;
}

export function getPerformanceBenchmarks(suiteName, profile) {
  const suite = PERFORMANCE_SUITES[suiteName];
  if (!suite) throw new Error(`Unsupported performance suite: ${suiteName}`);
  if (profile !== "full" && profile !== "extended") {
    throw new Error(`Unsupported performance profile: ${profile}`);
  }
  return profile === "full" ? [...suite.extended, ...suite.fullOnly] : [...suite.extended];
}

export function getPrecompileTargets(suiteName) {
  const suite = PERFORMANCE_SUITES[suiteName];
  if (!suite) throw new Error(`Unsupported performance suite: ${suiteName}`);
  return [...suite.precompile];
}

export function getPrecompileTargetsForSuites(suiteNames) {
  const seen = new Set();
  const targets = [];
  for (const suiteName of suiteNames) {
    for (const target of getPrecompileTargets(suiteName)) {
      if (seen.has(target.targetKey)) continue;
      seen.add(target.targetKey);
      targets.push(target);
    }
  }
  return targets;
}

export function getRequiredBinaryKeys(suiteName) {
  const suite = PERFORMANCE_SUITES[suiteName];
  if (!suite) throw new Error(`Unsupported performance suite: ${suiteName}`);
  return [...new Set(
    [...suite.extended, ...suite.fullOnly].map((item) => item.targetKey),
  )];
}

export function getFixtureWorkingFiles(suiteName, profile) {
  const suite = PERFORMANCE_SUITES[suiteName];
  if (!suite) throw new Error(`Unsupported performance suite: ${suiteName}`);
  if (profile !== "full" && profile !== "extended") {
    throw new Error(`Unsupported performance profile: ${profile}`);
  }
  if (suiteName !== "library-content") return [];
  const rows = profile === "full" ? [100_000, 1_000_000] : [100_000];
  return rows.flatMap((rowCount) => [
    `file-library-${rowCount}-query.sqlite3`,
    `file-library-${rowCount}-library-migration.sqlite3`,
    `file-library-${rowCount}-content-migration.sqlite3`,
  ]);
}

export function getFixtureWorkingFilesForSuites(suiteNames, profile) {
  return [...new Set(suiteNames.flatMap((suiteName) => getFixtureWorkingFiles(suiteName, profile)))];
}

export function getFixtureKeys(suiteName, profile) {
  const suite = PERFORMANCE_SUITES[suiteName];
  if (!suite) throw new Error(`Unsupported performance suite: ${suiteName}`);
  if (profile === "extended") {
    return suite.fixtureKeys.filter((key) => !key.endsWith("-1m"));
  }
  if (profile === "full") return [...suite.fixtureKeys];
  throw new Error(`Unsupported performance profile: ${profile}`);
}

export { CARGO_TEST_ARGS };
