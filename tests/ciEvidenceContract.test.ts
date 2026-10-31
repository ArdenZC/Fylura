import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  assertCheckoutEvidence,
  buildCheckoutEvidence,
  isValidSha,
  resolveCiEvidence,
} from "../scripts/ciEvidence.mjs";

const BASE_SHA = "1".repeat(40);
const HEAD_SHA = "2".repeat(40);
const MERGE_SHA = "3".repeat(40);
const TREE_SHA = "4".repeat(40);

function readWorkflow(relativePath: string) {
  return readFileSync(relativePath, "utf8").replace(/\r\n?/gu, "\n");
}

function pullRequestInput(overrides: Record<string, unknown> = {}) {
  return {
    eventName: "pull_request",
    repository: "ArdenZC/Fylura",
    eventSha: MERGE_SHA,
    selectedRef: "refs/pull/94/merge",
    prBaseSha: BASE_SHA,
    prHeadSha: HEAD_SHA,
    prHeadRepository: "ArdenZC/Fylura",
    prHeadRef: "fix/w2-r1-ci-evidence-governance",
    ...overrides,
  };
}

describe("CI source and merge-integration evidence", () => {
  it("records same-repository exact-head validation and proves the checkout", () => {
    const evidence = resolveCiEvidence({
      ...pullRequestInput(),
      lane: "head_validation",
    });

    expect(evidence.checkout_repository).toBe("ardenzc/fylura");
    expect(evidence.checkout_ref).toBe(HEAD_SHA);
    expect(evidence.expected_checkout_sha).toBe(HEAD_SHA);
    expect(evidence.diff_base).toBe(BASE_SHA);
    expect(evidence.diff_head).toBe(HEAD_SHA);
    expect(evidence.head_repository_kind).toBe("same_repository");
    expect(() => assertCheckoutEvidence(HEAD_SHA, MERGE_SHA)).toThrow(/does not match/);
  });

  it("checks out a fork head by immutable SHA without changing the trust model", () => {
    const evidence = resolveCiEvidence({
      ...pullRequestInput({
        lane: "head_validation",
        prHeadRepository: "contributor/Fylura",
        prHeadRef: "feature/untrusted-change",
      }),
    });

    expect(evidence.checkout_repository).toBe("contributor/fylura");
    expect(evidence.checkout_ref).toBe(HEAD_SHA);
    expect(evidence.expected_checkout_sha).toBe(HEAD_SHA);
    expect(evidence.head_repository_kind).toBe("fork");
    expect(evidence.source_is_trusted).toBe(false);
  });

  it("keeps merge integration distinct from exact-head evidence", () => {
    const evidence = resolveCiEvidence({
      ...pullRequestInput(),
      lane: "merge_integration",
    });

    expect(evidence.checkout_repository).toBe("ardenzc/fylura");
    expect(evidence.checkout_ref).toBe(MERGE_SHA);
    expect(evidence.integration_commit_sha).toBe(MERGE_SHA);
    expect(evidence.expected_checkout_sha).toBe(MERGE_SHA);
    expect(evidence.expected_checkout_sha).not.toBe(evidence.expected_pr_head_sha);
    expect(evidence.diff_base).toBe(BASE_SHA);
    expect(evidence.diff_head).toBe(HEAD_SHA);
  });

  it("maps a direct master push to the pushed commit without PR semantics", () => {
    const evidence = resolveCiEvidence({
      eventName: "push",
      lane: "push",
      repository: "ArdenZC/Fylura",
      eventSha: HEAD_SHA,
      eventBefore: BASE_SHA,
      selectedRef: "refs/heads/master",
    });

    expect(evidence.checkout_ref).toBe(HEAD_SHA);
    expect(evidence.expected_checkout_sha).toBe(HEAD_SHA);
    expect(evidence.diff_base).toBe(BASE_SHA);
    expect(evidence.expected_pr_head_sha).toBeNull();
  });

  it.each([
    ["schedule", "scheduled_full_validation"],
    ["workflow_dispatch", "manual_full_validation"],
  ])("records %s Full Validation against an immutable event SHA", (eventName, expectedLane) => {
    const evidence = resolveCiEvidence({
      eventName,
      repository: "ArdenZC/Fylura",
      eventSha: HEAD_SHA,
      selectedRef: "refs/heads/master",
    });

    expect(evidence.lane).toBe(expectedLane);
    expect(evidence.checkout_ref).toBe(HEAD_SHA);
    expect(evidence.expected_checkout_sha).toBe(HEAD_SHA);
    expect(evidence.diff_head).toBe(HEAD_SHA);
  });

  it("fails closed when pull-request base information is missing or malformed", () => {
    expect(() => resolveCiEvidence(pullRequestInput({ prBaseSha: "" }))).toThrow(/PR base SHA/);
    expect(() => resolveCiEvidence(pullRequestInput({ prBaseSha: "not-a-sha" }))).toThrow(/PR base SHA/);
    expect(() => resolveCiEvidence(pullRequestInput({ prBaseSha: "0".repeat(40) }))).toThrow(/PR base SHA/);
  });

  it("does not allow a metadata claim to substitute for actual checkout proof", () => {
    expect(isValidSha(HEAD_SHA)).toBe(true);
    expect(isValidSha("head-branch")).toBe(false);
    expect(() => buildCheckoutEvidence({
      ...pullRequestInput({ lane: "merge_integration" }),
      actualCheckoutSha: HEAD_SHA,
      actualCheckoutTree: TREE_SHA,
    })).toThrow(/does not match/);
  });
});

describe("CI workflow evidence wiring", () => {
  const interactiveWorkflow = readWorkflow(".github/workflows/ci.yml");
  const fullWorkflow = readWorkflow(".github/workflows/ci-full.yml");

  it("uses explicit merge/source refs and keeps fork validation read-only", () => {
    expect(interactiveWorkflow).toContain("CI_LANE: ${{ github.event_name == 'pull_request' && 'head_validation' || 'push' }}");
    expect(interactiveWorkflow).toContain("CI_LANE: ${{ github.event_name == 'pull_request' && 'merge_integration' || 'push' }}");
    expect(interactiveWorkflow).toContain("repository: ${{ github.event_name == 'pull_request' && github.event.pull_request.head.repo.full_name || github.repository }}");
    expect(interactiveWorkflow).toContain("ref: ${{ github.event_name == 'pull_request' && github.event.pull_request.head.sha || github.sha }}");
    expect(interactiveWorkflow).toContain("persist-credentials: false");
    expect(interactiveWorkflow.match(/include-hidden-files: true/g)).toHaveLength(2);
    expect(interactiveWorkflow).not.toContain("pull_request_target");
    expect(interactiveWorkflow).toContain("needs: source-evidence");
    expect(fullWorkflow).toContain("CI_LANE: ${{ github.event_name == 'workflow_dispatch' && 'manual_full_validation' || 'scheduled_full_validation' }}");
    expect(fullWorkflow).toContain("ref: ${{ github.sha }}");
    expect(fullWorkflow).toContain("include-hidden-files: true");
  });

  it("records actual browser checkout identity separately from W201_SOURCE_HEAD", () => {
    const browserScript = readFileSync("scripts/runW2-01BrowserGate.mjs", "utf8");
    const w210BrowserScript = readFileSync("scripts/runW2-10BrowserGate.mjs", "utf8");
    expect(browserScript).toContain("ACTUAL_CHECKOUT_SHA");
    expect(browserScript).toContain("ACTUAL_CHECKOUT_TREE");
    expect(browserScript).toContain("assertCheckoutEvidence");
    expect(interactiveWorkflow).toContain("W201_EXPECTED_CHECKOUT_SHA: ${{ matrix.validation_lane == 'head_validation' && github.event.pull_request.head.sha || github.sha }}");
    expect(fullWorkflow).toContain("W201_EXPECTED_CHECKOUT_SHA: ${{ github.sha }}");
    expect(browserScript).toContain("claimedSourceHead: SOURCE_HEAD");
    expect(w210BrowserScript).toContain("ACTUAL_CHECKOUT_SHA");
    expect(w210BrowserScript).toContain("ACTUAL_CHECKOUT_TREE");
    expect(w210BrowserScript).toContain("assertCheckoutEvidence");
    expect(interactiveWorkflow).toContain("W210_EXPECTED_CHECKOUT_SHA: ${{ matrix.validation_lane == 'head_validation' && github.event.pull_request.head.sha || github.sha }}");
    expect(fullWorkflow).toContain("W210_EXPECTED_CHECKOUT_SHA: ${{ github.sha }}");
  });
});
