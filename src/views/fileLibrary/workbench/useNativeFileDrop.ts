import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect, useState, type RefObject } from "react";
import { fileManagementApi } from "../../../api/fileManagementApi";
import { releaseTransferTickets } from "../management/transferTicketOwnership";
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

export class NativeDropRequestGate {
  private current: { token: number; cancelled: boolean } | null = null;
  private nextToken = 0;

  begin() {
    if (this.current) return null;
    const request = { token: ++this.nextToken, cancelled: false };
    this.current = request;
    return request.token;
  }

  cancel(token: number) {
    if (this.current?.token === token) this.current.cancelled = true;
  }

  isCurrent(token: number) {
    return this.current?.token === token && !this.current.cancelled;
  }

  finish(token: number) {
    if (this.current?.token === token) this.current = null;
  }
}

const nativeDropRequestGate = new NativeDropRequestGate();

export function sameExternalDropTarget(a: ExternalDropTarget | null, b: ExternalDropTarget | null) {
  return Boolean(a && b && a.tabId === b.tabId && a.paneId === b.paneId && a.destination === b.destination &&
    a.destinationEntry?.kind === b.destinationEntry?.kind &&
    a.destinationEntry?.browseSessionId === b.destinationEntry?.browseSessionId &&
    a.destinationEntry?.entryId === b.destinationEntry?.entryId);
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
    let activeRequestToken: number | null = null;

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
            let requestToken: number | null = null;
            let capturedTickets: Awaited<ReturnType<typeof fileManagementApi.captureTransfer>> = [];
            try {
              const point = physicalDropPointToClient(payload.position, window.devicePixelRatio);
              const initialStore = useFileManagementStore.getState();
              const dropAction = payload.type === "drop" ? initialStore.externalDropAction : "copy";
              if (payload.type === "drop") useFileManagementStore.setState({ externalDropAction: "copy" });
              const canAccept = !locked && !initialStore.transferActive && !initialStore.transferOpen && !initialStore.pendingTransfer;
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
              if (initialStore.transferActive || initialStore.pendingTransfer) {
                useAppStore.getState().showError(t("fmTransferBusy"));
                return;
              }
              if (dropInFlight) return;
              requestToken = nativeDropRequestGate.begin();
              if (requestToken === null) return;
              activeRequestToken = requestToken;
              dropInFlight = true;
              candidateCount = 0;
              setFeedback(null);
              if (!target || !paths) {
                useAppStore.getState().showError(t("wbExternalDropUnsupported"));
                return;
              }
              const destinationSignature = initialStore.surfaces[target.paneId]?.read().signature;
              if (!destinationSignature) {
                useAppStore.getState().showError(t("wbSelectionChanged"));
                return;
              }
              const tickets = await fileManagementApi.captureTransfer({ externalPaths: paths });
              capturedTickets = tickets;
              const currentStore = useFileManagementStore.getState();
              const currentTarget = resolveExternalDropTarget(
                point.x,
                point.y,
                workbenchRef.current,
                model.getSnapshot(),
                wide,
                active && !locked && !currentStore.transferActive && !currentStore.transferOpen && !currentStore.pendingTransfer,
              );
              if (!nativeDropRequestGate.isCurrent(requestToken) || cancelled || currentStore.transferActive || currentStore.transferOpen || currentStore.pendingTransfer || currentStore.surfaces[target.paneId]?.read().signature !== destinationSignature || !sameExternalDropTarget(target, currentTarget)) {
                await releaseTransferTickets(tickets);
                capturedTickets = [];
                if (!cancelled && nativeDropRequestGate.isCurrent(requestToken)) {
                  useAppStore.getState().showError(t("wbSelectionChanged"));
                }
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
                  destinationSignature,
                  tickets,
                  action: dropAction,
                  destination: target.destination,
                  destinationEntry: target.destinationEntry,
                  external: true,
                },
              });
              capturedTickets = [];
            } catch (cause) {
              if (capturedTickets.length) {
                await releaseTransferTickets(capturedTickets).catch(() => undefined);
                capturedTickets = [];
              }
              if (!cancelled) {
                setFeedback(null);
                useAppStore.getState().showError(`${t("wbExternalDropUnsupported")} ${String(cause)}`);
              }
            } finally {
              dropInFlight = false;
              if (requestToken !== null) {
                nativeDropRequestGate.finish(requestToken);
                if (activeRequestToken === requestToken) activeRequestToken = null;
              }
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
      if (activeRequestToken !== null) nativeDropRequestGate.cancel(activeRequestToken);
      unlisten?.();
    };
  }, [active, focus, locked, model, t, wide, workbenchRef]);

  return feedback;
}
