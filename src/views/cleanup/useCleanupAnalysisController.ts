import { useCallback, useEffect, type Dispatch, type MutableRefObject, type SetStateAction } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type {
  AnalysisDetector,
  AnalysisDetectorDescriptor,
  AnalysisFinding,
  AnalysisFindingGroup,
  AnalysisRun
} from "../../types/domain";
import {
  FINDING_PAGE_SIZE,
  isCleanupRun,
  isRunInProgress,
  scopeKey,
  scopePaths,
  type CleanupTier
} from "./cleanupModel";
import type { CleanupApi } from "./cleanupControllerTypes";

type Props = {
  api: CleanupApi;
  initialRoots?: string[];
  run: AnalysisRun | null;
  activeTier: CleanupTier;
  activeTierRef: MutableRefObject<CleanupTier>;
  activeTierEpoch: MutableRefObject<number>;
  findingsEpoch: MutableRefObject<number>;
  scopeEpoch: MutableRefObject<number>;
  runRef: MutableRefObject<AnalysisRun | null>;
  scopeHydrated: MutableRefObject<boolean>;
  defaultSelectionRuns: MutableRefObject<Set<string>>;
  setDetectors: Dispatch<SetStateAction<AnalysisDetectorDescriptor[]>>;
  setRuns: Dispatch<SetStateAction<AnalysisRun[]>>;
  setRun: Dispatch<SetStateAction<AnalysisRun | null>>;
  setRunDetectors: Dispatch<SetStateAction<AnalysisDetector[]>>;
  setGroups: Dispatch<SetStateAction<AnalysisFindingGroup[]>>;
  setFindings: Dispatch<SetStateAction<AnalysisFinding[]>>;
  reconcileUpdatedFindings: (findings: AnalysisFinding[]) => void;
  setNextCursor: Dispatch<SetStateAction<string | null>>;
  setSelectedRoots: Dispatch<SetStateAction<string[]>>;
  setLoading: Dispatch<SetStateAction<boolean>>;
  setLoadingFindings: Dispatch<SetStateAction<boolean>>;
  setUnsupported: Dispatch<SetStateAction<boolean>>;
  selectBackendDefaultSafeFindings: (page: readonly AnalysisFinding[]) => void;
  reportError: (value: unknown) => void;
};

export function useCleanupAnalysisController({
  api,
  initialRoots,
  run,
  activeTier,
  activeTierRef,
  activeTierEpoch,
  findingsEpoch,
  scopeEpoch,
  runRef,
  scopeHydrated,
  defaultSelectionRuns,
  setDetectors,
  setRuns,
  setRun,
  setRunDetectors,
  setFindings,
  setGroups,
  reconcileUpdatedFindings,
  setNextCursor,
  setSelectedRoots,
  setLoading,
  setLoadingFindings,
  setUnsupported,
  selectBackendDefaultSafeFindings,
  reportError
}: Props) {
  const loadFindings = useCallback(async (
    runId: string,
    tier: CleanupTier,
    cursor: string | null = null,
    append = false,
    expectedRunRevision: number | null = runRef.current?.revision ?? null
  ) => {
    if (!api.listAnalysisFindings) return;
    const epoch = ++findingsEpoch.current;
    const expectedScopeEpoch = scopeEpoch.current;
    const expectedTierEpoch = activeTierEpoch.current;
    const ownsRequest = () => expectedScopeEpoch === scopeEpoch.current
      && expectedTierEpoch === activeTierEpoch.current
      && activeTierRef.current === tier
      && epoch === findingsEpoch.current
      && runRef.current?.id === runId
      && (expectedRunRevision === null || runRef.current.revision === expectedRunRevision);
    setLoadingFindings(true);
    try {
      const page = await api.listAnalysisFindings({
        runId,
        tier,
        status: "active",
        cursor,
        limit: FINDING_PAGE_SIZE
      });
      if (!ownsRequest() || page.findings.some((finding) => finding.runId !== runId)) return;
      if (page.groups) setGroups(page.groups);
      setFindings((current) => append ? [...current, ...page.findings] : page.findings);
      setNextCursor(page.nextCursor);
      reconcileUpdatedFindings(page.findings);
      if (!append && tier === "safe" && !defaultSelectionRuns.current.has(runId)) {
        defaultSelectionRuns.current.add(runId);
        selectBackendDefaultSafeFindings(page.findings);
      }
    } catch (loadError) {
      if (ownsRequest()) reportError(loadError);
    } finally {
      if (ownsRequest()) setLoadingFindings(false);
    }
  }, [activeTierEpoch, activeTierRef, api, defaultSelectionRuns, findingsEpoch, reportError, runRef, scopeEpoch, selectBackendDefaultSafeFindings, reconcileUpdatedFindings, setFindings, setGroups, setLoadingFindings, setNextCursor]);

  const loadRunDetails = useCallback(async (runId: string, clearFindings = true, expectedScopeEpoch = scopeEpoch.current) => {
    if (!api.getAnalysisRun || !api.listAnalysisRunDetectors) return;
    try {
      const [nextRun, nextDetectors] = await Promise.all([
        api.getAnalysisRun(runId),
        api.listAnalysisRunDetectors(runId)
      ]);
      if (expectedScopeEpoch !== scopeEpoch.current) return;
      const currentRun = runRef.current;
      if (currentRun && currentRun.id === nextRun.id && nextRun.revision < currentRun.revision) return;
      if (clearFindings || !currentRun || currentRun.id !== nextRun.id || nextRun.revision > currentRun.revision) findingsEpoch.current += 1;
      if (clearFindings) {
        setFindings([]);
        setNextCursor(null);
      }
      runRef.current = nextRun;
      setRun((current) => current && current.id === nextRun.id && nextRun.revision < current.revision ? current : nextRun);
      setRunDetectors(nextDetectors);
      setRuns((current) => {
        const withoutCurrent = current.filter((item) => item.id !== nextRun.id);
        return [nextRun, ...withoutCurrent].slice(0, 20);
      });
      if (!scopeHydrated.current) {
        const paths = scopePaths(nextRun);
        if (paths.length) {
          scopeHydrated.current = true;
          setSelectedRoots(paths);
        }
      }
    } catch (loadError) {
      if (expectedScopeEpoch === scopeEpoch.current) reportError(loadError);
    }
  }, [api, findingsEpoch, reportError, runRef, scopeEpoch, scopeHydrated, setFindings, setNextCursor, setRun, setRunDetectors, setRuns, setSelectedRoots]);

  useEffect(() => {
    let disposed = false;
    const hydrationScopeEpoch = scopeEpoch.current;
    async function hydrate() {
      if (!api.listAnalysisDetectors || !api.listAnalysisRuns) {
        setUnsupported(true);
        setLoading(false);
        return;
      }
      try {
        const activePromise = api.getActiveAnalysisRun ? api.getActiveAnalysisRun() : Promise.resolve(null);
        const [availableDetectors, listedRuns, activeRun] = await Promise.all([
          api.listAnalysisDetectors(),
          api.listAnalysisRuns(20),
          activePromise
        ]);
        if (disposed || hydrationScopeEpoch !== scopeEpoch.current) return;
        setDetectors(availableDetectors);
        const cleanupRuns = listedRuns.filter(isCleanupRun);
        setRuns(cleanupRuns);
        const candidates = (activeRun && isCleanupRun(activeRun)
          ? [activeRun, ...cleanupRuns.filter((listedRun) => listedRun.id !== activeRun.id)]
          : cleanupRuns)
          .slice()
          .sort((left, right) => right.updatedAt - left.updatedAt || right.createdAt - left.createdAt);
        const requestedScopeKey = scopeKey(initialRoots ?? []);
        const candidate = requestedScopeKey
          ? candidates.find((listedRun) => scopeKey(scopePaths(listedRun)) === requestedScopeKey) ?? null
          : candidates[0] ?? null;
        if (candidate) {
          if (hydrationScopeEpoch !== scopeEpoch.current) return;
          await loadRunDetails(candidate.id);
        }
      } catch (loadError) {
        if (!disposed) reportError(loadError);
      } finally {
        if (!disposed) setLoading(false);
      }
    }
    void hydrate().catch(() => undefined);
    return () => {
      disposed = true;
    };
  }, [api, initialRoots, loadRunDetails, reportError, scopeEpoch, setDetectors, setLoading, setRuns, setUnsupported]);

  useEffect(() => {
    if (!run || !api.listAnalysisFindings || isRunInProgress(run)) return;
    void loadFindings(run.id, activeTier, null, false, run.revision).catch(() => undefined);
  }, [activeTier, api.listAnalysisFindings, loadFindings, run]);

  useEffect(() => {
    const disposers: UnlistenFn[] = [];
    let disposed = false;
    const register = async (subscription: Promise<UnlistenFn> | undefined) => {
      const off = await subscription;
      if (!off) return;
      if (disposed) off();
      else disposers.push(off);
    };
    async function subscribe() {
      await Promise.all([
        register(api.onAnalysisRunUpdated?.((updated) => {
          const current = runRef.current;
          if (disposed || !current || !isCleanupRun(updated) || updated.id !== current.id) return;
          if (updated.revision >= current.revision) void loadRunDetails(updated.id).catch(() => undefined);
        })),
        register(api.onAnalysisFindingsPublished?.((updated) => {
          if (!disposed && updated.id === runRef.current?.id) void loadRunDetails(updated.id).catch(() => undefined);
        })),
        register(api.onAnalysisDetectorUpdated?.((updated) => {
          if (!disposed && updated.runId === runRef.current?.id) void loadRunDetails(updated.runId, false).catch(() => undefined);
        }))
      ]);
      // Native registration is asynchronous. Reconcile after every listener is
      // attached so a fast run cannot finish inside the subscription gap and
      // leave the UI permanently running. The backend remains the authority.
      const current = runRef.current;
      if (!disposed && current && isRunInProgress(current)) await loadRunDetails(current.id, false);
    }
    void subscribe().catch(() => undefined);
    return () => {
      disposed = true;
      while (disposers.length) disposers.pop()?.();
    };
  }, [api, loadRunDetails, run?.id, runRef]);

  return { loadFindings, loadRunDetails };
}
