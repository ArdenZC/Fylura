// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WatcherReconciliationStatus } from "../src/api/tauriApi";
import { makeTranslator } from "../src/i18n";
import type { ScanRootSetting } from "../src/types/domain";
import { FileSourcesSettingsSection } from "../src/views/settings/sections/FileSourcesSettingsSection";
import { ToastContainer } from "../src/components/AppShell";
import { ChromeProvider, type ChromeContextValue } from "../src/contexts/AppContexts";
import { useFsWatcher } from "../src/hooks/useFsWatcher";
import { tauriApi, type ScanRootDto } from "../src/api/tauriApi";
import { useAppStore } from "../src/store/useAppStore";
import { useWatcherStatusStore } from "../src/store/useWatcherStatusStore";

const t = makeTranslator("zh");
const rootSetting: ScanRootSetting = {
  id: "scan-root-a",
  path: "C:/Managed",
  label: "Managed",
  enabled: true,
  createdAt: "2026-08-02T00:00:00.000Z"
};

function watcherStatus(overrides: Partial<WatcherReconciliationStatus> = {}): WatcherReconciliationStatus {
  return {
    scanRootId: rootSetting.id,
    path: rootSetting.path,
    rootRevision: 3,
    watcherRevision: 3,
    watcherAppliedRevision: 3,
    pending: false,
    needsReconciliation: false,
    healthStatus: "healthy",
    activeRunId: null,
    lastEventAt: 1,
    lastAppliedAt: 1,
    lastErrorCode: null,
    lastErrorMessage: null,
    timestamp: 1,
    ...overrides
  };
}

let root: Root;
let container: HTMLDivElement;

function renderStatus(status: WatcherReconciliationStatus) {
  act(() => {
    root.render(createElement(FileSourcesSettingsSection, {
      t,
      defaultScanFolders: [rootSetting],
      watcherRootStatuses: { [rootSetting.id]: status },
      organizeRootMode: "current_folder",
      organizeRootPath: null,
      onAddScanFolder: vi.fn(),
      onSetScanRootEnabled: vi.fn(),
      onScanRootNow: vi.fn(),
      onRequestDelete: vi.fn(),
      onOrganizeRootMode: vi.fn(),
      onOrganizeRootPath: vi.fn(),
      onChooseOrganizeRootPath: vi.fn()
    }));
  });
}

describe("mounted watcher health language priority", () => {
  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it.each([
    ["permission wins over reconciliation and retry", watcherStatus({ healthStatus: "permission_required", pending: true, needsReconciliation: true, lastErrorCode: "watcher_reconciliation_retry_exhausted" }), "需要权限"],
    ["retry exhaustion wins over reconciliation", watcherStatus({ healthStatus: "reconciliation_required", pending: true, needsReconciliation: true, lastErrorCode: "watcher_reconciliation_retry_exhausted" }), "重试次数已用尽"],
    ["partial wins over reconciliation and syncing", watcherStatus({ healthStatus: "degraded", pending: true, needsReconciliation: true, activeRunId: "run-1" }), "覆盖不完整"],
    ["reconciliation wins over syncing", watcherStatus({ healthStatus: "reconciliation_required", pending: true, needsReconciliation: true, activeRunId: "run-1" }), "正在校准索引"],
    ["partial wins over syncing", watcherStatus({ healthStatus: "partial", activeRunId: "run-1" }), "覆盖不完整"],
    ["pending is presented as syncing when reconciliation is not required", watcherStatus({ pending: true, activeRunId: "run-1" }), "正在同步变化"],
    ["syncing is shown before healthy", watcherStatus({ healthStatus: "scanning", activeRunId: "run-1" }), "正在同步变化"],
    ["stale is distinct from a healthy root", watcherStatus({ healthStatus: "stale" }), "需要重新同步"],
    ["unknown is shown when the backend has no health state", watcherStatus({ healthStatus: "unknown" }), "状态读取中"],
    ["healthy is shown when no attention state exists", watcherStatus(), "已同步"]
  ])("uses the stable priority when %s", (_name, status, expected) => {
    renderStatus(status);
    expect(container.textContent).toContain(expected);
  });
});

describe("durable Watcher producer → store → shell alert", () => {
  let statusEvent: (status: WatcherReconciliationStatus) => void;
  let warningEvent: (status: WatcherReconciliationStatus | { message: string }) => void;
  let resolveRoots: (roots: ScanRootDto[]) => void;
  const refresh = vi.fn(async () => {});
  const chrome = { view: "library", t, language: "zh" } as ChromeContextValue;

  function Runtime() {
    useFsWatcher({ onRefreshData: refresh, onError: useAppStore.getState().showError });
    return createElement(ChromeProvider, { value: chrome, children: createElement(ToastContainer) });
  }

  async function emit(status: WatcherReconciliationStatus, warning = false) {
    await act(async () => (warning ? warningEvent : statusEvent)(status));
  }

  function pending(revision: number, id = rootSetting.id) {
    return watcherStatus({ scanRootId: id, rootRevision: revision, watcherRevision: 11,
      watcherAppliedRevision: 9, pending: true, needsReconciliation: true,
      healthStatus: "reconciliation_required" });
  }

  beforeEach(async () => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    refresh.mockClear();
    useWatcherStatusStore.getState().reset();
    useAppStore.setState({ toast: null, language: "zh" });
    vi.spyOn(tauriApi, "getRuntimeCapabilities").mockResolvedValue({ backendWatcherReconciliation: true } as never);
    vi.spyOn(tauriApi, "listScanRoots").mockReturnValue(new Promise((resolve) => { resolveRoots = resolve; }));
    vi.spyOn(tauriApi, "onWatcherReconciliationStatus").mockImplementation(async (handler) => {
      statusEvent = (payload) => handler(payload, { event: "watcher-reconciliation-status", id: 0, payload });
      return () => {};
    });
    vi.spyOn(tauriApi, "onFsWatcherWarning").mockImplementation(async (handler) => {
      warningEvent = (payload) => handler(payload as never, { event: "fs-watcher-warning", id: 0, payload: payload as never });
      return () => {};
    });
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => root.render(createElement(Runtime)));
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.restoreAllMocks();
    useWatcherStatusStore.getState().reset();
    useAppStore.setState({ toast: null });
    vi.useRealTimers();
  });

  it("removes the reconciliation alert only from newer durable completion and keeps refresh", async () => {
    await emit(pending(36));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain(t("watcherReconciliationRequired"));
    await emit(watcherStatus({ rootRevision: 37, watcherRevision: 11, watcherAppliedRevision: 11 }));
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(useAppStore.getState().toast).toBeNull();
    await act(async () => vi.advanceTimersByTimeAsync(100));
    expect(refresh).toHaveBeenCalledOnce();
  });

  it("filters old/equal warning events and delayed hydration across remounts", async () => {
    await emit(watcherStatus({ rootRevision: 37 }));
    await emit(pending(36), true);
    await emit(pending(37));
    await act(async () => resolveRoots([{
      id: rootSetting.id, revision: 30, normalizedPath: rootSetting.path,
      watcherRevision: 9, watcherAppliedRevision: 8,
      needsReconciliation: true, healthStatus: "reconciliation_required"
    } as ScanRootDto]));
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(useWatcherStatusStore.getState().roots[rootSetting.id].rootRevision).toBe(37);
    await act(async () => root.render(createElement("div")));
    await act(async () => root.render(createElement(Runtime)));
    await emit(pending(35));
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("keeps unresolved state against old healthy responses, then accepts durable hydration completion", async () => {
    await emit(pending(36));
    await emit(watcherStatus({ rootRevision: 35 }), true);
    await emit(watcherStatus({ rootRevision: 36 }));
    expect(container.querySelector('[role="alert"]')?.textContent).toBe(t("watcherReconciliationRequired"));
    await act(async () => resolveRoots([{
      id: rootSetting.id, revision: 37, normalizedPath: rootSetting.path,
      watcherRevision: 11, watcherAppliedRevision: 11,
      activeRunId: null, needsReconciliation: false, healthStatus: "healthy",
      lastErrorCode: null, watcherLastErrorCode: null
    } as ScanRootDto]));
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it.each(["permission_required", "partial", "retry_exhausted", "reconciliation_required"])(
    "keeps another root's %s warning when one root completes", async (healthStatus) => {
      await emit(pending(36));
      await emit(watcherStatus({ scanRootId: "root-b", rootRevision: 2, healthStatus }), true);
      await emit(watcherStatus({ rootRevision: 37 }));
      expect(container.querySelector('[role="alert"]')).not.toBeNull();
      expect(useWatcherStatusStore.getState().roots["root-b"].healthStatus).toBe(healthStatus);
      await emit(watcherStatus({ scanRootId: "root-b", rootRevision: 3 }));
      expect(container.querySelector('[role="alert"]')).toBeNull();
    }
  );

  it("preserves an unrelated identical-text error and legacy partial warning", async () => {
    await emit(pending(36));
    await act(async () => useAppStore.getState().showError(t("watcherReconciliationRequired")));
    await emit(watcherStatus({ rootRevision: 37 }));
    expect(useAppStore.getState().toast?.message).toBe(t("watcherReconciliationRequired"));
    expect(container.querySelector('[role="alert"]')?.textContent).toBe(t("watcherReconciliationRequired"));
    await act(async () => warningEvent({ message: "legacy" }));
    await emit(watcherStatus({ rootRevision: 38 }));
    expect(container.querySelector('[role="alert"]')?.textContent).toBe(t("fsWatcherPartialIndexWarning"));
  });
});
