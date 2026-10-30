import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function readWorkflow(relativePath: string) {
  return readFileSync(relativePath, "utf8").replace(/\r\n?/gu, "\n");
}

const interactiveWorkflow = readWorkflow(".github/workflows/ci.yml");
const fullWorkflow = readWorkflow(".github/workflows/ci-full.yml");
const releaseWorkflow = readWorkflow(".github/workflows/release-build.yml");
const classifierSource = readFileSync("scripts/classifyCiChanges.mjs", "utf8");
const globalIndexServiceQualification = readFileSync("scripts/qualifyWindowsGlobalIndexService.ps1", "utf8");
const globalIndexProbeSource = readFileSync("src-tauri/native-qa/global_index_probe.rs", "utf8");
const packageJson = JSON.parse(readFileSync("package.json", "utf8")) as {
  scripts: Record<string, string>;
};

function section(source: string, job: string, nextJob?: string) {
  const start = source.indexOf(`  ${job}:`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = nextJob ? source.indexOf(`  ${nextJob}:`, start + 1) : source.length;
  return source.slice(start, end < 0 ? source.length : end);
}

type WorkflowJob = {
  id: string;
  runsOn: string | null;
  timeoutMinutes: number | null;
};

function parseWorkflowJobs(source: string): WorkflowJob[] {
  const lines = source.split("\n");
  const jobsStart = lines.findIndex((line) => line === "jobs:");
  if (jobsStart < 0) throw new Error("workflow is missing the jobs mapping");

  const jobs: WorkflowJob[] = [];
  let current: WorkflowJob | null = null;
  for (const line of lines.slice(jobsStart + 1)) {
    const jobMatch = line.match(/^  ([A-Za-z0-9_-]+):\s*$/u);
    if (jobMatch) {
      current = { id: jobMatch[1], runsOn: null, timeoutMinutes: null };
      jobs.push(current);
      continue;
    }
    if (!current) continue;
    const propertyMatch = line.match(/^    (runs-on|timeout-minutes):\s*(.+?)\s*$/u);
    if (!propertyMatch) continue;
    if (propertyMatch[1] === "runs-on") current.runsOn = propertyMatch[2];
    if (propertyMatch[1] === "timeout-minutes") current.timeoutMinutes = Number(propertyMatch[2]);
  }
  return jobs;
}

function workflowStep(source: string, name: string, nextName: string) {
  const start = source.indexOf(`      - name: ${name}\n`);
  expect(start).toBeGreaterThanOrEqual(0);
  const end = source.indexOf(`      - name: ${nextName}\n`, start + 1);
  expect(end).toBeGreaterThan(start);
  return source.slice(start, end);
}

describe("CI final performance remediation contract", () => {
  it("keeps Interactive and Full triggers, concurrency, and stable check names distinct", () => {
    expect(interactiveWorkflow).toContain("pull_request: {}");
    expect(interactiveWorkflow).toContain("ci-interactive-${{ github.ref }}");
    expect(interactiveWorkflow).not.toContain("schedule:");
    expect(interactiveWorkflow).not.toContain("workflow_dispatch:");
    expect(fullWorkflow).toContain("schedule:");
    expect(fullWorkflow).toContain("workflow_dispatch:");
    expect(fullWorkflow).toContain("ci-full-${{ github.ref }}");
    expect(fullWorkflow).not.toContain("pull_request:");
    expect(fullWorkflow).not.toContain("push:");
    for (const requiredName of [
      "name: Quality (windows-latest)",
      "name: Quality (macos-latest)",
      "name: Performance profile",
    ]) {
      expect(interactiveWorkflow).toContain(requiredName);
      expect(fullWorkflow).toContain(requiredName);
    }
  });

  it("declares one prepare job and six independent consumer shards", () => {
    const performanceJobs = [
      "performance-search",
      "performance-scan-schema",
      "performance-library-content",
      "performance-intelligence",
      "performance-workspace-foundation",
      "performance-preview-platform",
    ];
    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      expect(workflow).toContain("  performance-prepare:");
      expect(workflow).toContain("name: Performance / Prepare");
      for (const [index, job] of performanceJobs.entries()) {
        expect(workflow).toContain(`  ${job}:`);
        const nextJob = performanceJobs[index + 1]
          ?? (workflow === fullWorkflow ? "build-windows" : "performance-profile");
        const shard = section(workflow, job, nextJob);
        expect(shard).toContain("actions/download-artifact");
        expect(shard).not.toContain("Swatinem/rust-cache");
        if (job === "performance-library-content") {
          expect(shard).toContain("actions/cache/restore@");
          expect(shard).not.toContain("actions/cache/save@");
        } else {
          expect(shard).not.toContain("actions/cache/restore@");
          expect(shard).not.toContain("actions/cache/save@");
        }
        expect(shard).not.toContain("cargo test");
        expect(shard).toContain(".performance-temp");
      }
      expect(section(workflow, "performance-profile")).toContain("performance-prepare");
    }
    expect(section(interactiveWorkflow, "performance-prepare", "performance-search")).toContain("preparePerformanceBinaries.mjs");
    expect(section(interactiveWorkflow, "performance-prepare", "performance-search")).toContain("preparePerformanceFixtures.mjs");
    expect(section(fullWorkflow, "performance-prepare", "performance-search")).toContain("preparePerformanceBinaries.mjs");
  });

  it("makes Performance Prepare the only performance Rust cache writer", () => {
    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      const prepare = section(workflow, "performance-prepare");
      expect(prepare).toContain("Swatinem/rust-cache");
      expect(prepare).toContain("shared-key: zen-canvas-Windows-performance-v3");
      expect(prepare).toContain("add-job-id-key: false");
      expect(prepare).toContain("cache-workspace-crates: true");
      expect(prepare).toContain("cache-targets: true");
      expect(prepare).toContain("cache-on-failure: true");
      expect((workflow.match(/shared-key: zen-canvas-Windows-performance-v3/g) ?? []).length).toBe(1);
      expect(workflow).not.toContain("zen-canvas-Windows-performance-v2");
      expect(prepare).toContain("actions/cache/restore@5a3ec84eff668545956fd18022155c47e93e2684 # v4.2.3");
      expect(prepare).toContain("actions/cache/save@5a3ec84eff668545956fd18022155c47e93e2684 # v4.2.3");
      expect(prepare).toContain(".performance-cache/binaries");
      expect(prepare).toContain("performanceBuildIdentity.mjs");
      expect(prepare).toContain("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7");
      expect(prepare).not.toContain("zen-canvas-perf-binaries-${{ github.sha }}");
      expect(prepare).not.toContain("github.run_id");
    }
  });

  it("runs Rust lint before costly test execution in interactive and full validation", () => {
    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      for (const [job, format, clippy, tests] of [
        ["rust-windows", "Rust format", "Rust clippy", "Rust tests"],
        ["rust-macos", "Rust format", "Rust clippy", "Rust tests"],
        ["windows-native-preview-handler", "Native workspace format", "Native workspace clippy", "Native workspace tests"],
      ]) {
        const nextJob = job === "rust-windows"
          ? "windows-native-preview-handler"
          : job === "windows-native-preview-handler"
            ? workflow === interactiveWorkflow ? "windows-global-index-service-qualification" : "rust-macos"
            : "performance-prepare";
        const block = section(workflow, job, nextJob);
        const fmtAt = block.indexOf("      - name: " + format);
        const clippyAt = block.indexOf("      - name: " + clippy);
        const testAt = block.indexOf("      - name: " + tests);
        expect(fmtAt, job + " format").toBeGreaterThanOrEqual(0);
        expect(clippyAt, job + " clippy").toBeGreaterThan(fmtAt);
        expect(testAt, job + " tests").toBeGreaterThan(clippyAt);
        expect((block.match(new RegExp("      - name: " + clippy, "g")) ?? []).length).toBe(1);
      }
    }
  });

  it("runs ordinary macOS coverage once and the promoted race matrix once", () => {
    const expectedRaceIterations = new Map([
      [interactiveWorkflow, "10000"],
      [fullWorkflow, "100000"],
    ]);
    const raceTests = [
      "macos_mutation_parity_supports_move_copy_replace_restore_and_delete",
      "macos_symlink_and_package_mutations_keep_namespace_boundaries",
      "macos_target_conflict_preserves_both_objects_without_claim_artifacts",
      "macos_target_creation_race_never_loses_source_payload",
      "macos_expanded_adversarial_attack_matrix_reports_zero_wrong_commit_or_loss",
      "macos_cross_volume_source_mutation_is_rejected_when_real_fixture_is_provided",
    ];

    for (const [workflow, iterations] of expectedRaceIterations) {
      const macosQuality = section(workflow, "rust-macos", "performance-prepare");
      const stepsStart = macosQuality.indexOf("    steps:");
      expect(stepsStart).toBeGreaterThanOrEqual(0);
      const jobHeader = macosQuality.slice(0, stepsStart);
      const ordinaryStart = macosQuality.indexOf("      - name: Rust tests");
      const ordinaryEnd = macosQuality.indexOf("\n      - name:", ordinaryStart + 1);
      expect(ordinaryStart).toBeGreaterThanOrEqual(0);
      expect(ordinaryEnd).toBeGreaterThan(ordinaryStart);
      const ordinaryRustTests = macosQuality.slice(ordinaryStart, ordinaryEnd);
      const raceStart = macosQuality.indexOf("      - name: macOS race validation (serial, once)");
      const raceEnd = macosQuality.length;
      expect(raceStart).toBeGreaterThanOrEqual(0);
      expect(raceEnd).toBeGreaterThan(raceStart);
      const raceStep = macosQuality.slice(raceStart, raceEnd);

      expect(jobHeader).not.toContain("ZEN_CANVAS_MACOS_RACE_ITERATIONS");
      expect(jobHeader).not.toContain("ZEN_CANVAS_MACOS_EXPANDED_RACE_ITERATIONS");
      expect(macosQuality).toContain("macOS race validation (serial, once)");
      expect(ordinaryRustTests).toContain("cargo test --manifest-path src-tauri/Cargo.toml --features \"desktop-runtime native-qa\" -- \\");
      expect((ordinaryRustTests.match(/--skip /g) ?? []).length).toBe(raceTests.length);
      for (const testName of raceTests) expect(ordinaryRustTests).toContain("--skip " + testName);
      expect(raceStep).toContain("        env:\n          ZEN_CANVAS_MACOS_RACE_ITERATIONS: \"" + iterations + "\"\n          ZEN_CANVAS_MACOS_EXPANDED_RACE_ITERATIONS: \"" + iterations + "\"");
      expect(raceStep).toContain("--test macos_mutation_fail_closed");
      expect((raceStep.match(/--test macos_mutation_fail_closed/g) ?? []).length).toBe(1);
      expect((raceStep.match(/cargo test /g) ?? []).length).toBe(1);
      expect(raceStep).toContain("--test-threads=1");
      expect(raceStep).not.toContain("--skip ");
      for (const removedFilter of [
        "platform::macos",
        "content::eligibility",
        "temp_safety_tests",
        "macos_native_hardening_smoke",
      ]) {
        // The ordinary suite must remain unfiltered. A separately required
        // native lifecycle qualification may legitimately name its module.
        expect(ordinaryRustTests).not.toContain(removedFilter);
      }
    }
  });

  it("uses semantic performance cache keys and blocks fork cache publication", () => {
    const prepare = section(interactiveWorkflow, "performance-prepare", "performance-search");
    const libraryConsumer = section(interactiveWorkflow, "performance-library-content", "performance-intelligence");
    const semanticBinaryKey = "zen-canvas-perf-binaries-${{ runner.os }}-${{ runner.arch }}-${{ steps.binary-build-identity.outputs.build_identity }}";
    const semanticFixtureKey = "key: ${{ steps.fixture-identity.outputs.fixture_cache_key }}";

    expect(prepare).toContain(semanticBinaryKey);
    expect(prepare).toContain(semanticFixtureKey);
    expect(prepare).not.toContain("VALIDATION_TREE_SHA");
    expect(prepare).not.toContain("matrix.validation_lane }}-${{ env.");
    expect(prepare).toContain("CACHE_WRITE_ALLOWED: ${{ github.event_name != 'pull_request' || (github.event.pull_request.head.repo.full_name == github.repository && matrix.validation_lane == 'merge_integration') }}");
    expect((prepare.match(/env\.CACHE_WRITE_ALLOWED == 'true'/g) ?? []).length).toBe(2);
    expect(libraryConsumer).toContain("key: ${{ steps.binary-identity.outputs.fixture_cache_key }}");
    expect(libraryConsumer).not.toContain("matrix.validation_lane == 'head_validation' && needs.validation-plan.outputs.head_tree_sha");

    const fullPrepare = section(fullWorkflow, "performance-prepare", "performance-search");
    expect(fullPrepare).toContain(semanticBinaryKey);
    expect(fullPrepare).toContain(semanticFixtureKey);
    expect(fullPrepare).not.toContain("VALIDATION_TREE_SHA");
    expect(fullPrepare).not.toContain("matrix.validation_lane");
  });

  it("delivers exact Library/Content fixtures between jobs even when GitHub cache restore is unavailable", () => {
    for (const [workflow, lane] of [[interactiveWorkflow, true], [fullWorkflow, false]] as const) {
      const prepare = section(workflow, "performance-prepare", "performance-search");
      const library = section(workflow, "performance-library-content", "performance-intelligence");
      const artifactName = lane ? "perf-fixture-library-content-${{ matrix.validation_lane }}" : "perf-fixture-library-content";
      expect(prepare).toContain("Upload exact Library and Content fixtures for downstream shard");
      expect(prepare).toContain(`name: ${artifactName}\n`);
      expect(prepare).toContain("path: .tmp-performance-fixtures/cache/${{ steps.fixture-identity.outputs.fixture_identity }}");
      expect(prepare).toContain("if-no-files-found: error");
      expect(library).toContain("Restore exact Library and Content fixture cache");
      expect(library).toContain("Download exact fixture handoff when shared cache is inaccessible");
      expect(library).toContain("if: ${{ steps.fixture-cache.outputs.cache-hit != 'true' }}");
      expect(library).toContain(`name: ${artifactName}\n`);
      expect(library).toContain("Require exact Library and Content fixture manifest");
      expect(library).toContain("Test-Path -LiteralPath $manifest -PathType Leaf");
      expect(library).toContain("runPerformanceSuite.mjs");
      expect(library).toContain("--fixture-identity=");
      expect(library).not.toContain("Exact prepared fixture cache was not restored.");
    }
  });

  it("reuses native prepared binaries without disconnecting Cargo target storage", () => {
    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      const native = section(workflow, "performance-macos", workflow === fullWorkflow ? "quality-windows" : "build-windows");
      expect(native).toContain("Compute native prepared binary identity");
      expect(native).toContain("actions/cache/restore@5a3ec84eff668545956fd18022155c47e93e2684 # v4.2.3");
      expect(native).toContain("actions/cache/save@5a3ec84eff668545956fd18022155c47e93e2684 # v4.2.3");
      expect(native).toContain("zen-canvas-perf-binaries-${{ runner.os }}-${{ runner.arch }}-${{ steps.native-binary-build-identity.outputs.build_identity }}");
      expect(native).toContain("--cache-root=.performance-cache/binaries");
      expect(native).toContain("--output=.performance-artifacts/binaries");
      expect(native).toContain('--build-identity="${{ steps.native-binary-build-identity.outputs.build_identity }}"');
      expect(native).toContain("workspaces: src-tauri -> target");
      expect(native).not.toContain("CARGO_TARGET_DIR");
      expect(native).not.toContain(".performance-cache/target");
    }
    expect(section(fullWorkflow, "performance-macos", "quality-windows"))
      .toContain("full_native_profile_streams_a_logical_ten_gib_source_once");
  });

  it("pins cargo-audit, caches only the tool, and still runs the real RustSec audit", () => {
    const rustAudit = packageJson.scripts["security:audit:rust"];
    expect(rustAudit).toContain("cargo audit --file src-tauri/Cargo.lock");
    expect(rustAudit).toContain("cargo audit --file src-tauri/native/Cargo.lock");
    expect(rustAudit).toMatch(/cargo audit --file src-tauri\/Cargo\.lock\s+&&\s+cargo audit --file src-tauri\/native\/Cargo\.lock/u);
    expect(rustAudit).not.toContain("||");

    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      const audit = section(workflow, "dependency-audit", "quality-windows");
      expect(audit).toContain("CARGO_AUDIT_VERSION: \"0.22.2\"");
      expect(audit).toContain("Cache pinned cargo-audit");
      expect(audit).toContain("cargo install cargo-audit --version \"$CARGO_AUDIT_VERSION\" --locked --root \"$CARGO_AUDIT_ROOT\"");
      expect(audit).toContain("cargo-audit\" --version | grep -F \"$CARGO_AUDIT_VERSION\"");
      expect(audit).toContain("npm run security:audit:rust");
      expect(audit).not.toContain("cargo install cargo-audit --locked");
    }
  });

  it("uses domain-specific artifacts and direct prepared-binary execution", () => {
    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      for (const artifact of [
        "perf-bin-search",
        "perf-bin-scan-schema",
        "perf-bin-library-content",
        "perf-bin-intelligence",
        "perf-bin-workspace-foundation",
        "perf-bin-preview-platform",
      ]) {
        expect(workflow).toContain(`name: ${artifact}`);
      }
      // Prepared synthetic fixtures are now handed off as exact same-run artifacts when cache scope differs.
      expect(workflow).toContain("name: perf-fixture-library-content");
      expect(workflow).not.toContain(".performance-artifacts/fixtures");
      expect(workflow).toContain("fixture_cache_key");
      expect(workflow).toContain("fixture_identity");
      expect(workflow).toContain("--fixture-root=.tmp-performance-fixtures/cache");
      expect(workflow).toContain("--prepared-binaries=.performance-artifacts/binaries");
      expect(workflow).not.toContain("--prepare-missing-fixtures");
    }
    expect(section(interactiveWorkflow, "performance-profile")).toContain("EXPECTED_ANY");
    expect(section(interactiveWorkflow, "performance-profile")).toContain("expected skipped");
  });

  it("preserves the Round 2 macOS routing and native performance contract", () => {
    for (const prefix of [
      '"src-tauri/src/platform/macos/"',
      '"src-tauri/src/global_index/macos/"',
      '"src-tauri/src/runtime_capabilities.rs"',
      '"src-tauri/src/scanner.rs"',
    ]) {
      expect(classifierSource).toContain(prefix);
    }
    expect(classifierSource).not.toContain('path.startsWith("src-tauri/src/scanner/")');
    expect(interactiveWorkflow).toContain("performance_sensitive: ${{ steps.classify.outputs.performance_sensitive }}");
    expect(interactiveWorkflow).toContain("  performance-macos:");
    expect(interactiveWorkflow).toContain("name: Native macOS performance (arm64)");
    expect(interactiveWorkflow).toContain("macos_file_provider_feasibility");
    expect(interactiveWorkflow).toContain("Prepare native Workspace Foundation performance binary");
    expect(interactiveWorkflow).toContain("--suites=workspace-foundation");
    expect(interactiveWorkflow).toContain("--suites=workspace-foundation,preview-platform");
    expect(interactiveWorkflow).toContain("--suite=workspace-foundation");
    expect(interactiveWorkflow).toContain("--suite=preview-platform");
    expect(fullWorkflow).toContain("  performance-macos:");
    expect(fullWorkflow).toContain("name: Native macOS performance (arm64)");
    expect(fullWorkflow).toContain("macos_native_bookkeeping_benchmark_is_bounded_by_unique_identity");
    expect(fullWorkflow).toContain("Prepare native Workspace Foundation performance binary");
    expect(fullWorkflow).toContain("--suites=workspace-foundation");
    expect(fullWorkflow).toContain("--suites=workspace-foundation,preview-platform");
    expect(fullWorkflow).toContain("--suite=workspace-foundation");
    expect(fullWorkflow).toContain("--suite=preview-platform");
  });

  it("preserves routing, release, package, and build boundaries", () => {
    for (const output of [
      "perf_search",
      "perf_scan_schema",
      "perf_library_content",
      "perf_intelligence",
      "perf_workspace_foundation",
      "perf_preview_platform",
      "frontend_changed",
      "windows_native_preview_handler_changed",
      "rust_changed",
      "macos_sensitive",
      "performance_sensitive",
      "high_risk",
      "package_sensitive",
      "dependency_sensitive",
    ]) {
      expect(interactiveWorkflow).toContain(output + ": ${{ steps.classify.outputs." + output + " }}");
    }
    expect(interactiveWorkflow).toContain("needs.change-scope.outputs.rust_changed == 'true'");
    expect(interactiveWorkflow).toContain("needs.change-scope.outputs.windows_native_preview_handler_changed == 'true'");
    expect(interactiveWorkflow).toContain("needs.change-scope.outputs.macos_sensitive == 'true'");
    expect(interactiveWorkflow).toContain("needs.change-scope.outputs.release_sensitive == 'true'");
    expect(packageJson.scripts["build:check"]).toContain("build:frontend");
    expect(packageJson.scripts["build:check"]).toContain("check:rust:release");
    expect(packageJson.scripts["check:rust:release"]).not.toContain("vite");
    expect(releaseWorkflow).not.toContain("npm run test:performance:pr");
    expect(fullWorkflow).toContain("npm run build -- --no-sign");
  });

  it("keeps release packaging focused and fail-closed on exact-SHA Full Validation", () => {
    expect(releaseWorkflow).toContain("  release-qualified-validation:");
    expect(releaseWorkflow).toContain("name: Require CI Full Validation for exact SHA");
    expect(releaseWorkflow).toContain("actions: read");
    expect(releaseWorkflow).toContain("contents: read");
    expect(releaseWorkflow).toContain(
      "actions/workflows/ci-full.yml/runs?head_sha=${EXPECTED_SHA}&status=completed&per_page=100",
    );
    expect(releaseWorkflow).toContain("node scripts/releaseQualification.mjs select-run");
    expect(releaseWorkflow).toContain("actions/runs/${RUN_ID}/jobs?per_page=100");
    expect(releaseWorkflow).toContain("node scripts/releaseQualification.mjs verify-jobs");
    expect(releaseWorkflow).toContain("needs: release-qualified-validation");
    expect(releaseWorkflow).not.toContain("  ordinary-ci:");
    expect(releaseWorkflow).not.toContain("Require ordinary CI for exact SHA");
    expect(releaseWorkflow).not.toContain("actions/workflows/ci.yml/runs?head_sha=${EXPECTED_SHA}");
    expect(releaseWorkflow).toContain("Distribution model: UNSIGNED");
    expect(releaseWorkflow).toContain("Windows Authenticode: NOT PROVIDED");
    expect(releaseWorkflow).toContain("macOS Developer ID: NOT PROVIDED");
    expect(releaseWorkflow).toContain("Apple notarization: NOT PROVIDED");
    expect(releaseWorkflow).toContain("Stapling: NOT PROVIDED");
    expect(releaseWorkflow).toContain("Release qualification: exact-SHA CI Full Validation");
    expect(releaseWorkflow).toContain("Node/Rust CycloneDX SBOMs");
    expect(releaseWorkflow).not.toContain("npm high audit clean");

    for (const removedStep of [
      "Typecheck",
      "Test",
      "Rust format",
      "Rust dependency audit",
      "Bounded 100k search performance test",
      "Security audit",
      "macOS race validation (serial, once)",
    ]) {
      expect(releaseWorkflow).not.toContain(`      - name: ${removedStep}\n`);
    }
    expect(releaseWorkflow).not.toContain("cargo test --manifest-path src-tauri/Cargo.toml");
    expect(releaseWorkflow).not.toContain("cargo clippy --manifest-path src-tauri/Cargo.toml");

    for (const retainedStep of [
      "Verify checkout provenance and version metadata",
      "Package Windows installer",
      "Package macOS DMG",
      "Verify Windows NSIS artifact",
      "Verify unsigned macOS DMG",
      "Verify Windows NSIS registry authority semantics",
      "Verify Windows NSIS service runtime authority semantics",
      "Generate Windows checksums",
      "Generate macOS checksums",
      "Upload installers",
      "Generate Node SBOM",
      "Generate Rust SBOM",
      "Verify SBOM outputs",
    ]) {
      expect(releaseWorkflow).toContain(`      - name: ${retainedStep}\n`);
    }
  });

  it("runs the bounded Windows Preview Handler native lane on both CI workflows", () => {
    expect(classifierSource).toContain('"src-tauri/native/"');
    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      const native = section(workflow, "windows-native-preview-handler", "rust-macos");
      expect(native).toContain("runs-on: windows-latest");
      expect(native).toContain("cargo fmt --manifest-path src-tauri/native/Cargo.toml --all -- --check");
      expect(native).toContain("cargo test --manifest-path src-tauri/native/Cargo.toml");
      expect(native).toContain("cargo clippy --manifest-path src-tauri/native/Cargo.toml --all-targets -- -D warnings");
      expect(native).toContain("cargo build --release --manifest-path src-tauri/native/windows-preview-handler/Cargo.toml --features test-observability");
      expect(native).toContain("cargo build --release --manifest-path src-tauri/native/windows-preview-handler-harness/Cargo.toml");
      expect(native).toContain("cargo build --release --manifest-path src-tauri/native/windows-preview-handler-harness/Cargo.toml --bin zen-canvas-windows-preview-dll-servicing-smoke");
      expect(native).toContain("zen-canvas-windows-preview-handler-harness.exe");
      expect(native).toContain("zen-canvas-windows-preview-dll-servicing-smoke.exe");
      expect(native).toContain("zen_canvas_windows_preview_handler.dll");
      expect(native).toContain("w4-03-v2-harness");
      expect(native).toContain("w4-04-preview-dll-servicing-smoke");
      expect(native).toContain("node scripts/verifyWindowsNsisPreviewResource.mjs");
      expect(native).toContain("--source src-tauri/native/target/release/zen_canvas_windows_preview_handler.dll");
    }
    expect(interactiveWorkflow).toContain("windows-native-preview-handler.result");
    expect(fullWorkflow).toContain("WINDOWS_NATIVE: ${{ needs.windows-native-preview-handler.result }}");
    expect(releaseWorkflow).toContain("name: Verify Windows Preview resource path and servicing smoke");
    expect(releaseWorkflow).toContain("node scripts/verifyWindowsNsisPreviewResource.mjs");
  });

  it("qualifies the exact Global Index service on an isolated task-owned NTFS and USN volume", () => {
    expect(classifierSource).toContain("windows_global_index_service_qualification_changed");
    const qualification = section(
      interactiveWorkflow,
      "windows-global-index-service-qualification",
      "rust-macos",
    );
    expect(qualification).toContain("needs.change-scope.outputs.windows_global_index_service_qualification_changed == 'true'");
    expect(qualification).toContain("runs-on: windows-latest");
    expect(qualification).toContain("timeout-minutes: 60");
    expect(qualification).toContain("npm ci");
    expect(qualification).toContain("npm run build:frontend");
    expect(qualification).toContain("--features \"desktop-runtime native-qa\" --bin zen-canvas");
    expect(qualification).toContain("--bin zb05-global-index-qa");
    expect(qualification).toContain("scripts/qualifyWindowsGlobalIndexService.ps1");
    expect(qualification).toContain("if: ${{ always() }}");
    expect(interactiveWorkflow).toContain("windows-global-index-service-qualification.result");
    expect(interactiveWorkflow).toContain("needs.windows-global-index-service-qualification");
    expect(interactiveWorkflow).toContain("GLOBAL_INDEX_SERVICE_EXPECTED");
    expect(interactiveWorkflow).toContain("check_expected \"$GLOBAL_INDEX_SERVICE_EXPECTED\" \"$GLOBAL_INDEX_SERVICE\" windows-global-index-service-qualification");

    expect(globalIndexServiceQualification).toContain('"create vdisk file=`"$qualificationVhdPath`" maximum=512 type=expandable"');
    expect(globalIndexServiceQualification).toContain("fsutil.exe usn createjournal m=16777216 a=2097152");
    expect(globalIndexServiceQualification).toContain("qualificationVolumeKind = \"task-owned-vhdx\"");
    expect(globalIndexServiceQualification).toContain("Initialize-QualificationProfile");
    expect(globalIndexServiceQualification).toContain("expectedQualificationSourceId");
    expect(globalIndexServiceQualification).toContain("enabledSourceCountAfterDiscovery");
    expect(globalIndexServiceQualification).not.toContain("$originalLocalAppData");
    expect(globalIndexServiceQualification).not.toContain("$originalUserProfile");
    expect(globalIndexProbeSource).toContain("--prepare-profile");
    expect(globalIndexProbeSource).toContain("source.volume.enabled = is_target");
    expect(globalIndexProbeSource).toContain("database.upsert_global_volume(&source.volume)?");
    expect(globalIndexProbeSource).toContain("enabled_sources.len() != 1");

    const fixtureCreated = globalIndexServiceQualification.indexOf("created before the Global Index baseline");
    const journalProbe = globalIndexServiceQualification.indexOf("fsutil.exe usn queryjournal");
    const preexistingGuard = globalIndexServiceQualification.indexOf("refusing to replace or manage a pre-existing");
    const serviceCreated = globalIndexServiceQualification.indexOf('Invoke-ServiceControl @("create", $serviceName');
    expect(fixtureCreated).toBeGreaterThanOrEqual(0);
    expect(journalProbe).toBeGreaterThanOrEqual(0);
    expect(journalProbe).toBeLessThan(fixtureCreated);
    expect(preexistingGuard).toBeGreaterThanOrEqual(0);
    expect(serviceCreated).toBeGreaterThan(fixtureCreated);
    expect(serviceCreated).toBeGreaterThan(preexistingGuard);
    expect(globalIndexServiceQualification.indexOf("    New-QualificationVolume")).toBeLessThan(fixtureCreated);
    expect(globalIndexServiceQualification.indexOf("    Initialize-QualificationProfile")).toBeLessThan(serviceCreated);
    expect(globalIndexServiceQualification).toContain("sameImage");
    expect(globalIndexServiceQualification).toContain("baselineEntryCount");
    expect(globalIndexServiceQualification).toContain("$baselineTimeoutMinutes = 5");
    expect(globalIndexServiceQualification).toContain("baselineWaitMs");
    expect(globalIndexServiceQualification).toContain("baselineFixtureSearchFound");
    const baselinePollingStart = globalIndexServiceQualification.indexOf("$baselineDeadline =");
    const baselinePollingEnd = globalIndexServiceQualification.indexOf("$script:evidence.baselineWaitMs =");
    const baselinePolling = globalIndexServiceQualification.slice(baselinePollingStart, baselinePollingEnd);
    expect(baselinePolling).toContain("$baselineSnapshot = Get-ProbeSnapshot");
    expect(baselinePolling).not.toContain("Get-ProbeSnapshot $preexistingToken");
    expect(globalIndexServiceQualification).toContain('$preexistingName = "$preexistingToken.txt"');
    const completedBaselineGuard = globalIndexServiceQualification.indexOf("if ([long]$baselineSource.entryCount -le 0)");
    const preexistingSearch = globalIndexServiceQualification.indexOf("$baselineSnapshot = Get-ProbeSnapshot $preexistingName");
    expect(preexistingSearch).toBeGreaterThan(completedBaselineGuard);
    expect(globalIndexServiceQualification).toContain("createLatencyMs");
    expect(globalIndexServiceQualification).toContain("renameLatencyMs");
    expect(globalIndexServiceQualification).toContain("deleteLatencyMs");
    expect(globalIndexServiceQualification).toContain("settledCoordinatorCycleDelta");
    expect(globalIndexServiceQualification).toContain("settledCoordinatorWaitDelta");
    expect(globalIndexServiceQualification).toContain("LastCoordinatorEvent");
    expect(globalIndexServiceQualification).toContain('$counts.LastCoordinatorEvent -ceq "coordinator_wait"');
    expect(globalIndexServiceQualification).not.toContain('$counts.Lines[-1] -ceq "coordinator_wait"');
    expect(globalIndexServiceQualification).toContain("finally {");
    expect(globalIndexServiceQualification).toContain("CreateService failed; exact SCM output");
    expect(globalIndexServiceQualification).toContain("service-created-by-qualification.txt");
    expect(globalIndexServiceQualification).toContain("usnJournalProbes");
    expect(globalIndexServiceQualification).toContain("fixtureRootRemoved");
    expect(globalIndexServiceQualification).toContain("qualificationVolumeDetached");
    expect(globalIndexServiceQualification).toContain("qualificationVhdRemoved");
    expect(qualification).toContain("fixture-root-created-by-qualification.txt");
    expect(qualification).toContain("leaving unexpected qualification fixture root untouched");
    expect(qualification).toContain("qualification-volume-created-by-task.txt");
    expect(qualification).toContain("qualification-volume-drive-letter.txt");
    expect(qualification).toContain("Dismount-DiskImage -ImagePath $qualificationVhdPath");
    expect(qualification.indexOf("Stop-Process -Id $process.ProcessId -Force")).toBeLessThan(qualification.indexOf("Dismount-DiskImage -ImagePath $qualificationVhdPath"));
  });

  it("pins actions and keeps packaging and quality checks authoritative", () => {
    for (const workflow of [interactiveWorkflow, fullWorkflow, releaseWorkflow]) {
      expect(workflow).toContain("actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7");
      expect(workflow).toContain("actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7");
      expect(workflow).not.toContain("sccache");
    }
    expect(fullWorkflow).toContain("name: Package NSIS");
    expect(fullWorkflow).toContain("name: Package unsigned DMG");
    expect(interactiveWorkflow).toContain("Package metadata smoke");
  });

  it("bounds every executable workflow job with an explicit timeout budget", () => {
    for (const [workflowName, workflow] of [
      ["Interactive", interactiveWorkflow],
      ["Full", fullWorkflow],
    ] as const) {
      const jobs = parseWorkflowJobs(workflow);
      expect(jobs.length, workflowName).toBeGreaterThan(0);
      for (const job of jobs) {
        if (job.runsOn === null) continue;
        expect(Number.isInteger(job.timeoutMinutes), `${workflowName}/${job.id}`).toBe(true);
        expect(job.timeoutMinutes, `${workflowName}/${job.id}`).toBeGreaterThanOrEqual(5);
        expect(job.timeoutMinutes, `${workflowName}/${job.id}`).toBeLessThanOrEqual(60);
      }

      const frontend = jobs.find((job) => job.id === "frontend-quality");
      expect(frontend, `${workflowName}/frontend-quality`).toBeDefined();
      expect(frontend?.timeoutMinutes, `${workflowName}/frontend-quality`).toBe(20);
    }
  });

  it("bounds Playwright dependency and browser installation with finite retries", () => {
    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      const frontend = section(workflow, "frontend-quality", "rust-windows");
      const dependencies = workflowStep(
        frontend,
        "Install Chromium system dependencies",
        "Install Chromium browser",
      );
      const browser = workflowStep(
        frontend,
        "Install Chromium browser",
        "Frontend tests and architecture checks",
      );

      expect(dependencies).toContain("shell: bash");
      expect(dependencies).toContain("max_attempts=2");
      expect(dependencies).toContain("per_attempt_timeout=7m");
      expect(dependencies).toContain("backoff_seconds=10");
      expect(dependencies).toContain("for attempt in $(seq 1 \"$max_attempts\"); do");
      expect(dependencies).toContain(
        "timeout --signal=TERM --kill-after=30s \"$per_attempt_timeout\" npx playwright install-deps chromium",
      );
      expect(dependencies).toMatch(
        /if timeout --signal=TERM --kill-after=30s "\$per_attempt_timeout" npx playwright install-deps chromium; then\s+status=0\s+else\s+status=\$\?\s+fi/u,
      );
      expect(dependencies).toContain("cleanup_apt_processes()");
      expect(dependencies).toContain("pgrep -x apt-get");
      expect(dependencies).toContain('sudo -n kill -TERM "$pid"');
      expect(dependencies).toContain('sudo -n kill -KILL "$pid"');
      expect(dependencies).toContain("[playwright-deps] attempt=");
      expect(dependencies).toContain("[playwright-deps] PASS attempt=");
      expect(dependencies).toContain("[playwright-deps] TIMEOUT attempt=");
      expect(dependencies).toContain("[playwright-deps] exhausted retries");
      expect(dependencies).toContain('if [ "$attempt" -eq "$max_attempts" ]; then');
      expect(dependencies).toContain('exit "$status"');
      expect(dependencies).toContain('sleep "$backoff_seconds"');
      expect(dependencies).not.toContain("|| true");
      expect(dependencies).not.toContain("while true");

      expect(browser).toContain("shell: bash");
      expect(browser).toContain("max_attempts=2");
      expect(browser).toContain("per_attempt_timeout=5m");
      expect(browser).toContain("for attempt in $(seq 1 \"$max_attempts\"); do");
      expect(browser).toContain(
        "timeout --signal=TERM --kill-after=30s \"$per_attempt_timeout\" npx playwright install chromium",
      );
      expect(browser).toMatch(
        /if timeout --signal=TERM --kill-after=30s "\$per_attempt_timeout" npx playwright install chromium; then\s+status=0\s+else\s+status=\$\?\s+fi/u,
      );
      expect(browser).toContain("[playwright-browser] exhausted retries");
      expect(browser).toContain('if [ "$attempt" -eq "$max_attempts" ]; then');
      expect(browser).toContain('exit "$status"');
      expect(browser).not.toContain("|| true");
      expect(browser).not.toContain("while true");
    }
  });

  it("runs the committed W2-01 real-browser gate with a lockfile-keyed Chromium cache", () => {
    for (const workflow of [interactiveWorkflow, fullWorkflow]) {
      const frontend = section(workflow, "frontend-quality", "rust-windows");
      expect(frontend).toContain("PLAYWRIGHT_BROWSERS_PATH");
      expect(frontend).toContain("id: playwright-browser-cache");
      expect(frontend).toContain("actions/cache@5a3ec84eff668545956fd18022155c47e93e2684 # v4.2.3");
      expect(frontend).toContain("zen-canvas-playwright-${{ runner.os }}-${{ runner.arch }}-${{ hashFiles('package-lock.json') }}");
      expect(frontend).toContain("npx playwright install-deps chromium");
      expect(frontend).toContain("if: steps.playwright-browser-cache.outputs.cache-hit != 'true'");
      expect(frontend).toContain(
        "timeout --signal=TERM --kill-after=30s \"$per_attempt_timeout\" npx playwright install chromium",
      );
      expect(frontend).not.toContain("npx playwright install --with-deps chromium");
      expect(frontend).toContain("W2-01 real browser regression gate");
      expect(frontend).toContain("W201_SOURCE_HEAD: ${{ github.event.pull_request.head.sha || github.sha }}");
      expect(frontend).toContain("npm run test:browser:w2-01:real");
      expect(frontend).toContain("w2-01-browser-gate-failure");
      expect(frontend).toContain("W2-10 interaction accessibility responsive browser gate");
      expect(frontend).toContain("npm run test:browser:w2-10:real");
      expect(frontend).toContain("w2-10-browser-gate-failure");
      expect(frontend).toContain("W2-11 integrated experience performance browser gate");
      expect(frontend).toContain("npm run test:browser:w2-11:real");
      expect(frontend).toContain("w2-11-browser-gate-failure");
    }
  });
  it("runs U3/U4 mock product gates with exact candidate identity and W3-10 only in Full", () => {
    const interactive = section(interactiveWorkflow, "frontend-quality", "rust-windows");
    const full = section(fullWorkflow, "frontend-quality", "rust-windows");
    for (const [id, script] of [["U3", "u3"], ["U4", "u4"]]) {
      for (const frontend of [interactive, full]) {
        expect(frontend).toContain(`run: npm run test:browser:${script}`);
        expect(frontend).toContain(`${id}_EXPECTED_CHECKOUT_SHA:`);
        expect(frontend).toContain(`name: ${script}-browser-failure`);
      }
      expect(interactive).toContain(id + "_EXPECTED_CHECKOUT_SHA: ${{ matrix.validation_lane == 'head_validation' && github.event.pull_request.head.sha || github.sha }}");
      expect(full).toContain(id + "_EXPECTED_CHECKOUT_SHA: ${{ github.sha }}");
      const source = readFileSync(`scripts/run${id}FileLibraryBrowser.mjs`, "utf8");
      expect(source).toContain('port: 0, strictPort: false');
      expect(source).toContain("assertCheckoutEvidence(expectedCheckoutSha, actualCheckoutSha)");
      expect(source).toContain("server.resolvedUrls?.local");
      expect(source).not.toMatch(/127\.0\.0\.1:517[45]/u);
    }
    expect(interactive).not.toContain("npm run test:browser:w3-10:real");
    expect(full).toContain("npm run test:browser:w3-10:real");
    expect(full).toContain("W310_EXPECTED_CHECKOUT_SHA: ${{ github.sha }}");
  });

});
