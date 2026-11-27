// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChromeProvider, type ChromeContextValue } from "../src/contexts/AppContexts";
import { makeTranslator } from "../src/i18n";
import { useOperationQueueStore } from "../src/store/useOperationQueueStore";
import type { OperationLog } from "../src/types/domain";
import { RestoreView } from "../src/views/restore/RestoreView";

const apiMocks = vi.hoisted(() => ({
  getOperationLogs: vi.fn(),
  listCleanupTrashBatches: vi.fn(),
  previewRestoreCleanupTrash: vi.fn()
}));

vi.mock("../src/api/tauriApi", () => ({ tauriApi: apiMocks }));

const t = makeTranslator("zh");
const chrome = {
  t,
  language: "zh",
  setView: vi.fn()
} as unknown as ChromeContextValue;

function log(id: string, overrides: Partial<OperationLog> = {}): OperationLog {
  return {
    id,
    batch_id: "batch-1",
    operation_type: "move",
    source_path: `C:/before/${id}`,
    target_path: `C:/after/${id}`,
    old_name: `${id}.txt`,
    new_name: `${id}.txt`,
    status: "success",
    error_message: null,
    created_at: "1710000000000",
    can_undo: true,
    path_before: `C:/before/${id}`,
    path_after: `C:/after/${id}`,
    name_before: `${id}.txt`,
    name_after: `${id}.txt`,
    can_restore: true,
    restored_at: null,
    restore_status: "not_restored",
    restore_error: null,
    ...overrides
  };
}

let root: Root;
let container: HTMLDivElement;

async function flush(count = 4) {
  for (let index = 0; index < count; index += 1) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
}

async function flushFrame() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 25));
  });
}

describe("Restore More filters", () => {
  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.clearAllMocks();
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      callback(0);
      return 0;
    });
    apiMocks.getOperationLogs.mockResolvedValue([]);
    apiMocks.listCleanupTrashBatches.mockResolvedValue([]);
    apiMocks.previewRestoreCleanupTrash.mockResolvedValue({ items: [] });
    useOperationQueueStore.setState({
      operationLogs: [],
      selectedOperationIds: new Set(),
      restoreIntent: null,
      restoreError: "",
      cleanupRestoreError: "",
      restoreTechnicalError: "",
      operationProgress: null,
      activeOperationKind: null,
      cleanupRestoreProgress: null,
      cleanupRestoreResult: null,
      listenersRegistered: false,
      registrationPromise: null
    });
    document.body.innerHTML = '<div id="test-root"></div>';
    container = document.getElementById("test-root") as HTMLDivElement;
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    vi.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it("closes More filters, updates the filter, and restores focus to the trigger", async () => {
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(RestoreView) })));
    await flush();

    const trigger = container.querySelector<HTMLButtonElement>('[aria-controls="history-more-filters"]')!;
    await act(async () => trigger.click());
    await flushFrame();

    const dialog = container.querySelector<HTMLElement>('[role="dialog"][aria-label="更多筛选"]')!;
    const firstFilter = dialog.querySelector<HTMLButtonElement>("button")!;
    expect(document.activeElement).toBe(firstFilter);

    await act(async () => firstFilter.click());
    await flush();

    expect(container.querySelector('[role="dialog"][aria-label="更多筛选"]')).toBeNull();
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(trigger);

    await act(async () => trigger.click());
    await flushFrame();
    expect(container.querySelector<HTMLElement>('[role="dialog"][aria-label="更多筛选"]')?.querySelector<HTMLButtonElement>("button")?.getAttribute("aria-pressed")).toBe("true");
  });

  it("keeps Escape focus restoration for the More filters dialog", async () => {
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(RestoreView) })));
    await flush();

    const trigger = container.querySelector<HTMLButtonElement>('[aria-controls="history-more-filters"]')!;
    await act(async () => trigger.click());
    await flush();
    expect(container.querySelector('[role="dialog"][aria-label="更多筛选"]')).not.toBeNull();

    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    await flush();

    expect(container.querySelector('[role="dialog"][aria-label="更多筛选"]')).toBeNull();
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
    expect(document.activeElement).toBe(trigger);
  });

  it("drops a selected history item when refresh makes it non-restorable", async () => {
    const current = log("refresh-me");
    apiMocks.getOperationLogs.mockResolvedValue([current]);
    await act(async () => root.render(createElement(ChromeProvider, { value: chrome, children: createElement(RestoreView) })));
    await flush();

    const batchCheckbox = container.querySelector<HTMLInputElement>('input[aria-label="选择此批次可恢复项目"]')!;
    await act(async () => batchCheckbox.click());
    await flush();
    expect(container.textContent).toContain("已选择 1 项");

    const batchRow = container.querySelector<HTMLElement>('[role="option"]')!;
    await act(async () => batchRow.click());
    await flush();

    act(() => useOperationQueueStore.setState({ operationLogs: [log("refresh-me", { can_restore: false })] }));
    await flush();

    expect(container.textContent).not.toContain("已选择 1 项");
    expect(container.querySelector<HTMLInputElement>('input[aria-label="选择此批次可恢复项目"]')?.disabled).toBe(true);
    expect(container.querySelector<HTMLInputElement>('input[aria-label^="选择此操作"]')?.disabled).toBe(true);
  });
});
