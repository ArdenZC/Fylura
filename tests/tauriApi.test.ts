import { beforeEach, describe, expect, it, vi } from "vitest";
import { tauriApi } from "../src/api/tauriApi";
import { mockInvokeCommand } from "../src/api/browserMockApi";
import type { FileLibraryScopeV2, LibraryScope } from "../src/types/domain";

const apiMocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn()
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: apiMocks.invoke
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: apiMocks.listen
}));

describe("tauriApi", () => {
  it("does not expose the legacy unscoped cleanup scan", () => {
    expect("scanStorageCleanup" in tauriApi).toBe(false);
  });

  beforeEach(() => {
    delete (globalThis as typeof globalThis & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    vi.stubGlobal("navigator", { platform: "Win32", userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" });
    apiMocks.invoke.mockReset().mockResolvedValue({
      files: [],
      total: 0,
      limit: 50,
      offset: 0
    });
    apiMocks.listen.mockReset().mockResolvedValue(() => undefined);
  });

  it("reads backend runtime capabilities before exposing optional UI", async () => {
    await tauriApi.getRuntimeCapabilities();
    expect(apiMocks.invoke).toHaveBeenCalledWith("get_runtime_capabilities", undefined);
  });

  it("leaves macOS mutation routing to the backend strategy ladder", async () => {
    vi.stubGlobal("navigator", { platform: "MacIntel", userAgent: "Mozilla/5.0 (Macintosh)" });
    const results = await Promise.allSettled([
      tauriApi.executeMoves([{ id: "op", fileId: "file", old_name: "a", new_name: "b", operationFingerprint: "fingerprint" } as never]),
      tauriApi.restoreMoves([{ id: "log" } as never]),
      tauriApi.moveCleanupCandidatesToSafeTrash("job", [{ findingId: "item", expectedRevision: 1 }]),
      tauriApi.restoreCleanupTrashItems(["item"])
    ]);

    expect(results.every((result) => result.status === "fulfilled")).toBe(true);
    expect(apiMocks.invoke.mock.calls.map(([command]) => command)).toEqual([
      "execute_moves",
      "restore_moves",
      "move_cleanup_candidates_to_safe_trash",
      "restore_cleanup_trash_items"
    ]);

    await tauriApi.executeRulesForScopeV2({ kind: "all_enabled_roots" }, 7);
    expect(apiMocks.invoke).toHaveBeenCalledWith("execute_rules_for_scope_v2", {
      request: {
        scope: { kind: "all_enabled_roots" },
        expectedCatalogRevision: 7,
        mode: "inbox_only",
        confirmed: true
      }
    });
    vi.unstubAllGlobals();
  });

  it("sends only backend preview identity and fingerprint for execution", async () => {
    await tauriApi.executeMoves([{
      id: "preview-id",
      fileId: "file-id",
      old_name: "before.txt",
      new_name: "after.txt",
      operationFingerprint: "preview-fingerprint"
    } as never]);

    expect(apiMocks.invoke).toHaveBeenCalledWith("execute_moves", {
      request: {
        operations: [{
          id: "preview-id",
          fileId: "file-id",
          operationFingerprint: "preview-fingerprint",
          expectedRevision: "preview-fingerprint",
          newName: "after.txt"
        }]
      }
    });
    const request = apiMocks.invoke.mock.calls[0]?.[1] as { request: { operations: Array<Record<string, unknown>> } };
    expect(Object.keys(request.request.operations[0]).sort()).toEqual([
      "expectedRevision",
      "fileId",
      "id",
      "newName",
      "operationFingerprint"
    ]);
  });

  it("rejects execution without a complete preview fingerprint before invoking native code", async () => {
    await expect(tauriApi.executeMoves([{
      id: "preview-id",
      fileId: "file-id",
      old_name: "before.txt",
      new_name: "after.txt"
    } as never])).rejects.toThrow("operation_preview_stale");
    expect(apiMocks.invoke).not.toHaveBeenCalled();
  });

  it("requests a manual rename using only file identity and a basename", async () => {
    await tauriApi.getManualRenameOperationPreview("file-id", "中文.txt");
    expect(apiMocks.invoke).toHaveBeenCalledWith("get_manual_rename_operation_preview", { fileId: "file-id", newName: "中文.txt" });
    await expect(mockInvokeCommand("get_manual_rename_operation_preview", { fileId: "file-id", newName: "中文.txt" }))
      .rejects.toThrow("file_mutation_unsupported_in_browser");
  });

  it("requests permanent-delete previews through the backend command", async () => {
    await tauriApi.getPermanentDeleteOperationPreview("file-id");
    expect(apiMocks.invoke).toHaveBeenCalledWith("get_permanent_delete_operation_preview", { fileId: "file-id" });
  });

  it("keeps the browser mock fail-closed for native permanent delete", async () => {
    const capabilities = await mockInvokeCommand<{ permanentDeleteAvailable: boolean }>("get_runtime_capabilities");
    expect(capabilities.permanentDeleteAvailable).toBe(false);
    await expect(mockInvokeCommand("get_permanent_delete_operation_preview", { fileId: "file-id" }))
      .rejects.toThrow("permanent_delete_unsupported");
  });

  it("sends paged library filters alongside query and scope", async () => {
    const scope: LibraryScope = { kind: "roots", roots: ["F:/Downloads"] };

    await tauriApi.getPagedFiles(50, 25, "pdf", scope, { libraryFilter: "review" });

    expect(apiMocks.invoke).toHaveBeenCalledWith("get_paged_files", {
      limit: 50,
      offset: 25,
      query: "pdf",
      scope,
      filter: { libraryFilter: "review" }
    });
  });

  it("requests operation previews for a full library scope", async () => {
    const scope: LibraryScope = { kind: "roots", roots: ["F:/Downloads"] };

    await tauriApi.getOperationPreviewsForScope(scope, { libraryFilter: "active" }, 500, 1000);

    expect(apiMocks.invoke).toHaveBeenCalledWith("get_operation_previews_for_scope", {
      scope,
      filter: { libraryFilter: "active" },
      limit: 500,
      offset: 1000
    });
  });

  it("calls storage cleanup commands with conservative arguments", async () => {
    const selections = [{ findingId: "storage-safe-1", expectedRevision: 3 }];
    await tauriApi.startStorageCleanupScan(["F:/Downloads"]);
    await tauriApi.getStorageCleanupScanStatus("job-1");
    await tauriApi.cancelStorageCleanupScan("job-1");
    await tauriApi.revealStorageCandidate("F:/Downloads/big.zip");
    await tauriApi.previewCleanupCandidates("job-1", selections);
    await tauriApi.previewCleanupOperations("job-1", selections);
    await tauriApi.analyzeCleanupCandidatesWithAI("job-1", ["storage-safe-1"]);
    await tauriApi.moveCleanupCandidatesToSafeTrash("job-1", selections);
    await tauriApi.listCleanupTrashBatches();
    await tauriApi.previewRestoreCleanupTrash("batch-1");
    await tauriApi.restoreCleanupTrashItems(["item-1"]);
    await tauriApi.cancelCleanupRestore("cleanup-job-1");

    expect(apiMocks.invoke).toHaveBeenNthCalledWith(1, "start_storage_cleanup_scan", {
      roots: ["F:/Downloads"]
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(2, "get_storage_cleanup_scan_status", {
      jobId: "job-1"
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(3, "cancel_storage_cleanup_scan", {
      jobId: "job-1"
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(4, "reveal_storage_candidate", {
      path: "F:/Downloads/big.zip"
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(5, "preview_cleanup_candidates", {
      jobId: "job-1",
      selections
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(6, "preview_cleanup_operations", {
      jobId: "job-1",
      selections
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(7, "analyze_cleanup_candidates_with_ai", {
      jobId: "job-1",
      ids: ["storage-safe-1"]
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(8, "move_cleanup_candidates_to_safe_trash", {
      jobId: "job-1",
      selections
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(9, "list_cleanup_trash_batches", undefined);
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(10, "preview_restore_cleanup_trash", {
      batchId: "batch-1"
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(11, "restore_cleanup_trash_items", {
      itemIds: ["item-1"],
      jobId: null
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(12, "cancel_cleanup_restore", {
      jobId: "cleanup-job-1"
    });
  });

  it("sends explicit rule execution mode for scoped rule runs", async () => {
    const scope: FileLibraryScopeV2 = { kind: "roots", scanRootIds: ["scan-root-downloads"] };

    await tauriApi.executeRulesForScopeV2(scope, 9, "all_changed_or_rule_changed");

    expect(apiMocks.invoke).toHaveBeenCalledWith("execute_rules_for_scope_v2", {
      request: {
        scope,
        expectedCatalogRevision: 9,
        mode: "all_changed_or_rule_changed",
        confirmed: true
      }
    });
  });

  it("exposes AI classification cancellation and progress events", async () => {
    apiMocks.listen.mockResolvedValueOnce(() => undefined);

    await tauriApi.cancelAIClassification();
    await tauriApi.onAIClassificationProgress(() => undefined);

    expect(apiMocks.invoke).toHaveBeenCalledWith("cancel_ai_classification", undefined);
    expect(apiMocks.listen).toHaveBeenCalledWith("ai-classification-progress", expect.any(Function));
  });

  it("reads and refreshes global hotkey registration status", async () => {
    await tauriApi.getGlobalHotkeyStatus();
    await tauriApi.registerGlobalSearchHotkey("Alt+Space");

    expect(apiMocks.invoke).toHaveBeenNthCalledWith(1, "get_global_hotkey_status", undefined);
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(2, "register_global_search_hotkey", {
      accelerator: "Alt+Space"
    });
  });

  it("acknowledges navigation using only the fixed Main binding/outcome DTO", async () => {
    const request = { generation: 7, nonce: 2, sessionId: 4, revision: 9, applied: true };
    await tauriApi.acknowledgeSearchNavigation(request);
    expect(apiMocks.invoke).toHaveBeenCalledWith("acknowledge_search_navigation", { request });
  });

  it("subscribes to the Rust-owned search lifecycle and main readiness event", async () => {
    apiMocks.listen.mockResolvedValueOnce(() => undefined);

    await tauriApi.onMainWindowReadyRequest(() => undefined);

    expect(apiMocks.listen).toHaveBeenCalledWith("search-main-ready-request", expect.any(Function));
  });

  it("sends the versioned global-search request and lifecycle CAS payloads", async () => {
    const request = {
      version: 2 as const,
      requestId: "spotlight:4:9",
      query: "报告",
      limit: 80,
      offset: 0,
      cursor: null
    };
    const snapshot = { sessionId: 4, revision: 9, phase: "visible_collapsed" as const };

    await tauriApi.searchGlobalEntries(request);
    await tauriApi.resizeSearchWindow(snapshot, true);
    await tauriApi.hideSearchWindow(snapshot);

    expect(apiMocks.invoke).toHaveBeenNthCalledWith(1, "search_global_entries", { request });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(2, "resize_search_window", {
      request: { sessionId: 4, expectedRevision: 9, expanded: true }
    });
    expect(apiMocks.invoke).toHaveBeenNthCalledWith(3, "hide_search_window_command", {
      request: { sessionId: 4, expectedRevision: 9 }
    });
  });

  it("falls back to browser mock data when the Tauri runtime is unavailable in dev", async () => {
    apiMocks.invoke.mockRejectedValueOnce(new Error("Cannot read properties of undefined (reading 'invoke')"));

    const result = await tauriApi.getPagedFiles(50, 0, "report", { kind: "all" });

    expect(result.files.length).toBeGreaterThan(0);
    expect(result.files[0].name).toContain("report");
  });

  it("treats partial Tauri internals as unavailable in browser preview", async () => {
    (globalThis as typeof globalThis & { __TAURI_INTERNALS__?: { transformCallback?: unknown } }).__TAURI_INTERNALS__ = {};
    apiMocks.invoke.mockRejectedValueOnce(new Error("Cannot read properties of undefined (reading 'transformCallback')"));

    const result = await tauriApi.getPagedFiles(50, 0, "report", { kind: "all" });

    expect(result.files.length).toBeGreaterThan(0);
    expect(result.files[0].name).toContain("report");
  });

  it("returns a noop listener when the Tauri event runtime is unavailable in dev", async () => {
    const { listen } = await import("@tauri-apps/api/event");
    vi.mocked(listen).mockRejectedValueOnce(new Error("Cannot read properties of undefined (reading 'listen')"));

    const dispose = await tauriApi.onSearchNavigate(() => undefined);

    expect(dispose()).toBeUndefined();
  });
});
