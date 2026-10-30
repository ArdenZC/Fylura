import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useState, type RefObject } from "react";
import { fileManagementApi } from "../../../api/fileManagementApi";
import { useAppStore } from "../../../store/useAppStore";
import type { BrowseEntryRef } from "../../../types/fileWorkspace";
import type { Translator } from "../../../types/ui";
import { useFileManagementStore } from "../management/fileManagementStore";
import type { FileWorkbenchModel } from "./workbenchModel";

interface ExternalDropTarget {
  tabId: string;
  paneId: string;
  destination: string;
  destinationEntry?: BrowseEntryRef;
  label: string;
}

export interface ExternalDropFeedback extends ExternalDropTarget {
  count: number;
}

export function physicalDropPointToClient(
  position: { x: number; y: number },
  devicePixelRatio: number,
) {
  const pixelsPerClientPixel = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0
    ? devicePixelRatio
    : 1;
  return {
    x: position.x / pixelsPerClientPixel,
    y: position.y / pixelsPerClientPixel,
  };
}

export function nativeDropPaths(payload: { type: string; paths?: string[] }) {
  return payload.type === "drop" && Array.isArray(payload.paths) && payload.paths.length > 0
    ? [...payload.paths]
    : null;
}

function resolveExternalDropTarget(
  x: number,
  y: number,
  workbench: HTMLDivElement | null,
  snapshot: ReturnType<FileWorkbenchModel["getSnapshot"]>,
  wide: boolean,
  workspaceActive: boolean,
): ExternalDropTarget | null {
  if (!workbench || !workspaceActive) return null;
  const activeModal = [...document.querySelectorAll<HTMLElement>('[aria-modal="true"]')]
    .some((element) => element.getClientRects().length > 0);
  if (activeModal) return null;
  const element = document.elementFromPoint(x, y);
  const paneElement = element?.closest<HTMLElement>("[data-pane-id]");
  if (!element || !paneElement || !workbench.contains(paneElement)) return null;
  const paneId = paneElement.dataset.paneId;
  const tab = snapshot.tabs.find((item) => item.id === snapshot.activeTabId);
  if (!paneId || !tab) return null;
  const paneIndex = tab.panes.findIndex((item) => item.id === paneId);
  const pane = tab.panes[paneIndex];
  const paneVisible = tab.dual ? wide || tab.activePaneId === paneId : paneIndex === 0;
  if (!pane || !paneVisible || paneElement.hidden || paneElement.getClientRects().length === 0) return null;
  if (pane.controller.getState().mode !== "browse") return null;
  const content = element.closest<HTMLElement>('[data-workspace-slot="content"]');
  if (!content || !paneElement.contains(content)) return null;
  if (element.closest("button,input,select,textarea,a,[role=button],[role=menuitem],[role=dialog]")) return null;
  const row = element.closest<HTMLElement>("[data-file-entry-kind]");
  if (row) {
    if (row.dataset.fileEntryKind !== "directory") return null;
    const browseSessionId = row.dataset.browseSessionId;
    const entryId = row.dataset.browseEntryId ?? row.dataset.browseGridEntryId;
    if (!browseSessionId || !entryId) return null;
    return {
      tabId: tab.id,
      paneId,
      destination: "",
      destinationEntry: { kind: "ephemeral", browseSessionId, entryId },
      label: row.dataset.fileEntryName ?? pane.path.split(/[\\/]/).filter(Boolean).at(-1) ?? "",
    };
  }
  if (!pane.path) return null;
  return {
    tabId: tab.id,
    paneId,
    destination: pane.path,
    label: pane.path.split(/[\\/]/).filter(Boolean).at(-1) ?? pane.path,
  };
}

export function useNativeFileDrop({
  workbenchRef,
  model,
  wide,
  active,
  locked,
  focus,
  t,
}: {
  workbenchRef: RefObject<HTMLDivElement | null>;
  model: FileWorkbenchModel;
  wide: boolean;
  active: boolean;
  locked: boolean;
  focus: (tabId: string, paneId: string) => void;
  t: Translator;
}) {
  const [feedback, setFeedback] = useState<ExternalDropFeedback | null>(null);

  useEffect(() => {
    if (!active || !isTauri()) {
      setFeedback(null);
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    let candidateCount = 0;
    let dropInFlight = false;

    void (async () => {
      try {
        const dispose = await getCurrentWebview().onDragDropEvent(({ payload }) => {
          if (payload.type === "leave") {
            candidateCount = 0;
            setFeedback(null);
            return;
          }
          if (payload.type === "enter") candidateCount = payload.paths.length;
          if (payload.type === "drop" && payload.paths.length === 0) {
            candidateCount = 0;
            setFeedback(null);
            return;
          }
          const paths = nativeDropPaths(payload);
          const count = paths?.length ?? candidateCount;
          void (async () => {
            try {
              const point = physicalDropPointToClient(payload.position, window.devicePixelRatio);
              const canAccept = !locked && !useFileManagementStore.getState().transferOpen;
              const target = resolveExternalDropTarget(
                point.x,
                point.y,
                workbenchRef.current,
                model.getSnapshot(),
                wide,
                active && canAccept,
              );
              if (payload.type !== "drop") {
                setFeedback(target ? { ...target, count } : null);
                return;
              }
              if (dropInFlight) return;
              dropInFlight = true;
              candidateCount = 0;
              setFeedback(null);
              if (!target || !paths) {
                useAppStore.getState().showError(t("wbExternalDropUnsupported"));
                return;
              }
              const tickets = await fileManagementApi.captureTransfer({ externalPaths: paths });
              if (cancelled) {
                await fileManagementApi.captureTransfer({ releaseIds: tickets.map((ticket) => ticket.id) });
                return;
              }
              if (!tickets.length) {
                useAppStore.getState().showError(t("wbExternalDropUnsupported"));
                return;
              }
              focus(target.tabId, target.paneId);
              useFileManagementStore.setState({
                pendingTransfer: {
                  paneId: target.paneId,
                  tickets,
                  action: "copy",
                  destination: target.destination,
                  destinationEntry: target.destinationEntry,
                  external: true,
                },
              });
            } catch (cause) {
              if (!cancelled) {
                setFeedback(null);
                useAppStore.getState().showError(`${t("wbExternalDropUnsupported")} ${String(cause)}`);
              }
            } finally {
              dropInFlight = false;
            }
          })();
        });
        if (cancelled) dispose();
        else unlisten = dispose;
      } catch (cause) {
        if (!cancelled) console.error("Native file drop listener unavailable", cause);
      }
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [active, focus, locked, model, t, wide, workbenchRef]);

  return feedback;
}
