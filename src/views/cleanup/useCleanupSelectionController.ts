import { useCallback, useMemo, useRef, useState, type Dispatch, type SetStateAction } from "react";
import type {
  AnalysisFinding,
  AnalysisRun,
  CleanupFindingSelection
} from "../../types/domain";
import { resolveReclaimableBytes } from "../../utils/reclaimableBytes";
import {
  isBackendDefaultSafeFinding,
  isFindingSelectable,
  reconcileAuthoritativeFindingUpdates,
  retainSelectedCleanupFindingsAfterExecution
} from "./cleanupModel";
import type { CleanupExecutionResult } from "../../types/domain";

type Props = {
  findingCache: Record<string, AnalysisFinding>;
  setFindingCache: Dispatch<SetStateAction<Record<string, AnalysisFinding>>>;
  run: AnalysisRun | null;
  invalidatePreviewState: () => void;
};

export function useCleanupSelectionController({ findingCache, setFindingCache, run, invalidatePreviewState }: Props) {
  const findingCacheRef = useRef(findingCache);
  findingCacheRef.current = findingCache;
  const [selectedFindingIds, setSelectedFindingIds] = useState<Set<string>>(() => new Set());
  const selectedFindingIdsRef = useRef(selectedFindingIds);
  const selectionRevision = useRef(0);

  const commitSelection = useCallback((update: (current: Set<string>) => Set<string>) => {
    selectionRevision.current += 1;
    invalidatePreviewState();
    setSelectedFindingIds((current) => {
      const next = update(current);
      selectedFindingIdsRef.current = next;
      return next;
    });
  }, [invalidatePreviewState]);

  const resetSelection = useCallback(() => {
    selectionRevision.current += 1;
    selectedFindingIdsRef.current = new Set();
    setSelectedFindingIds(selectedFindingIdsRef.current);
  }, []);

  const selectBackendDefaultSafeFindings = useCallback((page: readonly AnalysisFinding[]) => {
    commitSelection((current) => {
      const next = new Set(current);
      for (const finding of page) {
        if (isBackendDefaultSafeFinding(finding)) next.add(finding.id);
      }
      return next;
    });
  }, [commitSelection]);

  const selectedFindings = useMemo(
    () => [...selectedFindingIds]
      .map((id) => findingCache[id])
      .filter((finding): finding is AnalysisFinding => Boolean(finding)),
    [findingCache, selectedFindingIds]
  );
  const selectedBytes = useMemo(
    () => selectedFindings.reduce((sum, finding) => sum + resolveReclaimableBytes({
      exact: finding.exactReclaimableBytes,
      potential: finding.potentialReclaimableBytes,
      legacy: finding.sizeBytes
    }).bytes, 0),
    [selectedFindings]
  );
  const runReclaimable = useMemo(
    () => resolveReclaimableBytes({ exact: run?.exactReclaimableBytes, potential: run?.potentialReclaimableBytes }),
    [run]
  );

  const buildSelections = useCallback((): CleanupFindingSelection[] => {
    if (!selectedFindings.length) return [];
    return selectedFindings.map((finding) => {
      if (!isFindingSelectable(finding)) throw new Error("cleanup_selection_not_executable");
      const selection: CleanupFindingSelection = { findingId: finding.id, expectedRevision: finding.revision };
      if (finding.tier === "review") {
        if (finding.decision !== "acknowledged" || finding.decisionRevision == null) {
          throw new Error("cleanup_review_confirmation_required");
        }
        selection.reviewConfirmation = { decisionRevision: finding.decisionRevision };
      }
      return selection;
    });
  }, [selectedFindings]);

  const reconcileUpdatedFindings = useCallback((updatedFindings: AnalysisFinding[]) => {
    if (!updatedFindings.length) return;
    const selectedBeforeUpdate = selectedFindingIdsRef.current;
    const selectedRevisionAffected = updatedFindings.some((finding) => {
      if (!selectedBeforeUpdate.has(finding.id)) return false;
      const old = findingCacheRef.current[finding.id];
      return !old || old.revision !== finding.revision || old.decisionRevision !== finding.decisionRevision
        || old.decision !== finding.decision || old.status !== finding.status || old.tier !== finding.tier
        || old.executable !== finding.executable || old.actionKind !== finding.actionKind;
    });
    setFindingCache((cache) => {
      const next = { ...cache };
      for (const finding of updatedFindings) next[finding.id] = finding;
      findingCacheRef.current = next;
      return next;
    });
    const nextSelected = reconcileAuthoritativeFindingUpdates(selectedBeforeUpdate, updatedFindings);
    selectedFindingIdsRef.current = nextSelected;
    if (selectedRevisionAffected) {
      selectionRevision.current += 1;
      setSelectedFindingIds(nextSelected);
      invalidatePreviewState();
    }
  }, [invalidatePreviewState]);

  const removeSelectionsForIds = useCallback((findingIds: readonly string[]) => {
    if (!findingIds.length) return;
    const rejected = new Set(findingIds);
    const nextSelected = new Set([...selectedFindingIdsRef.current].filter((id) => !rejected.has(id)));
    selectedFindingIdsRef.current = nextSelected;
    selectionRevision.current += 1;
    setSelectedFindingIds(nextSelected);
    invalidatePreviewState();
  }, [invalidatePreviewState]);

  const reconcileAfterSafeTrash = useCallback((
    selectedIds: readonly string[],
    result: CleanupExecutionResult,
    refreshedFindings: readonly AnalysisFinding[]
  ) => {
    const successfulIds = new Set(result.logs
      .filter((log) => log.status === "success" && typeof log.candidateId === "string" && log.candidateId.length > 0)
      .map((log) => log.candidateId as string));
    setFindingCache((cache) => {
      const next = { ...cache };
      for (const finding of refreshedFindings) next[finding.id] = finding;
      findingCacheRef.current = next;
      return next;
    });
    const nextSelected = retainSelectedCleanupFindingsAfterExecution(selectedIds, successfulIds, refreshedFindings);
    selectionRevision.current += 1;
    selectedFindingIdsRef.current = nextSelected;
    setSelectedFindingIds(nextSelected);
  }, [setFindingCache]);

  return {
    selectedFindingIds,
    selectedFindingIdsRef,
    selectionRevision,
    selectedFindings,
    selectedBytes,
    runReclaimable,
    commitSelection,
    resetSelection,
    selectBackendDefaultSafeFindings,
    buildSelections,
    reconcileUpdatedFindings,
    removeSelectionsForIds,
    reconcileAfterSafeTrash
  };
}
