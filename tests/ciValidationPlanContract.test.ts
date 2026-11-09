import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { classifyCiScope } from "../scripts/classifyCiChanges.mjs";
import {
  buildValidationPlan,
  evaluateValidationAggregate,
  readValidationLaneJobResults,
  summarizeValidationLaneResults,
  validationLaneResultsForPlan,
  type ValidationLane,
} from "../scripts/ciValidationPlan.mjs";

const BASE_COMMIT = "1".repeat(40);
const HEAD_COMMIT = "2".repeat(40);
const MERGE_COMMIT = "3".repeat(40);
const HEAD_TREE = "4".repeat(40);
const INTEGRATION_TREE = "5".repeat(40);

function readWorkflow(relativePath: string) {
  return readFileSync(relativePath, "utf8").replace(/\r\n?/gu, "\n");
}

function workflowJobBlock(workflow: string, jobId: string) {
  const lines = workflow.split("\n");
  const start = lines.findIndex((line) => line === "  " + jobId + ":");
  if (start < 0) throw new Error("workflow job not found: " + jobId);
  const nextJob = lines.findIndex((line, index) => index > start && /^  [a-z0-9-]+:$/u.test(line));
  return lines.slice(start, nextJob < 0 ? undefined : nextJob).join("\n");
}

function pullRequestInput(overrides: Record<string, unknown> = {}) {
  return {
    eventName: "pull_request",
    sourceEvidenceResult: "success",
    changeScopeResult: "success",
    headCheckoutSha: HEAD_COMMIT,
    headTreeSha: HEAD_TREE,
    integrationCheckoutSha: MERGE_COMMIT,
    integrationTreeSha: INTEGRATION_TREE,
    prHeadSha: HEAD_COMMIT,
    eventSha: MERGE_COMMIT,
    ...overrides,
  };
}

function aggregateInput(plan: ReturnType<typeof buildValidationPlan>, overrides: Record<string, unknown> = {}) {
  return {
    eventName: "pull_request",
    planResult: "success",
    planValid: plan.plan_valid,
    treeEquivalent: plan.tree_equivalent,
    headValidationRequired: plan.head_validation_required,
    validationLanes: plan.validation_lanes,
    laneJobResult: null,
    laneValidationRequired: true,
    headValidationResult: plan.tree_equivalent ? null : "success",
    integrationValidationResult: "success",
    ...overrides,
  };
}

describe("tree-equivalence validation plan behavior", () => {
  it("uses one substantive lane when different commits have the same tree", () => {
    const plan = buildValidationPlan(pullRequestInput({
      headTreeSha: HEAD_TREE,
      integrationTreeSha: HEAD_TREE,
    }));

    expect(plan.plan_valid).toBe(true);
    expect(plan.head_checkout_sha).toBe(HEAD_COMMIT);
    expect(plan.integration_checkout_sha).toBe(MERGE_COMMIT);
    expect(plan.head_checkout_sha).not.toBe(plan.integration_checkout_sha);
    expect(plan.tree_equivalent).toBe(true);
    expect(plan.head_validation_required).toBe(false);
    expect(plan.validation_lanes).toEqual(["merge_integration"]);
    expect(evaluateValidationAggregate(aggregateInput(plan))).toMatchObject({ pass: true });
    expect(evaluateValidationAggregate(aggregateInput(plan, {
      integrationValidationResult: null,
    })).pass).toBe(false);
    expect(evaluateValidationAggregate(aggregateInput(plan, {
      integrationValidationResult: "skipped",
    })).pass).toBe(false);
  });

  it("requires exact-head validation when the trees differ", () => {
    const plan = buildValidationPlan(pullRequestInput());

    expect(plan.tree_equivalent).toBe(false);
    expect(plan.head_validation_required).toBe(true);
    expect(plan.validation_lanes).toEqual(["head_validation", "merge_integration"]);
  });

  it("fails the aggregate when merge succeeds but the required head lane fails", () => {
    const plan = buildValidationPlan(pullRequestInput());
    const result = evaluateValidationAggregate(aggregateInput(plan, {
      laneJobResult: null,
      headValidationResult: "failure",
      integrationValidationResult: "success",
    }));

    expect(result.pass).toBe(false);
    expect(result.reason).toMatch(/head validation/i);
  });

  it.each([
    ["head lane missing", null, "success"],
    ["integration lane missing", "success", null],
    ["head lane failed", "failure", "success"],
    ["integration lane failed", "success", "failure"],
    ["head lane cancelled", "cancelled", "success"],
    ["integration lane cancelled", "success", "cancelled"],
    ["head lane skipped", "skipped", "success"],
    ["integration lane skipped", "success", "skipped"],
  ])("fails closed when %s", (_case, headValidationResult, integrationValidationResult) => {
    const plan = buildValidationPlan(pullRequestInput());
    const result = evaluateValidationAggregate(aggregateInput(plan, {
      laneJobResult: "success",
      headValidationResult,
      integrationValidationResult,
    }));

    expect(result.pass).toBe(false);
    expect(result.reason).toMatch(/head validation|merge-integration/i);
  });

  it("fails closed when source evidence or the head lane is missing", () => {
    const invalidPlan = buildValidationPlan(pullRequestInput({
      sourceEvidenceResult: "failure",
      headCheckoutSha: undefined,
      headTreeSha: undefined,
    }));
    expect(invalidPlan.plan_valid).toBe(false);
    expect(evaluateValidationAggregate({
      eventName: "pull_request",
      planResult: "failure",
      treeEquivalent: false,
      headValidationRequired: true,
      validationLanes: ["head_validation", "merge_integration"],
    }).pass).toBe(false);
  });

  it("allows the non-equivalent aggregate only when both lanes succeed", () => {
    const plan = buildValidationPlan(pullRequestInput());
    expect(evaluateValidationAggregate(aggregateInput(plan, {
      laneJobResult: "success",
      headValidationResult: "success",
      integrationValidationResult: "success",
    })).pass).toBe(true);
  });

  it("applies the same two-lane contract to docs-only changes", () => {
    const plan = buildValidationPlan(pullRequestInput());
    expect(evaluateValidationAggregate(aggregateInput(plan, {
      laneJobResult: "success",
      headValidationResult: "success",
      integrationValidationResult: "success",
    })).pass).toBe(true);
  });

  it("keeps commit identity and tree identity as separate fields", () => {
    const plan = buildValidationPlan(pullRequestInput({
      headTreeSha: HEAD_TREE,
      integrationTreeSha: HEAD_TREE,
    }));

    expect(plan.head_checkout_sha).toBe(HEAD_COMMIT);
    expect(plan.integration_checkout_sha).toBe(MERGE_COMMIT);
    expect(plan.head_tree_sha).toBe(HEAD_TREE);
    expect(plan.integration_tree_sha).toBe(HEAD_TREE);
    expect(plan.tree_equivalent).toBe(true);
  });

  it("keeps frontend and high-risk routing obligations on both unequal-tree lanes", () => {
    const scope = classifyCiScope({
      event: "pull_request",
      changedPaths: ["src/App.tsx", "src-tauri/src/file_ops.rs"],
    });
    const plan = buildValidationPlan(pullRequestInput());

    expect(scope.frontend_changed).toBe(true);
    expect(scope.high_risk).toBe(true);
    expect(plan.tree_equivalent).toBe(false);
    expect(plan.validation_lanes).toEqual(["head_validation", "merge_integration"]);
  });

  it("routes root Rust toolchain changes through Rust validation", () => {
    const scope = classifyCiScope({
      event: "pull_request",
      changedPaths: ["rust-toolchain.toml"],
    });

    expect(scope.docs_only).toBe(false);
    expect(scope.rust_changed).toBe(true);
    expect(scope.macos_sensitive).toBe(true);
    expect(scope.release_sensitive).toBe(true);
  });

  it("keeps performance-sensitive routing obligations on both unequal-tree lanes", () => {
    const scope = classifyCiScope({
      event: "pull_request",
      changedPaths: ["src-tauri/src/file_workspace/browse/mod.rs"],
    });
    const plan = buildValidationPlan(pullRequestInput());

    expect(scope.performance_sensitive).toBe(true);
    expect(scope.performance_any).toBe(true);
    expect(plan.head_validation_required).toBe(true);
    expect(plan.validation_lanes).toEqual(["head_validation", "merge_integration"]);
  });

  it("keeps scheduled and manual Full Validation as one immutable event lane", () => {
    const plan = buildValidationPlan({
      eventName: "workflow_dispatch",
      validationLane: "manual_full_validation",
      sourceEvidenceResult: "success",
      eventCheckoutSha: HEAD_COMMIT,
      eventTreeSha: HEAD_TREE,
    });

    expect(plan.plan_valid).toBe(true);
    expect(plan.tree_equivalent).toBeNull();
    expect(plan.validation_lanes).toEqual(["manual_full_validation"]);
  });
});


describe("actual validation lane result collection", () => {
  const lanes: ValidationLane[] = ["head_validation", "merge_integration"];
  const expectations = { "frontend-quality": true };
  const laneResult = (
    lane: ValidationLane,
    result: string,
    jobId = "frontend-quality",
    runAttempt: string | number = "1",
  ) => {
    const normalizedAttempt = String(runAttempt);
    return {
      job_id: jobId,
      lane,
      run_attempt: normalizedAttempt,
      result,
      artifact_name: `ci-lane-result-${normalizedAttempt}-${jobId}-${lane}`,
    };
  };

  const summarize = (
    results: ReturnType<typeof laneResult>[],
    laneExpectations: Record<string, boolean | "true" | "false"> = expectations,
    validationLanes: ValidationLane[] = lanes,
    currentAttempt: string | number = "1",
  ) => summarizeValidationLaneResults(results, validationLanes, laneExpectations, currentAttempt);

  it("projects only the lanes required by the plan", () => {
    expect(validationLaneResultsForPlan(["merge_integration"], {
      merge_integration: "success",
    })).toEqual({
      headValidationResult: null,
      integrationValidationResult: "success",
    });
    expect(validationLaneResultsForPlan(lanes, {
      head_validation: "success",
      merge_integration: "success",
    })).toEqual({
      headValidationResult: "success",
      integrationValidationResult: "success",
    });
  });

  it("passes only when both non-equivalent lane artifacts report SUCCESS", () => {
    const plan = buildValidationPlan(pullRequestInput());
    const results = summarizeValidationLaneResults([
      laneResult("head_validation", "success"),
      laneResult("merge_integration", "success"),
    ], lanes, expectations, "1");

    expect(results).toEqual({
      head_validation: "success",
      merge_integration: "success",
    });
    const projected = validationLaneResultsForPlan(lanes, results);
    expect(evaluateValidationAggregate(aggregateInput(plan, projected)).pass).toBe(true);
  });

  it("passes equivalent trees from the integration artifact alone", () => {
    const plan = buildValidationPlan(pullRequestInput({
      headTreeSha: HEAD_TREE,
      integrationTreeSha: HEAD_TREE,
    }));
    const results = summarizeValidationLaneResults([
      laneResult("merge_integration", "success"),
    ], plan.validation_lanes, expectations, "1");
    const projected = validationLaneResultsForPlan(plan.validation_lanes, results);

    expect(results).toEqual({ merge_integration: "success" });
    expect(projected.headValidationResult).toBeNull();
    expect(evaluateValidationAggregate(aggregateInput(plan, projected)).pass).toBe(true);
  });

  it("reads the sole merge-integration artifact from the download root and verifies its identity", () => {
    const plan = buildValidationPlan(pullRequestInput({
      headTreeSha: HEAD_TREE,
      integrationTreeSha: HEAD_TREE,
    }));
    const artifactRoot = mkdtempSync(join(tmpdir(), "ci-lane-single-root-"));
    const record = laneResult("merge_integration", "success", "docs-only-lanes", "1");
    writeFileSync(join(artifactRoot, "ci-lane-result.json"), JSON.stringify(record));

    try {
      const readResults = readValidationLaneJobResults(artifactRoot, "1");
      expect(readResults).toEqual([record]);
      const laneResults = summarizeValidationLaneResults(
        readResults,
        plan.validation_lanes,
        { "docs-only-lanes": true },
        "1",
      );
      expect(laneResults).toEqual({ merge_integration: "success" });
      expect(evaluateValidationAggregate(aggregateInput(plan,
        validationLaneResultsForPlan(plan.validation_lanes, laneResults),
      )).pass).toBe(true);
    } finally {
      rmSync(artifactRoot, { recursive: true, force: true });
    }
  });

  it.each([
    ["two named lane artifacts pass only when both lanes succeed", "success", "success", true],
    ["head success and merge failure fail", "success", "failure", false],
    ["head failure and merge success fail", "failure", "success", false],
  ])("reads non-equivalent lane artifacts: %s", (_case, headResult, integrationResult, expectedPass) => {
    const plan = buildValidationPlan(pullRequestInput());
    const artifactRoot = mkdtempSync(join(tmpdir(), "ci-lane-multiple-"));
    const records = [
      laneResult("head_validation", headResult, "frontend-quality", "1"),
      laneResult("merge_integration", integrationResult, "frontend-quality", "1"),
    ];
    for (const record of records) {
      const artifactDirectory = join(artifactRoot, record.artifact_name);
      mkdirSync(artifactDirectory, { recursive: true });
      writeFileSync(join(artifactDirectory, "ci-lane-result.json"), JSON.stringify(record));
    }

    try {
      const readResults = readValidationLaneJobResults(artifactRoot, "1");
      const laneResults = summarizeValidationLaneResults(readResults, plan.validation_lanes, expectations, "1");
      expect(laneResults).toEqual({
        head_validation: headResult,
        merge_integration: integrationResult,
      });
      expect(evaluateValidationAggregate(aggregateInput(plan,
        validationLaneResultsForPlan(plan.validation_lanes, laneResults),
      )).pass).toBe(expectedPass);
    } finally {
      rmSync(artifactRoot, { recursive: true, force: true });
    }
  });

  it("fails a non-equivalent aggregate when the merge artifact is absent", () => {
    const plan = buildValidationPlan(pullRequestInput());
    const artifactRoot = mkdtempSync(join(tmpdir(), "ci-lane-missing-merge-"));
    const record = laneResult("head_validation", "success", "frontend-quality", "1");
    const artifactDirectory = join(artifactRoot, record.artifact_name);
    mkdirSync(artifactDirectory, { recursive: true });
    writeFileSync(join(artifactDirectory, "ci-lane-result.json"), JSON.stringify(record));

    try {
      const laneResults = summarizeValidationLaneResults(
        readValidationLaneJobResults(artifactRoot, "1"),
        plan.validation_lanes,
        expectations,
        "1",
      );
      expect(laneResults.merge_integration).toBe("missing");
      expect(evaluateValidationAggregate(aggregateInput(plan,
        validationLaneResultsForPlan(plan.validation_lanes, laneResults),
      )).pass).toBe(false);
    } finally {
      rmSync(artifactRoot, { recursive: true, force: true });
    }
  });

  it("fails a docs-only aggregate when the sole merge-integration result is absent", () => {
    const plan = buildValidationPlan(pullRequestInput({
      headTreeSha: HEAD_TREE,
      integrationTreeSha: HEAD_TREE,
    }));
    const artifactRoot = mkdtempSync(join(tmpdir(), "ci-lane-single-missing-"));

    try {
      const laneResults = summarizeValidationLaneResults(
        readValidationLaneJobResults(artifactRoot, "1"),
        plan.validation_lanes,
        { "docs-only-lanes": true },
        "1",
      );
      expect(laneResults).toEqual({ merge_integration: "missing" });
      expect(evaluateValidationAggregate(aggregateInput(plan,
        validationLaneResultsForPlan(plan.validation_lanes, laneResults),
      )).pass).toBe(false);
    } finally {
      rmSync(artifactRoot, { recursive: true, force: true });
    }
  });

  it("rejects a sole root artifact whose embedded artifact name disagrees with its metadata", () => {
    const artifactRoot = mkdtempSync(join(tmpdir(), "ci-lane-single-root-mismatch-"));
    const record = {
      ...laneResult("merge_integration", "success", "docs-only-lanes", "1"),
      artifact_name: "ci-lane-result-1-other-job-merge_integration",
    };
    writeFileSync(join(artifactRoot, "ci-lane-result.json"), JSON.stringify(record));

    try {
      expect(() => readValidationLaneJobResults(artifactRoot, "1"))
        .toThrow(/artifact name does not match/u);
    } finally {
      rmSync(artifactRoot, { recursive: true, force: true });
    }
  });

  it("rejects a sole root artifact that omits its artifact identity", () => {
    const artifactRoot = mkdtempSync(join(tmpdir(), "ci-lane-single-root-no-name-"));
    const record = {
      job_id: "docs-only-lanes",
      lane: "merge_integration",
      run_attempt: "1",
      result: "success",
    };
    writeFileSync(join(artifactRoot, "ci-lane-result.json"), JSON.stringify(record));

    try {
      expect(() => readValidationLaneJobResults(artifactRoot, "1"))
        .toThrow(/artifact name does not match/u);
    } finally {
      rmSync(artifactRoot, { recursive: true, force: true });
    }
  });

  it("fails closed for malformed JSON and mixed single/multiple artifact layouts", () => {
    const malformedRoot = mkdtempSync(join(tmpdir(), "ci-lane-malformed-root-"));
    writeFileSync(join(malformedRoot, "ci-lane-result.json"), "{");
    try {
      expect(() => readValidationLaneJobResults(malformedRoot, "1")).toThrow();
    } finally {
      rmSync(malformedRoot, { recursive: true, force: true });
    }

    const mixedRoot = mkdtempSync(join(tmpdir(), "ci-lane-mixed-layout-"));
    const record = laneResult("merge_integration", "success", "docs-only-lanes", "1");
    writeFileSync(join(mixedRoot, "ci-lane-result.json"), JSON.stringify(record));
    const artifactDirectory = join(mixedRoot, record.artifact_name);
    mkdirSync(artifactDirectory, { recursive: true });
    writeFileSync(join(artifactDirectory, "ci-lane-result.json"), JSON.stringify(record));
    try {
      expect(() => readValidationLaneJobResults(mixedRoot, "1"))
        .toThrow(/layout is ambiguous/u);
    } finally {
      rmSync(mixedRoot, { recursive: true, force: true });
    }
  });

  it.each([
    ["head missing", [laneResult("merge_integration", "success")], "missing", "success"],
    ["integration missing", [laneResult("head_validation", "success")], "success", "missing"],
    ["head failure", [laneResult("head_validation", "failure"), laneResult("merge_integration", "success")], "failure", "success"],
    ["integration failure", [laneResult("head_validation", "success"), laneResult("merge_integration", "failure")], "success", "failure"],
    ["head cancelled", [laneResult("head_validation", "cancelled"), laneResult("merge_integration", "success")], "cancelled", "success"],
    ["integration cancelled", [laneResult("head_validation", "success"), laneResult("merge_integration", "cancelled")], "success", "cancelled"],
    ["head skipped", [laneResult("head_validation", "skipped"), laneResult("merge_integration", "success")], "skipped", "success"],
    ["integration skipped", [laneResult("head_validation", "success"), laneResult("merge_integration", "skipped")], "success", "skipped"],
    ["head incomplete", [laneResult("head_validation", "in_progress"), laneResult("merge_integration", "success")], "in_progress", "success"],
  ])("fails closed when %s", (_case, artifacts, expectedHead, expectedIntegration) => {
    const results = summarizeValidationLaneResults(artifacts, lanes, expectations, "1");
    expect(results).toEqual({
      head_validation: expectedHead,
      merge_integration: expectedIntegration,
    });

    const plan = buildValidationPlan(pullRequestInput());
    const projected = validationLaneResultsForPlan(lanes, results);
    expect(evaluateValidationAggregate(aggregateInput(plan, {
      ...projected,
    })).pass).toBe(false);
  });

  it("fails closed when a required matrix lane artifact is absent", () => {
    expect(summarize([
      laneResult("merge_integration", "success"),
    ]).head_validation).toBe("missing");
  });

  it("fails on duplicate records for the same job, lane, and attempt", () => {
    expect(() => summarize([
      laneResult("head_validation", "success"),
      laneResult("head_validation", "success"),
      laneResult("merge_integration", "success"),
    ])).toThrow(/duplicate lane result record/u);
  });

  it("reads sanitized records from all available attempts and carries forward earlier successes", () => {
    const artifactRoot = mkdtempSync(join(tmpdir(), "ci-lane-results-"));
    const records = [
      laneResult("merge_integration", "success", "frontend-quality", "1"),
      laneResult("merge_integration", "success", "performance-search", "1"),
      laneResult("merge_integration", "success", "performance-workspace-foundation", "1"),
      laneResult("merge_integration", "success", "performance-workspace-foundation", "2"),
    ];
    for (const record of records) {
      const artifactName = `ci-lane-result-${record.run_attempt}-${record.job_id}-${record.lane}`;
      const artifactDirectory = join(artifactRoot, artifactName);
      mkdirSync(artifactDirectory, { recursive: true });
      writeFileSync(join(artifactDirectory, "ci-lane-result.json"), JSON.stringify(record));
    }

    try {
      const readResults = readValidationLaneJobResults(artifactRoot, "2");
      expect(readResults).toEqual(records);
      expect(summarizeValidationLaneResults(readResults, ["merge_integration"], {
        "frontend-quality": true,
        "performance-search": true,
        "performance-workspace-foundation": true,
      }, "2")).toEqual({ merge_integration: "success" });
      expect(() => readValidationLaneJobResults(artifactRoot, "1")).toThrow(/future attempt/u);
    } finally {
      rmSync(artifactRoot, { recursive: true, force: true });
    }
  });

  it("uses the latest attempt when a failure is repaired", () => {
    expect(summarize([
      laneResult("merge_integration", "failure", "performance-workspace-foundation", "1"),
      laneResult("merge_integration", "success", "performance-workspace-foundation", "2"),
    ], { "performance-workspace-foundation": true }, ["merge_integration"], "2"))
      .toEqual({ merge_integration: "success" });
  });

  it("does not fall back to an earlier success after a rerun regresses", () => {
    const plan = buildValidationPlan(pullRequestInput({
      headTreeSha: HEAD_TREE,
      integrationTreeSha: HEAD_TREE,
    }));
    const results = summarize([
      laneResult("merge_integration", "success", "performance-workspace-foundation", "1"),
      laneResult("merge_integration", "failure", "performance-workspace-foundation", "2"),
    ], { "performance-workspace-foundation": true }, plan.validation_lanes, "2");
    const projected = validationLaneResultsForPlan(plan.validation_lanes, results);

    expect(results).toEqual({ merge_integration: "failure" });
    expect(evaluateValidationAggregate(aggregateInput(plan, projected)).pass).toBe(false);
  });

  it("inherits an earlier successful required job when the current attempt did not rerun it", () => {
    expect(summarize([
      laneResult("merge_integration", "success", "frontend-quality", "1"),
    ], { "frontend-quality": true }, ["merge_integration"], "2"))
      .toEqual({ merge_integration: "success" });
  });

  it("selects the newest evidence independently for each lane in a non-equivalent tree", () => {
    const plan = buildValidationPlan(pullRequestInput());
    const required = {
      "frontend-quality": true,
      "performance-search": true,
      "performance-workspace-foundation": true,
    };
    const results = summarize([
      laneResult("head_validation", "success", "frontend-quality", "1"),
      laneResult("merge_integration", "success", "frontend-quality", "1"),
      laneResult("head_validation", "success", "performance-search", "1"),
      laneResult("merge_integration", "success", "performance-search", "1"),
      laneResult("head_validation", "success", "performance-workspace-foundation", "1"),
      laneResult("merge_integration", "failure", "performance-workspace-foundation", "1"),
      laneResult("head_validation", "success", "performance-search", "2"),
      laneResult("merge_integration", "success", "performance-workspace-foundation", "2"),
    ], required, plan.validation_lanes, "2");

    expect(plan.validation_lanes).toEqual(["head_validation", "merge_integration"]);
    expect(results).toEqual({
      head_validation: "success",
      merge_integration: "success",
    });
    expect(evaluateValidationAggregate(aggregateInput(
      plan,
      validationLaneResultsForPlan(plan.validation_lanes, results),
    )).pass).toBe(true);
  });

  it("keeps equivalent-tree aggregation integration-only across a partial rerun", () => {
    const plan = buildValidationPlan(pullRequestInput({
      headTreeSha: HEAD_TREE,
      integrationTreeSha: HEAD_TREE,
    }));
    const results = summarize([
      laneResult("merge_integration", "success", "frontend-quality", "1"),
      laneResult("merge_integration", "failure", "performance-workspace-foundation", "1"),
      laneResult("merge_integration", "success", "performance-workspace-foundation", "2"),
    ], {
      "frontend-quality": true,
      "performance-workspace-foundation": true,
    }, plan.validation_lanes, "2");

    expect(plan.validation_lanes).toEqual(["merge_integration"]);
    expect(results).toEqual({ merge_integration: "success" });
    expect(evaluateValidationAggregate(aggregateInput(
      plan,
      validationLaneResultsForPlan(plan.validation_lanes, results),
    )).pass).toBe(true);
  });

  it.each(["failure", "cancelled", "skipped", "in_progress", "unavailable"])(
    "does not let an earlier success override the latest %s result",
    (latestResult) => {
      const plan = buildValidationPlan(pullRequestInput({
        headTreeSha: HEAD_TREE,
        integrationTreeSha: HEAD_TREE,
      }));
      const results = summarize([
        laneResult("merge_integration", "success", "frontend-quality", "1"),
        laneResult("merge_integration", latestResult, "frontend-quality", "2"),
      ], expectations, plan.validation_lanes, "2");

      expect(results).toEqual({ merge_integration: latestResult });
      expect(evaluateValidationAggregate(aggregateInput(
        plan,
        validationLaneResultsForPlan(plan.validation_lanes, results),
      )).pass).toBe(false);
    },
  );

  it("fails when required evidence is absent from every attempt", () => {
    const plan = buildValidationPlan(pullRequestInput({
      headTreeSha: HEAD_TREE,
      integrationTreeSha: HEAD_TREE,
    }));
    const results = summarize([], { "frontend-quality": true }, plan.validation_lanes, "2");

    expect(results).toEqual({ merge_integration: "missing" });
    expect(evaluateValidationAggregate(aggregateInput(
      plan,
      validationLaneResultsForPlan(plan.validation_lanes, results),
    )).pass).toBe(false);
  });

  it("rejects non-positive and non-integer attempts", () => {
    for (const invalidAttempt of ["0", "-1", "1.5", "01", " 1 ", "current"]) {
      expect(() => summarize([
        laneResult("merge_integration", "success", "frontend-quality", invalidAttempt),
      ], expectations, ["merge_integration"], "2")).toThrow(/positive integer/u);
    }
    expect(() => summarize([], expectations, ["merge_integration"], "0"))
      .toThrow(/positive integer/u);
  });

  it("rejects a record newer than the current workflow attempt", () => {
    expect(() => summarize([
      laneResult("merge_integration", "success", "frontend-quality", "3"),
    ], expectations, ["merge_integration"], "2")).toThrow(/future attempt/u);
  });

  it.each([
    ["job_id", laneResult("head_validation", "success", "performance-search", "1")],
    ["job_id whitespace", { ...laneResult("head_validation", "success", "frontend-quality", "1"), job_id: " frontend-quality" }],
    ["lane", laneResult("merge_integration", "success", "frontend-quality", "1")],
    ["run_attempt", laneResult("head_validation", "success", "frontend-quality", "2")],
    ["artifact_name", { ...laneResult("head_validation", "success", "frontend-quality", "1"), artifact_name: "ci-lane-result-1-other-job-head_validation" }],
  ])("rejects artifact filename and record %s mismatch", (_field, record) => {
    const artifactRoot = mkdtempSync(join(tmpdir(), "ci-lane-results-mismatch-"));
    const artifactDirectory = join(artifactRoot, "ci-lane-result-1-frontend-quality-head_validation");
    mkdirSync(artifactDirectory, { recursive: true });
    writeFileSync(join(artifactDirectory, "ci-lane-result.json"), JSON.stringify(record));

    try {
      expect(() => readValidationLaneJobResults(artifactRoot, "2"))
        .toThrow();
    } finally {
      rmSync(artifactRoot, { recursive: true, force: true });
    }
  });

  it("rejects duplicate sanitized records even if both report success", () => {
    const duplicate = laneResult("merge_integration", "success", "frontend-quality", "1");
    expect(() => summarize([duplicate, duplicate], expectations, ["merge_integration"], "2"))
      .toThrow(/duplicate lane result record/u);
  });

  it("keeps unrouted matrix domains explicitly not required", () => {
    expect(summarize([], {
      "performance-macos": "false",
    }, lanes, "2")).toEqual({
      head_validation: "not_required",
      merge_integration: "not_required",
    });
    const plan = buildValidationPlan(pullRequestInput());
    expect(evaluateValidationAggregate(aggregateInput(plan, {
      laneValidationRequired: false,
      headValidationResult: "not_required",
      integrationValidationResult: "not_required",
    })).pass).toBe(true);
    expect(evaluateValidationAggregate(aggregateInput(plan, {
      laneValidationRequired: false,
      headValidationResult: "skipped",
      integrationValidationResult: "not_required",
    })).pass).toBe(false);
  });

  it("makes the aggregate CLI fail closed when lane artifacts are unavailable", () => {
    const sanitizedParentEnv = Object.fromEntries(
      Object.entries(process.env).filter(([name]) =>
        !/^(?:CI_GITHUB_TOKEN|GITHUB_TOKEN|GH_TOKEN|ACTIONS_RUNTIME_TOKEN)$/u.test(name),
      ),
    );
    const result = spawnSync(process.execPath, ["scripts/ciValidationPlan.mjs", "--aggregate"], {
      cwd: process.cwd(),
      encoding: "utf8",
      env: {
        ...sanitizedParentEnv,
        EVENT_NAME: "pull_request",
        PLAN_RESULT: "success",
        PLAN_VALID: "true",
        TREE_EQUIVALENT: "false",
        HEAD_VALIDATION_REQUIRED: "true",
        VALIDATION_LANES: JSON.stringify(lanes),
        GITHUB_RUN_ATTEMPT: "1",
        MATRIX_JOB_EXPECTATIONS: JSON.stringify(expectations),
        LANE_JOB_RESULTS_DIRECTORY: join(tmpdir(), "missing-ci-lane-results-337"),
      },
    });

    expect(result.status).toBe(1);
    expect(result.stdout).toContain('"pass": false');
    expect(result.stdout).toMatch(/head validation success, got missing/u);
  });
});

describe("workflow lane and governance wiring", () => {
  const interactiveWorkflow = readWorkflow(".github/workflows/ci.yml");
  const fullWorkflow = readWorkflow(".github/workflows/ci-full.yml");
  const matrixJobDisplayNames: Record<string, string> = {
    "docs-only-lanes": "Documentation-only validation",
    "frontend-quality": "Frontend and format quality",
    "rust-windows": "Rust quality (windows-latest)",
    "windows-native-preview-handler": "Windows native Preview Handler",
    "rust-macos": "Rust quality (macos-latest)",
    "performance-prepare": "Performance / Prepare",
    "performance-search": "Performance / Search",
    "performance-scan-schema": "Performance / Scan & Schema",
    "performance-library-content": "Performance / Library & Content",
    "performance-intelligence": "Performance / Intelligence",
    "performance-workspace-foundation": "Performance / Workspace Foundation",
    "performance-preview-platform": "Performance / Preview Platform",
    "performance-macos": "Native macOS performance (arm64)",
    "build-windows": "Release compile (windows-latest)",
    "build-macos": "Release compile (macos-latest)",
    "package-windows": "Package NSIS",
    "package-macos": "Package unsigned DMG",
    "package-smoke": "Package metadata smoke",
    "dependency-audit": "Dependency audit",
  };

  it("pins the repository Rust toolchain authority", () => {
    const toolchainPath = "rust-toolchain.toml";

    expect(existsSync(toolchainPath)).toBe(true);
    const toolchain = readFileSync(toolchainPath, "utf8").replace(/\r\n?/gu, "\n");

    expect(toolchain).toMatch(/^\[toolchain\]\s*$/mu);
    expect(toolchain).toMatch(/^channel\s*=\s*"1\.97\.1"\s*$/mu);
    expect(toolchain).toMatch(/^profile\s*=\s*"minimal"\s*$/mu);
    expect(toolchain).toMatch(/^components\s*=\s*\["rustfmt",\s*"clippy"\]\s*$/mu);
  });

  it("normalizes CRLF and LF workflow sources before semantic assertions", () => {
    expect(interactiveWorkflow).not.toContain("\r");
    expect(interactiveWorkflow).toContain("name: Documentation-only validation\n");
    expect(fullWorkflow).not.toContain("\r");
  });

  it("routes frontend and high-risk validation through the lane matrix", () => {
    expect(interactiveWorkflow).toContain("frontend_changed: ${{ steps.classify.outputs.frontend_changed }}");
    expect(interactiveWorkflow).toContain("high_risk: ${{ steps.classify.outputs.high_risk }}");
    expect(interactiveWorkflow).toContain("  frontend-quality:");
    expect(interactiveWorkflow).toContain("validation_lane: ${{ fromJSON(needs.validation-plan.outputs.validation_lanes) }}");
    expect(interactiveWorkflow).toContain("needs: [change-scope, validation-plan]");
  });

  it("routes performance-sensitive validation through both applicable lanes", () => {
    expect(interactiveWorkflow).toContain("performance_sensitive: ${{ steps.classify.outputs.performance_sensitive }}");
    expect(interactiveWorkflow).toContain("  performance-prepare:");
    expect(interactiveWorkflow).toContain("  performance-macos:");
    expect(interactiveWorkflow).toContain("needs: [change-scope, validation-plan]");
    expect(interactiveWorkflow).toContain("perf-bin-search-${{ matrix.validation_lane }}");
    expect(interactiveWorkflow).toContain("PERF_CHECKOUT_SHA: ${{ matrix.validation_lane == 'head_validation' && github.event.pull_request.head.sha || github.sha }}");
    expect(interactiveWorkflow).toContain("Read prepared Search binary identity");
  });

  it("keeps fork source validation read-only and outside pull_request_target", () => {
    expect(interactiveWorkflow).toMatch(/permissions:\s+contents: read/);
    expect(interactiveWorkflow).not.toContain("pull_request_target");
    expect(interactiveWorkflow).not.toMatch(/secrets\./);
    expect(interactiveWorkflow).toContain("matrix.validation_lane == 'head_validation' && github.event.pull_request.head.repo.full_name");
    expect(interactiveWorkflow).toContain("persist-credentials: false");
  });

  it("preserves every aggregate call site, including non-PR Full Validation callers", () => {
    expect(interactiveWorkflow).toContain("name: Change scope / routing contract");
    expect(interactiveWorkflow).toContain("name: Documentation-only validation\n");
    expect(interactiveWorkflow).toContain("name: Quality (windows-latest)");
    expect(interactiveWorkflow).toContain("name: Quality (macos-latest)");
    expect(interactiveWorkflow.match(/node scripts\/ciValidationPlan\.mjs --aggregate/g)).toHaveLength(4);
    expect(fullWorkflow.match(/node scripts\/ciValidationPlan\.mjs --aggregate/g)).toHaveLength(3);
    expect(interactiveWorkflow.match(/name: Checkout CI validation helper/g)).toHaveLength(4);
    expect(fullWorkflow.match(/name: Checkout CI validation helper/g)).toHaveLength(3);
    expect(fullWorkflow).not.toContain("pull_request:");
    expect(fullWorkflow).toContain("  schedule:");
    expect(fullWorkflow).toContain("  workflow_dispatch:");
  });

  it("retains aggregate job-level needs dependency checks", () => {
    const performanceProfile = workflowJobBlock(interactiveWorkflow, "performance-profile");
    const qualityWindows = workflowJobBlock(interactiveWorkflow, "quality-windows");
    const qualityMacos = workflowJobBlock(interactiveWorkflow, "quality-macos");
    expect(performanceProfile).toContain(
      "needs: [change-scope, validation-plan, performance-prepare, performance-search, performance-scan-schema, performance-library-content, performance-intelligence, performance-workspace-foundation, performance-preview-platform]",
    );
    expect(qualityWindows).toContain(
      "needs: [change-scope, validation-plan, frontend-quality, rust-windows, windows-native-filesystem-smoke, windows-native-preview-handler, windows-global-index-service-qualification, performance-profile, build-windows, package-windows, package-smoke, dependency-audit]",
    );
    expect(qualityMacos).toContain(
      "needs: [change-scope, validation-plan, frontend-quality, rust-macos, performance-profile, performance-macos, build-macos, package-macos, package-smoke, dependency-audit]",
    );
    expect(performanceProfile).toContain(
      'check_shard "$EXPECTED_WORKSPACE_FOUNDATION" "$ACTUAL_WORKSPACE_FOUNDATION" workspace-foundation',
    );
    expect(qualityWindows).toContain(
      'check_expected "$GLOBAL_INDEX_SERVICE_EXPECTED" "$GLOBAL_INDEX_SERVICE" windows-global-index-service-qualification',
    );
    expect(qualityWindows).toContain('check_expected true "$PERFORMANCE" performance-profile');
    expect(qualityMacos).toContain('check_expected true "$PERFORMANCE" performance-profile');
  });

  it.each([
    ["docs-only", ["docs-only-lanes"]],
    ["performance-profile", [
      "performance-prepare",
      "performance-search",
      "performance-scan-schema",
      "performance-library-content",
      "performance-intelligence",
      "performance-workspace-foundation",
      "performance-preview-platform",
    ]],
    ["quality-windows", [
      "frontend-quality",
      "rust-windows",
      "windows-native-preview-handler",
      "performance-prepare",
      "performance-search",
      "performance-scan-schema",
      "performance-library-content",
      "performance-intelligence",
      "performance-workspace-foundation",
      "performance-preview-platform",
      "build-windows",
      "package-windows",
      "package-smoke",
      "dependency-audit",
    ]],
    ["quality-macos", [
      "frontend-quality",
      "rust-macos",
      "performance-macos",
      "performance-prepare",
      "performance-search",
      "performance-scan-schema",
      "performance-library-content",
      "performance-intelligence",
      "performance-workspace-foundation",
      "performance-preview-platform",
      "build-macos",
      "package-macos",
      "package-smoke",
      "dependency-audit",
    ]],
  ])("wires %s to sanitized per-lane artifacts", (jobId, expectedJobIds) => {
    const block = workflowJobBlock(interactiveWorkflow, jobId);
    expect(block).toContain("env -u GITHUB_TOKEN -u CI_GITHUB_TOKEN");
    expect(block).toContain("node scripts/ciValidationPlan.mjs --aggregate");
    expect(block).toContain("actions: read");
    expect(block).toContain("contents: read");
    expect(block).toContain("Download validation lane result artifacts");
    expect(block).toContain("actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c");
    expect(block).toContain("pattern: ci-lane-result-*");
    expect(block).toContain("continue-on-error: true");
    expect(block).toContain("LANE_JOB_RESULTS_DIRECTORY: ${{ runner.temp }}/ci-lane-results");
    expect(block).toContain("MATRIX_JOB_EXPECTATIONS:");
    expect(block).toContain("VALIDATION_LANES: ${{ needs.validation-plan.outputs.validation_lanes }}");
    expect(block).not.toContain("${{ github.token }}");
    expect(block).not.toMatch(/^\s+(?:CI_GITHUB_TOKEN|GITHUB_TOKEN|GH_TOKEN|GH_ENTERPRISE_TOKEN|GITHUB_APP_TOKEN|GH_APP_TOKEN|ACTIONS_RUNTIME_TOKEN|ACTIONS_ID_TOKEN_REQUEST_TOKEN|AUTHORIZATION|BEARER_TOKEN):/mu);
    expect(block).not.toMatch(/Authorization:\s*Bearer/u);
    expect(interactiveWorkflow.match(/artifact_name: process\.env\.VALIDATION_ARTIFACT_NAME/g))
      .toHaveLength(19);
    for (const matrixJobId of expectedJobIds) {
      const matrixBlock = workflowJobBlock(interactiveWorkflow, matrixJobId);
      expect(matrixBlock).toContain("name: " + matrixJobDisplayNames[matrixJobId] + " (${{ matrix.validation_lane }})");
      expect(matrixBlock).toContain("name: Upload sanitized validation lane result");
      expect(matrixBlock).toContain("ci-lane-result-${{ github.run_attempt }}-${{ github.job }}-${{ matrix.validation_lane }}");
      expect(matrixBlock).toContain("VALIDATION_JOB_ID: ${{ github.job }}");
      expect(matrixBlock).toContain("VALIDATION_LANE: ${{ matrix.validation_lane }}");
      expect(matrixBlock).toContain("VALIDATION_RUN_ATTEMPT: ${{ github.run_attempt }}");
      expect(matrixBlock).toContain("VALIDATION_ARTIFACT_NAME: ci-lane-result-${{ github.run_attempt }}-${{ github.job }}-${{ matrix.validation_lane }}");
      expect(matrixBlock).toContain("artifact_name: process.env.VALIDATION_ARTIFACT_NAME");
      expect(matrixBlock.match(/ci-lane-result-\$\{\{ github\.run_attempt \}\}-\$\{\{ github\.job \}\}-\$\{\{ matrix\.validation_lane \}\}/gu))
        .toHaveLength(2);
      expect(interactiveWorkflow).toContain(
        "    name: " + matrixJobDisplayNames[matrixJobId] + " (${{ matrix.validation_lane }})",
      );
    }
    const serializedExpectations = block.match(/MATRIX_JOB_EXPECTATIONS: >-\n\s+(\{.*\})/u)?.[1];
    expect(serializedExpectations).toBeDefined();
    const parseableExpectations = serializedExpectations?.replace(/\$\{\{[\s\S]*?\}\}/gu, "true");
    expect(Object.keys(JSON.parse(parseableExpectations ?? "{}"))).toEqual(expectedJobIds);
  });

  it("maps each required matrix group to its classifier route output", () => {
    const expectedMappings = [
      ["docs-only", "docs-only-lanes", "true"],
      ["performance-profile", "performance-prepare", "needs.change-scope.outputs.performance_any"],
      ["performance-profile", "performance-search", "needs.change-scope.outputs.perf_search"],
      ["performance-profile", "performance-scan-schema", "needs.change-scope.outputs.perf_scan_schema"],
      ["performance-profile", "performance-library-content", "needs.change-scope.outputs.perf_library_content"],
      ["performance-profile", "performance-intelligence", "needs.change-scope.outputs.perf_intelligence"],
      ["performance-profile", "performance-workspace-foundation", "needs.change-scope.outputs.perf_workspace_foundation"],
      ["performance-profile", "performance-preview-platform", "needs.change-scope.outputs.perf_preview_platform"],
      ["quality-windows", "frontend-quality", "needs.change-scope.outputs.frontend_changed"],
      ["quality-windows", "rust-windows", "needs.change-scope.outputs.rust_changed"],
      ["quality-windows", "windows-native-preview-handler", "needs.change-scope.outputs.windows_native_preview_handler_changed"],
      ["quality-windows", "performance-prepare", "needs.change-scope.outputs.performance_any"],
      ["quality-windows", "performance-search", "needs.change-scope.outputs.perf_search"],
      ["quality-windows", "performance-scan-schema", "needs.change-scope.outputs.perf_scan_schema"],
      ["quality-windows", "performance-library-content", "needs.change-scope.outputs.perf_library_content"],
      ["quality-windows", "performance-intelligence", "needs.change-scope.outputs.perf_intelligence"],
      ["quality-windows", "performance-workspace-foundation", "needs.change-scope.outputs.perf_workspace_foundation"],
      ["quality-windows", "performance-preview-platform", "needs.change-scope.outputs.perf_preview_platform"],
      ["quality-windows", "build-windows", "needs.change-scope.outputs.release_sensitive"],
      ["quality-windows", "package-windows", "needs.change-scope.outputs.full_validation"],
      ["quality-windows", "package-smoke", "needs.change-scope.outputs.package_sensitive == 'true' && needs.change-scope.outputs.full_validation != 'true'"],
      ["quality-windows", "dependency-audit", "needs.change-scope.outputs.dependency_sensitive"],
      ["quality-macos", "frontend-quality", "needs.change-scope.outputs.frontend_changed"],
      ["quality-macos", "rust-macos", "needs.change-scope.outputs.macos_sensitive"],
      ["quality-macos", "performance-macos", "needs.change-scope.outputs.full_validation == 'true' || needs.change-scope.outputs.performance_sensitive == 'true'"],
      ["quality-macos", "performance-prepare", "needs.change-scope.outputs.performance_any"],
      ["quality-macos", "performance-search", "needs.change-scope.outputs.perf_search"],
      ["quality-macos", "performance-scan-schema", "needs.change-scope.outputs.perf_scan_schema"],
      ["quality-macos", "performance-library-content", "needs.change-scope.outputs.perf_library_content"],
      ["quality-macos", "performance-intelligence", "needs.change-scope.outputs.perf_intelligence"],
      ["quality-macos", "performance-workspace-foundation", "needs.change-scope.outputs.perf_workspace_foundation"],
      ["quality-macos", "performance-preview-platform", "needs.change-scope.outputs.perf_preview_platform"],
      ["quality-macos", "build-macos", "needs.change-scope.outputs.release_sensitive"],
      ["quality-macos", "package-macos", "needs.change-scope.outputs.full_validation"],
      ["quality-macos", "package-smoke", "needs.change-scope.outputs.package_sensitive == 'true' && needs.change-scope.outputs.full_validation != 'true'"],
      ["quality-macos", "dependency-audit", "needs.change-scope.outputs.dependency_sensitive"],
    ] as const;

    for (const [aggregateJobId, matrixJobId, classifierOutput] of expectedMappings) {
      const block = workflowJobBlock(interactiveWorkflow, aggregateJobId);
      const expected = classifierOutput === "true"
        ? "true"
        : "${{ " + classifierOutput + " }}";
      expect(block).toContain('"' + matrixJobId + '":"' + expected + '"');
    }
  });

  it("publishes exactly one credential-free result artifact per validation matrix job", () => {
    expect(interactiveWorkflow.match(/name: Upload sanitized validation lane result/g)).toHaveLength(19);
    expect(interactiveWorkflow.match(/name: Download validation lane result artifacts/g)).toHaveLength(4);
    for (const [jobId, displayName] of Object.entries(matrixJobDisplayNames)) {
      const block = workflowJobBlock(interactiveWorkflow, jobId);
      expect(block).toContain("name: Write sanitized validation lane result");
      expect(block).toContain("VALIDATION_JOB_ID: ${{ github.job }}");
      expect(block).toContain("VALIDATION_LANE: ${{ matrix.validation_lane }}");
      expect(block).toContain("VALIDATION_JOB_RESULT: ${{ job.status }}");
      expect(block).toContain("VALIDATION_RESULT_PATH: ${{ runner.temp }}/ci-lane-result.json");
      expect(block).toContain("VALIDATION_RUN_ATTEMPT: ${{ github.run_attempt }}");
      expect(block).toContain("run_attempt: process.env.VALIDATION_RUN_ATTEMPT");
      expect(block).toContain("if: ${{ always() && github.event_name == 'pull_request' }}");
      expect(block).toContain("uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7");
      expect(block).toContain("name: ci-lane-result-${{ github.run_attempt }}-${{ github.job }}-${{ matrix.validation_lane }}");
      expect(block).toContain("name: " + displayName + " (${{ matrix.validation_lane }})");
    }
  });

  it("keeps the aggregate helper credential-free and preserves fork read-only boundaries", () => {
    expect(interactiveWorkflow).toMatch(/permissions:\s+contents: read/);
    expect(interactiveWorkflow).not.toContain("pull_request_target");
    expect(interactiveWorkflow).not.toMatch(/secrets\./);
    expect(interactiveWorkflow).toContain("persist-credentials: false");
    const helper = readFileSync("scripts/ciValidationPlan.mjs", "utf8");
    expect(helper).toContain('if (eventName === "pull_request")');
    for (const forbidden of ["github.token", "CI_GITHUB_TOKEN", "GITHUB_TOKEN", "Authorization", "Bearer "]) {
      expect(helper).not.toContain(forbidden);
    }
    for (const aggregateJobId of ["docs-only", "performance-profile", "quality-windows", "quality-macos"]) {
      const aggregateBlock = workflowJobBlock(interactiveWorkflow, aggregateJobId);
      expect(aggregateBlock).not.toContain("${{ github.token }}");
      expect(aggregateBlock).not.toMatch(/^\s+(?:CI_GITHUB_TOKEN|GITHUB_TOKEN|GH_TOKEN|GH_ENTERPRISE_TOKEN|GITHUB_APP_TOKEN|GH_APP_TOKEN|ACTIONS_RUNTIME_TOKEN|ACTIONS_ID_TOKEN_REQUEST_TOKEN|AUTHORIZATION|BEARER_TOKEN):/mu);
      expect(aggregateBlock).not.toMatch(/Authorization:\s*Bearer/u);
      expect(aggregateBlock).toContain("env -u GITHUB_TOKEN -u CI_GITHUB_TOKEN");
    }
    expect(interactiveWorkflow).toContain('test "$actual" = skipped');
  });
});
