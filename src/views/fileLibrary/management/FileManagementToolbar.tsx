import { usePane } from "../workbench/PaneContext";
import { useCallback, useEffect, useRef, useState } from "react";
import { Copy, Scissors, ClipboardPaste, FolderInput, PackagePlus, Star, X, FolderOpen, MoreHorizontal, Package, File, Check, AlertTriangle, ChevronRight, FolderPlus, Pencil, Trash2 } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { fileManagementApi as api, type TransferAction, type TransferPreview, type TransferTicket, type SavedPlace } from "../../../api/fileManagementApi";
import { tauriApi } from "../../../api/tauriApi";
import type { OperationProgressPayload } from "../../../api/types";
import type { BrowseEntryRef } from "../../../types/fileWorkspace";
import { useChromeContext, useI18nContext, useNavigationContext } from "../../../contexts/AppContexts";
import { useOperationQueueStore } from "../../../store/useOperationQueueStore";
import { buttonGhost, buttonPrimary, buttonSecondary, cn } from "../../../utils/tw";
import { readableError } from "../../../utils/viewHelpers";
import { ModalPortal } from "../../../components/modal/ModalPortal";
import { useFileLibraryExperience } from "../FileLibraryExperienceProvider";
import { isFileLibraryShortcutExcludedTarget } from "../fileLibraryInteraction";
import { usePreviewExperience } from "../preview/PreviewExperienceProvider";
import { useFileManagementStore } from "./fileManagementStore";
import { releaseTransferTickets } from "./transferTicketOwnership";
import { orchestrateTransfer } from "./transferOrchestrator";
import "./fileManagement.css";

const PAGE_SIZE = 40;
type TransferOrigin = "selection" | "clipboard" | "basket" | "external";
function parentPath(path: string) {
  const trimmed = path.replace(/[\\/]+$/, "");
  if (/^[a-z]:$/i.test(trimmed)) return `${trimmed}\\`;
  const separator = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (separator < 0) return "";
  if (separator === 0) return trimmed.slice(0, 1);
  if (separator === 2 && /^[a-z]:$/i.test(trimmed.slice(0, separator))) return trimmed.slice(0, 3);
  return trimmed.slice(0, separator);
}
interface TransferIntent {
  id: string;
  tickets: TransferTicket[];
  action: TransferAction;
  newName?: string;
  origin: TransferOrigin;
  destinationDirectory: string;
  destinationEntry?: BrowseEntryRef;
  destinationPaneId?: string;
  destinationSignature?: string;
  sourcePaneId?: string;
  sourceSignature?: string;
}
interface NameIntent {
  action: "rename" | "create_directory";
  tickets: TransferTicket[];
  destinationDirectory: string;
  sourcePaneId?: string;
  sourceSignature?: string;
  destinationPaneId?: string;
  destinationSignature?: string;
}
function PageControls({ count, page, change, previous, next }: { count: number; page: number; change: (page: number) => void; previous: string; next: string }) {
  if (count <= PAGE_SIZE) return null;
  return <div className="fm-toolbar"><button className={buttonGhost} disabled={page <= 0} onClick={() => change(page - 1)}>{previous}</button><span className="text-xs">{page * PAGE_SIZE + 1}–{Math.min((page + 1) * PAGE_SIZE, count)} / {count}</span><button className={buttonGhost} disabled={(page + 1) * PAGE_SIZE >= count} onClick={() => change(page + 1)}>{next}</button></div>;
}
export function FileManagementToolbar() {
  const { t } = useI18nContext(); const { platform } = useChromeContext(); const { setView } = useNavigationContext();
  const { controller } = useFileLibraryExperience(); const { controller: previewController } = usePreviewExperience();
  const pane = usePane();
  const state = useFileManagementStore();
  const currentSurface = () => pane ? useFileManagementStore.getState().surfaces[pane.id]?.read() : useFileManagementStore.getState().surface?.();
  const surface = pane ? state.surfaces[pane.id]?.read() : state.surface?.();
  const focused = !pane || pane.focused && pane.visible;
  const panelId = pane ? `fm-auxiliary-panel-${pane.id}` : "fm-auxiliary-panel";
  useEffect(() => { if (!focused) setPanel(null); }, [focused]);
  const [previewPage, setPreviewPage] = useState(0); const [basketPage, setBasketPage] = useState(0); const [resultPage, setResultPage] = useState(0);
  const [busy, setBusy] = useState(false); const [error, setError] = useState(""); const [notice, setNotice] = useState("");
  useEffect(() => {
    if (!notice) return;
    const timeout = window.setTimeout(() => setNotice(""), 4000);
    return () => window.clearTimeout(timeout);
  }, [notice]);
  const [places, setPlaces] = useState<SavedPlace[]>([]); const [currentPath, setCurrentPath] = useState("");
  const [locationInput, setLocationInput] = useState("");
  const [panel, setPanel] = useState<"more" | "places" | "results" | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const panelTriggerRef = useRef<HTMLElement | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const panelOpen = focused && (state.basketOpen || panel !== null);
  function closePanel(restore = true) {
    useFileManagementStore.setState({ basketOpen: false }); setPanel(null);
    if (restore) panelTriggerRef.current?.focus();
  }
  function openPanel(next: "more" | "places" | "results" | "basket", trigger?: HTMLElement) {
    if (trigger) panelTriggerRef.current = trigger;
    useFileManagementStore.setState({ basketOpen: next === "basket" });
    setPanel(next === "basket" ? null : next);
  }
  useEffect(() => {
    if (!panelOpen) return;
    const element = panelRef.current;
    element?.querySelector<HTMLElement>("button:not(:disabled), input")?.focus();
    const workspace = element?.closest(".file-library-workspace");
    const fitPanel = () => {
      if (element && workspace) element.style.maxHeight = `${Math.max(80, Math.min(480, workspace.getBoundingClientRect().bottom - element.getBoundingClientRect().top - 8))}px`;
    };
    fitPanel();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(fitPanel);
    if (workspace) observer?.observe(workspace);
    if (element?.parentElement) observer?.observe(element.parentElement);
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) { event.preventDefault(); closePanel(); }
    };
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !panelRef.current?.contains(event.target) && !panelTriggerRef.current?.contains(event.target)) closePanel(false);
    };
    const focusOutside = (event: FocusEvent) => {
      if (event.target instanceof Node && !panelRef.current?.contains(event.target) && !panelTriggerRef.current?.contains(event.target)) closePanel(false);
    };
    document.addEventListener("keydown", keydown);
    document.addEventListener("pointerdown", outside);
    document.addEventListener("focusin", focusOutside);
    return () => { observer?.disconnect(); document.removeEventListener("keydown", keydown); document.removeEventListener("pointerdown", outside); document.removeEventListener("focusin", focusOutside); };
  }, [panelOpen, panel, state.basketOpen]);
  const [transfer, setTransfer] = useState<TransferIntent | null>(null);
  const transferRef = useRef(transfer); transferRef.current = transfer;
  const [nameIntent, setNameIntent] = useState<NameIntent | null>(null);
  const [newName, setNewName] = useState("");
  const nameInputRef = useRef<HTMLInputElement | null>(null);
  const [plan, setPlan] = useState<TransferPreview | null>(null);
  const [conflictTicketIds, setConflictTicketIds] = useState<string[]>([]);
  const [skip, setSkip] = useState<string[]>([]); const [keepBoth, setKeepBoth] = useState<string[]>([]);
  const [progress, setProgress] = useState<OperationProgressPayload | null>(null);
  const [transferPhase, setTransferPhase] = useState<"preview" | "execute">("preview");
  const restoreRef = useRef<HTMLElement | null>(null);
  const mounted = useRef(true); const busyRef = useRef(false);
  const cancelRequested = useRef(false); const executionSent = useRef(false);
  const progressDispose = useRef<(() => void) | null>(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const active = transferRef.current;
      const activeIds = new Set(active?.tickets.map((ticket) => ticket.id) ?? []);
      const store = useFileManagementStore.getState();
      if (!store.transferRunning && store.transferTicketIds.some((id) => activeIds.has(id))) {
        useFileManagementStore.setState({ transferOpen: false, transferActive: false, transferRunning: false, activeTransferId: null, transferTicketIds: [] });
      }
      progressDispose.current?.(); progressDispose.current = null;
      if (!executionSent.current && !store.transferRunning && active && (active.origin === "selection" || active.origin === "external")) {
        void releaseUnused(active.tickets.map((ticket) => ticket.id)).catch(() => {});
      }
    };
  }, []);
  const describeError = useCallback((value: unknown) => {
    const detail = readableError(value);
    const key = detail.includes("transfer_windows_directory_move_unsupported") ? "fmDirectoryMoveUnsupported" : detail.includes("cross_volume_move") ? "fmCrossVolumeMove" : detail.includes("windows_directory") || detail.includes("empty_directory") ? "fmWindowsDirectories" : detail.includes("target_exists") ? "fmConflict" : detail.includes("limit_1000") ? "fmLimit" : detail.includes("source_changed") || detail.includes("source_tree_changed") || detail.includes("expired") || detail.includes("preview_stale") ? "fmStale" : "fmFailed";
    return `${t(key)} ${detail}`;
  }, [t]);
  const run = useCallback(async (work: () => Promise<void>) => {
    if (busyRef.current) return; busyRef.current = true; setBusy(true); setError("");
    try { await work(); } catch (value) { if (mounted.current) setError(describeError(value)); }
    finally { busyRef.current = false; if (mounted.current) setBusy(false); }
  }, [describeError]);

  useEffect(() => {
    let cancelled = false; const browse = surface?.browse;
    setCurrentPath("");
    void api.fileLibraryPlaces(browse ?? {}).then((loaded) => {
      if (cancelled) return; setPlaces(loaded); setCurrentPath(loaded.find((place) => place.current)?.routingHint ?? "");
    }).catch((value) => { if (!cancelled) setError(describeError(value)); });
    return () => { cancelled = true; };
    // Selection changes do not read all saved locations again.
  }, [surface?.browse?.sessionId, surface?.browse?.pathRef.id, describeError]);

  async function capturedSelection() {
    const current = currentSurface();
    if (!current || (current.count === 0 && current.selection?.kind !== "all_matching")) return [];
    const captured = await api.captureTransfer(current.selection ? { selection: current.selection } : { refs: current.refs ?? [] });
    if (currentSurface()?.signature !== current.signature) { await releaseUnused(captured.map((ticket) => ticket.id)); throw Error("transfer_selection_changed"); }
    return captured;
  }
  async function releaseUnused(ids: string[]) {
    await releaseTransferTickets(ids);
  }
  function collect(action: "copy" | "cut" | "basket") {
    void run(async () => {
      if (useFileManagementStore.getState().transferActive) { setError(t("fmTransferBusy")); return; }
      if (action === "cut" && platform === "win32" && currentSurface()?.hasDirectorySelection) { setError(t("fmDirectoryMoveUnsupported")); return; }
      const tickets = await capturedSelection(); if (!tickets.length) return;
      if (useFileManagementStore.getState().transferActive) {
        await releaseUnused(tickets.map((ticket) => ticket.id)); setError(t("fmTransferBusy")); return;
      }
      const old = useFileManagementStore.getState();
      if (action === "basket") {
        const merged = new Map(old.basket.map((ticket) => [ticket.id, ticket])); tickets.forEach((ticket) => merged.set(ticket.id, ticket));
        useFileManagementStore.setState({ basket: [...merged.values()], basketOpen: false });
        closePanel();
      } else {
        useFileManagementStore.setState({ clipboard: { action: action === "cut" ? "move" : "copy", tickets } });
        await releaseUnused(old.clipboard?.tickets.map((ticket) => ticket.id) ?? []);
      }
      setNotice(t(action === "basket" ? "fmAdded" : "fmClipboardReady"));
    });
  }
  function clearTransferState(intent?: TransferIntent) {
    const current = useFileManagementStore.getState();
    if (intent && current.activeTransferId !== intent.id) return;
    setTransfer(null); setPlan(null); setConflictTicketIds([]);
    useFileManagementStore.setState({ transferOpen: false, transferActive: false, transferRunning: false, activeTransferId: null, transferTicketIds: [] });
  }
  function releaseTransient(intent: TransferIntent) {
    if (intent.origin === "selection" || intent.origin === "external") {
      void releaseUnused(intent.tickets.map((ticket) => ticket.id)).catch((value) => {
        if (mounted.current) setError(describeError(value));
      });
    }
  }
  function transferContextIsCurrent(intent: TransferIntent) {
    const store = useFileManagementStore.getState();
    const ids = new Set(store.transferTicketIds);
    if (!mounted.current || cancelRequested.current || !store.transferActive || store.activeTransferId !== intent.id || !intent.tickets.every((ticket) => ids.has(ticket.id))) return false;
    if (intent.sourcePaneId) {
      const source = store.surfaces[intent.sourcePaneId]?.read();
      if (!source || source.signature !== intent.sourceSignature) return false;
    }
    if (intent.destinationPaneId) {
      const target = store.surfaces[intent.destinationPaneId]?.read();
      if (!target || target.signature !== intent.destinationSignature) return false;
      if (!intent.destinationEntry && intent.destinationDirectory && target.routingHint && target.routingHint !== intent.destinationDirectory) return false;
    }
    return true;
  }
  async function startTransfer(intent: TransferIntent, nextSkip: string[], nextKeepBoth: string[]) {
    const store = useFileManagementStore.getState();
    if (!store.transferActive || store.activeTransferId !== intent.id || !intent.tickets.every((ticket) => store.transferTicketIds.includes(ticket.id))) return;
    cancelRequested.current = false; executionSent.current = false;
    setCancelling(false); setError(""); setNotice(""); setProgress(null); setTransferPhase("preview");
    useFileManagementStore.setState({ transferRunning: true, transferOpen: false });
    let preserveConflict = false;
    const executionPreview = { current: null as TransferPreview | null };
    try {
      const outcome = await orchestrateTransfer({
        tickets: intent.tickets,
        action: intent.action,
        destinationDirectory: intent.destinationDirectory,
        destinationEntry: intent.destinationEntry,
        newName: intent.newName,
        skip: nextSkip,
        keepBoth: nextKeepBoth,
      }, {
        preview: (request) => api.previewTransfer(request),
        isCurrent: () => transferContextIsCurrent(intent),
        beforeExecute: async () => {
          if (pane) await pane.beforeMutation();
          else await previewController.closeForFileOperation();
          if (mounted.current) setTransferPhase("execute");
        },
        execute: async (acceptedPlan) => {
          executionPreview.current = acceptedPlan;
          setProgress({ kind: "execute", batchId: "", processed: 0,
            total: acceptedPlan.rows.filter((row) => !row.skipped).length,
            currentPath: acceptedPlan.rows.find((row) => !row.skipped && row.preview)?.preview?.source_path ?? "" });
          progressDispose.current = await tauriApi.onOperationProgress((update) => {
            if (mounted.current && update.kind === "execute") setProgress(update);
          });
          if (cancelRequested.current) throw Error("transfer_cancelled");
          executionSent.current = true;
          return api.executeTransfer({ previewId: acceptedPlan.id, confirmed: true });
        },
      });
      if (outcome.kind === "conflicts") {
        preserveConflict = true;
        setPlan(outcome.preview); setConflictTicketIds(outcome.ticketIds);
        setPreviewPage(Math.floor(Math.max(0, intent.tickets.findIndex((ticket) => ticket.id === outcome.ticketIds[0])) / PAGE_SIZE));
        const statuses = { ...useFileManagementStore.getState().statuses };
        outcome.ticketIds.forEach((id) => { statuses[id] = t("fmConflict"); });
        useFileManagementStore.setState({ statuses, transferOpen: true, transferRunning: false });
        return;
      }
      if (outcome.kind === "failed") {
        const statuses = { ...useFileManagementStore.getState().statuses };
        outcome.preview.rows.forEach((row) => { if (row.error && !row.skipped) statuses[row.ticketId] = describeError(row.error); });
        useFileManagementStore.setState({ statuses });
        setError([...new Set(outcome.errors)].map((item) => describeError(item)).join("\n"));
        clearTransferState(intent); releaseTransient(intent); return;
      }
      if (outcome.kind === "stale") {
        if (cancelRequested.current) setNotice(t("fmCancelled"));
        else setError(describeError("transfer_preview_stale"));
        clearTransferState(intent); releaseTransient(intent); return;
      }
      if (outcome.kind === "all-skipped") {
        const statuses = { ...useFileManagementStore.getState().statuses };
        outcome.preview.rows.forEach((row) => { if (row.skipped) statuses[row.ticketId] = t("fmSkipped"); });
        useFileManagementStore.setState({ statuses });
        setNotice(t("fmAllSkipped")); clearTransferState(intent); releaseTransient(intent); return;
      }

      const acceptedPlan = outcome.preview; const result = outcome.result;
      const rows = acceptedPlan.rows.filter((row) => !row.skipped && row.preview);
      const succeeded = new Set<string>(); const statuses = { ...useFileManagementStore.getState().statuses };
      acceptedPlan.rows.filter((row) => row.skipped).forEach((row) => { statuses[row.ticketId] = t("fmSkipped"); });
      rows.forEach((row, index) => {
        const log = result.logs[index];
        statuses[row.ticketId] = log?.status === "success" ? t("fmSucceeded") : log?.status === "skipped" ? t("fmSkipped") : log?.error_message || t("fmFailed");
        if (log?.status === "success") succeeded.add(row.ticketId);
      });
      const current = useFileManagementStore.getState();
      useFileManagementStore.setState({ results: result.logs, statuses, basketOpen: false,
        ...(intent.action === "move" || intent.action === "trash" ? {
          basket: current.basket.filter((ticket) => !succeeded.has(ticket.id)),
          clipboard: current.clipboard ? { ...current.clipboard, tickets: current.clipboard.tickets.filter((ticket) => !succeeded.has(ticket.id)) } : null,
        } : {}) });
      if (result.logs.some((log) => log.status !== "success")) setPanel("results");
      setNotice(cancelRequested.current ? t("fmCancelled") : t("fmFinished"));
      previewController.close("escape");
      const affectedSurface = intent.sourcePaneId ? useFileManagementStore.getState().surfaces[intent.sourcePaneId]?.read() : currentSurface();
      if (intent.tickets.length > 0 && (!intent.sourceSignature || affectedSurface?.signature === intent.sourceSignature)) affectedSurface?.clear();
      clearTransferState(intent);
      try {
        const directories = [acceptedPlan.destinationDirectory, ...intent.tickets.map((ticket) => parentPath(ticket.path))].filter(Boolean);
        if (pane) await pane.afterMutation(directories); else await affectedSurface?.refresh();
        await useOperationQueueStore.getState().refreshOperationLogs();
      } catch (value) { setError(`${t("fmRefreshFailed")} ${readableError(value)}`); }
      if (intent.origin === "selection" || intent.origin === "external") releaseTransient(intent);
      else if (intent.action === "move" || intent.action === "trash") {
        try { await releaseUnused([...succeeded]); } catch (value) { setError(`${t("fmRefreshFailed")} ${readableError(value)}`); }
      }
    } catch (value) {
      const detail = readableError(value);
      setError(executionSent.current
        ? `${t("fmExecutionUncertain")} ${detail}`
        : detail.includes("transfer_cancelled") ? t("fmCancelled") : describeError(value));
      clearTransferState(intent); releaseTransient(intent);
      if (executionSent.current) {
        try { await useOperationQueueStore.getState().refreshOperationLogs(); } catch { /* Keep the operation error visible. */ }
        try {
          const directories = [executionPreview.current?.destinationDirectory ?? intent.destinationDirectory,
            ...intent.tickets.map((ticket) => parentPath(ticket.path))].filter(Boolean);
          const affectedSurface = intent.sourcePaneId
            ? useFileManagementStore.getState().surfaces[intent.sourcePaneId]?.read()
            : currentSurface();
          if (pane) await pane.afterMutation(directories);
          else await affectedSurface?.refresh();
        } catch { /* The uncertain execution result remains visible above. */ }
      }
    } finally {
      progressDispose.current?.(); progressDispose.current = null;
      executionSent.current = false;
      if (mounted.current) { setProgress(null); setCancelling(false); }
      const current = useFileManagementStore.getState();
      if (!preserveConflict && current.transferActive && current.activeTransferId === intent.id) {
        clearTransferState(intent);
      }
    }
  }
  function begin(
    tickets: TransferTicket[],
    action: TransferAction,
    origin: TransferOrigin,
    options: Partial<Pick<TransferIntent, "destinationDirectory" | "destinationEntry" | "destinationPaneId" | "destinationSignature" | "sourcePaneId" | "sourceSignature" | "newName">> = {},
  ) {
    const store = useFileManagementStore.getState();
    if (!tickets.length && action !== "create_directory") return;
    if ((action === "rename" && (tickets.length !== 1 || !options.newName?.trim()))
      || (action === "create_directory" && !options.newName?.trim())) {
      setError(describeError(action === "rename" ? "transfer_rename_selection_invalid" : "transfer_create_directory_request_invalid"));
      if (origin === "selection" || origin === "external") void releaseUnused(tickets.map((ticket) => ticket.id));
      return;
    }
    if (store.transferActive || store.transferOpen) {
      setError(t("fmTransferBusy"));
      if (origin === "selection" || origin === "external") void releaseUnused(tickets.map((ticket) => ticket.id));
      return;
    }
    const destinationPaneId = options.destinationPaneId ?? (options.destinationDirectory === undefined ? pane?.id : undefined);
    const destinationSurface = destinationPaneId ? store.surfaces[destinationPaneId]?.read() : currentSurface();
    const destinationEntry = options.destinationEntry;
    const destinationDirectory = destinationEntry ? "" : options.destinationDirectory ?? destinationSurface?.routingHint ?? currentPath;
    if (!destinationEntry && !destinationDirectory.trim()) {
      setError(t("fmDestinationRequired"));
      if (origin === "selection" || origin === "external") void releaseUnused(tickets.map((ticket) => ticket.id));
      return;
    }
    const intent: TransferIntent = {
      id: crypto.randomUUID(), tickets, action, origin, destinationDirectory, destinationEntry,
      newName: options.newName?.trim(),
      destinationPaneId,
      destinationSignature: options.destinationSignature ?? destinationSurface?.signature,
      sourcePaneId: options.sourcePaneId,
      sourceSignature: options.sourceSignature,
    };
    closePanel(false); setCancelling(false); setError(""); setNotice("");
    restoreRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    useFileManagementStore.setState({ results: [], transferOpen: false, transferActive: true, transferRunning: true, activeTransferId: intent.id, transferTicketIds: tickets.map((ticket) => ticket.id) });
    setTransfer(intent); setPlan(null); setConflictTicketIds([]); setSkip([]); setKeepBoth([]); setPreviewPage(0); setResultPage(0);
    void startTransfer(intent, [], []);
  }
  function closeTransfer() {
    const active = transferRef.current;
    if (useFileManagementStore.getState().transferRunning) return;
    cancelRequested.current = true; clearTransferState(active ?? undefined); setSkip([]); setKeepBoth([]);
    if (active) releaseTransient(active);
  }
  function cancelRunningTransfer() {
    if (!transfer) return;
    if (!useFileManagementStore.getState().transferRunning) { closeTransfer(); return; }
    setCancelling(true); cancelRequested.current = true;
    if (executionSent.current) void tauriApi.cancelOperations().catch((value) => {
      setError(describeError(value)); setCancelling(false);
    });
  }
  async function moveSelection() {
    if (useFileManagementStore.getState().transferActive) { setError(t("fmTransferBusy")); return; }
    if (platform === "win32" && currentSurface()?.hasDirectorySelection) { setError(t("fmDirectoryMoveUnsupported")); return; }
    await run(async () => {
      const source = currentSurface();
      const sourceSignature = source?.signature;
      const tickets = await capturedSelection(); if (!tickets.length) return;
      const result = await open({ directory: true, multiple: false, title: t("fmDestination") });
      const destinationDirectory = Array.isArray(result) ? result[0] : result;
      if (!destinationDirectory) { await releaseUnused(tickets.map((ticket) => ticket.id)); return; }
      if (!sourceSignature || currentSurface()?.signature !== sourceSignature) {
        await releaseUnused(tickets.map((ticket) => ticket.id)); throw Error("transfer_selection_changed");
      }
      begin(tickets, "move", "selection", { destinationDirectory, sourcePaneId: pane?.id, sourceSignature });
    });
  }
  function trashSelection() {
    if (useFileManagementStore.getState().transferActive) { setError(t("fmTransferBusy")); return; }
    void run(async () => {
      const sourceSignature = currentSurface()?.signature;
      const tickets = await capturedSelection();
      if (!tickets.length || !sourceSignature || currentSurface()?.signature !== sourceSignature) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        if (tickets.length) throw Error("transfer_selection_changed");
        return;
      }
      begin(tickets, "trash", "selection", {
        destinationDirectory: parentPath(tickets[0]!.path),
        sourcePaneId: pane?.id,
        sourceSignature,
      });
    });
  }
  function renameSelection() {
    if (useFileManagementStore.getState().transferActive) { setError(t("fmTransferBusy")); return; }
    void run(async () => {
      const sourceSignature = currentSurface()?.signature;
      const tickets = await capturedSelection();
      if (tickets.length !== 1 || !sourceSignature || currentSurface()?.signature !== sourceSignature) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        throw Error("transfer_rename_selection_invalid");
      }
      closePanel(false);
      restoreRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      setNewName(tickets[0]!.name);
      setNameIntent({
        action: "rename",
        tickets,
        destinationDirectory: parentPath(tickets[0]!.path),
        sourcePaneId: pane?.id,
        sourceSignature,
      });
    });
  }
  function createDirectory() {
    const current = currentSurface();
    if (!current?.browse || !current.routingHint) return;
    if (useFileManagementStore.getState().transferActive) { setError(t("fmTransferBusy")); return; }
    closePanel(false);
    restoreRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setNewName(t("fmNewFolderDefault"));
    setNameIntent({
      action: "create_directory",
      tickets: [],
      destinationDirectory: current.routingHint,
      destinationPaneId: pane?.id,
      destinationSignature: current.signature,
    });
  }
  function cancelNameEntry() {
    const pending = nameIntent;
    setNameIntent(null);
    if (pending?.action === "rename") void releaseUnused(pending.tickets.map((ticket) => ticket.id));
    restoreRef.current?.focus();
  }
  function submitNameEntry() {
    const pending = nameIntent;
    const trimmedName = newName.trim();
    if (!pending || !trimmedName) return;
    setNameIntent(null);
    begin(pending.tickets, pending.action, "selection", {
      destinationDirectory: pending.destinationDirectory,
      sourcePaneId: pending.sourcePaneId,
      sourceSignature: pending.sourceSignature,
      destinationPaneId: pending.destinationPaneId,
      destinationSignature: pending.destinationSignature,
      newName: trimmedName,
    });
  }
  function paste() {
    const clipboard = useFileManagementStore.getState().clipboard;
    if (clipboard) begin(clipboard.tickets, clipboard.action, "clipboard");
  }
  const actionsRef = useRef({ collect, paste, moveSelection, renameSelection, createDirectory, trashSelection });
  actionsRef.current = { collect, paste, moveSelection, renameSelection, createDirectory, trashSelection };
  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (!focused || pane && useFileManagementStore.getState().activePaneId !== pane.id || event.defaultPrevented || event.isComposing || event.altKey || isFileLibraryShortcutExcludedTarget(event.target) || busyRef.current || useFileManagementStore.getState().transferActive) return;
      const key = event.key.toLowerCase();
      if (!event.ctrlKey && !event.metaKey && key === "f2") { event.preventDefault(); actionsRef.current.renameSelection(); return; }
      if ((!event.ctrlKey && !event.metaKey && key === "delete") || (event.metaKey && !event.ctrlKey && key === "backspace")) { event.preventDefault(); actionsRef.current.trashSelection(); return; }
      if ((event.ctrlKey || event.metaKey) && event.shiftKey && key === "n") { event.preventDefault(); actionsRef.current.createDirectory(); return; }
      if (!(event.ctrlKey || event.metaKey) || event.shiftKey || !["c", "x", "v"].includes(key)) return;
      const destination = currentSurface();
      if (key !== "v" && (!destination || destination.count === 0)) return;
      if (key === "v" && !destination) return;
      if (key === "v" && !useFileManagementStore.getState().clipboard?.tickets.length) return;
      const liveClipboard = useFileManagementStore.getState().clipboard;
      if (key === "v" && platform === "win32" && liveClipboard?.action === "move" && liveClipboard.tickets.some((ticket) => ticket.isDirectory)) return;
      if (key === "x" && platform === "win32" && currentSurface()?.hasDirectorySelection) return;
      event.preventDefault(); if (key === "v") actionsRef.current.paste(); else actionsRef.current.collect(key === "x" ? "cut" : "copy");
    };
    window.addEventListener("keydown", handle); return () => window.removeEventListener("keydown", handle);
  }, [focused, pane?.id]);
  useEffect(() => {
    const action = state.pendingAction; if (!action || !focused) return;
    useFileManagementStore.setState({ pendingAction: null });
    if (action === "paste") paste(); else if (action === "move") moveSelection(); else if (action === "rename") renameSelection(); else if (action === "trash") trashSelection(); else if (action === "create_directory") createDirectory(); else if (action === "copy_other" || action === "move_other") transferToPeer(action === "move_other" ? "move" : "copy"); else collect(action);
  }, [state.pendingAction, focused]);
  function transferToPeer(action: TransferAction) {
    if (useFileManagementStore.getState().transferActive) { setError(t("fmTransferBusy")); return; }
    if (!pane?.peerId) return;
    const destinationSurface = useFileManagementStore.getState().surfaces[pane.peerId]?.read();
    const source = currentSurface();
    if (!destinationSurface?.browse || !destinationSurface.routingHint || !source) { setError(t("wbUnsupportedDrop")); return; }
    const peerId = pane.peerId; const destination = destinationSurface.routingHint; const destinationSignature = destinationSurface.signature;
    void run(async () => {
      const tickets = await capturedSelection();
      if (action === "move" && platform === "win32" && source.hasDirectorySelection) { await releaseUnused(tickets.map((ticket) => ticket.id)); setError(t("fmDirectoryMoveUnsupported")); return; }
      if (useFileManagementStore.getState().transferActive || useFileManagementStore.getState().transferOpen || useFileManagementStore.getState().activePaneId !== pane.id || currentSurface()?.visible === false || useFileManagementStore.getState().surfaces[peerId]?.read().signature !== destinationSignature) { await releaseUnused(tickets.map((ticket) => ticket.id)); throw Error(t("wbSelectionChanged")); }
      pane.activatePeer();
      useFileManagementStore.setState({ pendingTransfer: { paneId: peerId, sourcePaneId: pane.id, sourceSignature: source.signature, destinationSignature, tickets, action, destination } });
    });
  }
  useEffect(() => {
    const pending = state.pendingTransfer;
    if (!pane || !focused || !pending || pending.paneId !== pane.id) return;
    useFileManagementStore.setState({ pendingTransfer: null });
    const target = useFileManagementStore.getState().surfaces[pending.paneId]?.read();
    if (useFileManagementStore.getState().transferActive || useFileManagementStore.getState().transferOpen || !target || target.signature !== pending.destinationSignature) {
      void releaseUnused(pending.tickets.map((ticket) => ticket.id)); setError(t("wbSelectionChanged")); return;
    }
    begin(pending.tickets, pending.action, pending.external ? "external" : "selection", {
      sourcePaneId: pending.sourcePaneId, sourceSignature: pending.sourceSignature,
      destinationPaneId: pending.paneId, destinationSignature: pending.destinationSignature,
      destinationDirectory: pending.destination, destinationEntry: pending.destinationEntry,
    });
  }, [state.pendingTransfer, focused, pane?.id]);
  async function go(path: string) {
    await run(async () => {
      if (platform !== "win32" && platform !== "darwin") throw Error("workspace_platform_not_supported");
      previewController.close("escape");
      const admitted = await controller.openBrowse({ platform: platform === "win32" ? "windows" : "macos", routingHint: path, displayHint: path });
      if (!admitted) throw Error("browse_admission_superseded"); closePanel();
    });
  }
  const effectiveBasketPage = Math.min(basketPage, Math.max(0, Math.ceil(state.basket.length / PAGE_SIZE) - 1));
  const pageControls = (count: number, page: number, change: (page: number) => void) => <PageControls count={count} page={page} change={change} previous={t("fmPrevious")} next={t("fmNext")}/>;
  const selected = surface?.count === null ? t("fmAllResults") : String(surface?.count ?? 0);
  const disabled = busy || state.transferActive || !surface || surface.count === 0;
  const windowsDirectorySelection = platform === "win32" && Boolean(surface?.hasDirectorySelection);
  const windowsDirectoryClipboardMove = platform === "win32" && state.clipboard?.action === "move"
    && Boolean(state.clipboard.tickets.some((ticket) => ticket.isDirectory));
  const windowsDirectoryBasket = platform === "win32" && state.basket.some((ticket) => ticket.isDirectory);
  const pasteDisabled = busy || state.transferActive || !surface || !state.clipboard?.tickets.length || windowsDirectoryClipboardMove;
  const conflictRows = plan?.rows.filter((row) => conflictTicketIds.includes(row.ticketId)) ?? [];
  const nameConflict = transfer?.action === "rename" || transfer?.action === "create_directory";
  const conflictsResolved = conflictTicketIds.length > 0 && conflictTicketIds.every((id) => skip.includes(id) || keepBoth.includes(id));
  const panelTitle = t(state.basketOpen ? "fmBasket" : panel === "places" ? "fmPlaces" : panel === "results" ? "fmResultsSummary" : "fmMore");
  const resultsSummary = `${t("fmResults")} · ${state.results.filter((log) => log.status === "success").length}/${state.results.length}`;
  const resolution = (id: string, value: string) => {
    setSkip((current) => value === "skip" ? [...new Set([...current, id])] : current.filter((item) => item !== id));
    setKeepBoth((current) => value === "both" ? [...new Set([...current, id])] : current.filter((item) => item !== id));
  };
  function applyConflictResolution(value: "skip" | "both") {
    if (value === "skip") {
      setSkip((current) => [...new Set([...current.filter((id) => !conflictTicketIds.includes(id)), ...conflictTicketIds])]);
      setKeepBoth((current) => current.filter((id) => !conflictTicketIds.includes(id)));
    } else {
      setKeepBoth((current) => [...new Set([...current.filter((id) => !conflictTicketIds.includes(id)), ...conflictTicketIds])]);
      setSkip((current) => current.filter((id) => !conflictTicketIds.includes(id)));
    }
  }
  function retryNameConflict() {
    if (!transfer || !["rename", "create_directory"].includes(transfer.action)) return;
    const name = newName.trim();
    if (!name) return;
    const retry = { ...transfer, newName: name };
    setTransfer(retry);
    setPlan(null);
    setConflictTicketIds([]);
    void startTransfer(retry, [], []);
  }
  const placeGroup = (favorite: boolean) => <section className="fm-place-group">
    <h3>{t(favorite ? "fmFavorites" : "fmRecent")}</h3>
    <ul className="fm-places">{places.filter((place) => place.favorite === favorite).map((place) => <li key={place.id}>
      <button className={cn(buttonGhost, "fm-place-link")} disabled={busy || place.unavailable} onClick={() => void go(place.routingHint)}>
        {favorite ? <Star size={14}/> : <FolderOpen size={14}/>}<span><strong>{place.name}</strong><small title={place.routingHint}>{place.unavailable ? t("fmUnavailable") : place.routingHint}</small></span>
      </button>
      <button className={buttonGhost} disabled={busy} aria-label={`${t(place.favorite ? "fmUnfavorite" : "fmRemove")} ${place.name}`} onClick={() => void run(async () => setPlaces(await api.fileLibraryPlaces({ removeId: place.id })))}><X size={14}/></button>
    </li>)}</ul>
    {!places.some((place) => place.favorite === favorite) && <p className="fm-muted">{t("fmEmptyPlaces")}</p>}
  </section>;
  return <section className="fm-workflow" aria-label={t("fmActions")} data-file-management>
    <div className="fm-toolbar fm-primary-toolbar">
      <span className="fm-selection" data-has-selection={!disabled} aria-live="polite">{t("fmSelection")} <strong>{selected}</strong></span>
      <button className={buttonGhost} disabled={busy || state.transferActive || !surface?.browse || !surface.routingHint} title={t("fmShortcutNewFolder")} onClick={createDirectory}><FolderPlus size={14}/>{t("fmNewFolder")}</button>
      <button className={buttonGhost} disabled={disabled} onClick={() => collect("copy")}><Copy size={14}/>{t("fmCopy")}</button>
      <button className={buttonGhost} disabled={disabled || windowsDirectorySelection} title={windowsDirectorySelection ? t("fmDirectoryMoveUnsupported") : undefined} onClick={() => collect("cut")}><Scissors size={14}/>{t("fmCut")}</button>
      <button className={buttonGhost} disabled={pasteDisabled} onClick={paste}><ClipboardPaste size={14}/>{t("fmPaste")}{state.clipboard?.tickets.length ? ` (${state.clipboard.tickets.length})` : ""}</button>
      <button className={buttonGhost} disabled={disabled || windowsDirectorySelection} title={windowsDirectorySelection ? t("fmDirectoryMoveUnsupported") : undefined} onClick={moveSelection}><FolderInput size={14}/>{t("fmMove")}</button>
      <button className={buttonGhost} disabled={disabled || surface?.count !== 1} title={t("fmShortcutRename")} onClick={renameSelection}><Pencil size={14}/>{t("fmRename")}</button>
      <button className={buttonGhost} disabled={disabled} title={t("fmShortcutTrash")} onClick={trashSelection}><Trash2 size={14}/>{t("fmSafeDelete")}</button>
      <label className="fm-drop-mode"><span>{t("fmExternalDropAction")}</span><select aria-label={t("fmExternalDropAction")} value={state.externalDropAction} disabled={state.transferActive} onChange={(event) => useFileManagementStore.setState({ externalDropAction: event.target.value as TransferAction })}><option value="copy">{t("fmCopy")}</option><option value="move">{t("fmMove")}</option></select></label>
      <button className={cn(buttonGhost, "fm-more-trigger")} aria-expanded={panel === "more"} aria-controls={panelId} onClick={(event) => panel === "more" ? closePanel() : openPanel("more", event.currentTarget)}><MoreHorizontal size={16}/>{t("fmMore")}{state.basket.length > 0 && <span className="fm-count">{state.basket.length}</span>}</button>
      {!!state.results.length && <button className={cn(buttonGhost, "fm-result-trigger")} aria-expanded={panel === "results"} onClick={(event) => panel === "results" ? closePanel() : openPanel("results", event.currentTarget)}>{state.results.some((log) => log.status !== "success") ? <AlertTriangle size={14}/> : <Check size={14}/>}<span>{resultsSummary}</span></button>}
    </div>
    {focused && (error || notice) && <div className="fm-notice" role={error ? "alert" : "status"}>{error || notice}<button className={buttonGhost} aria-label={t("fmDismiss")} onClick={() => { setError(""); setNotice(""); }}><X size={14}/></button></div>}
    {transfer && state.transferRunning && <aside className="fm-task-progress" role="status" aria-live="polite">
      <div className="fm-task-heading"><strong>{transferPhase === "preview" ? t("fmPreparing") : progress ? `${progress.processed}/${progress.total} · ${t("fmFiles")}` : t("fmExecuting")}</strong><button className={buttonGhost} disabled={cancelling} onClick={cancelRunningTransfer}>{t(cancelling ? "fmCancelPending" : "fmCancelRunning")}</button></div>
      <progress max={progress?.total || 1} value={progress?.processed ?? 0}/>
      <span className="fm-path" title={progress?.currentPath || transfer.tickets[0]?.path}>{progress?.currentPath || transfer.tickets[0]?.name}</span>
      <div className="fm-toolbar"><button className={buttonGhost} onClick={() => openPanel("results")}>{t("fmTaskDetails")}</button><button className={buttonGhost} onClick={() => setView("restore")}>{t("fmHistory")}</button></div>
    </aside>}
    {panelOpen && <div ref={panelRef} id={panelId} className="fm-panel" role="region" aria-label={panelTitle}>
      <div className="fm-panel-heading"><h2>{panelTitle}{state.basketOpen ? ` (${state.basket.length})` : ""}</h2><button className={buttonGhost} aria-label={t("fmDismiss")} onClick={() => closePanel()}><X size={16}/></button></div>
      <div className="fm-panel-body">
      {panel === "more" && <div className="fm-more-actions">
        <button className={buttonGhost} disabled={disabled} onClick={() => collect("basket")}><PackagePlus size={16}/>{t("fmAddBasket")}</button>
        <button className={buttonGhost} onClick={() => openPanel("basket")}><Package size={16}/>{t("fmBasket")} ({state.basket.length})<ChevronRight size={14}/></button>
        <button className={buttonGhost} onClick={() => openPanel("places")}><Star size={16}/>{t("fmPlaces")}<ChevronRight size={14}/></button>
        <hr/>
        <button className={buttonGhost} disabled={busy || !surface?.selectAll} onClick={() => { surface?.selectAll?.(); closePanel(); }}>{t(surface?.selectionScope === "loaded" ? "fmSelectLoaded" : "fmSelectAll")}</button>
        <button className={buttonGhost} disabled={disabled} onClick={() => { surface?.clear(); closePanel(); }}>{t("fmClearSelection")}</button>
      </div>}
      {panel === "places" && <>
        <form className="fm-location-form" onSubmit={(event) => { event.preventDefault(); void go(locationInput); }}><label>{t("fmGoFolder")}<input value={locationInput} onChange={(event) => setLocationInput(event.target.value)} placeholder={t("fmPathHint")} /></label><button className={buttonSecondary} disabled={busy || !locationInput.trim()}>{t("fmGo")}</button></form>
        <button className={buttonSecondary} disabled={busy || !surface?.browse} onClick={() => void run(async () => { setPlaces(await api.fileLibraryPlaces({ ...surface!.browse!, favorite: true })); })}><Star size={14}/>{t("fmFavoriteCurrent")}</button>
        {placeGroup(true)}{placeGroup(false)}
      </>}
      {state.basketOpen && <>
        <div className="fm-toolbar"><button className={buttonSecondary} disabled={busy || state.transferActive || !state.basket.length} onClick={() => begin(state.basket, "copy", "basket")}>{t("fmCopyBasket")}</button><button className={buttonSecondary} disabled={busy || state.transferActive || !state.basket.length || windowsDirectoryBasket} title={windowsDirectoryBasket ? t("fmDirectoryMoveUnsupported") : undefined} onClick={() => begin(state.basket, "move", "basket")}>{t("fmMoveBasket")}</button><button className={buttonGhost} disabled={busy || state.transferActive || !state.basket.length} onClick={() => void run(async () => { const ids = state.basket.map((ticket) => ticket.id); useFileManagementStore.setState({ basket: [] }); await releaseUnused(ids); })}>{t("fmClearBasket")}</button></div>
        <p className="fm-muted">{t("fmSessionOnly")}</p>
        {!state.basket.length && <p className="fm-empty">{t("fmEmptyBasket")}</p>}
        <ul className="fm-basket">{state.basket.slice(effectiveBasketPage * PAGE_SIZE, (effectiveBasketPage + 1) * PAGE_SIZE).map((ticket) => <li key={ticket.id}><File size={16}/><div className="min-w-0"><strong>{ticket.name}</strong><p className="fm-path" title={ticket.path}>{ticket.path}</p>{state.statuses[ticket.id] && <p className="fm-item-status">{state.statuses[ticket.id]}</p>}</div><button className={buttonGhost} disabled={busy || state.transferActive} aria-label={`${t("fmRemove")} ${ticket.name}`} onClick={() => void run(async () => { useFileManagementStore.setState((current) => ({ basket: current.basket.filter((item) => item.id !== ticket.id) })); await releaseUnused([ticket.id]); })}><X size={14}/></button></li>)}</ul>
        {pageControls(state.basket.length, effectiveBasketPage, setBasketPage)}
      </>}
      {panel === "results" && <>
        {transfer && state.transferRunning && <div className="fm-progress fm-task-details" role="status"><strong>{transferPhase === "preview" ? t("fmPreparing") : t("fmExecuting")}</strong><progress max={progress?.total || 1} value={progress?.processed ?? 0}/><span>{progress ? `${progress.processed}/${progress.total}` : `${transfer.tickets.length} ${t("fmFiles")}`} · {progress?.currentPath || transfer.tickets[0]?.path}</span><button className={buttonGhost} onClick={() => setView("restore")}>{t("fmHistory")}</button></div>}
        <p className="fm-muted">{resultsSummary}</p>
        <ul className="fm-results">{state.results.slice(resultPage * PAGE_SIZE, (resultPage + 1) * PAGE_SIZE).map((log) => <li key={log.id}><strong>{log.name_after}</strong><span className="fm-item-status">{log.status === "success" ? t("fmSucceeded") : log.status === "skipped" ? t("fmSkipped") : t("fmFailed")}</span><p className="fm-path" title={log.path_after}>{log.path_after}</p>{log.error_message && <p role="alert" className="fm-item-error">{log.error_message}</p>}</li>)}</ul>
        {pageControls(state.results.length, resultPage, setResultPage)}
        <div className="fm-toolbar"><button className={buttonSecondary} onClick={() => setView("restore")}>{t("fmHistory")}</button><button className={buttonGhost} onClick={() => void go(state.results[0]!.path_after.replace(/[/\\][^/\\]+$/, ""))}>{t("fmOpenDestination")}</button><button className={buttonGhost} onClick={() => { useFileManagementStore.setState({ results: [] }); closePanel(); }}>{t("fmDismiss")}</button></div>
        <p className="fm-muted">{t("fmRestoreHelp")}</p>
      </>}
      </div>
    </div>}
    {nameIntent && <ModalPortal initialFocusRef={nameInputRef} restoreFocus={() => restoreRef.current?.isConnected ? restoreRef.current : panelTriggerRef.current} onEscape={cancelNameEntry}><div className="fixed inset-0 grid place-items-center bg-[var(--zc-overlay)] p-4"><section className="fm-dialog" role="dialog" aria-modal="true" aria-labelledby={pane ? `fm-name-title-${pane.id}` : "fm-name-title"}>
      <header className="fm-dialog-header"><h2 id={pane ? `fm-name-title-${pane.id}` : "fm-name-title"}>{t(nameIntent.action === "rename" ? "fmRename" : "fmNewFolder")}</h2></header>
      <form onSubmit={(event) => { event.preventDefault(); submitNameEntry(); }}>
        <div className="fm-dialog-body"><label className="fm-destination">{t(nameIntent.action === "rename" ? "fmNewName" : "fmFolderName")}<input ref={nameInputRef} value={newName} onChange={(event) => setNewName(event.target.value)} /></label></div>
        <footer className="fm-dialog-footer"><button type="button" className={buttonSecondary} onClick={cancelNameEntry}>{t("fmCancel")}</button><button type="submit" className={buttonPrimary} disabled={!newName.trim()}>{t(nameIntent.action === "rename" ? "fmRename" : "fmNewFolder")}</button></footer>
      </form>
    </section></div></ModalPortal>}
    {transfer && state.transferOpen && plan && <ModalPortal initialFocusRef={nameConflict ? nameInputRef : undefined} restoreFocus={() => restoreRef.current?.isConnected ? restoreRef.current : panelTriggerRef.current} onEscape={closeTransfer}><div className="fixed inset-0 grid place-items-center bg-[var(--zc-overlay)] p-4"><section className="fm-dialog fm-conflict-dialog" role="dialog" aria-modal="true" aria-labelledby={pane ? `fm-conflict-title-${pane.id}` : "fm-conflict-title"} aria-busy={state.transferRunning}>
      <header className="fm-dialog-header">
        <div className="fm-dialog-title"><AlertTriangle size={21}/><div>
          <h2 id={pane ? `fm-conflict-title-${pane.id}` : "fm-conflict-title"}>{t(nameConflict ? "fmConflict" : "fmConflictsTitle")}</h2>
          <p className="fm-muted">{conflictTicketIds.length} · {t("fmConflictsPaused")}</p>
        </div></div>
        {!nameConflict && <p className="fm-muted">{t("fmNoOverwrite")}</p>}
        {!nameConflict && <div className="fm-toolbar fm-conflict-all">
          <span>{t("fmApplyToAll")}</span>
          <button className={buttonSecondary} disabled={state.transferRunning} onClick={() => applyConflictResolution("skip")}>{t("fmSkip")}</button>
          <button className={buttonSecondary} disabled={state.transferRunning} onClick={() => applyConflictResolution("both")}>{t("fmKeepBothShort")}</button>
        </div>}
      </header>
      <div className="fm-dialog-body">
        {nameConflict ? <label className="fm-destination">{t(transfer.action === "rename" ? "fmNewName" : "fmFolderName")}<input ref={nameInputRef} value={newName} onChange={(event) => setNewName(event.target.value)} /></label> : <ul className="fm-preview fm-conflicts">{conflictRows.map((row) => {
          const ticket = transfer.tickets.find((item) => item.id === row.ticketId);
          if (!ticket) return null;
          const value = skip.includes(ticket.id) ? "skip" : keepBoth.includes(ticket.id) ? "both" : "";
          return <li key={ticket.id}>
            <div className="fm-preview-file"><File size={17}/><div><strong>{ticket.relativePath || ticket.name}</strong><p className="fm-path" title={ticket.path}>{ticket.path}</p><p className="fm-item-error">{t("fmConflict")}</p></div></div>
            <select aria-label={`${ticket.name} · ${t("fmResolution")}`} disabled={state.transferRunning} value={value} onChange={(event) => resolution(ticket.id, event.target.value)}>
              <option value="">{t("fmChooseResolution")}</option>
              <option value="skip">{t("fmSkip")}</option>
              <option value="both">{t("fmKeepBothShort")}</option>
            </select>
          </li>;
        })}</ul>}
      </div>
      <footer className="fm-dialog-footer">
        <p className="fm-muted">{t(nameConflict ? "fmConflictsRepreview" : "fmConflictsRepreview")}</p>
        <div className="fm-toolbar fm-confirm-bar">
          <button className={buttonSecondary} disabled={state.transferRunning} onClick={closeTransfer}>{t("fmCancel")}</button>
          <button className={buttonPrimary} disabled={(nameConflict ? !newName.trim() : !conflictsResolved) || state.transferRunning} onClick={() => nameConflict ? retryNameConflict() : void startTransfer(transfer, skip, keepBoth)}>{nameConflict ? t(transfer.action === "rename" ? "fmRename" : "fmNewFolder") : t("fmApplyChoices")}</button>
        </div>
      </footer>
    </section></div></ModalPortal>}
  </section>;
}
