import { refreshRenamedSelection } from "./renameSelectionRefresh";
import { samePathLike } from "../../utils/viewHelpers";
import { explicitSingleSelectionId, useFileLibrarySelectionStore } from "../useFileLibraryV2Store";
import { tauriApi } from "../../api/tauriApi";
import type { OperationLog } from "../../types/domain";
import {
  createRestoreExecutionIntent,
  resolveOperationRestoreSelection,
  resolveRestoreExecutionIds,
  restoreIntentMatchesResolution,
  type RestoreExecutionIntent
} from "../../views/history/historyModel";
import type { RestoreConfirmationOutcome } from "../useOperationQueueStore";
import { useAppStore } from "../useAppStore";
import { useFileLibraryStore } from "../useFileLibraryStore";
import type { OperationQueueControllerContext } from "./controllerTypes";
import {
  createRestoreSessionId,
  currentT,
  localizedRestoreError,
  summarizeOperationRestore
} from "./restoreIntentResolver";

export async function prepareOperationRestoreIntent(
  { get, set }: OperationQueueControllerContext,
  selectedIds: ReadonlySet<string> | readonly string[]
): Promise<RestoreExecutionIntent | null> {
  const t = currentT();
  const requestedIds = [...new Set(selectedIds instanceof Set ? [...selectedIds] : selectedIds)];
  if (!requestedIds.length) return null;
  try {
    const authoritativeLogs = await get().refreshOperationLogs();
    const resolution = resolveOperationRestoreSelection(authoritativeLogs, requestedIds);
    const intent = createRestoreExecutionIntent(
      "operation_logs",
      resolution,
      createRestoreSessionId("operation_logs")
    );
    set({
      restoreIntent: intent,
      restoreError: resolution.executableCount ? "" : t("restoreNoExecutableSelected"),
      restoreTechnicalError: "",
      lastRestoreResult: [],
      lastRestoreSummary: null,
      cleanupRestoreResult: null,
      cleanupRestoreError: ""
    });
    return resolution.executableCount ? intent : null;
  } catch (error) {
    const copy = localizedRestoreError(error, t);
    set({ restoreError: copy.message, restoreTechnicalError: copy.technical });
    useAppStore.getState().showError(copy.message);
    return null;
  }
}

export async function confirmOperationRestore(
  { get, set }: OperationQueueControllerContext,
  sessionId: string
): Promise<RestoreConfirmationOutcome<OperationLog[]>> {
  const t = currentT();
  const intent = get().restoreIntent;
  if (!intent || intent.source !== "operation_logs" || intent.sessionId !== sessionId) {
    const message = t("historyRestoreSessionExpired");
    set({ restoreError: message, restoreTechnicalError: "" });
    return { status: "rejected", message };
  }

  let authoritativeLogs: OperationLog[];
  try {
    authoritativeLogs = await get().refreshOperationLogs();
  } catch (error) {
    const copy = localizedRestoreError(error, t);
    set({ restoreError: copy.message, restoreTechnicalError: copy.technical });
    useAppStore.getState().showError(copy.message);
    return { status: "rejected", message: copy.message };
  }

  const resolution = resolveOperationRestoreSelection(authoritativeLogs, intent.selectedIds);
  if (!restoreIntentMatchesResolution(intent, resolution)) {
    const nextIntent = createRestoreExecutionIntent(
      "operation_logs",
      resolution,
      createRestoreSessionId("operation_logs"),
      Date.now(),
      intent.revision + 1
    );
    set({ restoreIntent: nextIntent, restoreError: t("historyRestoreEligibilityChanged"), restoreTechnicalError: "" });
    return { status: "stale", intent: nextIntent };
  }

  const actualIds = resolveRestoreExecutionIds(intent.selectedIds, intent, resolution.executableIds);
  const logsById = new Map(resolution.executable.map((log) => [log.id, log]));
  const logs = actualIds.map((id) => logsById.get(id)).filter((log): log is OperationLog => Boolean(log));
  if (!logs.length) {
    const message = t("restoreNoExecutableSelected");
    set({ restoreError: message });
    return { status: "rejected", message };
  }

  set({
    activeOperationKind: "restore",
    isOperationCanceling: false,
    restoreError: "",
    restoreTechnicalError: "",
    lastRestoreResult: [],
    lastRestoreSummary: null,
    operationProgress: {
      kind: "restore",
      batchId: logs[0]?.batch_id ?? "",
      processed: 0,
      total: logs.length,
      currentPath: logs[0]?.path_after ?? ""
    }
  });

  try {
    const result = await tauriApi.restoreMoves(logs);
    const updatedById = new Map(result.logs.map((log) => [log.id, log]));
    const summary = summarizeOperationRestore(result.logs, intent.excludedCount);
    set((state) => ({
      operationLogs: state.operationLogs.map((log) => updatedById.get(log.id) ?? log),
      lastRestoreResult: result.logs,
      lastRestoreSummary: summary,
      restoreIntent: null,
      restoreError: "",
      restoreTechnicalError: ""
    }));
    for (const log of result.logs.filter((item) => item.operation_type === "rename" && item.restore_status === "restored")) {
      const selectedId = explicitSingleSelectionId(useFileLibrarySelectionStore.getState().selection);
      if (selectedId && samePathLike(selectedId, log.path_after)) await refreshRenamedSelection(selectedId, log.path_before);
    }
    await useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery).catch(() => undefined);
    const previewScope = get().previewScope;
    if (previewScope) await get().refreshPreviewsForScope(previewScope).catch(() => undefined);
    if (summary.failed > 0) {
      useAppStore.getState().showError(`${t("historyRestoreFailed")}: ${summary.failed.toLocaleString()}`);
    } else if (summary.canceled > 0 && summary.restored === 0) {
      useAppStore.getState().showSuccess(t("operationCanceled"));
    } else {
      useAppStore.getState().showSuccess(`${t("restored")}: ${summary.restored.toLocaleString()}`);
    }
    return { status: "executed", value: result.logs };
  } catch (error) {
    const copy = localizedRestoreError(error, t);
    set({ restoreError: copy.message, restoreTechnicalError: copy.technical });
    useAppStore.getState().showError(copy.message);
    return { status: "rejected", message: copy.message };
  } finally {
    set({ activeOperationKind: null, isOperationCanceling: false, operationProgress: null });
  }
}
