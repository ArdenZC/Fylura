import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { FolderOpen } from "lucide-react";
import { makeTranslator } from "../src/i18n";
import type { DashboardStats, OperationLog } from "../src/types/domain";
import { formatDate } from "../src/utils/format";
import {
  buildOverviewSummary,
  deriveOverviewScanState,
  operationActivityTitle,
  selectOverviewBackgroundTasks,
  selectOverviewPriorityTask,
  selectRecentOverviewActivity
} from "../src/views/overview/overviewModel";
import { OverviewPriorityTask } from "../src/views/overview/OverviewPriorityTask";
import { OverviewBackgroundTaskList, OverviewQuickActions, OverviewRecentActivityList } from "../src/views/overview/OverviewSections";
import { ScanCancelDialog } from "../src/views/overview/ScanCancelDialog";
import { ScanTaskPanel, formatElapsed } from "../src/views/overview/ScanTaskPanel";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const t = makeTranslator("zh");
const stats = (overrides: Partial<DashboardStats> = {}): DashboardStats => ({
  totalFiles: 10,
  totalSize: 1024,
  diskTotalSize: 0,
  diskFreeSize: 0,
  diskUsageRatio: 0,
  duplicateFiles: 0,
  largeFiles: 0,
  sensitiveFiles: 0,
  needsConfirmation: 0,
  byType: {},
  byLifecycle: {},
  lastScannedAt: "2026-07-12T08:00:00Z",
  ...overrides
});

const scan = (overrides: Record<string, unknown> = {}) => ({
  status: "idle" as const,
  isScanning: false,
  isCanceling: false,
  progress: null,
  error: null,
  ...overrides
});

describe("Overview v4", () => {
  it("selects exactly one priority task in the required order", () => {
    expect(selectOverviewPriorityTask({ scan: scan({ status: "error", error: "denied" }), stats: stats({ needsConfirmation: 8 }), cleanupCandidateCount: 4, reclaimableBytes: 900, indexNeedsUpdate: true })).toMatchObject({ kind: "scan-failed" });
    expect(selectOverviewPriorityTask({ scan: scan({ status: "scanning", isScanning: true }), stats: stats({ needsConfirmation: 8 }), cleanupCandidateCount: 4, reclaimableBytes: 900, indexNeedsUpdate: true })).toMatchObject({ kind: "scan-active" });
    expect(selectOverviewPriorityTask({ scan: scan({ status: "canceled", isScanning: true, isCanceling: true }), stats: stats({ needsConfirmation: 8 }), cleanupCandidateCount: 4, reclaimableBytes: 900, indexNeedsUpdate: true })).toMatchObject({ kind: "scan-canceling" });
    expect(selectOverviewPriorityTask({ scan: scan({ status: "error", error: "Permission denied" }), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false })).toMatchObject({ kind: "scan-permission" });
    expect(selectOverviewPriorityTask({ scan: scan({ status: "completed", progress: { files: 9, errors: 2 } }), stats: stats({ needsConfirmation: 8 }), cleanupCandidateCount: 4, reclaimableBytes: 900, indexNeedsUpdate: true })).toMatchObject({ kind: "scan-partial" });
    expect(selectOverviewPriorityTask({ scan: scan({ status: "canceled", progress: { files: 5 } }), stats: stats({ needsConfirmation: 8 }), cleanupCandidateCount: 4, reclaimableBytes: 900, indexNeedsUpdate: true })).toMatchObject({ kind: "scan-canceled" });
    expect(selectOverviewPriorityTask({ scan: scan(), stats: stats({ needsConfirmation: 8 }), cleanupCandidateCount: 4, reclaimableBytes: 900, indexNeedsUpdate: true })).toMatchObject({ kind: "review" });
    expect(selectOverviewPriorityTask({ scan: scan(), stats: stats(), cleanupCandidateCount: 4, reclaimableBytes: 900, indexNeedsUpdate: true })).toMatchObject({ kind: "cleanup" });
    expect(selectOverviewPriorityTask({ scan: scan(), stats: stats({ totalFiles: 0, totalSize: 0, lastScannedAt: null }), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false })).toMatchObject({ kind: "unindexed" });
    expect(selectOverviewPriorityTask({ scan: scan(), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: true })).toMatchObject({ kind: "update" });
    expect(selectOverviewPriorityTask({ scan: scan(), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false })).toMatchObject({ kind: "orderly" });
  });

  it("distinguishes scanning, canceling, canceled, partial, completed, failed, and first use", () => {
    expect(deriveOverviewScanState(scan({ isScanning: true, status: "scanning" }), false)).toBe("scanning");
    expect(deriveOverviewScanState(scan({ isScanning: true, isCanceling: true, status: "canceled" }), true)).toBe("canceling");
    expect(deriveOverviewScanState(scan({ status: "canceled" }), true)).toBe("canceled");
    expect(deriveOverviewScanState(scan({ status: "completed", progress: { errors: 2, skipped: 1 } }), true)).toBe("partial");
    expect(deriveOverviewScanState(scan({ status: "completed", progress: { errors: 0, skipped: 0 } }), true)).toBe("completed");
    expect(deriveOverviewScanState(scan({ status: "error", error: "offline" }), true)).toBe("failed");
    expect(deriveOverviewScanState(scan(), false)).toBe("first-use");
  });

  it("uses only real operation logs for recent activity", () => {
    const logs = [
      { id: "old", created_at: "2026-07-10T08:00:00Z", status: "success", operation_type: "move" },
      { id: "new", created_at: "2026-07-12T09:00:00Z", status: "failed", operation_type: "rename" }
    ] as OperationLog[];
    const activity = selectRecentOverviewActivity(logs, t, 3);
    expect(activity.map((item) => item.id)).toEqual(["operation:new", "operation:old"]);
    expect(activity[0]).toMatchObject({ status: "failed", destination: null });
  });

  it("keeps orderly state to one scan action without a duplicate custom-folder action", () => {
    const task = selectOverviewPriorityTask({ scan: scan(), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false });
    const html = renderToStaticMarkup(createElement(OverviewPriorityTask, { task, t, onPrimary: vi.fn(), onChooseFolder: vi.fn(), onCancel: vi.fn() }));
    expect(html.match(/扫描新位置/g)).toHaveLength(1);
    expect(html).not.toContain("选择自定义位置");
    expect(html.match(/<button/g)).toHaveLength(1);
  });

  it("uses status-aware copy for every recent operation type", () => {
    const cases = [
      ["organize", "success", "完成整理操作"],
      ["organize", "failed", "整理失败"],
      ["organize", "skipped", "已跳过整理"],
      ["rename", "success", "完成重命名"],
      ["rename", "failed", "重命名失败"],
      ["rename", "skipped", "已跳过重命名"],
      ["cleanup", "success", "完成清理操作"],
      ["cleanup", "failed", "清理失败"],
      ["cleanup", "skipped", "已跳过清理操作"],
      ["restore", "success", "完成恢复操作"],
      ["restore", "failed", "恢复失败"],
      ["restore", "skipped", "已跳过恢复"]
    ] as const;
    for (const [operationType, status, expected] of cases) {
      const title = operationActivityTitle(operationType, status, t);
      expect(title).toBe(expected);
    }
    const failedRename = selectRecentOverviewActivity([{
      id: "rename-failed",
      created_at: "2026-07-12T10:00:00Z",
      operation_type: "rename",
      status: "failed",
      path_after: "F:/failed.txt"
    } as OperationLog], t)[0];
    const skippedCleanup = selectRecentOverviewActivity([{
      id: "cleanup-skipped",
      created_at: "2026-07-12T11:00:00Z",
      operation_type: "cleanup",
      status: "skipped",
      path_after: "F:/skipped.tmp"
    } as OperationLog], t)[0];
    expect(failedRename.title).not.toContain("完成重命名");
    expect(skippedCleanup.title).not.toContain("完成清理操作");
  });

  it("renders scan details without repeating the priority state copy", () => {
    const html = renderToStaticMarkup(createElement(ScanTaskPanel, {
      state: "scanning",
      progress: { root: "F:/Documents", files: 41, elapsedMs: 41000, skipped: 2, errors: 1 },
      error: null,
      fallbackPath: "",
      t,
      language: "zh"
    }));
    expect(html).toContain("扫描详情");
    expect(html).toContain("当前位置");
    expect(html).toContain("已处理");
    expect(html).toContain("已用时间");
    expect(html).toContain("41 秒");
    expect(html).not.toContain("正在建立本地索引");
    expect(html).not.toContain("你可以安全离开此页面");
    expect(html.match(/aria-live="polite"/g)).toHaveLength(1);
    const repeatedStateCopy = [
      ["canceling", "正在停止扫描"],
      ["partial", "扫描部分完成"],
      ["failed", "扫描未能完成"],
      ["canceled", "扫描已取消"],
      ["completed", "扫描已完成"]
    ] as const;
    for (const [state, repeatedTitle] of repeatedStateCopy) {
      const stateHtml = renderToStaticMarkup(createElement(ScanTaskPanel, {
        state,
        progress: { files: 3, skipped: 0, errors: 0, elapsedMs: 1000 },
        error: null,
        fallbackPath: "",
        t,
        language: "zh"
      }));
      expect(stateHtml).not.toContain(repeatedTitle);
    }
  });

  it("keeps a single scan live region and only the details panel owns it", () => {
    const priority = readFileSync(resolve("src/views/overview/OverviewPriorityTask.tsx"), "utf8");
    const details = readFileSync(resolve("src/views/overview/ScanTaskPanel.tsx"), "utf8");
    expect(priority).not.toContain("aria-live");
    expect(details.match(/aria-live=/g)).toHaveLength(1);
  });

  it("formats scan elapsed time for Chinese and English", () => {
    expect(formatElapsed(41_000, "zh")).toBe("41 秒");
    expect(formatElapsed(182_000, "zh")).toBe("3 分 2 秒");
    expect(formatElapsed(3_848_000, "zh")).toBe("1 小时 4 分 8 秒");
    expect(formatElapsed(41_000, "en")).toBe("41s");
    expect(formatElapsed(182_000, "en")).toBe("3m 2s");
    expect(formatElapsed(3_848_000, "en")).toBe("1h 4m 8s");
  });

  it("formats Overview dates with the explicit application language", () => {
    const value = "2026-07-12T08:00:00Z";
    const chinese = formatDate(value, "zh");
    const english = formatDate(value, "en");
    expect(chinese).toContain("7月12日");
    expect(chinese).not.toContain("Jul");
    expect(english).toContain("Jul");
    expect(english).not.toMatch(/[月日]/);

    const summary = buildOverviewSummary(stats(), [], makeTranslator("en"), "en");
    expect(summary).toContain("Jul");
    expect(summary).not.toMatch(/[月日]/);

    const activityHtml = renderToStaticMarkup(createElement(OverviewRecentActivityList, {
      activities: [{
        id: "operation:date",
        createdAt: value,
        title: "Rename completed",
        description: "C:/Users/Zen/file.txt",
        status: "success",
        destination: null
      }],
      t: makeTranslator("en"),
      language: "en"
    }));
    expect(activityHtml).toContain("Jul");
    expect(activityHtml).not.toMatch(/[月日]/);
  });

  it("formats persisted epoch timestamps and fails closed for invalid dates", () => {
    expect(formatDate("1784106796123", "en")).not.toBe("-");
    expect(formatDate("1784106796", "en")).not.toBe("-");
    expect(formatDate("not-a-date", "en")).toBe("-");
  });

  it("uses five scan facts at wide widths and two columns below xl", () => {
    const source = readFileSync(resolve("src/views/overview/ScanTaskPanel.tsx"), "utf8");
    expect(source).toContain("sm:grid-cols-2 xl:grid-cols-5");
    expect(source).not.toContain("xl:grid-cols-4");
    expect(source).toContain("aria-live=\"polite\"");
    expect((source.match(/aria-live=/g) ?? [])).toHaveLength(1);
  });

  it("keeps critical chrome and overview copy reachable at 200% text scale", () => {
    const shell = readFileSync(resolve("src/components/AppShell.tsx"), "utf8");
    const priority = readFileSync(resolve("src/views/overview/OverviewPriorityTask.tsx"), "utf8");
    const sections = readFileSync(resolve("src/views/overview/OverviewSections.tsx"), "utf8");
    const shellCss = readFileSync(resolve("src/styles/v340-shell.css"), "utf8");
    // At 200% text the titlebar drops the secondary crumb and the search field
    // keeps a usable minimum instead of pushing the window controls off-screen.
    expect(shellCss).toContain("@media (max-width: 720px)");
    expect(shellCss).toContain(".zc-titlebar-crumb span");
    expect(shellCss).toContain("min-width: 96px");
    // The sidebar keeps its own scroll container with contained overscroll.
    expect(shellCss).toContain("overflow-y: auto;");
    expect(shellCss).toContain("overscroll-behavior: contain;");
    expect(priority).toContain("[overflow-wrap:anywhere]");
    expect(sections).toContain("[overflow-wrap:anywhere]");
  });

  it("hides background work when idle and exposes only real active or failed tasks", () => {
    expect(selectOverviewBackgroundTasks({ backgroundIndexing: false, pendingRoots: [], failedRoots: [], operationProgress: null, aiProgress: null, isClassifyingWithAI: false })).toEqual([]);
    const tasks = selectOverviewBackgroundTasks({
      backgroundIndexing: true,
      currentRoot: "F:/Documents",
      pendingRoots: ["F:/Downloads"],
      failedRoots: [{ path: "F:/Denied", message: "denied" }],
      operationProgress: { kind: "execute", processed: 2, total: 5, currentPath: "F:/a.txt", batchId: "b" },
      aiProgress: null,
      isClassifyingWithAI: false
    });
    expect(tasks.map((task) => task.kind)).toEqual(["background-index", "operation", "background-failure"]);
  });

  it("builds a relational summary without disk coverage or clutter percentages", () => {
    const summary = buildOverviewSummary(stats({ totalFiles: 12842, needsConfirmation: 42 }), ["F:/Downloads", "F:/Desktop"], t);
    expect(summary).toContain("12,842");
    expect(summary).toContain("42");
    expect(summary).not.toContain("%");
    expect(summary).not.toContain("参考磁盘容量");
  });

  it("renders one and only one primary action", () => {
    const tasks = [
      selectOverviewPriorityTask({ scan: scan(), stats: stats({ totalFiles: 0, totalSize: 0, lastScannedAt: null }), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false }),
      selectOverviewPriorityTask({ scan: scan({ status: "scanning", isScanning: true }), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false }),
      selectOverviewPriorityTask({ scan: scan({ status: "canceled", isScanning: true, isCanceling: true }), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false }),
      selectOverviewPriorityTask({ scan: scan({ status: "error", error: "fatal" }), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false }),
      selectOverviewPriorityTask({ scan: scan({ status: "completed", progress: { files: 9, errors: 2 } }), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false }),
      selectOverviewPriorityTask({ scan: scan(), stats: stats({ needsConfirmation: 4 }), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false }),
      selectOverviewPriorityTask({ scan: scan(), stats: stats(), cleanupCandidateCount: 3, reclaimableBytes: 1024, indexNeedsUpdate: false }),
      selectOverviewPriorityTask({ scan: scan(), stats: stats(), cleanupCandidateCount: 0, reclaimableBytes: 0, indexNeedsUpdate: false })
    ];
    for (const task of tasks) {
      const html = renderToStaticMarkup(createElement(OverviewPriorityTask, { task, t, onPrimary: vi.fn(), onChooseFolder: vi.fn(), onCancel: vi.fn() }));
      expect(html.match(/data-overview-primary="true"/g)).toHaveLength(1);
    }
  });

  it("hides empty background tasks and uses an accessible in-app cancel dialog", () => {
    expect(renderToStaticMarkup(createElement(OverviewBackgroundTaskList, { tasks: [], t, onRetryIndex: vi.fn() }))).toBe("");
    const dialog = renderToStaticMarkup(createElement(ScanCancelDialog, { open: true, isCanceling: false, t, onConfirm: vi.fn(), onCancel: vi.fn() }));
    expect(dialog).toContain('role="alertdialog"');
    expect(dialog).toContain('aria-modal="true"');
    expect(dialog).toContain("已经完成的目录仍会保留在本地索引中");
  });

  it("renders explicit next steps and a truthful empty activity state", () => {
    const openHistory = vi.fn();
    const quickActions = renderToStaticMarkup(createElement(OverviewQuickActions, {
      t,
      actions: [{ id: "files", label: "查看文件", description: "浏览已索引文件", icon: FolderOpen, onClick: vi.fn() }]
    }));
    expect(quickActions).toContain('data-overview-next-steps="true"');
    expect(quickActions).toContain("查看文件");

    const emptyActivity = renderToStaticMarkup(createElement(OverviewRecentActivityList, {
      activities: [],
      t,
      onOpenHistory: openHistory
    }));
    expect(emptyActivity).toContain("还没有已记录的文件操作");
    expect(emptyActivity).toContain("查看历史记录");
    expect(emptyActivity).toContain('id="overview-activity-title"');
  });

  it("removes the old scanner dashboard and keeps existing scan actions wired", () => {
    const scanner = readFileSync(resolve("src/views/scanner/ScannerView.tsx"), "utf8");
    expect(scanner).not.toContain("ScannerDisk");
    expect(scanner).not.toContain("ScannerSummaryChip");
    expect(scanner).not.toContain("clutterRatio");
    expect(scanner).not.toContain("diskTotalSize");
    expect(scanner).not.toContain("percent(");
    expect(scanner).toContain("handleScan");
    expect(scanner).toContain("handleChooseFolders");
    expect(scanner).toContain("await cancelScan()");
    expect(scanner).not.toContain("window.confirm");
    expect(scanner).not.toContain("globalThis.confirm");
  });
});
