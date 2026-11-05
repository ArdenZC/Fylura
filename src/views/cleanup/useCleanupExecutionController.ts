import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import type {
  AnalysisFinding,
  AnalysisRun,
  CleanupExecutionResult,
  CleanupFindingSelection,
  OperationPreviewResult
} from "../../types/domain";
import type { Translator } from "../../types/ui";
import { cleanupSelectionFingerprint, executableCleanupSubset } from "./cleanupModel";
import type { CleanupApi, CleanupMutationOwner } from "./cleanupControllerTypes";

type Props = {
  api: CleanupApi;
  run: AnalysisRun | null;
  runRef: MutableRefObject<AnalysisRun | null>;
  selectedFindings: readonly AnalysisFinding[];
  selectionRevision: MutableRefObject<number>;
  buildSelections: () => CleanupFindingSelection[];
  resetSelection: () => void;
  reconcileAfterSafeTrash: (selectedIds: readonly string[], result: CleanupExecutionResult, refreshedFindings: readonly AnalysisFinding[]) => void;
  preview: OperationPreviewResult | null;
  setPreview: Dispatch<SetStateAction<OperationPreviewResult | null>>;
  setConfirmPreviewOpen: Dispatch<SetStateAction<boolean>>;
  setExecutionResult: Dispatch<SetStateAction<CleanupExecutionResult | null>>;
  setError: Dispatch<SetStateAction<string>>;
  scopeEpoch: MutableRefObject<number>;
  aiOperationEpoch: MutableRefObject<number>;
  previewRequestEpoch: MutableRefObject<number>;
  previewSelectionFingerprint: MutableRefObject<string | null>;
  mutationOwnerRef: MutableRefObject<CleanupMutationOwner | null>;
  interactionLockedRef: MutableRefObject<boolean>;
  mutationUnavailable: string | null;
  beginMutation: (kind: "preview" | "safe_trash", runId?: string | null) => CleanupMutationOwner | null;
  releaseMutation: (owner: CleanupMutationOwner) => boolean;
  invalidatePreviewState: () => void;
  loadRunDetails: (runId: string, clearFindings?: boolean, expectedScopeEpoch?: number) => Promise<void>;
  reportError: (value: unknown) => void;
  t: Translator;
};

export function useCleanupExecutionController({
  api,
  run,
  runRef,
  selectedFindings,
  selectionRevision,
  buildSelections,
  resetSelection,
  reconcileAfterSafeTrash,
  preview,
  setPreview,
  setConfirmPreviewOpen,
  setExecutionResult,
  setError,
  scopeEpoch,
  aiOperationEpoch,
  previewRequestEpoch,
  previewSelectionFingerprint,
  mutationOwnerRef,
  interactionLockedRef,
  mutationUnavailable,
  beginMutation,
  releaseMutation,
  invalidatePreviewState,
  loadRunDetails,
  reportError,
  t
}: Props) {
  const previewSelected = useCallback(async () => {
    if (!run || !api.previewCleanupOperations || !selectedFindings.length || interactionLockedRef.current) return;
    const expectedScopeEpoch = scopeEpoch.current;
    const runId = run.id;
    const runRevision = run.revision;
    const expectedAiOperationEpoch = aiOperationEpoch.current;
    const expectedSelectionRevision = selectionRevision.current;
    if (mutationUnavailable) {
      reportError(t("storageCleanupMutationUnavailable"));
      return;
    }
    let selections: CleanupFindingSelection[];
    try {
      selections = buildSelections();
    } catch (selectionError) {
      reportError(selectionError);
      return;
    }
    const selectionFingerprint = cleanupSelectionFingerprint(runId, selections);
    const expectedPreviewRequestEpoch = previewRequestEpoch.current + 1;
    previewRequestEpoch.current = expectedPreviewRequestEpoch;
    const mutationOwner = beginMutation("preview", runId);
    if (!mutationOwner) return;
    setError("");
    const ownsRequest = () => expectedScopeEpoch === scopeEpoch.current
      && expectedAiOperationEpoch === aiOperationEpoch.current
      && expectedSelectionRevision === selectionRevision.current
      && expectedPreviewRequestEpoch === previewRequestEpoch.current
      && runRef.current?.id === runId
      && runRef.current.revision === runRevision
      && mutationOwnerRef.current?.id === mutationOwner.id;
    try {
      const result = await api.previewCleanupOperations(runId, selections);
      if (!ownsRequest()) return;
      previewSelectionFingerprint.current = selectionFingerprint;
      setPreview(result);
    } catch (previewError) {
      if (ownsRequest()) reportError(previewError);
    } finally {
      releaseMutation(mutationOwner);
    }
  }, [aiOperationEpoch, api, beginMutation, buildSelections, interactionLockedRef, mutationOwnerRef, mutationUnavailable, previewRequestEpoch, previewSelectionFingerprint, releaseMutation, reportError, run, runRef, selectedFindings.length, selectionRevision, scopeEpoch, setError, setPreview, t]);

  const moveSelectedToSafeTrash = useCallback(async () => {
    if (!run || !api.moveCleanupCandidatesToSafeTrash || !preview || !selectedFindings.length || interactionLockedRef.current) return;
    const expectedScopeEpoch = scopeEpoch.current;
    const runId = run.id;
    const expectedAiOperationEpoch = aiOperationEpoch.current;
    const expectedSelectionRevision = selectionRevision.current;
    const selections = (() => {
      try {
        return buildSelections();
      } catch (selectionError) {
        reportError(selectionError);
        return null;
      }
    })();
    if (!selections) return;
    const executable = executableCleanupSubset(preview, selections);
    if (!executable.length) return;
    const selectedIds = selectedFindings.map((finding) => finding.id);
    const selectionFingerprint = cleanupSelectionFingerprint(runId, selections);
    if (previewSelectionFingerprint.current !== selectionFingerprint) {
      invalidatePreviewState();
      return;
    }
    const mutationOwner = beginMutation("safe_trash", runId);
    if (!mutationOwner) return;
    setError("");
    const ownsRequest = () => expectedScopeEpoch === scopeEpoch.current
      && expectedAiOperationEpoch === aiOperationEpoch.current
      && expectedSelectionRevision === selectionRevision.current
      && previewSelectionFingerprint.current === selectionFingerprint
      && runRef.current?.id === runId
      && mutationOwnerRef.current?.id === mutationOwner.id;
    const ownsResultRefresh = () => expectedScopeEpoch === scopeEpoch.current
      && expectedAiOperationEpoch === aiOperationEpoch.current
      && runRef.current?.id === runId
      && mutationOwnerRef.current?.id === mutationOwner.id;
    try {
      const result = await api.moveCleanupCandidatesToSafeTrash(runId, executable);
      if (!ownsRequest()) return;
      invalidatePreviewState();
      setExecutionResult(result);
      previewSelectionFingerprint.current = null;
      setConfirmPreviewOpen(false);
      await loadRunDetails(runId, true, expectedScopeEpoch);
      if (!ownsResultRefresh()) return;
      if (!api.getAnalysisFinding) {
        resetSelection();
        reportError(new Error("cleanup_execution_finding_refresh_unavailable"));
        return;
      }
      try {
        const refreshed = await Promise.all(selectedIds.map((id) => api.getAnalysisFinding!(id)));
        if (!ownsResultRefresh()) return;
        reconcileAfterSafeTrash(
          selectedIds,
          result,
          refreshed.filter((finding): finding is AnalysisFinding => Boolean(finding))
        );
      } catch (refreshError) {
        if (ownsResultRefresh()) {
          resetSelection();
          reportError(new Error("cleanup_execution_finding_refresh_failed", { cause: refreshError }));
        }
      }
    } catch (executionError) {
      if (ownsRequest()) {
        invalidatePreviewState();
        resetSelection();
        await loadRunDetails(runId, true, expectedScopeEpoch);
        reportError(executionError);
      }
    } finally {
      releaseMutation(mutationOwner);
    }
  }, [aiOperationEpoch, api, beginMutation, buildSelections, invalidatePreviewState, interactionLockedRef, loadRunDetails, mutationOwnerRef, preview, previewSelectionFingerprint, reconcileAfterSafeTrash, releaseMutation, reportError, resetSelection, run, runRef, selectedFindings, selectionRevision, setConfirmPreviewOpen, setError, setExecutionResult, setPreview, scopeEpoch]);

  return { previewSelected, moveSelectedToSafeTrash };
}
