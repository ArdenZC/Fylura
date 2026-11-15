import { afterEach, describe, expect, it, vi } from "vitest";
import { mockInvokeCommand } from "../src/api/browserMockApi";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("PM-01 browser presentation fixtures", () => {
  it("keeps the same current-assessment finding visible across a later request failure and Preview fixture", async () => {
    vi.stubGlobal("location", new URL("http://localhost/?pm01-readiness=ready&pm01-cleanup=request-unavailable"));
    const readiness = await mockInvokeCommand<any>("get_ai_feature_readiness");
    expect(readiness.provider.state).toBe("ready");
    expect(readiness.cleanup.state).toBe("ready");

    const run = await mockInvokeCommand<any>("start_analysis_run", {
      request: {
        requestKey: "pm01-browser-current-assessment",
        scope: { kind: "approvedCleanupPaths", paths: ["C:/Presentation/Folders"] }
      }
    });
    expect(run.status).toBe("completed");

    const page = await mockInvokeCommand<any>("list_analysis_findings", { runId: run.id, limit: 100 });
    expect(page.findings).toHaveLength(2);
    const review = page.findings.find((finding: { tier: string }) => finding.tier === "review");
    const evidence = await mockInvokeCommand<any[]>("list_analysis_finding_evidence", { findingId: review.id });
    expect(evidence[0]).toMatchObject({ isCurrentAssessment: true, value: { presentationOnly: true } });

    await expect(mockInvokeCommand<any>("analyze_cleanup_candidates_with_ai", {
      jobId: run.id,
      ids: [review.id]
    })).rejects.toThrow("provider_request_unavailable");
    const retainedEvidence = await mockInvokeCommand<any[]>("list_analysis_finding_evidence", { findingId: review.id });
    expect(retainedEvidence[0]).toMatchObject({ isCurrentAssessment: true, value: { presentationOnly: true } });

    const decision = await mockInvokeCommand<any>("set_analysis_finding_decision", {
      findingKey: review.findingKey,
      decision: "acknowledged"
    });
    const acknowledged = await mockInvokeCommand<any>("get_analysis_finding", { findingId: review.id });
    expect(acknowledged).toMatchObject({ id: review.id, decision: "acknowledged", decisionRevision: decision.revision });
    const preview = await mockInvokeCommand<any>("preview_cleanup_operations", {
      jobId: run.id,
      selections: [{
        findingId: review.id,
        expectedRevision: acknowledged.revision,
        reviewConfirmation: { decisionRevision: acknowledged.decisionRevision }
      }]
    });
    expect(preview.previews).toHaveLength(1);
    expect(preview.previews[0]).toMatchObject({ fileId: review.id, source_path: review.pathSnapshot });
  });

  it("can show a failed analysis without claiming a backend readiness change", async () => {
    vi.stubGlobal("location", new URL("http://localhost/?pm01-readiness=ready&pm01-cleanup=analysis-failure"));
    const run = await mockInvokeCommand<any>("start_analysis_run", {
      request: {
        requestKey: "pm01-browser-analysis-failure",
        scope: { kind: "approvedCleanupPaths", paths: ["C:/Presentation/Downloads"] }
      }
    });
    expect(run).toMatchObject({ status: "failed", errorCode: "browser_presentation_analysis_failure" });
  });

  it("builds an eight-item organize plan from real plan/group projections", async () => {
    vi.stubGlobal("location", new URL("http://localhost/?pm01-organize=rich"));
    const plans = await mockInvokeCommand<any[]>("list_organization_plans");
    expect(plans).toHaveLength(1);
    expect(plans[0]).toMatchObject({ materializedCount: 8, requestedCount: 8 });
    const items = await mockInvokeCommand<any>("query_organization_plan_items", {
      request: { planId: plans[0].id, pageSize: 100 }
    });
    expect(items.items).toHaveLength(8);
    const groups = await mockInvokeCommand<any>("query_organization_plan_groups", {
      request: { planId: plans[0].id, pageSize: 100 }
    });
    expect(groups.groups.length).toBeGreaterThan(1);
  });

  it("publishes nine rich cleanup findings with safe, review and caution tiers", async () => {
    vi.stubGlobal("location", new URL("http://localhost/?pm01-cleanup=rich"));
    const run = await mockInvokeCommand<any>("start_analysis_run", {
      request: {
        requestKey: "pm01-browser-rich-cleanup",
        scope: { kind: "approvedCleanupPaths", paths: ["C:/Users/Zen/Documents"] }
      }
    });
    expect(run).toMatchObject({ status: "completed", findingsPublished: 9, safeCount: 6, reviewCount: 2, cautionCount: 1 });
    const page = await mockInvokeCommand<any>("list_analysis_findings", { runId: run.id, limit: 100 });
    expect(page.findings).toHaveLength(9);
    expect(page.findings.filter((finding: { tier: string }) => finding.tier === "safe")).toHaveLength(6);
    expect(page.findings.filter((finding: { tier: string }) => finding.tier === "review")).toHaveLength(2);
    expect(page.findings.filter((finding: { tier: string }) => finding.tier === "caution")).toHaveLength(1);
  });

  it("labels useful-folder and manual-index routes as browser presentation settings", async () => {
    vi.stubGlobal("location", new URL("http://localhost/?pm01-onboarding=folder-index-off"));
    const versioned = await mockInvokeCommand<any>("get_settings");
    expect(versioned.settings).toMatchObject({
      backgroundIndexOnStartup: false,
      defaultScanFolders: [{
        id: "browser-presentation-only-onboarding-root",
        path: "C:/Presentation/Zen Documents",
        enabled: true
      }]
    });
  });
});
