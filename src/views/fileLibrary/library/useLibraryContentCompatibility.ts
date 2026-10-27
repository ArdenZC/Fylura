import { usePaneProjection, usePane } from "../workbench/PaneContext";
import { useCallback, useEffect, useRef, useState } from "react";
import { tauriApi } from "../../../api/tauriApi";
import { explicitSingleSelectionId, useFileLibraryInspectorStore, type InspectorDetailLoadResult } from "../../../store/useFileLibraryV2Store";
import type { FileLibraryDetail } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import type { ContentRefreshResult } from "../../vault/components/ContentUnderstandingSheet";
import type { LibrarySourceOwner } from "./librarySourceOwner";

type LibraryContentSource = Pick<
  LibrarySourceOwner,
  "files" | "selection" | "selectedIds" | "setExplicitSelection" | "ownsSingleFileSelection" | "loadDetail" | "commitDetailIfCurrent"
>;

/**
 * Temporary W2 compatibility controller for the existing content sheet. The
 * sheet remains a leaf; all detail publication still goes through Inspector
 * V2 and its request-epoch guards.
 */
export function useLibraryContentCompatibility({
  source,
  t,
  onError,
  restoreFocus
}: {
  source: LibraryContentSource;
  t: Translator;
  onError: (message: string) => void;
  restoreFocus: (target: HTMLElement | null) => void;
}) {
  const { useFileLibraryInspectorStore } = usePaneProjection();
  const [contentDetail, setContentDetail] = useState<FileLibraryDetail | null>(null);
  const canonicalSingleSelectionId = explicitSingleSelectionId(source.selection);
  const contentTriggerRef = useRef<HTMLElement | null>(null);
  const contentRestoreTargetRef = useRef<HTMLElement | null>(null);
  const contentOpenEpoch = useRef(0);
  const pendingContentOpenRef = useRef<{ epoch: number; fileId: string } | null>(null);
  const contentRefreshEpoch = useRef(0);
  const contentDetailRef = useRef<FileLibraryDetail | null>(null);

  useEffect(() => {
    contentDetailRef.current = contentDetail;
  }, [contentDetail]);

  const closeContentUnderstanding = useCallback(() => {
    const restoreTarget = contentTriggerRef.current;
    contentRestoreTargetRef.current = restoreTarget;
    contentRefreshEpoch.current += 1;
    contentOpenEpoch.current += 1;
    pendingContentOpenRef.current = null;
    contentTriggerRef.current = null;
    contentDetailRef.current = null;
    setContentDetail(null);
    // ContentUnderstandingSheet delegates focus restoration to the existing
    // SideSheet/ModalPortal owner. Do not add a competing delayed callback.
  }, [restoreFocus]);

  useEffect(() => {
    if (contentDetail && contentDetail.id !== canonicalSingleSelectionId) closeContentUnderstanding();
  }, [canonicalSingleSelectionId, closeContentUnderstanding, contentDetail]);

  useEffect(() => () => {
    contentRefreshEpoch.current += 1;
    contentOpenEpoch.current += 1;
    pendingContentOpenRef.current = null;
    contentDetailRef.current = null;
  }, []);

  const openContentUnderstanding = useCallback((file: FileLibraryDetail, trigger?: HTMLElement) => {
    contentRefreshEpoch.current += 1;
    contentOpenEpoch.current += 1;
    pendingContentOpenRef.current = null;
    contentRestoreTargetRef.current = null;
    contentTriggerRef.current = trigger ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null);
    contentDetailRef.current = file;
    setContentDetail(file);
  }, []);

  const openContentForFile = useCallback(async (fileId: string, trigger?: HTMLElement, providedDetail?: FileLibraryDetail) => {
    const fileIndex = source.files.findIndex((file) => file.id === fileId);
    const operationEpoch = contentOpenEpoch.current + 1;
    contentOpenEpoch.current = operationEpoch;
    if (!source.ownsSingleFileSelection(fileId)) {
      pendingContentOpenRef.current = { epoch: operationEpoch, fileId };
      source.setExplicitSelection([fileId], fileId, fileIndex);
    }
    if (!source.ownsSingleFileSelection(fileId)) return;
    const inspector = useFileLibraryInspectorStore.getState();
    if (providedDetail?.id === fileId) {
      openContentUnderstanding(providedDetail, trigger);
      return;
    }
    if (!inspector.isLoading && inspector.selectedId === fileId && inspector.detail?.id === fileId) {
      openContentUnderstanding(inspector.detail, trigger);
      return;
    }
    pendingContentOpenRef.current = { epoch: operationEpoch, fileId };
    try {
      const outcome: InspectorDetailLoadResult = await source.loadDetail(fileId);
      if (pendingContentOpenRef.current?.epoch !== operationEpoch || !source.ownsSingleFileSelection(fileId)) return;
      if (outcome.status === "superseded") return;
      if (outcome.status === "failed") {
        onError(t("contentOpenFailed"));
        restoreFocus(trigger ?? null);
        return;
      }
      const current = useFileLibraryInspectorStore.getState();
      if (current.selectedId === fileId && current.detail?.id === fileId) openContentUnderstanding(outcome.detail, trigger);
    } catch {
      if (pendingContentOpenRef.current?.epoch === operationEpoch) {
        onError(t("contentOpenFailed"));
        restoreFocus(trigger ?? null);
      }
    } finally {
      if (pendingContentOpenRef.current?.epoch === operationEpoch) pendingContentOpenRef.current = null;
    }
  }, [onError, openContentUnderstanding, restoreFocus, source.files, source.loadDetail, source.ownsSingleFileSelection, source.setExplicitSelection, t]);

  const refreshContentDetail = useCallback(async (fileId: string): Promise<ContentRefreshResult> => {
    const refreshEpoch = contentRefreshEpoch.current + 1;
    contentRefreshEpoch.current = refreshEpoch;
    const ownsRefresh = () => refreshEpoch === contentRefreshEpoch.current && contentDetailRef.current?.id === fileId;
    const inspectorAtStart = useFileLibraryInspectorStore.getState();
    const expectedInspectorEpoch = inspectorAtStart.requestEpoch;
    const inspectorOwnedFile = inspectorAtStart.selectedId === fileId;
    try {
      const refreshed = await tauriApi.getFileLibraryDetail(fileId);
      if (!ownsRefresh()) return { status: "superseded" };
      const policy = refreshed.scanRootId ? await tauriApi.getContentScopePolicy(refreshed.scanRootId) : null;
      if (!ownsRefresh()) return { status: "superseded" };
      contentDetailRef.current = refreshed;
      setContentDetail(refreshed);
      const currentInspector = useFileLibraryInspectorStore.getState();
      if (inspectorOwnedFile && currentInspector.requestEpoch === expectedInspectorEpoch && currentInspector.selectedId === fileId) {
        source.commitDetailIfCurrent(fileId, refreshed, expectedInspectorEpoch);
      }
      return { status: "applied", detail: refreshed, policy };
    } catch (error) {
      if (!ownsRefresh()) return { status: "superseded" };
      onError(t("contentOpenFailed"));
      return { status: "failed", error };
    }
  }, [onError, source.commitDetailIfCurrent, t]);

  const refreshOpenContentDetail = useCallback(
    () => contentDetailRef.current ? refreshContentDetail(contentDetailRef.current.id) : Promise.resolve({ status: "superseded" as const }),
    [refreshContentDetail]
  );

  const isContentOpenPending = useCallback((fileId: string) => pendingContentOpenRef.current?.fileId === fileId, []);

  return {
    contentDetail,
    contentTriggerRef,
    contentRestoreTargetRef,
    closeContentUnderstanding,
    openContentForFile,
    refreshOpenContentDetail,
    isContentOpenPending
  };
}
