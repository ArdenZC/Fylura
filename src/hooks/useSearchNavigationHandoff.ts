import { useEffect } from "react";
import { flushSync } from "react-dom";
import { tauriApi } from "../api/tauriApi";
import { useAppStore } from "../store/useAppStore";
import { useFileLibraryStore } from "../store/useFileLibraryStore";
import { useFileLibrarySelectionStore } from "../store/useFileLibraryV2Store";
import { requestSettingsSection } from "../components/spotlight/commandRegistry";
import { applySearchNavigation, matchesSearchNavigationBinding, shouldApplySearchNavigation, type PendingSearchNavigation } from "../utils/searchNavigation";
import { installMainReadinessListenerBeforeReady } from "../utils/mainWindowReadiness";
import { readableError } from "../utils/viewHelpers";
import type { View } from "../types/ui";

/** Main-only transient bridge: readiness is not navigation acceptance. */
export function useSearchNavigationHandoff(
  isSearchMode: boolean,
  setView: (view: View) => void,
  activateFileLibraryFile: (id: string) => void,
  showError: (error: string) => void,
  openGlobalEntry?: (id: string, accept: () => boolean | Promise<boolean>) => Promise<boolean>
) {
  useEffect(() => {
    if (isSearchMode) return;
    let disposed = false;
    let pending: PendingSearchNavigation | null = null;
    let lastReadyNonce = 0;
    let disposeNavigation: (() => void) | undefined;
    let disposeReadiness: (() => void) | undefined;
    const mainGeneration = Number(new URLSearchParams(window.location.search).get("mainGeneration"));
    const report = (error: unknown) => { if (!disposed) showError(readableError(error)); };

    void (async () => {
      // Both listeners must exist before Rust can consider this Main ready.
      disposeNavigation = await tauriApi.onSearchNavigate((payload) => {
        void (async () => {
          if (disposed || !pending || !matchesSearchNavigationBinding(payload, pending)) return;
          const context = pending;
          const currentLibrary = useFileLibraryStore.getState();
          const currentSelection = useFileLibrarySelectionStore.getState();
          let applied = false;
          try {
            if (shouldApplySearchNavigation(payload, context, {
              view: useAppStore.getState().view,
              selectedFileId: currentLibrary.selectedFileId,
              librarySelection: currentSelection.selection,
              libraryFocusedId: currentSelection.focusedId
            })) {
              // Zustand writes are synchronous; flushSync also commits the mounted
              // Main surface before the positive ACK can permit native Search hide.
              if (typeof payload.globalEntryId === "string") {
                const accept = async () => {
                  const snapshot = payload.sessionId == null ? null : await tauriApi.getSearchWindowState();
                  return (snapshot === null || (snapshot.sessionId === payload.sessionId
                    && snapshot.revision === payload.revision && snapshot.phase.startsWith("visible_")))
                    && !disposed && pending === context && shouldApplySearchNavigation(payload, context, {
                      view: useAppStore.getState().view,
                      selectedFileId: useFileLibraryStore.getState().selectedFileId,
                      librarySelection: useFileLibrarySelectionStore.getState().selection,
                      libraryFocusedId: useFileLibrarySelectionStore.getState().focusedId
                    });
                };
                applied = await openGlobalEntry?.(payload.globalEntryId, accept) ?? false;
                if (applied) flushSync(() => setView("library"));
              } else applied = flushSync(() => applySearchNavigation(payload, setView,
                useFileLibraryStore.getState().setSelectedFileId,
                requestSettingsSection,
                activateFileLibraryFile));
            }
          } catch (error) { report(error); }
          if (pending === context) pending = null;
          void tauriApi.acknowledgeSearchNavigation({
            generation: context.generation, nonce: context.nonce,
            sessionId: context.sessionId, revision: context.revision, applied
          }).catch(report);
        })().catch(report);
      });
      if (disposed) { disposeNavigation(); return; }
      disposeReadiness = await installMainReadinessListenerBeforeReady(
        () => tauriApi.onMainWindowReadyRequest(({ nonce, generation, sessionId = null, revision = null }) => {
          if (disposed || generation !== mainGeneration || !Number.isSafeInteger(generation) || generation <= 0
            || !Number.isSafeInteger(nonce) || nonce <= lastReadyNonce) return;
          lastReadyNonce = nonce;
          pending = {
            generation, nonce, sessionId, revision,
            view: useAppStore.getState().view,
            selectedFileId: useFileLibraryStore.getState().selectedFileId,
            librarySelection: useFileLibrarySelectionStore.getState().selection,
            libraryFocusedId: useFileLibrarySelectionStore.getState().focusedId
          };
          void tauriApi.acknowledgeMainWindowReady(nonce).catch(report);
        }),
        () => tauriApi.markMainWindowReady(true),
        () => disposed
      );
      if (disposed) disposeReadiness?.();
    })().catch((error) => { disposeNavigation?.(); disposeReadiness?.(); report(error); });

    return () => {
      disposed = true;
      pending = null;
      disposeNavigation?.();
      disposeReadiness?.();
      void tauriApi.markMainWindowReady(false).catch(() => undefined);
    };
  }, [activateFileLibraryFile, isSearchMode, openGlobalEntry, setView, showError]);
}
