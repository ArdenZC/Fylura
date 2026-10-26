import { useRef, type DragEvent } from "react";
import {
  fileManagementApi,
  type TransferTicket,
} from "../../../api/fileManagementApi";
import { useFileManagementStore } from "../management/fileManagementStore";
import { useAppStore } from "../../../store/useAppStore";
import type { Translator } from "../../../types/ui";
import type { WorkbenchPane } from "./workbenchModel";
const DRAG_TYPE = "application/x-zen-canvas-transfer";
/** Only session-local opaque drag tokens are accepted. Paths never become source authority. */
export function useWorkbenchDrag({
  focus,
  locked,
  t,
}: {
  focus: (tabId: string, paneId: string) => void;
  locked: boolean;
  t: Translator;
}) {
  const drag = useRef<{
    token: string;
    sourceId: string;
    signature: string;
    tickets: Promise<TransferTicket[]>;
  } | null>(null);
  async function startDrag(event: DragEvent, pane: WorkbenchPane) {
    const row =
      event.target instanceof Element
        ? event.target.closest('[aria-selected="true"][draggable="true"]')
        : null;
    const surface = useFileManagementStore.getState().surfaces[pane.id]?.read();
    if (!row || !surface || surface.count === 0 || locked) {
      event.preventDefault();
      return;
    }
    const token = crypto.randomUUID();
    const tickets = fileManagementApi.captureTransfer(
      surface.selection
        ? { selection: surface.selection }
        : { refs: surface.refs },
    );
    // Attach a rejection handler immediately, including abandoned drags.
    void tickets.catch(() => undefined);
    drag.current = {
      token,
      sourceId: pane.id,
      signature: surface.signature,
      tickets,
    };
    event.dataTransfer.effectAllowed = "copyMove";
    event.dataTransfer.setData(DRAG_TYPE, token);
  }
  async function drop(event: DragEvent, pane: WorkbenchPane, tabId: string) {
    event.preventDefault();
    const captured = drag.current;
    drag.current = null;
    if (
      locked ||
      !captured ||
      event.dataTransfer.getData(DRAG_TYPE) !== captured.token ||
      captured.sourceId === pane.id ||
      !pane.path
    ) {
      useAppStore.getState().showError(t("wbUnsupportedDrop"));
      return;
    }
    const action = event.shiftKey ? "move" : "copy";
    const destination = pane.path;
    const targetSignature = useFileManagementStore
      .getState()
      .surfaces[pane.id]?.read().signature;
    let tickets: TransferTicket[] = [];
    try {
      tickets = await captured.tickets;
      if (
        useFileManagementStore.getState().transferOpen ||
        useFileManagementStore.getState().surfaces[pane.id]?.read().visible === false ||
        useFileManagementStore.getState().surfaces[captured.sourceId]?.read()
          .signature !== captured.signature ||
        useFileManagementStore.getState().surfaces[pane.id]?.read()
          .signature !== targetSignature
      )
        throw Error(t("wbSelectionChanged"));
      focus(tabId, pane.id);
      useFileManagementStore.setState({
        pendingTransfer: {
          paneId: pane.id,
          sourcePaneId: captured.sourceId,
          sourceSignature: captured.signature,
          tickets,
          action,
          destination,
        },
      });
    } catch (cause) {
      const retained = new Set(
        [
          ...useFileManagementStore.getState().basket,
          ...(useFileManagementStore.getState().clipboard?.tickets ?? []),
        ].map((ticket) => ticket.id),
      );
      if (tickets.length)
        await fileManagementApi
          .captureTransfer({
            releaseIds: tickets
              .filter((ticket) => !retained.has(ticket.id))
              .map((ticket) => ticket.id),
          })
          .catch(() => undefined);
      useAppStore.getState().showError(String(cause));
    }
  }
  const endDrag = () => {
    const abandoned = drag.current;
    drag.current = null;
    if (abandoned)
      void abandoned.tickets
        .then((tickets) => {
          const retained = new Set(
            [
              ...useFileManagementStore.getState().basket,
              ...(useFileManagementStore.getState().clipboard?.tickets ?? []),
            ].map((ticket) => ticket.id),
          );
          return fileManagementApi.captureTransfer({
            releaseIds: tickets
              .filter((ticket) => !retained.has(ticket.id))
              .map((ticket) => ticket.id),
          });
        })
        .catch(() => undefined);
  };

  return { startDrag, drop, endDrag };
}
