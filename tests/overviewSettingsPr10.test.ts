import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";
import {
  selectOverviewPriorityTask,
  type OverviewHealthSnapshot,
  type OverviewScanSnapshot
} from "../src/views/overview/overviewModel";
import type { DashboardStats } from "../src/types/domain";

function read(relativePath: string) {
  return readFileSync(resolve(relativePath), "utf8");
}

const scan: OverviewScanSnapshot = {
  status: "idle",
  isScanning: false,
  isCanceling: false,
  progress: null,
  error: null
};

const stats: DashboardStats = {
  totalFiles: 12,
  totalSize: 4096,
  diskTotalSize: 10000,
  diskFreeSize: 5000,
  diskUsageRatio: 0.5,
  duplicateFiles: 0,
  largeFiles: 0,
  sensitiveFiles: 0,
  needsConfirmation: 0,
  byType: {},
  byLifecycle: {},
  lastScannedAt: "2026-08-02T00:00:00.000Z"
};

const baseHealth = (): OverviewHealthSnapshot => ({
  globalIndex: null,
  watcher: {
    permissionRequired: 0,
    reconciliationRequired: 0,
    partialCoverage: 0,
    retryExhausted: 0,
    stale: 0
  },
  plan: null,
  cleanupRun: null,
  contentRun: null,
  operation: { active: false, attentionCount: 0 }
});

function select(health: OverviewHealthSnapshot | null, reclaimableBytes = 0, cleanupCandidateCount = 0) {
  return selectOverviewPriorityTask({
    scan,
    stats,
    cleanupCandidateCount,
    reclaimableBytes,
    indexNeedsUpdate: false,
    health
  });
}

describe("V4.3 PR10 overview health projections", () => {
  it("keeps no-source, permission, and provider error search states distinct", () => {
    const noSource = baseHealth();
    noSource.globalIndex = { status: "unavailable", enabled: true, collectionComplete: false, lastError: null, noSource: true };
    expect(select(noSource)).toMatchObject({ kind: "search-permission", reason: "no_source" });

    const permission = baseHealth();
    permission.globalIndex = { status: "permission_required", enabled: true, collectionComplete: false, lastError: null };
    expect(select(permission)).toMatchObject({ kind: "search-permission", reason: "permission" });

    const providerError = baseHealth();
    providerError.globalIndex = { status: "error", enabled: true, collectionComplete: false, lastError: "source unavailable" };
    expect(select(providerError)).toMatchObject({ kind: "search-permission", reason: "error", error: "source unavailable" });
  });

  it("uses the deterministic product priority for operations, plans, cleanup, content, and watcher health", () => {
    const operation = baseHealth();
    operation.operation = { active: true, attentionCount: 0 };
    expect(select(operation)).toMatchObject({ kind: "operation", reason: "active" });
    operation.operation.attentionCount = 1;
    expect(select(operation)).toMatchObject({ kind: "operation", reason: "failed", count: 1 });

    const plan = baseHealth();
    plan.plan = {
      status: "ready",
      summary: { undecided: 2, needsReview: 3, pendingReview: 1 },
      effectiveSummary: { ready: 0, reviewed: 0, pendingReview: 1, blocked: 0 }
    } as OverviewHealthSnapshot["plan"];
    expect(select(plan)).toMatchObject({ kind: "review", count: 1 });

    const canceledPlan = baseHealth();
    canceledPlan.plan = {
      status: "cancelled",
      summary: { undecided: 4, pendingReview: 4 },
      effectiveSummary: { ready: 0, reviewed: 0, pendingReview: 4, blocked: 0 }
    } as OverviewHealthSnapshot["plan"];
    expect(select(canceledPlan).kind).not.toBe("review");

    const cleanup = baseHealth();
    cleanup.cleanupRun = { safeCount: 2, reviewCount: 0, cautionCount: 0, exactReclaimableBytes: 8192 } as OverviewHealthSnapshot["cleanupRun"];
    expect(select(cleanup)).toMatchObject({ kind: "cleanup", count: 2, bytes: 8192 });

    const content = baseHealth();
    content.contentRun = { status: "failed", lastErrorDetail: "provider failed" } as OverviewHealthSnapshot["contentRun"];
    expect(select(content)).toMatchObject({ kind: "content-failure", reason: "failed", error: "provider failed" });

    const watcher = baseHealth();
    watcher.watcher = { permissionRequired: 0, reconciliationRequired: 1, partialCoverage: 1, retryExhausted: 0, stale: 1 };
    expect(select(watcher)).toMatchObject({ kind: "managed-root-stale", count: 1, reason: "partial" });
    watcher.watcher.partialCoverage = 0;
    expect(select(watcher)).toMatchObject({ kind: "managed-root-stale", count: 1, reason: "reconciliation" });
  });

  it("preserves watcher priority and chooses potential cleanup bytes only when exact bytes are absent", () => {
    const watcher = baseHealth();
    watcher.watcher = { permissionRequired: 1, reconciliationRequired: 1, partialCoverage: 1, retryExhausted: 1, stale: 1 };
    expect(select(watcher)).toMatchObject({ kind: "managed-root-stale", count: 1, reason: "permission" });
    watcher.watcher.permissionRequired = 0;
    expect(select(watcher)).toMatchObject({ kind: "managed-root-stale", count: 1, reason: "retry_exhausted" });
    watcher.watcher.retryExhausted = 0;
    expect(select(watcher)).toMatchObject({ kind: "managed-root-stale", count: 1, reason: "partial" });
    watcher.watcher.partialCoverage = 0;
    expect(select(watcher)).toMatchObject({ kind: "managed-root-stale", count: 1, reason: "reconciliation" });
    watcher.watcher.reconciliationRequired = 0;
    expect(select(watcher)).toMatchObject({ kind: "managed-root-stale", count: 1, reason: "stale" });

    const potential = baseHealth();
    potential.cleanupRun = { safeCount: 0, reviewCount: 2, cautionCount: 1, exactReclaimableBytes: 0, potentialReclaimableBytes: 8192 } as OverviewHealthSnapshot["cleanupRun"];
    expect(select(potential)).toMatchObject({ kind: "cleanup", count: 3, bytes: 8192, bytesAreEstimated: true });
    potential.cleanupRun = { safeCount: 0, reviewCount: 2, cautionCount: 1, exactReclaimableBytes: 4096, potentialReclaimableBytes: 8192 } as OverviewHealthSnapshot["cleanupRun"];
    expect(select(potential)).toMatchObject({ kind: "cleanup", count: 3, bytes: 4096, bytesAreEstimated: false });
    potential.cleanupRun = { safeCount: 0, reviewCount: 0, cautionCount: 0, exactReclaimableBytes: 8192, potentialReclaimableBytes: 8192 } as OverviewHealthSnapshot["cleanupRun"];
    expect(select(potential).kind).not.toBe("cleanup");
    potential.cleanupRun = { safeCount: 0, reviewCount: 2, cautionCount: 0, exactReclaimableBytes: 0, potentialReclaimableBytes: 0 } as OverviewHealthSnapshot["cleanupRun"];
    expect(select(potential).kind).not.toBe("cleanup");

    const noDurableBytes = baseHealth();
    noDurableBytes.cleanupRun = {
      reviewCount: 2,
      cautionCount: 0,
      exactReclaimableBytes: 0,
      potentialReclaimableBytes: 0
    } as OverviewHealthSnapshot["cleanupRun"];
    expect(select(noDurableBytes, 2048).kind).not.toBe("cleanup");

    expect(select(null, 2048, 2)).toMatchObject({ kind: "cleanup", bytes: 2048, bytesAreEstimated: false });
  });

  it("returns the calm no-action state when durable health has no work", () => {
    expect(select(baseHealth())).toEqual({ kind: "orderly", fileCount: 12 });

    const empty = { ...stats, totalFiles: 0, totalSize: 0 };
    expect(selectOverviewPriorityTask({
      scan,
      stats: empty,
      cleanupCandidateCount: 0,
      reclaimableBytes: 0,
      indexNeedsUpdate: false,
      health: baseHealth()
    })).toEqual({ kind: "unindexed" });
  });
});

describe("V4.3 PR10 status language and section contracts", () => {
  it("keeps watcher health outcomes separately translatable in both languages", () => {
    const zh = makeTranslator("zh");
    const en = makeTranslator("en");
    const keys = [
      "watcherStatusPermission",
      "watcherStatusReconciling",
      "watcherStatusPartial",
      "watcherStatusRetryExhausted"
    ] as const;

    for (const key of keys) {
      expect(zh(key)).not.toBe(key);
      expect(en(key)).not.toBe(key);
    }

    const watcherPresentation = read("src/utils/watcherPresentation.ts");
    expect(watcherPresentation).toContain('labelKey: "watcherStatusPermission"');
    expect(watcherPresentation).toContain('labelKey: "watcherStatusReconciling"');
    expect(watcherPresentation).toContain('labelKey: "watcherStatusPartial"');
    expect(watcherPresentation).toContain('labelKey: "watcherStatusRetryExhausted"');
  });

  it("projects real coverage into Overview and keeps Settings orchestration sectioned", () => {
    const overviewSections = read("src/views/overview/OverviewSections.tsx");
    const scanner = read("src/views/scanner/ScannerView.tsx");
    const settings = read("src/views/settings/SettingsView.tsx");

    expect(overviewSections).toContain("OverviewSystemCoverage");
    // Coverage is a fact panel in the v3.40 page grammar, not a metric strip.
    expect(overviewSections).toContain("v340-fact-list");
    expect(scanner).toContain("getGlobalIndexStatus");
    expect(scanner).toContain("listManagedScopes");
    expect(scanner).toContain("getActiveAnalysisRun");
    expect(scanner).toContain("listAnalysisRuns");
    expect(scanner).toContain("listContentRuns");
    for (const section of [
      "GeneralSettingsSection",
      "FileSourcesSettingsSection",
      "GlobalSearchSettingsSection",
      "GlobalIndexSettingsSection",
      "PlatformDiagnosticsSettingsSection",
      "ManagedLibrarySettingsSection",
      "AutomationSettingsSection",
      "AISettingsSection",
      "PrivacyContentSettingsSection",
      "AboutSettingsSection",
      "DeveloperDiagnosticsSection"
    ]) {
      expect(settings).toContain(`import { ${section} }`);
    }
    expect(settings).toContain("data-ai-save-bar");
    expect(settings).toContain("SettingsDisclosure");
  });
});
