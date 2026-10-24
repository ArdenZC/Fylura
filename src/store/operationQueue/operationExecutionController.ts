import { refreshRenamedSelection } from "./renameSelectionRefresh";
import { tauriApi } from "../../api/tauriApi";
import { makeTranslator } from "../../i18n";
import type {
  OperationLog,
  OperationPreview,
  OperationPreviewResult,
  RuleExecutionSummary
} from "../../types/domain";
import { applyPreviewNameOverride, localizedStableError, readableError, samePathLike } from "../../utils/viewHelpers";
import { useAppStore } from "../useAppStore";
import { useFileLibraryStore } from "../useFileLibraryStore";
import { resolveLegacyLibraryScope } from "../useFileLibraryV2Store";
import { useRulesStore } from "../useRulesStore";
import {
  requiresExplicitMaterialization,
  resolveExecutableSelectedPreviews,
  type PreviewExecutionIntent
} from "./selectors";
import type { OperationQueueControllerContext } from "./controllerTypes";

function isAuthoritativePreviewStale(error: unknown) {
  return /operation_preview_stale|authoritative preview/i.test(readableError(error));
}

function mergeRefreshedPreviews(
  result: OperationPreviewResult,
  additionalPreviews: OperationPreview[]
): OperationPreviewResult {
  const previews = [...result.previews];
  const indexById = new Map(previews.map((preview, index) => [preview.id, index]));
  let added = 0;
  for (const preview of additionalPreviews) {
    const existingIndex = indexById.get(preview.id);
    if (existingIndex === undefined) {
      indexById.set(preview.id, previews.length);
      previews.push(preview);
      added += 1;
    } else {
      previews[existingIndex] = preview;
    }
  }
  return {
    ...result,
    previews,
    total: result.total + added,
    limit: Math.max(result.limit, previews.length)
  };
}

async function reacquireStaleOperationPreviews(
  { get }: OperationQueueControllerContext,
  staleOperations: OperationPreview[]
) {
  const state = get();
  const previewScope = state.previewScope ?? useFileLibraryStore.getState().scope;
  const ordinaryOperations = staleOperations.filter(
    (operation) => operation.operation_type !== "permanent_delete" && !operation.id.startsWith("op-manual-rename-")
  );
  const permanentFileIds = [
    ...new Set(
      staleOperations
        .filter((operation) => operation.operation_type === "permanent_delete")
        .map((operation) => operation.fileId)
    )
  ];

  let refreshedOrdinary: OperationPreviewResult | null = null;
  if (ordinaryOperations.length) {
    refreshedOrdinary = state.previewScope && state.previewSelection
      ? await get().refreshPreviewsForSelection(state.previewScope, state.previewSelection)
      : await get().refreshPreviewsForScope(previewScope);
  }

  const manualRenames = staleOperations.filter((operation) => operation.id.startsWith("op-manual-rename-"));
  if (!permanentFileIds.length && !manualRenames.length) return;

  const refreshedPermanent = await Promise.all(
    [...permanentFileIds.map((fileId) => tauriApi.getPermanentDeleteOperationPreview(fileId)),
      ...manualRenames.map((operation) => tauriApi.getManualRenameOperationPreview(operation.fileId, operation.new_name))]
  );
  if (refreshedOrdinary) {
    get().setPreviewResult(
      mergeRefreshedPreviews(refreshedOrdinary, refreshedPermanent),
      previewScope,
      state.previewSelection
    );
    return;
  }

  const previews = refreshedPermanent.filter(
    (preview, index, all) => all.findIndex((candidate) => candidate.id === preview.id) === index
  );
  get().setPreviewResult(
    {
      previews,
      total: previews.length,
      limit: previews.length,
      offset: 0,
      truncated: false,
      hasMore: false
    },
    previewScope,
    state.previewSelection
  );
}

export async function runDispatch(
  { get }: OperationQueueControllerContext,
  confirmed: boolean
): Promise<RuleExecutionSummary> {
  const t = makeTranslator(useAppStore.getState().language);
  if (!confirmed) {
    return { scanned: 0, updated: 0, skipped: 0, needsConfirmation: 0 };
  }
  try {
    const scope = useFileLibraryStore.getState().scope;
    const durableScope = await resolveLegacyLibraryScope(scope);
    const { summary } = await tauriApi.executeRulesForScopeV2(
      durableScope,
      useRulesStore.getState().catalogRevision,
      "inbox_only",
      true
    );
    await useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery);
    await get().refreshPreviewsForScope(scope);
    useAppStore.getState().showSuccess(
      `${t("success")}: ${summary.updated.toLocaleString()} / ${summary.scanned.toLocaleString()} (${t("skipped")}: ${summary.skipped.toLocaleString()})`
    );
    return summary;
  } catch (error) {
    useAppStore.getState().showError(readableError(error));
    throw error;
  }
}

export async function executeSelected(
  { get, set }: OperationQueueControllerContext,
  confirmed: boolean
): Promise<OperationLog[]> {
  const t = makeTranslator(useAppStore.getState().language);
  if (!confirmed) return [];
  const { displayPreviews, selectedOperationIds, executionIntent } = get();
  let { operations } = resolveExecutableSelectedPreviews(
    displayPreviews,
    selectedOperationIds,
    executionIntent
  );
  if (!operations.length) return [];

  const materializationTargets = operations.filter(requiresExplicitMaterialization);

  set({
    activeOperationKind: materializationTargets.length ? "materialize" : "execute",
    lastExecutionLogs: [],
    executionError: "",
    isOperationCanceling: false,
    operationProgress: {
      kind: materializationTargets.length ? "materialize" : "execute",
      batchId: "",
      processed: 0,
      total: materializationTargets.length || operations.length,
      currentPath: operations[0]?.source_path ?? ""
    }
  });

  try {
    const initialCanonicalById = new Map(get().previews.map((preview) => [preview.id, preview]));
    const nameOverrides = get().previewNameOverrides;
    const materializedFingerprints = new Map<string, string>();
    for (const [index, preview] of materializationTargets.entries()) {
      set({
        operationProgress: {
          kind: "materialize",
          batchId: preview.id,
          processed: index,
          total: materializationTargets.length,
          currentPath: preview.source_path
        }
      });
      const materialized = await tauriApi.materializeProviderPreview(preview);
      if (materialized.previewId !== preview.id || materialized.fileId !== preview.fileId) {
        throw new Error("The authoritative preview changed during materialization; refresh it before executing.");
      }
      const nextFingerprint = materialized.nextOperationFingerprint;
      if (!nextFingerprint) {
        throw new Error("The authoritative preview did not return a post-materialization fingerprint.");
      }
      materializedFingerprints.set(preview.id, nextFingerprint);
    }
    if (materializationTargets.length) {
      const previewScope = get().previewScope;
      const previewSelection = get().previewSelection;
      let freshPreviews: OperationPreview[];
      if (previewScope) {
        if (previewSelection) {
          await get().refreshPreviewsForSelection(previewScope, previewSelection);
        } else {
          await get().refreshPreviewsForScope(previewScope);
        }
        freshPreviews = get().previews;
      } else {
        freshPreviews = await tauriApi.getOperationPreviewsByFileIds(
          operations.map((operation) => operation.fileId)
        );
      }

      const freshById = new Map(freshPreviews.map((preview) => [preview.id, preview]));
      for (const original of operations) {
        const fresh = freshById.get(original.id);
        const baseline = initialCanonicalById.get(original.id) ?? original;
        if (!fresh
          || fresh.fileId !== baseline.fileId
          || fresh.source_path !== baseline.source_path
          || fresh.target_path !== baseline.target_path
          || fresh.operation_type !== baseline.operation_type
          || fresh.conflict_policy !== baseline.conflict_policy
          || fresh.providerIdentityFingerprint !== baseline.providerIdentityFingerprint
          || (materializedFingerprints.has(original.id)
            && fresh.operationFingerprint !== materializedFingerprints.get(original.id))) {
          throw new Error("The authoritative preview changed during materialization; refresh it before executing.");
        }
      }

      const freshDisplayPreviews = freshPreviews.map((preview) =>
        applyPreviewNameOverride(preview, nameOverrides[preview.id])
      );
      const refreshedSelection = resolveExecutableSelectedPreviews(
        freshDisplayPreviews,
        selectedOperationIds,
        executionIntent
      );
      const originalIds = new Set(operations.map((operation) => operation.id));
      const refreshedIds = new Set(refreshedSelection.operations.map((operation) => operation.id));
      if (refreshedSelection.operations.length !== operations.length
        || [...originalIds].some((id) => !refreshedIds.has(id))) {
        throw new Error("The authoritative preview is no longer executable; refresh it before executing.");
      }
      operations = refreshedSelection.operations;
      if (previewScope) {
        const validOverrides = Object.fromEntries(
          Object.entries(nameOverrides).filter(([id]) => freshById.has(id))
        );
        set({
          previewNameOverrides: validOverrides,
          displayPreviews: freshDisplayPreviews
        });
      }
      set({
        activeOperationKind: "execute",
        operationProgress: {
          kind: "execute",
          batchId: "",
          processed: 0,
          total: operations.length,
          currentPath: operations[0]?.source_path ?? ""
        }
      });
    }
    const result = await tauriApi.executeMoves(operations as OperationPreview[]);
    set((state) => ({
      operationLogs: [...result.logs, ...state.operationLogs].slice(0, 500),
      lastExecutionLogs: result.logs,
      selectedOperationIds: new Set()
    }));
    for (const operation of operations.filter((item) => item.id.startsWith("op-manual-rename-"))) {
      const log = result.logs.find((item) => item.status === "success" && item.operation_type === "rename" && item.new_name === operation.new_name && samePathLike(item.source_path, operation.source_path));
      if (log) await refreshRenamedSelection(operation.fileId, log.path_after);
    }
    await useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery);
    const previewScope = get().previewScope;
    if (previewScope) await get().refreshPreviewsForScope(previewScope);
    const succeeded = result.logs.filter((log) => log.status === "success").length;
    const failed = result.logs.filter((log) => log.status === "failed").length;
    const skipped = result.logs.filter((log) => log.status === "skipped").length;
    if (failed > 0) {
      useAppStore.getState().showError(`${t("failed")}: ${failed.toLocaleString()}`);
    } else if (succeeded === 0 && skipped > 0) {
      useAppStore.getState().showSuccess(t("operationCanceled"));
    } else {
      useAppStore.getState().showSuccess(
        `${t("success")}: ${succeeded.toLocaleString()}${skipped ? ` (${t("skipped")}: ${skipped.toLocaleString()})` : ""}`
      );
    }
    return result.logs;
  } catch (error) {
    const stale = isAuthoritativePreviewStale(error);
    if (stale) {
      if (operations.some((operation) => operation.id.startsWith("op-manual-rename-"))) {
        set({ selectedOperationIds: new Set() });
      }
      // A stale batch is fail-closed: reacquire only the current authoritative
      // previews and leave the next execution to a new explicit user action.
      await reacquireStaleOperationPreviews({ get, set }, operations).catch(() => undefined);
    }
    const message = stale ? t("organizePreviewInvalidated") : localizedStableError(error, t);
    set({ executionError: message });
    useAppStore.getState().showError(message);
    return [];
  } finally {
    set({
      activeOperationKind: null,
      isOperationCanceling: false,
      operationProgress: null
    });
  }
}

export type { PreviewExecutionIntent };
