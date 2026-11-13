import type { BrowseEntryRef } from "../../../types/fileWorkspace";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import {
  fileManagementApi,
  type TransferAction,
  type TransferBrowseDestination,
  type TransferTicket,
} from "../../../api/fileManagementApi";
import { transferDestinationContextKey, transferSourceContextKey, useFileManagementStore } from "../management/fileManagementStore";
import { releaseTransferTickets } from "../management/transferTicketOwnership";
import { useAppStore } from "../../../store/useAppStore";
import type { Translator } from "../../../types/ui";
import type { FileWorkbenchModel, WorkbenchPane } from "./workbenchModel";
import {
  WorkbenchPointerSessionRegistry,
  workbenchPointerDropDisposition,
  workbenchPointerCaptureRequest,
  workbenchPointerSessionIsCurrent,
  workbenchPointerDestinationIsCurrent,
  type WorkbenchPointerSession,
  type WorkbenchPointerCaptureRequest,
} from "./workbenchPointerSession";

interface PointerSession extends WorkbenchPointerSession {
  sourceElement: HTMLElement;
  observer?: MutationObserver;
  tickets?: Promise<TransferTicket[]>;
  captureRequest: WorkbenchPointerCaptureRequest;
  ticketsReleased?: boolean;
  handedOff?: boolean;
  nativeDragOutStarted?: boolean;
}

export interface InternalDropFeedback {
  paneId: string;
  action: TransferAction;
  label: string;
}

type PointerPaneTarget =
  | { kind: "same-pane" }
  | { kind: "target-pane"; pane: WorkbenchPane }
  | { kind: "folder"; pane: WorkbenchPane; destinationEntry: BrowseEntryRef; label: string }
  | { kind: "browse-path"; pane: WorkbenchPane; destinationBrowse: TransferBrowseDestination; destination: string; label: string }
  | { kind: "saved-place"; pane: WorkbenchPane; destinationSavedPlace: { id: string; routingHint: string }; destination: string; label: string }
  | { kind: "unsupported" };

function hasActiveModal() {
  return [...document.querySelectorAll<HTMLElement>('[aria-modal="true"]')].some(
    (element) => element.getClientRects().length > 0,
  );
}

function resolvePointerPaneTarget(
  x: number,
  y: number,
  session: PointerSession,
  workbench: HTMLDivElement | null,
  model: FileWorkbenchModel,
  wide: boolean,
  workspaceActive: boolean,
): PointerPaneTarget {
  if (!workbench || !workspaceActive || hasActiveModal()) return { kind: "unsupported" };
  const element = document.elementFromPoint(x, y);
  if (!element) return { kind: "unsupported" };
  const snapshot = model.getSnapshot();
  const tab = snapshot.tabs.find((item) => item.id === session.tabId);
  if (!tab || snapshot.activeTabId !== session.tabId) return { kind: "unsupported" };
  const savedPlaceTarget = element.closest<HTMLElement>('[data-file-drop-target-kind="saved-place"]');
  const placeId = savedPlaceTarget?.dataset.fileDropPlaceId;
  const placePath = savedPlaceTarget?.dataset.fileDropPath;
  if (savedPlaceTarget && placeId && placePath) {
    const pane = tab.panes.find((item) => item.id === session.sourcePaneId);
    const paneElement = workbench.querySelector<HTMLElement>(`[data-pane-id="${session.sourcePaneId}"]`);
    const surface = useFileManagementStore.getState().surfaces[session.sourcePaneId]?.read();
    if (!pane || !paneElement || paneElement.hidden || paneElement.getClientRects().length === 0 ||
        pane.controller.getState().mode !== "browse" || surface?.visible !== true ||
        !workbenchPointerDestinationIsCurrent(session, pane.id, pane.path, transferDestinationContextKey(surface), true))
      return { kind: "unsupported" };
    return {
      kind: "saved-place",
      pane,
      destinationSavedPlace: { id: placeId, routingHint: placePath },
      destination: placePath,
      label: savedPlaceTarget.dataset.fileDropLabel ?? placePath.split(/[\\/]/).filter(Boolean).at(-1) ?? placePath,
    };
  }
  const paneElement = element?.closest<HTMLElement>("[data-pane-id]");
  if (!element || !paneElement || !workbench.contains(paneElement)) return { kind: "unsupported" };
  const targetPaneId = paneElement.dataset.paneId ?? null;
  const disposition = workbenchPointerDropDisposition(
    session.sourcePaneId,
    Object.keys(session.destinations),
    targetPaneId,
  );
  if (disposition === "unsupported") return { kind: "unsupported" };
  const pane = tab.panes.find((item) => item.id === targetPaneId);
  if (
    !pane ||
    paneElement.hidden ||
    paneElement.getClientRects().length === 0 ||
    pane.controller.getState().mode !== "browse" ||
    (tab.panes.indexOf(pane) >= tab.paneCount || (!wide && tab.activePaneId !== pane.id)) ||
    !workbenchPointerDestinationIsCurrent(session, pane.id, pane.path,
      transferDestinationContextKey(useFileManagementStore.getState().surfaces[pane.id]?.read()),
      useFileManagementStore.getState().surfaces[pane.id]?.read()?.visible === true)
  ) return { kind: "unsupported" };
  const treeTarget = element.closest<HTMLElement>('[data-file-drop-target-kind="browse-path"]');
  const treeSessionId = treeTarget?.dataset.fileDropTargetSessionId;
  const treePathId = treeTarget?.dataset.fileDropTargetPathRef;
  const treePath = treeTarget?.dataset.fileDropTargetPath;
  if (treeTarget && treeSessionId && treePathId && treePath) {
    return {
      kind: "browse-path",
      pane,
      destinationBrowse: { sessionId: treeSessionId, pathRef: { id: treePathId } },
      destination: treePath,
      label: treePath.split(/[\\/]/).filter(Boolean).at(-1) ?? treePath,
    };
  }
  const content = element.closest<HTMLElement>('[data-workspace-slot="content"]');
  if (!content || !paneElement.contains(content)) return { kind: "unsupported" };
  if (element.closest("button,input,select,textarea,a,[role=button],[role=menuitem],[role=dialog]")) return { kind: "unsupported" };
  const row = element.closest<HTMLElement>("[data-file-entry-kind]");
  if (row?.dataset.fileEntryKind === "directory") {
    const browseSessionId = row.dataset.browseSessionId;
    const entryId = row.dataset.browseEntryId ?? row.dataset.browseGridEntryId;
    if (!browseSessionId || !entryId || row === session.sourceElement) return { kind: "unsupported" };
    return { kind: "folder", pane, destinationEntry: { kind: "ephemeral", browseSessionId, entryId }, label: row.dataset.fileEntryName ?? "" };
  }
  if (disposition === "same-pane") return { kind: "same-pane" };
  return { kind: "target-pane", pane };
}

export function useWorkbenchDrag({
  workbenchRef,
  model,
  wide,
  active,
  locked,
  nativeDragOut = false,
  focus,
  t,
}: {
  workbenchRef: RefObject<HTMLDivElement | null>;
  model: FileWorkbenchModel;
  wide: boolean;
  active: boolean;
  locked: boolean;
  nativeDragOut?: boolean;
  focus: (tabId: string, paneId: string) => void;
  t: Translator;
}) {
  const registry = useRef(new WorkbenchPointerSessionRegistry());
  const mounted = useRef(true);
  const [feedback, setFeedback] = useState<InternalDropFeedback | null>(null);
  const suppressedClick = useRef<HTMLElement | null>(null);
  const clickReset = useRef<number | undefined>(undefined);

  const release = useCallback(async (tickets: TransferTicket[]) => {
    await releaseTransferTickets(tickets);
  }, []);

  const releaseSessionTickets = useCallback((session: PointerSession) => {
    if (session.ticketsReleased || session.handedOff || !session.tickets) return;
    session.ticketsReleased = true;
    void session.tickets.then(release).catch(() => undefined);
  }, [release]);

  const cancelSession = useCallback((token?: string) => {
    const current = registry.current.current as PointerSession | null;
    if (!current || (token && current.token !== token)) return;
    const cancelled = registry.current.cancel(current.pointerId, current.token) as PointerSession | null;
    if (!cancelled) return;
    cancelled.observer?.disconnect();
    try {
      if (cancelled.sourceElement.hasPointerCapture(cancelled.pointerId))
        cancelled.sourceElement.releasePointerCapture(cancelled.pointerId);
    } catch {
      // Pointer capture can already be released by the browser during cancel.
    }
    if (mounted.current) setFeedback(null);
    releaseSessionTickets(cancelled);
  }, [releaseSessionTickets]);

  const isCurrent = useCallback((session: PointerSession) => {
    const snapshot = model.getSnapshot();
    const tab = snapshot.tabs.find((item) => item.id === session.tabId);
    const sourcePane = tab?.panes.find((item) => item.id === session.sourcePaneId);
    const store = useFileManagementStore.getState();
    const sourceSurface = store.surfaces[session.sourcePaneId]?.read();
    const paneElements = [...(workbenchRef.current?.querySelectorAll<HTMLElement>("[data-pane-id]") ?? [])];
    const sourceElement = paneElements.find((element) => element.dataset.paneId === session.sourcePaneId);
    return workbenchPointerSessionIsCurrent(session, {
      currentToken: registry.current.current?.token ?? null,
      activeTabId: snapshot.activeTabId,
      sourceSignature: transferSourceContextKey(sourceSurface),
      sourcePath: sourcePane?.path ?? null,
      sourceVisible: !!sourceElement && !sourceElement.hidden && sourceElement.getClientRects().length > 0 && sourceSurface?.visible !== false,
      workspaceActive: active && !!tab,
      transferOpen: locked || store.transferOpen,
      modalOpen: hasActiveModal(),
    });
  }, [active, locked, model, workbenchRef]);

  const startNativeDragOut = useCallback(async (session: PointerSession) => {
    if (!nativeDragOut || !session.tickets || session.nativeDragOutStarted || session.handedOff) return;
    session.nativeDragOutStarted = true;
    let tickets: TransferTicket[] = [];
    try {
      tickets = await session.tickets;
      if (registry.current.current?.token !== session.token || !isCurrent(session) || !tickets.length) {
        if (registry.current.current?.token === session.token) cancelSession(session.token);
        return;
      }
      const finished = registry.current.finish(session.pointerId, session.token) as PointerSession | null;
      if (!finished) return;
      finished.ended = true;
      finished.handedOff = true;
      finished.observer?.disconnect();
      setFeedback(null);
      try {
        if (finished.sourceElement.hasPointerCapture(finished.pointerId))
          finished.sourceElement.releasePointerCapture(finished.pointerId);
      } catch {
        // Native drag has taken over; browser pointer capture may already be gone.
      }
      await fileManagementApi.nativeDragOut({ ticketIds: tickets.map((ticket) => ticket.id) });
    } catch (cause) {
      if (registry.current.current?.token === session.token) cancelSession(session.token);
      useAppStore.getState().showError(String(cause));
    } finally {
      if (tickets.length && !session.ticketsReleased) {
        session.ticketsReleased = true;
        await release(tickets).catch(() => undefined);
      }
    }
  }, [cancelSession, isCurrent, nativeDragOut, release]);

  const onPointerDown = useCallback((
    event: ReactPointerEvent<HTMLDivElement>,
    tabId: string,
    sourcePaneId: string,
    visible: boolean,
  ) => {
    if (
      event.button !== 0 ||
      !event.isPrimary ||
      !visible ||
      !active ||
      locked ||
      hasActiveModal() ||
      registry.current.current
    ) return;
    const target = event.target;
    if (!(target instanceof Element) || target.closest("button,input,select,textarea,a,[role=button],[role=menuitem]")) return;
    const row = target.closest<HTMLElement>("[data-file-entry-kind]");
    if (!row) return;
    const snapshot = model.getSnapshot();
    const tab = snapshot.tabs.find((item) => item.id === tabId);
    const sourcePane = tab?.panes.find((item) => item.id === sourcePaneId);
    const store = useFileManagementStore.getState();
    const sourceSurface = store.surfaces[sourcePaneId]?.read();
    if (!tab || snapshot.activeTabId !== tabId || !sourcePane?.path ||
        sourcePane.controller.getState().mode !== "browse" || !sourceSurface || sourceSurface.visible === false) return;
    const destinations: PointerSession["destinations"] = {};
    for (const pane of tab.panes.slice(0, tab.paneCount)) {
      const surface = store.surfaces[pane.id]?.read();
      const signature = transferDestinationContextKey(surface);
      const element = workbenchRef.current?.querySelector<HTMLElement>(`[data-pane-id="${pane.id}"]`);
      if (signature && surface?.visible && element && !element.hidden && element.getClientRects().length > 0)
        destinations[pane.id] = { path: pane.path, signature };
    }
    if (!destinations[sourcePaneId]) return;
    const captureRequest = workbenchPointerCaptureRequest({
      selected: row.getAttribute("aria-selected") === "true",
      managedFileId: row.dataset.libraryRow ?? row.dataset.libraryGridRow,
      browseSessionId: row.dataset.browseSessionId,
      browseEntryId: row.dataset.browseEntryId ?? row.dataset.browseGridEntryId,
    }, sourceSurface);
    if (!captureRequest) return;
    const session: PointerSession = {
      token: crypto.randomUUID(),
      pointerId: event.pointerId,
      tabId,
      sourcePaneId,
      sourceSignature: transferSourceContextKey(sourceSurface) ?? "",
      sourcePath: sourcePane.path,
      destinations,
      startX: event.clientX,
      startY: event.clientY,
      moved: false,
      ended: false,
      cancelled: false,
      sourceElement: row,
      captureRequest,
    };
    if (!registry.current.begin(session)) return;
    try {
      row.setPointerCapture(event.pointerId);
    } catch {
      cancelSession(session.token);
      return;
    }
    if (typeof MutationObserver !== "undefined" && document.body) {
      session.observer = new MutationObserver(() => {
        if (hasActiveModal()) cancelSession(session.token);
      });
      session.observer.observe(document.body, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["aria-modal", "hidden"],
      });
    }
  }, [active, cancelSession, locked, model, wide, workbenchRef]);

  const onPointerMove = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const session = registry.current.current as PointerSession | null;
    if (!session || session.pointerId !== event.pointerId || session.ended) return;
    if (!registry.current.markMoved(event.pointerId, event.clientX, event.clientY)) return;
    event.preventDefault();
    if (!session.tickets) {
      if (!isCurrent(session)) {
        cancelSession(session.token);
        useAppStore.getState().showError(t("wbSelectionChanged"));
        return;
      }
      if (!useFileManagementStore.getState().surfaces[session.sourcePaneId]?.read()) {
        cancelSession(session.token);
        return;
      }
      const tickets = fileManagementApi.captureTransfer(session.captureRequest);
      session.tickets = tickets;
      void tickets.catch((cause) => {
        if (registry.current.current?.token !== session.token) return;
        cancelSession(session.token);
        useAppStore.getState().showError(String(cause));
      });
    }
    if (nativeDragOut && (event.clientX < -8 || event.clientY < -48 || event.clientX > window.innerWidth + 8 || event.clientY > window.innerHeight + 8)) {
      void startNativeDragOut(session);
      return;
    }
    const target = resolvePointerPaneTarget(
      event.clientX,
      event.clientY,
      session,
      workbenchRef.current,
      model,
      wide,
      active,
    );
    const next = target.kind === "target-pane" || target.kind === "folder" || target.kind === "browse-path" || target.kind === "saved-place"
      ? {
          paneId: target.pane.id,
          action: (event.shiftKey ? "move" : "copy") as TransferAction,
          label: target.kind === "folder" || target.kind === "browse-path" || target.kind === "saved-place" ? target.label : target.pane.path.split(/[\\/]/).filter(Boolean).at(-1) ?? target.pane.path,
        }
      : null;
    setFeedback((current) =>
      current?.paneId === next?.paneId && current?.action === next?.action && current?.label === next?.label
        ? current
        : next,
    );
  }, [active, cancelSession, isCurrent, model, nativeDragOut, startNativeDragOut, t, wide, workbenchRef]);

  const completeDrop = useCallback(async (
    session: PointerSession,
    target: Extract<PointerPaneTarget, { kind: "target-pane" | "folder" | "browse-path" | "saved-place" }>,
    action: TransferAction,
  ) => {
    const targetPane = target.pane;
    const destinationEntry = target.kind === "folder" ? target.destinationEntry : undefined;
    const destinationBrowse = target.kind === "browse-path" ? target.destinationBrowse : undefined;
    const destinationSavedPlace = target.kind === "saved-place" ? target.destinationSavedPlace : undefined;
    const destination = target.kind === "browse-path" ? target.destination : undefined;
    const savedPlaceDestination = target.kind === "saved-place" ? target.destination : undefined;
    let tickets: TransferTicket[] = [];
    try {
      if (!session.tickets) throw Error(t("wbUnsupportedDrop"));
      tickets = await session.tickets;
      if (registry.current.current?.token !== session.token) return;
      if (!isCurrent(session) || !workbenchPointerDestinationIsCurrent(session, targetPane.id,
          model.getSnapshot().tabs.find((tab) => tab.id === session.tabId)?.panes.find((pane) => pane.id === targetPane.id)?.path ?? null,
          transferDestinationContextKey(useFileManagementStore.getState().surfaces[targetPane.id]?.read()),
          !!workbenchRef.current?.querySelector<HTMLElement>(`[data-pane-id="${targetPane.id}"]:not([hidden])`) &&
          useFileManagementStore.getState().surfaces[targetPane.id]?.read()?.visible === true) || !tickets.length) {
        cancelSession(session.token);
        useAppStore.getState().showError(t("wbSelectionChanged"));
        return;
      }
      const finished = registry.current.finish(session.pointerId, session.token) as PointerSession | null;
      if (!finished) {
        await release(tickets);
        return;
      }
      finished.observer?.disconnect();
      setFeedback(null);
      focus(session.tabId, targetPane.id);
      useFileManagementStore.setState({
        pendingTransfer: {
          paneId: targetPane.id,
          sourcePaneId: session.sourcePaneId,
          sourceContextKey: session.sourceSignature,
          destinationContextKey: destinationSavedPlace ? JSON.stringify({ savedPlace: destinationSavedPlace }) : transferDestinationContextKey(useFileManagementStore.getState().surfaces[targetPane.id]?.read(), destinationEntry, destinationBrowse) ?? undefined,
          destinationEntry,
          destinationBrowse,
          destinationSavedPlace,
          tickets,
          action,
          destination: destinationEntry ? "" : savedPlaceDestination ?? destination ?? session.destinations[targetPane.id].path,
        },
      });
      finished.handedOff = true;
    } catch (cause) {
      if (registry.current.current?.token === session.token) cancelSession(session.token);
      else releaseSessionTickets(session);
      if (mounted.current) useAppStore.getState().showError(String(cause));
    }
  }, [cancelSession, focus, isCurrent, model, releaseSessionTickets, t, workbenchRef]);

  const onPointerUp = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const session = registry.current.current as PointerSession | null;
    if (!session || session.pointerId !== event.pointerId) return;
    if (session.nativeDragOutStarted) {
      event.preventDefault();
      return;
    }
    if (!session.moved) {
      cancelSession(session.token);
      return;
    }
    event.preventDefault();
    session.ended = true;
    suppressedClick.current = session.sourceElement;
    if (clickReset.current !== undefined) window.clearTimeout(clickReset.current);
    clickReset.current = window.setTimeout(() => {
      suppressedClick.current = null;
      clickReset.current = undefined;
    }, 0);
    setFeedback(null);
    const target = resolvePointerPaneTarget(
      event.clientX,
      event.clientY,
      session,
      workbenchRef.current,
      model,
      wide,
      active,
    );
    if (target.kind === "same-pane") {
      cancelSession(session.token);
      return;
    }
    if (target.kind !== "target-pane" && target.kind !== "folder" && target.kind !== "browse-path" && target.kind !== "saved-place") {
      cancelSession(session.token);
      if (!hasActiveModal()) useAppStore.getState().showError(t("wbUnsupportedDrop"));
      return;
    }
    void completeDrop(session, target, event.shiftKey ? "move" : "copy");
  }, [active, cancelSession, completeDrop, model, t, wide, workbenchRef]);

  const onPointerCancel = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const session = registry.current.current;
    if (session?.pointerId === event.pointerId && !(session as PointerSession).nativeDragOutStarted) cancelSession(session.token);
  }, [cancelSession]);

  const onLostPointerCapture = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const session = registry.current.current;
    if (session?.pointerId === event.pointerId && !session.ended && !(session as PointerSession).nativeDragOutStarted)
      cancelSession(session.token);
  }, [cancelSession]);

  const onClickCapture = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const row = target.closest<HTMLElement>("[data-file-entry-kind]");
    if (!row || row !== suppressedClick.current) return;
    event.preventDefault();
    event.stopPropagation();
    suppressedClick.current = null;
    if (clickReset.current !== undefined) window.clearTimeout(clickReset.current);
    clickReset.current = undefined;
  }, []);

  useEffect(() => {
    mounted.current = true;
    const unsubscribe = model.subscribe(() => {
      const session = registry.current.current;
      if (session && model.getSnapshot().activeTabId !== session.tabId) cancelSession(session.token);
    });
    const escape = (event: KeyboardEvent) => {
      const session = registry.current.current;
      if (event.key !== "Escape" || !session) return;
      event.preventDefault();
      cancelSession(session.token);
    };
    window.addEventListener("keydown", escape);
    return () => {
      mounted.current = false;
      unsubscribe();
      window.removeEventListener("keydown", escape);
      const cancelled = registry.current.dispose() as PointerSession | null;
      cancelled?.observer?.disconnect();
      try {
        if (cancelled?.sourceElement.hasPointerCapture(cancelled.pointerId))
          cancelled.sourceElement.releasePointerCapture(cancelled.pointerId);
      } catch {
        // Pointer capture can already be released by the browser during cleanup.
      }
      if (cancelled) releaseSessionTickets(cancelled);
      if (clickReset.current !== undefined) window.clearTimeout(clickReset.current);
    };
  }, [cancelSession, model, releaseSessionTickets]);

  useEffect(() => {
    cancelSession();
  }, [active, cancelSession, locked, wide]);

  return {
    feedback,
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel,
    onLostPointerCapture,
    onClickCapture,
  };
}
