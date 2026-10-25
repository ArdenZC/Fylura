import { invokeCommand, listenTo, type EventHandler } from "./core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  ExecuteOperationRequest,
  ExecuteOperationResult,
  FileLibraryFilters,
  LibraryScope,
  LibrarySelectionV1,
  OperationLog,
  OperationPreview,
  OperationPreviewResult,
  RecoveryAction,
  RecoveryActionResult,
  RestoreMovesResult
} from "../types/domain";
import { rejectUnavailableFileMutation } from "../utils/fileMutationCapability";
import type { OperationProgressPayload } from "./types";

export const operationApi = {
  executeMoves(operations: OperationPreview[]): Promise<ExecuteOperationResult> {
    const unavailable = rejectUnavailableFileMutation<ExecuteOperationResult>();
    if (unavailable) return unavailable;
    const operationFingerprints = operations.map((operation) => operation.operationFingerprint?.trim() ?? "");
    if (operationFingerprints.some((fingerprint) => !fingerprint)) {
      return Promise.reject(new Error("operation_preview_stale"));
    }
    const request: ExecuteOperationRequest = {
      operations: operations.map((operation, index) => ({
        id: operation.id,
        fileId: operation.fileId,
        operationFingerprint: operationFingerprints[index],
        expectedRevision: operationFingerprints[index],
        ...(operation.new_name !== operation.old_name ? { newName: operation.new_name } : {})
      }))
    };
    return invokeCommand<ExecuteOperationResult>("execute_moves", { request });
  },
  restoreMoves(logs: OperationLog[]): Promise<RestoreMovesResult> {
    const unavailable = rejectUnavailableFileMutation<RestoreMovesResult>();
    if (unavailable) return unavailable;
    return invokeCommand<RestoreMovesResult>("restore_moves", { request: { logIds: logs.map((log) => log.id) } });
  },
  resolveOperationRecovery(logId: string, action: RecoveryAction, targetPath?: string): Promise<RecoveryActionResult> {
    const unavailable = rejectUnavailableFileMutation<RecoveryActionResult>();
    if (unavailable) return unavailable;
    return invokeCommand<RecoveryActionResult>("resolve_operation_recovery", {
      request: {
        logId,
        action,
        targetPath: targetPath ?? null
      }
    });
  },
  cancelOperations(): Promise<void> {
    return invokeCommand<void>("cancel_operations");
  },
  materializeProviderPreview(preview: OperationPreview): Promise<{
    previewId: string;
    fileId: string;
    materialization: string;
    nextOperationFingerprint: string | null;
  }> {
    const unavailable = rejectUnavailableFileMutation<{
      previewId: string;
      fileId: string;
      materialization: string;
      nextOperationFingerprint: string | null;
    }>();
    if (unavailable) return unavailable;
    const operationFingerprint = preview.operationFingerprint;
    if (!operationFingerprint) {
      return Promise.reject(new Error("This preview must be refreshed before materialization."));
    }
    return invokeCommand("materialize_provider_preview", {
      request: {
        previewId: preview.id,
        fileId: preview.fileId,
        operationFingerprint,
        expectedRevision: operationFingerprint
      }
    });
  },
  getOperationLogs(limit = 500): Promise<OperationLog[]> {
    return invokeCommand<OperationLog[]>("get_operation_logs", { limit });
  },
  getOperationPreviewsForScope(scope: LibraryScope, filters?: FileLibraryFilters, limit?: number, offset?: number): Promise<OperationPreviewResult> {
    return invokeCommand<OperationPreviewResult>("get_operation_previews_for_scope", {
      scope,
      filter: filters ?? null,
      limit,
      offset
    });
  },
  getOperationPreviewsByFileIds(fileIds: string[]): Promise<OperationPreview[]> {
    return invokeCommand<OperationPreview[]>("get_operation_previews_by_file_ids", { fileIds });
  },
  getManualRenameOperationPreview(fileId: string, newName: string): Promise<OperationPreview> {
    return invokeCommand<OperationPreview>("get_manual_rename_operation_preview", { fileId, newName });
  },
  getPermanentDeleteOperationPreview(fileId: string): Promise<OperationPreview> {
    return invokeCommand<OperationPreview>("get_permanent_delete_operation_preview", { fileId });
  },
  getOperationPreviewsForSelection(selection: LibrarySelectionV1, limit?: number, offset?: number): Promise<OperationPreviewResult> {
    return invokeCommand<OperationPreviewResult>("get_operation_previews_for_selection", { selection, limit, offset });
  },
  revealInFolder(path: string): Promise<void> {
    return invokeCommand<void>("reveal_in_folder", { path });
  },
  onOperationProgress(handler: EventHandler<OperationProgressPayload>): Promise<UnlistenFn> {
    return listenTo("operation-progress", handler);
  }
};

export type OperationApi = typeof operationApi;
