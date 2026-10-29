import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { describe, expect, it, vi } from "vitest";
import {
  createBinaryManifest,
  createFixtureManifest,
  readJson,
  sha256File,
  validateBinaryManifest,
  validateFixtureManifest,
  writeJson,
} from "../scripts/performanceArtifactManifest.mjs";
import {
  getFixtureWorkingFiles,
  getPerformanceBenchmarks,
  getPrecompileTargets,
  getPrecompileTargetsForSuites,
  getRequiredBinaryKeys,
  PERFORMANCE_BUILD_FEATURES,
  PREVIEW_FIXTURES,
  PREVIEW_PERFORMANCE_CONTRACT,
  PERFORMANCE_SUITE_NAMES,
  PERFORMANCE_SUITES,
} from "../scripts/performanceManifest.mjs";
import { buildPreparedTestArgs, runPreparedTestBinary } from "../scripts/runPreparedPerformanceBinary.mjs";
import { resolvePerformanceProfile } from "../scripts/performanceProfile.mjs";
import { createPerformanceBuildIdentity } from "../scripts/performanceBuildIdentity.mjs";
import { createPerformanceFixtureIdentity, PERFORMANCE_FIXTURE_SCHEMA_VERSION } from "../scripts/performanceFixtureIdentity.mjs";

const TEST_RUST_VERSION = "rustc test";

const WORKSPACE_BENCHMARK_TEST_NAMES = {
  workspace_foundation_harness_smoke:
    "file_workspace::integration::performance::harness::harness_smoke",
  workspace_foundation_browse_100k:
    "file_workspace::integration::performance::browse::browse_100k_progressive_bounded_ownership",
  workspace_foundation_browse_session_capacity:
    "file_workspace::integration::performance::browse::browse_session_capacity_remains_bounded",
  workspace_foundation_scheduler_pressure:
    "file_workspace::integration::performance::scheduler::managed_scan_pressure_preserves_foreground_browse_and_releases",
  workspace_foundation_browse_stage_observation_contract:
    "file_workspace::integration::performance::scheduler::browse_stage_observation_scope_and_percentile_contract",
  workspace_foundation_sqlite_admission_wait:
    "db::queries::scan::tests::managed_scan_admission_waits_for_a_short_real_writer_lock",
  workspace_foundation_sqlite_admission_timeout:
    "db::queries::scan::tests::managed_scan_admission_fails_closed_after_busy_timeout_without_partial_authority",
  workspace_foundation_sqlite_batch_busy:
    "db::queries::scan::tests::managed_scan_batch_write_returns_busy_atomically_and_succeeds_after_release",
  workspace_foundation_scan_writer_contention:
    "scanner::tests::issue345_concurrent_real_managed_scan_writers_hold_and_release_authority",
  workspace_foundation_resource_steady_state:
    "file_workspace::integration::performance::steady_state::resource_and_registry_steady_state_after_browse_preview_switches",
} as const;

const PREVIEW_BENCHMARK_TEST_NAMES = {
  preview_shell_first_visible:
    "file_workspace::integration::performance::preview::preview_shell_first_visible",
  preview_provider_useful_representation:
    "file_workspace::integration::performance::preview::preview_provider_useful_representation",
  preview_folder_scale:
    "file_workspace::integration::performance::preview::preview_folder_scale",
  preview_zip_scale:
    "file_workspace::integration::performance::preview::preview_zip_scale",
  preview_rapid_switch_100:
    "file_workspace::integration::performance::preview::preview_rapid_switch_100",
  preview_rapid_switch_100_mixed_provider_families:
    "file_workspace::integration::performance::preview::preview_rapid_switch_100_mixed_provider_families",
  preview_rapid_switch_100_deferred_correctness:
    "file_workspace::integration::performance::preview::preview_rapid_switch_100_deferred_correctness",
  preview_resource_steady_state:
    "file_workspace::integration::performance::preview::preview_repeated_cycle_steady_state",
} as const;

function read(relativePath: string) {
  return fs.readFileSync(path.join(process.cwd(), relativePath), "utf8");
}

function createTaskTempRoot(prefix: string) {
  const parent = path.join(process.cwd(), ".tmp-tests");
  fs.mkdirSync(parent, { recursive: true });
  return fs.mkdtempSync(path.join(parent, prefix));
}

function withFakePreparedBinary(check: (executable: string, marker: string) => void) {
  const root = createTaskTempRoot("prepared-test-list-");
  const executable = path.join(root, "fake libtest.cjs");
  const marker = path.join(root, "executed.txt");
  fs.writeFileSync(executable, `
const fs = require("node:fs");
const listing = process.argv.includes("--list");
const phase = listing ? "list" : "run";
if (process.env.TEST_TIMEOUT_PHASE === phase) {
  process.stdout.write(phase + " before timeout\\n");
  process.stderr.write(phase + " timeout stderr\\n");
  setInterval(() => {}, 1000);
} else if (listing) {
  const selected = !process.argv.includes("--ignored") || process.env.TEST_IGNORED !== "0";
  const count = selected ? Number(process.env.TEST_MATCHES ?? "1") : 0;
  for (let i = 0; i < count; i++) process.stdout.write(process.argv[1] + ": test\\r\\n");
  process.stdout.write("\\r\\n" + count + " tests, 0 benchmarks\\r\\n");
  process.stderr.write("listing stderr\\n");
  process.exit(Number(process.env.TEST_LIST_STATUS ?? "0"));
} else {
  fs.writeFileSync(process.env.TEST_RUN_MARKER, "executed");
  process.stdout.write("benchmark stdout\\n");
  process.stderr.write("benchmark stderr\\n");
  process.exit(Number(process.env.TEST_RUN_STATUS ?? "0"));
}
`);
  try {
    check(executable, marker);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

describe("performance profile and manifest contract", () => {
  it("supports the named suites and two profiles", () => {
    expect(PERFORMANCE_SUITE_NAMES).toEqual([
      "search",
      "scan-schema",
      "library-content",
      "intelligence",
      "workspace-foundation",
      "preview-platform",
    ]);
    expect(Object.keys(PERFORMANCE_SUITES)).toEqual([...PERFORMANCE_SUITE_NAMES]);
    expect(resolvePerformanceProfile([])).toBe("full");
    expect(resolvePerformanceProfile(["--profile=extended"])).toBe("extended");
    expect(() => resolvePerformanceProfile(["--profile=pr"])).toThrow("Unsupported performance profile: pr");
  });

  it("keeps one benchmark in exactly one suite and retains every 1M gate in Full", () => {
    const ids = new Set<string>();
    for (const suite of PERFORMANCE_SUITE_NAMES) {
      const extended = getPerformanceBenchmarks(suite, "extended") as ReadonlyArray<{ id: string }>;
      const full = getPerformanceBenchmarks(suite, "full") as ReadonlyArray<{ id: string }>;
      expect(extended.every((benchmark) => !benchmark.id.includes("1m"))).toBe(true);
      expect(new Set(full.map((benchmark) => benchmark.id)).size).toBe(full.length);
      for (const benchmark of extended) expect(full.some((item) => item.id === benchmark.id)).toBe(true);
      for (const benchmark of full) {
        expect(ids.has(benchmark.id)).toBe(false);
        ids.add(benchmark.id);
      }
      expect(getPrecompileTargets(suite).length).toBeGreaterThan(0);
      expect(getRequiredBinaryKeys(suite).length).toBeGreaterThan(0);
    }
    expect(ids.has("global_search_1m")).toBe(true);
    expect(ids.has("file_library_1m")).toBe(true);
    expect(ids.has("file_library_migration_1m")).toBe(true);
    expect(ids.has("content_migration_1m")).toBe(true);
    expect(ids.has("rule_proposal_1m")).toBe(true);
    expect(ids.has("workspace_foundation_harness_smoke")).toBe(true);
    expect(ids.has("workspace_foundation_browse_100k")).toBe(true);
    expect(ids.has("workspace_foundation_browse_session_capacity")).toBe(true);
    expect(ids.has("workspace_foundation_scheduler_pressure")).toBe(true);
    expect(ids.has("workspace_foundation_sqlite_admission_wait")).toBe(true);
    expect(ids.has("workspace_foundation_sqlite_admission_timeout")).toBe(true);
    expect(ids.has("workspace_foundation_sqlite_batch_busy")).toBe(true);
    expect(ids.has("workspace_foundation_scan_writer_contention")).toBe(true);
    expect(ids.has("workspace_foundation_resource_steady_state")).toBe(true);
    expect(ids.has("preview_shell_first_visible")).toBe(true);
    expect(ids.has("preview_provider_useful_representation")).toBe(true);
    expect(ids.has("preview_folder_scale")).toBe(true);
    expect(ids.has("preview_zip_scale")).toBe(true);
    expect(ids.has("preview_rapid_switch_100")).toBe(true);
    expect(ids.has("preview_rapid_switch_100_mixed_provider_families")).toBe(true);
    expect(ids.has("preview_rapid_switch_100_deferred_correctness")).toBe(true);
    expect(ids.has("preview_resource_steady_state")).toBe(true);
  });

  it("locks Workspace Foundation manifest IDs to exact Rust test names", () => {
    const benchmarks = getPerformanceBenchmarks("workspace-foundation", "full");
    const source = [
      read("src-tauri/src/file_workspace/integration/performance/harness.rs"),
      read("src-tauri/src/file_workspace/integration/performance/browse.rs"),
      read("src-tauri/src/file_workspace/integration/performance/scheduler.rs"),
      read("src-tauri/src/file_workspace/integration/performance/steady_state.rs"),
      read("src-tauri/src/db/queries/scan.rs"),
      read("src-tauri/src/scanner.rs"),
    ].join("\n");
    for (const [id, testName] of Object.entries(WORKSPACE_BENCHMARK_TEST_NAMES)) {
      expect(benchmarks.find((benchmark) => benchmark.id === id)?.testName).toBe(testName);
      expect(source).toContain(`fn ${testName.split("::").at(-1)}(`);
    }
    expect(benchmarks.find((benchmark) => benchmark.id === "workspace_foundation_browse_stage_observation_contract"))
      .toMatchObject({ targetKey: "lib", targetArgs: ["--lib", "--features", PERFORMANCE_BUILD_FEATURES], ignored: false, testThreads: 1 });
    for (const id of [
      "workspace_foundation_sqlite_admission_wait",
      "workspace_foundation_sqlite_admission_timeout",
      "workspace_foundation_sqlite_batch_busy",
    ]) {
      expect(benchmarks.find((benchmark) => benchmark.id === id)).toMatchObject({
        ignored: true,
        testThreads: 1,
      });
    }
  });

  it("maps Global Search 100k and Full 1M to the real ignored benchmark and explicit scales", () => {
    const testName = "global_index::tests::global_search_benchmark::global_search_synthetic_benchmark_baseline";
    const extended = getPerformanceBenchmarks("search", "extended");
    const full = getPerformanceBenchmarks("search", "full");
    expect(extended.find((item) => item.id === "global_search_100k")).toMatchObject({
      testName, ignored: true, targetKey: "lib",
      env: { ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES: "100000" },
    });
    expect(extended.some((item) => item.id === "global_search_1m")).toBe(false);
    expect(full.find((item) => item.id === "global_search_1m")).toMatchObject({
      testName, ignored: true, targetKey: "lib",
      env: { ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES: "1000000" },
    });
    const source = read("src-tauri/src/global_index/tests/global_search_benchmark.rs");
    expect(source).toContain("fn global_search_synthetic_benchmark_baseline()");
    expect(source).toContain('env::var("ZC_GLOBAL_SEARCH_BENCHMARK_ENTRIES")');
    expect(source).toContain("const WARM_QUERY_P95_LIMIT_MS: f64 = 100.0;");
  });

  it("locks Preview Phase A manifest IDs, targets and fixture vocabulary", () => {
    const benchmarks = getPerformanceBenchmarks("preview-platform", "full");
    const source = [
      read("src-tauri/src/file_workspace/integration/performance/mod.rs"),
      read("src-tauri/src/file_workspace/integration/performance/preview.rs"),
      read("src-tauri/src/file_workspace/integration/performance/fixture.rs"),
      read("src-tauri/src/file_workspace/integration/performance/metrics.rs"),
      read("scripts/runW3-10PhaseABrowserHarness.mjs"),
    ].join("\n");
    for (const [id, testName] of Object.entries(PREVIEW_BENCHMARK_TEST_NAMES)) {
      expect(benchmarks.find((benchmark) => benchmark.id === id)?.testName).toBe(testName);
      expect(source).toContain(`fn ${testName.split("::").at(-1)}(`);
    }
    expect(benchmarks.find((benchmark) => benchmark.id === "preview_rapid_switch_100_deferred_correctness")?.ignored)
      .toBe(false);
    expect(PREVIEW_PERFORMANCE_CONTRACT).toMatchObject({
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
    expect(PREVIEW_FIXTURES).toHaveLength(17);
    expect(PREVIEW_FIXTURES.find((fixture) => fixture.id === "png-normal")).toMatchObject({
      fileName: "preview-image.png",
      providerId: "builtin.image",
      representationFamily: "image",
    });
    expect(PREVIEW_FIXTURES.find((fixture) => fixture.id === "malformed-json")).toMatchObject({
      fixtureClass: "corrupt-malformed",
      representationFamily: "metadata",
    });
    for (const fixture of PREVIEW_FIXTURES) {
      expect(source).toContain(`id: "${fixture.id}"`);
      expect(source).toContain(`file_name: "${fixture.fileName}"`);
      expect(source).toContain(`provider_id: "${fixture.providerId}"`);
      expect(source).toContain(`representation_family: "${fixture.representationFamily}"`);
      expect(source).toContain(`fixture_class: "${fixture.fixtureClass}"`);
    }
    expect(source).toContain("PREVIEW_RAPID_SWITCH_ENTRIES");
    expect(source).toContain("timing_fields");
    expect(source).toContain("active_lease_count");
    expect(source).toContain("preview_assets.counts");
    expect(source).toContain("actualDomVisibilityMeasured");
    expect(source).toContain("getBoundingClientRect");
    expect(source).toContain("performance.now()");
    expect(source).toContain("summarizeBrowserTiming");
    expect(source).toContain("browser_accepted_command_to_visible_dom");
  });

  it("deduplicates shared Cargo targets in the single Prepare plan", () => {
    const targets = getPrecompileTargetsForSuites([...PERFORMANCE_SUITE_NAMES]);
    expect(targets.map((target) => target.targetKey)).toEqual([
      "lib",
      "fts",
      "migrations",
      "fileLibrary",
      "fixtureBuilder",
    ]);
  });

  it("keeps the performance-only Tauri mock feature out of production desktop tests", () => {
    expect(PERFORMANCE_BUILD_FEATURES).toBe("performance-test-tauri");
    expect(getPrecompileTargets("workspace-foundation").find((target) => target.targetKey === "lib")?.targetArgs)
      .toEqual(["--lib", "--features", "performance-test-tauri"]);
    expect(read("src-tauri/Cargo.toml")).toContain('performance-test-tauri = ["tauri/test"]');
    expect(read("src-tauri/Cargo.toml")).not.toContain("[dev-dependencies]");
  });

  it("keeps fixture ownership explicit and profile-sized", () => {
    expect(getFixtureWorkingFiles("search", "full")).toEqual([]);
    expect(getFixtureWorkingFiles("library-content", "extended")).toHaveLength(3);
    expect(getFixtureWorkingFiles("library-content", "full")).toHaveLength(6);
    expect(getFixtureWorkingFiles("library-content", "full")).toContain("file-library-1000000-content-migration.sqlite3");
  });

  it("rejects unknown suite arguments before starting a prepared binary", () => {
    const result = spawnSync(
      process.execPath,
      [path.join(process.cwd(), "scripts/runPerformanceSuite.mjs"), "--suite=unknown", "--profile=extended"],
      { cwd: process.cwd(), env: process.env, encoding: "utf8" },
    );
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("Unsupported performance suite: unknown");
  });

  it("fails closed when CI consumers lack prepared binaries or fixtures", () => {
    const script = path.join(process.cwd(), "scripts/runPerformanceSuite.mjs");
    const missingBinary = spawnSync(
      process.execPath,
      [script, "--suite=search", "--profile=extended"],
      {
        cwd: process.cwd(),
        env: { ...process.env, CI: "true", GITHUB_ACTIONS: "true" },
        encoding: "utf8",
      },
    );
    expect(missingBinary.status).toBe(1);
    expect(missingBinary.stderr).toContain("CI performance shards require --prepared-binaries");

    const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "zen-canvas-performance-missing-fixture-"));
    const binary = path.join(tempRoot, "fileLibrary.exe");
    const libraryIdentity = createPerformanceBuildIdentity({
      profile: "extended",
      suiteNames: ["library-content"],
      targetKeys: getPrecompileTargetsForSuites(["library-content"]).map((target) => target.targetKey),
      rust: TEST_RUST_VERSION,
    });
    fs.writeFileSync(binary, "prepared-binary");
    writeJson(path.join(tempRoot, "manifest.json"), createBinaryManifest({
      commit: "commit-1",
      profile: "extended",
      suites: ["library-content"],
      rustVersion: libraryIdentity.rustVersion,
      cargoLockSha256: libraryIdentity.cargoLockSha256,
      buildIdentity: libraryIdentity.buildIdentity,
      targets: {
        fileLibrary: { path: "fileLibrary.exe", size: fs.statSync(binary).size, sha256: sha256File(binary) },
      },
    }));
    const missingFixture = spawnSync(
      process.execPath,
      [
        script,
        "--suite=library-content",
        "--profile=extended",
        `--prepared-binaries=${tempRoot}`,
        `--build-identity=${libraryIdentity.buildIdentity}`,
      ],
      {
        cwd: process.cwd(),
        env: { ...process.env, CI: "true", GITHUB_ACTIONS: "true", GITHUB_SHA: "commit-1" },
        encoding: "utf8",
      },
    );
    expect(missingFixture.status).toBe(1);
    expect(missingFixture.stderr).toContain("requires --fixture-root");
  });

  it("moves compilation and fixture preparation out of the consumer runner", () => {
    const suite = read("scripts/runPerformanceSuite.mjs");
    const prepareBinaries = read("scripts/preparePerformanceBinaries.mjs");
    const prepareFixtures = read("scripts/preparePerformanceFixtures.mjs");
    expect(suite).toContain("--prepared-binaries");
    expect(suite).toContain("CI performance shards require --prepared-binaries");
    expect(suite).toContain("runPreparedTestBinary");
    expect(suite).not.toContain("cargo test");
    expect(suite).not.toContain('"--no-run"');
    expect(suite).toContain("prepareMissing");
    expect(prepareBinaries).toContain('"--no-run"');
    expect(prepareBinaries).toContain("cargo");
    expect(prepareBinaries).toContain('execFileSync("rustc", ["-Vv"]');
    expect(prepareFixtures).toContain("runPreparedTestBinary");
    expect(prepareFixtures).not.toContain("cargo test");
    expect(read("scripts/runPerformanceProfile.mjs")).toContain("preparePerformanceBinaries.mjs");
  });

  it("guards the Prepare test Rust identity override before Rust resolution", () => {
    const tempRoot = createTaskTempRoot("zen-canvas-performance-test-rust-");
    const script = path.join(process.cwd(), "scripts/preparePerformanceBinaries.mjs");
    try {
      const nonTest = spawnSync(
        process.execPath,
        [
          script,
          "--suites=search",
          "--profile=extended",
          `--output=${path.join(tempRoot, "non-test-output")}`,
          `--cache-root=${path.join(tempRoot, "non-test-cache")}`,
          `--test-rust-version=${TEST_RUST_VERSION}`,
        ],
        {
          cwd: process.cwd(),
          env: { ...process.env, NODE_ENV: "production" },
          encoding: "utf8",
        },
      );
      expect(nonTest.status).toBe(1);
      expect(nonTest.stderr).toContain("--test-rust-version is only available when NODE_ENV=test.");
      expect(nonTest.stderr).not.toContain("rustc -Vv");

      const empty = spawnSync(
        process.execPath,
        [
          script,
          "--suites=search",
          "--profile=extended",
          `--output=${path.join(tempRoot, "empty-output")}`,
          `--cache-root=${path.join(tempRoot, "empty-cache")}`,
          "--test-rust-version=",
        ],
        {
          cwd: process.cwd(),
          env: { ...process.env, NODE_ENV: "test" },
          encoding: "utf8",
        },
      );
      expect(empty.status).toBe(1);
      expect(empty.stderr).toContain("--test-rust-version must not be empty.");
      expect(empty.stderr).not.toContain("rustc -Vv");
    } finally {
      fs.rmSync(tempRoot, { recursive: true, force: true });
    }
  });

  it("preserves direct Rust test-binary CLI semantics and exit propagation", () => {
    expect(buildPreparedTestArgs("test_name", { ignored: true, testThreads: 1 })).toEqual([
      "test_name",
      "--exact",
      "--ignored",
      "--nocapture",
      "--test-threads=1",
    ]);
    const calls: Array<{ command: string; args: string[] }> = [];
    runPreparedTestBinary({
      executable: "fake.exe",
      testName: "passing_test",
      cwd: process.cwd(),
      env: process.env,
      spawnImpl: (command: string, args: string[]) => {
        calls.push({ command, args });
        return { status: 0, signal: null, stdout: args.includes("--list") ? `${args[0]}: test\n\n1 test, 0 benchmarks\n` : "" };
      },
    });
    expect(calls[0]).toEqual({
      command: "fake.exe",
      args: ["passing_test", "--list", "--exact", "--ignored"],
    });
    expect(calls[1]).toEqual({
      command: "fake.exe",
      args: ["passing_test", "--exact", "--ignored", "--nocapture"],
    });
    expect(() => runPreparedTestBinary({
      executable: "fake.exe",
      testName: "failing_test",
      cwd: process.cwd(),
      env: process.env,
      spawnImpl: (_command: string, args: string[]) => args.includes("--list")
        ? { status: 0, signal: null, stdout: `${args[0]}: test\n` }
        : { status: 1, signal: null, stdout: "out", stderr: "err" },
    })).toThrow(/exit code 1/);
    expect(() => runPreparedTestBinary({
      executable: "missing.exe",
      testName: "missing_test",
      cwd: process.cwd(),
      env: process.env,
      spawnImpl: () => ({
        status: null,
        signal: null,
        error: new Error("not found"),
      }),
    })).toThrow("not found");
    expect(() => runPreparedTestBinary({
      executable: "fake.exe",
      testName: "timeout_test",
      cwd: process.cwd(),
      env: process.env,
      spawnImpl: () => ({
        status: null,
        signal: null,
        error: new Error("timed out"),
      }),
    })).toThrow("timed out");
  });

  it.each([0, 2])("rejects %i real fake-binary registrations before benchmark execution", (matches) => {
    withFakePreparedBinary((executable, marker) => {
      expect(() => runPreparedTestBinary({
        executable: process.execPath, testName: executable, stdio: "pipe",
        cwd: process.cwd(),
        env: { ...process.env, TEST_MATCHES: String(matches), TEST_RUN_MARKER: marker },
      })).toThrow(`matched ${matches}`);
      expect(fs.existsSync(marker)).toBe(false);
    });
  });

  it.each([true, false])("runs exactly one selected test with ignored=%s and preserves output", (ignored) => {
    withFakePreparedBinary((executable, marker) => {
      const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
      try {
        const result = runPreparedTestBinary({
          executable: process.execPath, testName: executable, ignored, stdio: "pipe",
          cwd: process.cwd(),
          env: { ...process.env, TEST_IGNORED: ignored ? "1" : "0", TEST_RUN_MARKER: marker },
        });
        expect(result.stdout).toBe("benchmark stdout\n");
        expect(result.stderr).toBe("benchmark stderr\n");
        expect(stderr).toHaveBeenCalledWith("listing stderr\n");
        expect(fs.readFileSync(marker, "utf8")).toBe("executed");
      } finally {
        stderr.mockRestore();
      }
    });
  });

  it("rejects an ignored registration when the invocation would skip it", () => {
    withFakePreparedBinary((executable, marker) => {
      expect(() => runPreparedTestBinary({
        executable: process.execPath, testName: executable, ignored: false, stdio: "pipe",
        cwd: process.cwd(),
        env: { ...process.env, TEST_IGNORED: "1", TEST_RUN_MARKER: marker },
      })).toThrow("requested a nonignored test but matched an ignored registration");
      expect(fs.existsSync(marker)).toBe(false);
    });
  });

  it.each(["list", "run"])("propagates real %s process failure and diagnostic output", (phase) => {
    withFakePreparedBinary((executable, marker) => {
      const env = {
        ...process.env, TEST_RUN_MARKER: marker,
        ...(phase === "list" ? { TEST_LIST_STATUS: "7" } : { TEST_RUN_STATUS: "7" }),
      };
      expect(() => runPreparedTestBinary({
        executable: process.execPath, testName: executable, env, stdio: "pipe",
        cwd: process.cwd(),
      })).toThrow(new RegExp(`exit code 7\\.[\\s\\S]*${phase === "list" ? "listing stderr" : "benchmark stderr"}`));
      expect(fs.existsSync(marker)).toBe(phase === "run");
    });
  });

  it.each(["list", "run"])("propagates real %s timeout and captured output", (phase) => {
    withFakePreparedBinary((executable, marker) => {
      expect(() => runPreparedTestBinary({
        executable: process.execPath, testName: executable, timeoutMs: 1000, stdio: "pipe",
        cwd: process.cwd(),
        env: { ...process.env, TEST_TIMEOUT_PHASE: phase, TEST_RUN_MARKER: marker },
      })).toThrow(new RegExp(`ETIMEDOUT[\\s\\S]*${phase} before timeout[\\s\\S]*${phase} timeout stderr`));
    });
  });

  it.each(["list", "run"])("propagates a %s process signal", (phase) => {
    expect(() => runPreparedTestBinary({
      executable: "fake.exe", testName: "signal_test",
      cwd: process.cwd(),
      env: process.env,
      spawnImpl: (_command: string, args: string[]) => {
        if ((args.includes("--list") ? "list" : "run") === phase) {
          return { status: null, signal: "SIGTERM", stdout: "before signal", stderr: "signal stderr" };
        }
        return { status: 0, signal: null, stdout: "signal_test: test\n" };
      },
    })).toThrow(/signal SIGTERM[\s\S]*before signal[\s\S]*signal stderr/);
  });

  it.each([
    "1 test, 0 benchmarks\n",
    "different_test: test\n\n1 test, 0 benchmarks\n",
    "expected_test: benchmark\n\n0 tests, 1 benchmark\n",
  ])("rejects malformed or wrong test listing %j", (stdout) => {
    const spawn = vi.fn(() => ({ status: 0, signal: null, stdout, stderr: "list diagnostics" }));
    expect(() => runPreparedTestBinary({
      executable: "fake.exe", testName: "expected_test", spawnImpl: spawn,
      cwd: process.cwd(),
      env: process.env,
    })).toThrow(/expected_test[\s\S]*expected exactly one[\s\S]*list diagnostics/);
    expect(spawn).toHaveBeenCalledTimes(1);
  });

  it("validates binary commit/hash/path and fixture manifest contracts", () => {
    const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "zen-canvas-performance-artifact-"));
    const binary = path.join(tempRoot, "bin.exe");
    fs.writeFileSync(binary, "prepared-binary");
    writeJson(path.join(tempRoot, "manifest.json"), createBinaryManifest({
      commit: "commit-1",
      profile: "extended",
      suites: ["search"],
      rustVersion: TEST_RUST_VERSION,
      cargoLockSha256: "lock-hash",
      buildIdentity: "build-identity-1",
      targets: {
        lib: { path: "bin.exe", size: fs.statSync(binary).size, sha256: sha256File(binary) },
      },
    }));
    expect(validateBinaryManifest(tempRoot, {
      expectedCommit: "commit-1",
      expectedProfile: "extended",
      requiredTargets: ["lib"],
    }).commit).toBe("commit-1");
    fs.writeFileSync(binary, "tampered-binary");
    expect(() => validateBinaryManifest(tempRoot, {
      expectedCommit: "commit-1",
      requiredTargets: ["lib"],
    })).toThrow("hash mismatch");
    fs.writeFileSync(binary, "prepared-binary");
    expect(() => validateBinaryManifest(tempRoot, { expectedCommit: "wrong" })).toThrow("commit mismatch");

    const fixture = path.join(tempRoot, "fixture.sqlite3");
    fs.writeFileSync(fixture, "fixture");
    writeJson(path.join(tempRoot, "manifest.json"), createFixtureManifest({
      commit: "commit-1",
      profile: "extended",
      suites: ["library-content"],
      schemaVersion: 35,
      fixtureFormatVersion: 1,
      fixtureIdentity: "fixture-identity-1",
      rowCounts: [100_000],
      files: { "fixture.sqlite3": { size: fs.statSync(fixture).size, sha256: sha256File(fixture) } },
    }));
    expect(validateFixtureManifest(tempRoot, {
      expectedProfile: "extended",
      expectedFixtureIdentity: "fixture-identity-1",
      expectedSchemaVersion: 35,
      expectedFixtureFormatVersion: 1,
      expectedRowCounts: [100_000],
      requiredFiles: ["fixture.sqlite3"],
    }).schemaVersion).toBe(35);
    expect(() => validateFixtureManifest(tempRoot, { expectedFixtureIdentity: "wrong" }))
      .toThrow("identity mismatch");
    expect(() => validateFixtureManifest(tempRoot, {
      expectedFixtureIdentity: "fixture-identity-1",
      expectedRowCounts: [1_000_000],
      requiredFiles: ["fixture.sqlite3"],
    })).toThrow("row-count identity mismatch");
    // The consumer must enforce row-count identity, not just manifest hashes.
    expect(read("scripts/runPerformanceSuite.mjs"))
      .toContain("expectedRowCounts: fixtureIdentityInfo.rowCounts");
    const staleSchema41 = createFixtureManifest({
      commit: "commit-1", profile: "extended", suites: ["library-content"],
      schemaVersion: 41, fixtureFormatVersion: 1, fixtureIdentity: "fixture-identity-1",
      rowCounts: [100_000],
      files: { "fixture.sqlite3": { size: fs.statSync(fixture).size, sha256: sha256File(fixture) } },
    });
    writeJson(path.join(tempRoot, "manifest.json"), staleSchema41);
    expect(() => validateFixtureManifest(tempRoot, {
      expectedFixtureIdentity: "fixture-identity-1",
      expectedSchemaVersion: PERFORMANCE_FIXTURE_SCHEMA_VERSION,
      requiredFiles: ["fixture.sqlite3"],
    })).toThrow("schema mismatch: expected 46, got 41");
    const persistedManifest = readJson(path.join(tempRoot, "manifest.json"));
    if (typeof persistedManifest !== "object" || persistedManifest === null
      || !("schemaVersion" in persistedManifest)
      || typeof persistedManifest.schemaVersion !== "number") {
      throw new Error("persisted fixture manifest must contain a numeric schemaVersion");
    }
    expect(persistedManifest.schemaVersion).toBe(41);
  });

  it("reuses a build-identity binary cache without invoking Cargo and refreshes provenance", () => {
    const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "zen-canvas-performance-binary-cache-"));
    const cacheRoot = path.join(tempRoot, "cache");
    const outputRoot = path.join(tempRoot, "output");
    const identity = createPerformanceBuildIdentity({
      profile: "extended",
      suiteNames: ["search"],
      targetKeys: getPrecompileTargetsForSuites(["search"]).map((target) => target.targetKey),
      rust: TEST_RUST_VERSION,
    });
    const cacheEntryRoot = path.join(cacheRoot, identity.buildIdentity);
    const commit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    const targets = Object.fromEntries(getPrecompileTargetsForSuites(["search"]).map((target) => {
      const relativePath = `bin/${target.targetKey}.exe`;
      const targetPath = path.join(cacheEntryRoot, relativePath);
      fs.mkdirSync(path.dirname(targetPath), { recursive: true });
      fs.writeFileSync(targetPath, `cached-${target.targetKey}`);
      return [target.targetKey, {
        targetKey: target.targetKey,
        path: relativePath,
        size: fs.statSync(targetPath).size,
        sha256: sha256File(targetPath)
      }];
    }));
    writeJson(path.join(cacheEntryRoot, "manifest.json"), createBinaryManifest({
      commit: "older-run-commit",
      generatedFromCommit: "older-run-commit",
      profile: "extended",
      suites: ["search"],
      rustVersion: identity.rustVersion,
      cargoLockSha256: identity.cargoLockSha256,
      buildIdentity: identity.buildIdentity,
      runner: identity.runner,
      cacheScope: "build-identity",
      targets
    }));

    const result = spawnSync(
      process.execPath,
      [
        path.join(process.cwd(), "scripts/preparePerformanceBinaries.mjs"),
        "--suites=search",
        "--profile=extended",
        `--cache-root=${cacheRoot}`,
        `--output=${outputRoot}`,
        `--test-rust-version=${identity.rustVersion}`,
      ],
      {
        cwd: process.cwd(),
        env: {
          ...process.env,
          NODE_ENV: "test",
          GITHUB_SHA: commit
        },
        encoding: "utf8"
      }
    );

    expect(result.status).toBe(0);
    expect(result.stdout).toContain("[perf-prepare] prepared-binary-cache=hit");
    expect(result.stdout).toContain("[perf-prepare] cargo-compile-ms=0");
    expect(result.stdout).not.toContain("phase=cargo-compile");
    const currentRun = readJson(path.join(outputRoot, "_prepare", "manifest.json")) as {
      commit: string;
      generatedFromCommit: string;
      buildIdentity: string;
    };
    expect(currentRun.commit).toBe(commit);
    expect(currentRun.generatedFromCommit).toBe(commit);
    expect(currentRun.buildIdentity).toBe(identity.buildIdentity);
  });

  it("keeps binary cache identity independent of commit and run-attempt metadata", () => {
    const first = createPerformanceBuildIdentity({
      profile: "full",
      runnerOs: "Windows",
      runnerArch: "X64",
      rust: TEST_RUST_VERSION,
    });
    const second = createPerformanceBuildIdentity({
      profile: "full",
      runnerOs: "Windows",
      runnerArch: "X64",
      rust: TEST_RUST_VERSION,
    });

    expect(second.buildIdentity).toBe(first.buildIdentity);
    expect(JSON.stringify(first.payload)).not.toContain("GITHUB_SHA");
    expect(JSON.stringify(first.payload)).not.toContain("GITHUB_RUN_ID");
    expect(JSON.stringify(first.payload)).not.toContain("package.json");
    expect(JSON.stringify(first.payload)).not.toContain("docs/");
  });

  it("keys fixture caches by semantic inputs and profile size, not checkout metadata", () => {
    const full = createPerformanceFixtureIdentity({
      profile: "full",
      runnerOs: "Windows",
      runnerArch: "X64",
    });
    const fullRepeat = createPerformanceFixtureIdentity({
      profile: "full",
      runnerOs: "Windows",
      runnerArch: "X64",
    });
    const extended = createPerformanceFixtureIdentity({
      profile: "extended",
      runnerOs: "Windows",
      runnerArch: "X64",
    });

    expect(PERFORMANCE_FIXTURE_SCHEMA_VERSION).toBe(46);
    expect(full.schemaVersion).toBe(46);
    expect(full.payload.schemaVersion).toBe(46);
    expect(fs.readFileSync("src-tauri/tests/support/performance_fixture.rs", "utf8"))
      .toContain("pub const FIXTURE_SCHEMA_VERSION: i64 = zen_canvas_tauri::db::SUPPORTED_SCHEMA_VERSION as i64;");
    expect(full.payload.inputs).toEqual(expect.arrayContaining([
      { path: "src-tauri/src/db/schema_global_generations.rs", sha256: sha256File("src-tauri/src/db/schema_global_generations.rs") },
      { path: "src-tauri/src/db/schema_global_generations.sql", sha256: sha256File("src-tauri/src/db/schema_global_generations.sql") },
    ]));
    expect(full.fixtureIdentity).toBe(fullRepeat.fixtureIdentity);
    expect(full.fixtureCacheKey).toBe(fullRepeat.fixtureCacheKey);
    expect(full.rowCounts).toEqual([100_000, 1_000_000]);
    expect(extended.rowCounts).toEqual([100_000]);
    expect(full.fixtureIdentity).not.toBe(extended.fixtureIdentity);
    expect(full.fixtureCacheKey).not.toBe(extended.fixtureCacheKey);
    expect(JSON.stringify(full.payload)).not.toContain("GITHUB_SHA");
    expect(JSON.stringify(full.payload)).not.toContain("GITHUB_RUN_ID");
    expect(JSON.stringify(full.payload)).not.toContain("docs/");
  });

  it("separates binary cache identities by profile and prepared target set", () => {
    const search = createPerformanceBuildIdentity({
      profile: "full",
      suiteNames: ["search"],
      targetKeys: getPrecompileTargetsForSuites(["search"]).map((target) => target.targetKey),
      rust: TEST_RUST_VERSION,
    });
    const all = createPerformanceBuildIdentity({
      profile: "full",
      suiteNames: [...PERFORMANCE_SUITE_NAMES],
      targetKeys: getPrecompileTargetsForSuites([...PERFORMANCE_SUITE_NAMES]).map((target) => target.targetKey),
      rust: TEST_RUST_VERSION,
    });
    const extended = createPerformanceBuildIdentity({
      profile: "extended",
      suiteNames: [...PERFORMANCE_SUITE_NAMES],
      targetKeys: getPrecompileTargetsForSuites([...PERFORMANCE_SUITE_NAMES]).map((target) => target.targetKey),
      rust: TEST_RUST_VERSION,
    });
    const changedFeatures = createPerformanceBuildIdentity({
      profile: "full",
      suiteNames: [...PERFORMANCE_SUITE_NAMES],
      features: "performance-test-tauri,changed",
      targetKeys: getPrecompileTargetsForSuites([...PERFORMANCE_SUITE_NAMES]).map((target) => target.targetKey),
      rust: TEST_RUST_VERSION,
    });

    expect(search.buildIdentity).not.toBe(all.buildIdentity);
    expect(all.buildIdentity).not.toBe(extended.buildIdentity);
    expect(all.buildIdentity).not.toBe(changedFeatures.buildIdentity);
    const inputs = all.payload.inputs as Array<{ path: string }>;
    expect(inputs.some((input) => input.path === "src-tauri/src/db/queries/library/mod.rs")).toBe(true);
    expect(inputs.some((input) => input.path === "src-tauri/Cargo.lock")).toBe(true);
    expect(inputs.some((input) => input.path === "package.json")).toBe(false);
    expect(inputs.some((input) => input.path.startsWith("docs/"))).toBe(false);
  });

  it("separates binary cache identities for suites that share a precompiled target", () => {
    const libraryContent = createPerformanceBuildIdentity({
      profile: "extended",
      suiteNames: ["library-content"],
      targetKeys: ["fileLibrary"],
      rust: TEST_RUST_VERSION,
    });
    const previewPlatform = createPerformanceBuildIdentity({
      profile: "extended",
      suiteNames: ["preview-platform"],
      targetKeys: ["fileLibrary"],
      rust: TEST_RUST_VERSION,
    });

    expect(previewPlatform.buildIdentity).not.toBe(libraryContent.buildIdentity);
    expect(previewPlatform.payload.suiteNames).toEqual(["preview-platform"]);
  });

  it("keeps the PR compatibility command bounded and preserves gates", () => {
    const source = read("scripts/runPerformanceTestPr.mjs");
    expect(source).toContain('ZC_PERFORMANCE_PROFILE: "extended"');
    expect(source).toContain('ZC_FTS_FULL_PROFILE: "false"');
    expect(source).toContain("fts_benchmark_100k");
    expect(source).toContain('"--locked"');
    expect(source.match(/fts_benchmark_100k/g)).toHaveLength(1);
    const fts = read("src-tauri/tests/fts_benchmark.rs");
    const library = read("src-tauri/tests/file_library_performance.rs");
    expect(fts).toContain("const DEFAULT_ROWS: usize = 100_000");
    expect(fts).toContain("const DEFAULT_P95_MS: f64 = 1_000.0");
    expect(fts).toContain("if full_profile");
    expect(fts).not.toContain("INSERT_BATCH_SIZE");
    expect(library).toContain("seed_library_for_benchmark");
    expect(library).toContain("performance_1m_file_library_query_matrix");
    expect(library).toContain("performance_1m_schema_32_to_33_rule_proposal_migration");
    expect(read("src-tauri/src/db/queries/rule_proposals/mod.rs")).toContain("for index in 100_000..1_000_000_usize");
  });
});
