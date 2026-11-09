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
}

export interface InternalDropFeedback {
  paneId: string;
  action: TransferAction;
  label: string;
}

type PointerPaneTarget =
  | { kind: "same-pane" }
  | { kind: "peer-pane"; pane: WorkbenchPane }
  | { kind: "folder"; pane: WorkbenchPane; destinationEntry: BrowseEntryRef; label: string }
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
  const paneElement = element?.closest<HTMLElement>("[data-pane-id]");
  if (!element || !paneElement || !workbench.contains(paneElement)) return { kind: "unsupported" };
  const targetPaneId = paneElement.dataset.paneId ?? null;
  const disposition = workbenchPointerDropDisposition(
    session.sourcePaneId,
    session.peerPaneId,
    targetPaneId,
  );
  if (disposition === "unsupported") return { kind: "unsupported" };
  const snapshot = model.getSnapshot();
  const tab = snapshot.tabs.find((item) => item.id === session.tabId);
  if (!tab || snapshot.activeTabId !== session.tabId) return { kind: "unsupported" };
  const pane = tab.panes.find((item) => item.id === targetPaneId);
  if (
    !pane ||
    paneElement.hidden ||
    paneElement.getClientRects().length === 0 ||
    pane.controller.getState().mode !== "browse" ||
    (tab.dual && !wide && tab.activePaneId !== pane.id) ||
    pane.path !== (pane.id === session.sourcePaneId ? session.sourcePath : session.peerPath)
  ) return { kind: "unsupported" };
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
  return { kind: "peer-pane", pane };
}

export function useWorkbenchDrag({
  workbenchRef,
  model,
  wide,
  dual,
  active,
  locked,
  focus,
  t,
}: {
  workbenchRef: RefObject<HTMLDivElement | null>;
  model: FileWorkbenchModel;
  wide: boolean;
  dual: boolean;
  active: boolean;
  locked: boolean;
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
    const peerPane = tab?.panes.find((item) => item.id === session.peerPaneId);
    const store = useFileManagementStore.getState();
    const sourceSurface = store.surfaces[session.sourcePaneId]?.read();
    const peerSurface = store.surfaces[session.peerPaneId]?.read();
    const paneElements = [...(workbenchRef.current?.querySelectorAll<HTMLElement>("[data-pane-id]") ?? [])];
    const sourceElement = paneElements.find((element) => element.dataset.paneId === session.sourcePaneId);
    const peerElement = paneElements.find((element) => element.dataset.paneId === session.peerPaneId);
    return workbenchPointerSessionIsCurrent(session, {
      currentToken: registry.current.current?.token ?? null,
      activeTabId: snapshot.activeTabId,
      sourceSignature: transferSourceContextKey(sourceSurface),
      peerSignature: transferDestinationContextKey(peerSurface),
      sourcePath: sourcePane?.path ?? null,
      peerPath: peerPane?.path ?? null,
      sourceVisible: !!sourceElement && !sourceElement.hidden && sourceElement.getClientRects().length > 0 && sourceSurface?.visible !== false,
      peerVisible: !!peerElement && !peerElement.hidden && peerElement.getClientRects().length > 0 && peerSurface?.visible !== false,
      workspaceActive: active && !!tab,
      transferOpen: locked || store.transferOpen,
      modalOpen: hasActiveModal(),
    });
  }, [active, locked, model, workbenchRef]);

  const onPointerDown = useCallback((
    event: ReactPointerEvent<HTMLDivElement>,
    tabId: string,
    sourcePaneId: string,
    peerPaneId: string | undefined,
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
    const effectivePeerId = dual && wide && peerPaneId ? peerPaneId : sourcePaneId;
    const peerPane = tab?.panes.find((item) => item.id === effectivePeerId);
    const store = useFileManagementStore.getState();
    const sourceSurface = store.surfaces[sourcePaneId]?.read();
    const peerSurface = store.surfaces[effectivePeerId]?.read();
    if (
      !tab ||
      snapshot.activeTabId !== tabId ||
      !sourcePane?.path ||
      !peerPane?.path ||
      sourcePane.controller.getState().mode !== "browse" ||
      peerPane.controller.getState().mode !== "browse" ||
      !sourceSurface ||
      !peerSurface ||
      sourceSurface.visible === false ||
      peerSurface.visible === false ||
      ![...(workbenchRef.current?.querySelectorAll<HTMLElement>("[data-pane-id]") ?? [])]
        .some((element) => element.dataset.paneId === effectivePeerId && !element.hidden && element.getClientRects().length > 0)
    ) return;
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
      peerPaneId: effectivePeerId,
      sourceSignature: transferSourceContextKey(sourceSurface) ?? "",
      peerSignature: transferDestinationContextKey(peerSurface) ?? "",
      sourcePath: sourcePane.path,
      peerPath: peerPane.path,
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
  }, [active, cancelSession, dual, locked, model, wide, workbenchRef]);

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
    const target = resolvePointerPaneTarget(
      event.clientX,
      event.clientY,
      session,
      workbenchRef.current,
      model,
      wide,
      active,
    );
    const next = target.kind === "peer-pane" || target.kind === "folder"
      ? {
          paneId: target.pane.id,
          action: (event.shiftKey ? "move" : "copy") as TransferAction,
          label: target.kind === "folder" ? target.label : target.pane.path.split(/[\\/]/).filter(Boolean).at(-1) ?? target.pane.path,
        }
      : null;
    setFeedback((current) =>
      current?.paneId === next?.paneId && current?.action === next?.action && current?.label === next?.label
        ? current
        : next,
    );
  }, [active, cancelSession, isCurrent, model, t, wide, workbenchRef]);

  const completeDrop = useCallback(async (
    session: PointerSession,
    target: Extract<PointerPaneTarget, { kind: "peer-pane" | "folder" }>,
    action: TransferAction,
  ) => {
    const targetPane = target.pane;
    const destinationEntry = target.kind === "folder" ? target.destinationEntry : undefined;
    let tickets: TransferTicket[] = [];
    try {
      if (!session.tickets) throw Error(t("wbUnsupportedDrop"));
      tickets = await session.tickets;
      if (registry.current.current?.token !== session.token) return;
      if (!isCurrent(session) || (targetPane.id !== session.peerPaneId && targetPane.id !== session.sourcePaneId) || !tickets.length) {
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
          destinationContextKey: transferDestinationContextKey(useFileManagementStore.getState().surfaces[targetPane.id]?.read(), destinationEntry) ?? undefined,
          destinationEntry,
          tickets,
          action,
          destination: destinationEntry ? "" : session.peerPath,
        },
      });
      finished.handedOff = true;
    } catch (cause) {
      if (registry.current.current?.token === session.token) cancelSession(session.token);
      else releaseSessionTickets(session);
      if (mounted.current) useAppStore.getState().showError(String(cause));
    }
  }, [cancelSession, focus, isCurrent, releaseSessionTickets, t]);

  const onPointerUp = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const session = registry.current.current as PointerSession | null;
    if (!session || session.pointerId !== event.pointerId) return;
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
    if (target.kind !== "peer-pane" && target.kind !== "folder") {
      cancelSession(session.token);
      if (!hasActiveModal()) useAppStore.getState().showError(t("wbUnsupportedDrop"));
      return;
    }
    void completeDrop(session, target, event.shiftKey ? "move" : "copy");
  }, [active, cancelSession, completeDrop, model, t, wide, workbenchRef]);

  const onPointerCancel = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const session = registry.current.current;
    if (session?.pointerId === event.pointerId) cancelSession(session.token);
  }, [cancelSession]);

  const onLostPointerCapture = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    const session = registry.current.current;
    if (session?.pointerId === event.pointerId && !session.ended)
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
    if (!active || locked) cancelSession();
  }, [active, cancelSession, dual, locked, wide]);

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
