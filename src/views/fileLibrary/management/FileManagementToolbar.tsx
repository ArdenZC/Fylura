import { usePane } from "../workbench/PaneContext";
import { useCallback, useEffect, useRef, useState } from "react";
import { Copy, Scissors, ClipboardPaste, FolderInput, PackagePlus, Star, X, FolderOpen, MoreHorizontal, Package, File, Check, AlertTriangle, ChevronRight, FolderPlus, Pencil, Trash2 } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { fileManagementApi as api, type TransferAction, type TransferPreview, type TransferTicket, type SavedPlace } from "../../../api/fileManagementApi";
import { tauriApi } from "../../../api/tauriApi";
import type { OperationProgressPayload } from "../../../api/types";
import { useChromeContext, useI18nContext, useNavigationContext } from "../../../contexts/AppContexts";
import { useOperationQueueStore } from "../../../store/useOperationQueueStore";
import { buttonGhost, buttonPrimary, buttonSecondary, cn } from "../../../utils/tw";
import { readableError } from "../../../utils/viewHelpers";
import { ModalPortal } from "../../../components/modal/ModalPortal";
import { useFileLibraryExperience } from "../FileLibraryExperienceProvider";
import { isFileLibraryShortcutExcludedTarget } from "../fileLibraryInteraction";
import { usePreviewExperience } from "../preview/PreviewExperienceProvider";
import { useFileManagementStore } from "./fileManagementStore";
import "./fileManagement.css";

const PAGE_SIZE = 40;
function PageControls({ count, page, change, previous, next }: { count: number; page: number; change: (page: number) => void; previous: string; next: string }) {
  if (count <= PAGE_SIZE) return null;
  return <div className="fm-toolbar"><button className={buttonGhost} disabled={page <= 0} onClick={() => change(page - 1)}>{previous}</button><span className="text-xs">{page * PAGE_SIZE + 1}–{Math.min((page + 1) * PAGE_SIZE, count)} / {count}</span><button className={buttonGhost} disabled={(page + 1) * PAGE_SIZE >= count} onClick={() => change(page + 1)}>{next}</button></div>;
}
type TransferIntent = {
  tickets: TransferTicket[];
  action: TransferAction;
  origin: "selection" | "clipboard" | "basket";
  sourcePaneId?: string;
  sourceSignature?: string;
  automatic: boolean;
};

export function FileManagementToolbar() {
  const { t } = useI18nContext(); const { platform } = useChromeContext(); const { setView } = useNavigationContext();
  const windows = platform === "win32";
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
  const [newName, setNewName] = useState("");
  const [destination, setDestination] = useState(""); const [plan, setPlan] = useState<TransferPreview | null>(null);
  const [skip, setSkip] = useState<string[]>([]); const [keepBoth, setKeepBoth] = useState<string[]>([]);
  const [progress, setProgress] = useState<OperationProgressPayload | null>(null);
  const [automaticTask, setAutomaticTask] = useState<"preparing" | "executing" | null>(null);
  const destinationRef = useRef<HTMLInputElement | null>(null); const restoreRef = useRef<HTMLElement | null>(null);
  const nameInputRef = useRef<HTMLInputElement | null>(null);
  const preparing = useRef(0); const mounted = useRef(true); const busyRef = useRef(false);
  const cancelRequested = useRef(false); const executionSent = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; preparing.current++; }; }, []);
  const describeError = useCallback((value: unknown) => {
    const detail = readableError(value);
    const key = detail.includes("cross_volume_move") ? "fmCrossVolumeMove" : detail.includes("windows_directory") ? "fmWindowsDirectories" : detail.includes("target_exists") ? "fmConflict" : detail.includes("limit_1000") ? "fmLimit" : detail.includes("source_changed") || detail.includes("expired") || detail.includes("preview_stale") ? "fmStale" : "fmFailed";
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
    const live = useFileManagementStore.getState();
    const retained = new Set([...live.basket, ...(live.clipboard?.tickets ?? [])].map((ticket) => ticket.id));
    const releaseIds = ids.filter((id) => !retained.has(id));
    if (releaseIds.length) await api.captureTransfer({ releaseIds });
  }
  function collect(action: "copy" | "cut" | "basket") {
    void run(async () => {
      const tickets = await capturedSelection(); if (!tickets.length) return;
      if (windows && tickets.some((ticket) => ticket.isDirectory)) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        throw Error("transfer_windows_directory_not_supported");
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
  function begin(
    tickets: TransferTicket[],
    action: TransferAction,
    origin: TransferIntent["origin"],
    options?: Pick<TransferIntent, "sourcePaneId" | "sourceSignature"> & { destination?: string },
  ) {
    if (!tickets.length || useFileManagementStore.getState().transferOpen) return;
    if (windows && (action === "copy" || action === "move") && tickets.some((ticket) => ticket.isDirectory)) {
      setError(t("fmWindowsDirectories"));
      return;
    }
    closePanel(false);
    setCancelling(false);
    restoreRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const nextDestination = options?.destination ?? (action === "rename" && tickets[0]
      ? tickets[0].path.replace(/[/\\][^/\\]+$/, "")
      : currentPath);
    const automatic = (action === "copy" || action === "move") && nextDestination.trim().length > 0;
    const nextTransfer: TransferIntent = { tickets, action, origin, automatic, sourcePaneId: options?.sourcePaneId, sourceSignature: options?.sourceSignature };
    useFileManagementStore.setState({ results: [], transferOpen: true }); setPreviewPage(0); setResultPage(0);
    setTransfer(nextTransfer);
    setDestination(nextDestination);
    setPlan(null); setSkip([]); setKeepBoth([]); setNewName(action === "rename" ? tickets[0]?.name ?? "" : ""); setError("");
    if (automatic) void prepareForTransfer(nextTransfer, nextDestination, "", [], [], true);
  }
  function closeTransfer() {
    preparing.current++; setTransfer(null); setPlan(null); useFileManagementStore.setState({ transferOpen: false });
    setAutomaticTask(null);
    if (transfer?.origin === "selection") void releaseUnused(transfer.tickets.map((ticket) => ticket.id)).catch((value) => { if (mounted.current) setError(describeError(value)); });
  }
  function moveSelection() {
    let accepted: { tickets: TransferTicket[]; destination: string; sourceSignature: string | undefined } | null = null;
    void run(async () => {
      const sourceSignature = currentSurface()?.signature;
      const tickets = await capturedSelection();
      if (windows && tickets.some((ticket) => ticket.isDirectory)) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        throw Error("transfer_windows_directory_not_supported");
      }
      if (!tickets.length) return;
      let selectedDestination: string | string[] | null;
      try {
        selectedDestination = await open({ directory: true, multiple: false, title: t("fmDestination") });
      } catch (error) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        throw error;
      }
      const destination = Array.isArray(selectedDestination) ? selectedDestination[0] : selectedDestination;
      if (!destination) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        return;
      }
      if (currentSurface()?.signature !== sourceSignature) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        throw Error("transfer_selection_changed");
      }
      accepted = { tickets, destination, sourceSignature };
    }).then(() => {
      if (!accepted) return;
      if (useFileManagementStore.getState().transferOpen || currentSurface()?.signature !== accepted.sourceSignature) {
        void releaseUnused(accepted.tickets.map((ticket) => ticket.id)).catch((error) => setError(describeError(error)));
        setError(t("wbSelectionChanged"));
        return;
      }
      begin(accepted.tickets, "move", "selection", {
        destination: accepted.destination,
        sourcePaneId: pane?.id,
        sourceSignature: accepted.sourceSignature,
      });
    });
  }
  function trashSelection() { void run(async () => { begin(await capturedSelection(), "trash", "selection"); }); }
  function renameSelection() {
    void run(async () => {
      const tickets = await capturedSelection();
      if (tickets.length !== 1) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        throw Error("transfer_rename_selection_invalid");
      }
      begin(tickets, "rename", "selection");
    });
  }
  function createDirectory() {
    if (!currentPath || !surface?.browse || useFileManagementStore.getState().transferOpen) return;
    closePanel(false);
    setCancelling(false);
    restoreRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    useFileManagementStore.setState({ results: [], transferOpen: true });
    setPreviewPage(0); setResultPage(0);
    setTransfer({ tickets: [], action: "create_directory", origin: "selection", automatic: false });
    setDestination(currentPath); setNewName(t("fmNewFolderDefault")); setPlan(null); setSkip([]); setKeepBoth([]); setError("");
  }
  function paste() { const clipboard = useFileManagementStore.getState().clipboard; if (clipboard) begin(clipboard.tickets, clipboard.action, "clipboard"); }
  const actionsRef = useRef({ collect, paste, moveSelection, trashSelection, renameSelection, createDirectory }); actionsRef.current = { collect, paste, moveSelection, trashSelection, renameSelection, createDirectory };
  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (!focused || pane && useFileManagementStore.getState().activePaneId !== pane.id || event.defaultPrevented || event.isComposing || event.altKey || isFileLibraryShortcutExcludedTarget(event.target) || busyRef.current) return;
      const key = event.key.toLowerCase();
      if (!event.ctrlKey && !event.metaKey && key === "f2") { event.preventDefault(); actionsRef.current.renameSelection(); return; }
      if ((!event.ctrlKey && !event.metaKey && key === "delete") || (event.metaKey && !event.ctrlKey && key === "backspace")) { event.preventDefault(); actionsRef.current.trashSelection(); return; }
      if ((event.ctrlKey || event.metaKey) && event.shiftKey && key === "n") { event.preventDefault(); actionsRef.current.createDirectory(); return; }
      if (!(event.ctrlKey || event.metaKey) || event.shiftKey || !["c", "x", "v"].includes(key)) return;
      if (key !== "v" && (!useFileManagementStore.getState().surface || useFileManagementStore.getState().surface?.()?.count === 0)) return;
      if (key === "v" && !useFileManagementStore.getState().clipboard?.tickets.length) return;
      event.preventDefault(); if (key === "v") actionsRef.current.paste(); else actionsRef.current.collect(key === "x" ? "cut" : "copy");
    };
    window.addEventListener("keydown", handle); return () => window.removeEventListener("keydown", handle);
  }, [focused]);
  useEffect(() => {
    const action = state.pendingAction; if (!action || !focused) return;
    useFileManagementStore.setState({ pendingAction: null });
    if (action === "paste") paste(); else if (action === "move") moveSelection(); else if (action === "rename") renameSelection(); else if (action === "trash") trashSelection(); else if (action === "create_directory") createDirectory(); else if (action === "copy_other" || action === "move_other") transferToPeer(action === "move_other" ? "move" : "copy"); else collect(action);
  }, [state.pendingAction, focused]);
  function transferToPeer(action: TransferAction) {
    if (!pane?.peerId) return;
    const destinationSurface = useFileManagementStore.getState().surfaces[pane.peerId]?.read();
    const source = currentSurface();
    if (!destinationSurface?.browse || !destinationSurface.routingHint || !source) { setError(t("wbUnsupportedDrop")); return; }
    const peerId = pane.peerId; const destination = destinationSurface.routingHint; const destinationSignature = destinationSurface.signature;
    void run(async () => {
      const tickets = await capturedSelection();
      if (windows && tickets.some((ticket) => ticket.isDirectory)) {
        await releaseUnused(tickets.map((ticket) => ticket.id));
        throw Error("transfer_windows_directory_not_supported");
      }
      if (useFileManagementStore.getState().transferOpen || useFileManagementStore.getState().activePaneId !== pane.id || currentSurface()?.visible === false || useFileManagementStore.getState().surfaces[peerId]?.read().signature !== destinationSignature) { await releaseUnused(tickets.map((ticket) => ticket.id)); throw Error(t("wbSelectionChanged")); }
      pane.activatePeer();
      useFileManagementStore.setState({ pendingTransfer: { paneId: peerId, sourcePaneId: pane.id, sourceSignature: source.signature, tickets, action, destination } });
    });
  }
  useEffect(() => {
    const pending = state.pendingTransfer;
    if (!pane || !focused || !pending || pending.paneId !== pane.id) return;
    useFileManagementStore.setState({ pendingTransfer: null });
    if (useFileManagementStore.getState().transferOpen) { void releaseUnused(pending.tickets.map((ticket) => ticket.id)); return; }
    begin(pending.tickets, pending.action, "selection", {
      sourcePaneId: pending.sourcePaneId,
      sourceSignature: pending.sourceSignature,
      destination: pending.destination,
    });
  }, [state.pendingTransfer, focused, pane?.id]);
  async function chooseDestination() {
    const result = await open({ directory: true, multiple: false, title: t("fmDestination") });
    const path = Array.isArray(result) ? result[0] : result;
    if (path) { setDestination(path); setPlan(null); }
  }
  async function prepareForTransfer(intent: TransferIntent, target: string, name: string, nextSkip: string[], nextBoth: string[], automatic: boolean) {
    const epoch = ++preparing.current;
    setPlan(null);
    if (automatic) setAutomaticTask("preparing");
    await run(async () => {
      let next: TransferPreview;
      try {
        next = await api.previewTransfer({ ticketIds: intent.tickets.map((ticket) => ticket.id), action: intent.action, destinationDirectory: target, ...(intent.action === "rename" || intent.action === "create_directory" ? { newName: name } : {}), skip: nextSkip, keepBoth: nextBoth });
      } catch (value) {
        if (automatic && mounted.current && preparing.current === epoch) {
          setTransfer(null); setAutomaticTask(null); useFileManagementStore.setState({ transferOpen: false });
          if (intent.origin === "selection") await releaseUnused(intent.tickets.map((ticket) => ticket.id));
        }
        throw value;
      }
      if (!mounted.current || preparing.current !== epoch) return;
      setPlan(next);
      const firstProblem = next.rows.find((row) => row.error && !row.skipped);
      if (firstProblem) setPreviewPage(Math.floor(Math.max(0, intent.tickets.findIndex((ticket) => ticket.id === firstProblem.ticketId)) / PAGE_SIZE));
      const statuses = { ...useFileManagementStore.getState().statuses };
      next.rows.forEach((row) => { if (row.error) statuses[row.ticketId] = describeError(row.error); else delete statuses[row.ticketId]; });
      useFileManagementStore.setState({ statuses });

      if (!automatic) return;
      const hasConflict = next.rows.some((row) => !row.skipped && row.error?.includes("target_exists"));
      if (hasConflict) {
        setTransfer({ ...intent, automatic: false });
        setAutomaticTask(null);
        return;
      }
      const blocking = next.rows.find((row) => row.error && !row.skipped);
      const incomplete = next.rows.length !== intent.tickets.length || next.rows.some((row) => !row.skipped && !row.preview);
      if (blocking || incomplete) {
        setError(describeError(blocking?.error ?? "transfer_preview_incomplete"));
        setTransfer(null); setPlan(null); setAutomaticTask(null); useFileManagementStore.setState({ transferOpen: false });
        if (intent.origin === "selection") await releaseUnused(intent.tickets.map((ticket) => ticket.id));
        return;
      }
      setAutomaticTask("executing");
      await executePlan(next, intent);
    });
  }
  async function prepare(nextSkip = skip, nextBoth = keepBoth) {
    if (!transfer) return;
    await prepareForTransfer(transfer, destination, newName, nextSkip, nextBoth, false);
  }
  async function executePlan(acceptedPlan: TransferPreview, acceptedTransfer: TransferIntent) {
      cancelRequested.current = false; executionSent.current = false;
      setProgress({ kind: "execute", batchId: "", processed: 0, total: acceptedPlan.rows.filter((row) => !row.skipped).length, currentPath: acceptedPlan.rows.find((row) => !row.skipped && row.preview)?.preview?.source_path ?? "" });
      let dispose: (() => void) | undefined;
      try {
        dispose = await tauriApi.onOperationProgress((update) => { if (mounted.current && update.kind === "execute") setProgress(update); });
        if (pane) await pane.beforeMutation(); else await previewController.closeForFileOperation();
        if (cancelRequested.current) {
          setTransfer(null); setAutomaticTask(null); useFileManagementStore.setState({ transferOpen: false });
          if (acceptedTransfer.origin === "selection") await releaseUnused(acceptedTransfer.tickets.map((ticket) => ticket.id));
          return;
        }
        executionSent.current = true;
        let result;
        try { result = await api.executeTransfer({ previewId: acceptedPlan.id, confirmed: true }); }
        catch (value) {
          setError(`${t("fmExecutionUncertain")} ${readableError(value)}`);
          if (acceptedTransfer.automatic) { setTransfer(null); setPlan(null); setAutomaticTask(null); useFileManagementStore.setState({ transferOpen: false }); }
          return;
        }
        const rows = acceptedPlan.rows.filter((row) => !row.skipped && row.preview);
        const succeeded = new Set<string>(); const statuses = { ...useFileManagementStore.getState().statuses };
        rows.forEach((row, index) => { const log = result.logs[index]; statuses[row.ticketId] = log?.status === "success" ? t("fmSucceeded") : log?.status === "skipped" ? t("fmSkipped") : log?.error_message || t("fmFailed"); if (log?.status === "success") succeeded.add(row.ticketId); });
        const current = useFileManagementStore.getState();
        useFileManagementStore.setState({ results: result.logs, statuses, basketOpen: false,
          ...(["move", "trash"].includes(acceptedTransfer.action) ? { basket: current.basket.filter((ticket) => !succeeded.has(ticket.id)), clipboard: current.clipboard ? { ...current.clipboard, tickets: current.clipboard.tickets.filter((ticket) => !succeeded.has(ticket.id)) } : null } : {}) });
        setTransfer(null); setPlan(null); setAutomaticTask(null); setNotice(t("fmFinished")); useFileManagementStore.setState({ transferOpen: false });
        if (result.logs.some((log) => log.status !== "success")) setPanel("results");
        previewController.close("escape");
        const affectedSurface = acceptedTransfer.sourcePaneId ? useFileManagementStore.getState().surfaces[acceptedTransfer.sourcePaneId]?.read() : currentSurface();
        if (!acceptedTransfer.sourceSignature || affectedSurface?.signature === acceptedTransfer.sourceSignature) affectedSurface?.clear();
        try { if (pane) await pane.afterMutation([acceptedPlan.destinationDirectory, ...acceptedTransfer.tickets.map((ticket) => ticket.path.replace(/[/\\][^/\\]+$/, ""))]); else await affectedSurface?.refresh(); await useOperationQueueStore.getState().refreshOperationLogs(); } catch (value) { setError(`${t("fmRefreshFailed")} ${readableError(value)}`); }
        if (acceptedTransfer.action === "move" || acceptedTransfer.origin === "selection") {
          try { await releaseUnused(acceptedTransfer.origin === "selection" ? acceptedTransfer.tickets.map((ticket) => ticket.id) : [...succeeded]); } catch (value) { setError(`${t("fmRefreshFailed")} ${readableError(value)}`); }
        }
      } finally { dispose?.(); executionSent.current = false; setProgress(null); setPlan(null); }
  }
  async function execute() {
    if (!plan || !transfer) return;
    const acceptedPlan = plan; const acceptedTransfer = transfer;
    await run(async () => {
      await executePlan(acceptedPlan, acceptedTransfer);
    });
  }
  function cancelTask() {
    if (!progress) { closeTransfer(); return; }
    setCancelling(true); cancelRequested.current = true;
    if (executionSent.current) void tauriApi.cancelOperations().catch((value) => { setError(describeError(value)); setCancelling(false); });
  }
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
  const disabled = busy || !surface || surface.count === 0;
  const rowsReady = plan && plan.rows.some((row) => !row.skipped && row.preview) && plan.rows.every((row) => row.skipped || !row.error);
  const readyCount = plan?.rows.filter((row) => !row.skipped && row.preview && !row.error).length ?? 0;
  const skippedCount = skip.length;
  const previewTickets = transfer?.action === "create_directory"
    ? [{ id: "create-directory", name: newName || t("fmNewFolderDefault"), path: destination, isDirectory: true }]
    : transfer?.tickets ?? [];
  const windowsDirectorySelection = windows && !!surface?.hasDirectorySelection;
  const windowsDirectoryClipboard = windows && !!state.clipboard?.tickets.some((ticket) => ticket.isDirectory);
  const windowsDirectoryBasket = windows && state.basket.some((ticket) => ticket.isDirectory);
  const transferTitle = transfer?.action === "copy" ? "fmCopyPreview"
    : transfer?.action === "move" ? "fmMovePreview"
      : transfer?.action === "rename" ? "fmRenamePreview"
        : transfer?.action === "trash" ? "fmTrashPreview"
          : "fmCreateFolderPreview";
  const panelTitle = t(state.basketOpen ? "fmBasket" : panel === "places" ? "fmPlaces" : panel === "results" ? "fmResultsSummary" : "fmMore");
  const resultsSummary = `${t("fmResults")} · ${state.results.filter((log) => log.status === "success").length}/${state.results.length}`;
  const resolution = (id: string, value: string) => {
    setSkip((current) => value === "skip" ? [...current.filter((item) => item !== id), id] : current.filter((item) => item !== id));
    setKeepBoth((current) => value === "both" ? [...current.filter((item) => item !== id), id] : current.filter((item) => item !== id));
    preparing.current++; setPlan(null);
  };
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
      <button className={buttonGhost} disabled={busy || !surface?.browse || !currentPath} title={t("fmShortcutNewFolder")} onClick={createDirectory}><FolderPlus size={14}/>{t("fmNewFolder")}</button>
      <button className={buttonGhost} disabled={disabled || windowsDirectorySelection} title={windowsDirectorySelection ? t("fmWindowsDirectories") : undefined} onClick={() => collect("copy")}><Copy size={14}/>{t("fmCopy")}</button>
      <button className={buttonGhost} disabled={disabled || windowsDirectorySelection} title={windowsDirectorySelection ? t("fmWindowsDirectories") : undefined} onClick={() => collect("cut")}><Scissors size={14}/>{t("fmCut")}</button>
      <button className={buttonGhost} disabled={busy || !state.clipboard?.tickets.length || windowsDirectoryClipboard} title={windowsDirectoryClipboard ? t("fmWindowsDirectories") : undefined} onClick={paste}><ClipboardPaste size={14}/>{t("fmPaste")}{state.clipboard?.tickets.length ? ` (${state.clipboard.tickets.length})` : ""}</button>
      <button className={buttonGhost} disabled={disabled || windowsDirectorySelection} title={windowsDirectorySelection ? t("fmWindowsDirectories") : undefined} onClick={moveSelection}><FolderInput size={14}/>{t("fmMove")}</button>
      <button className={buttonGhost} disabled={disabled || surface?.count !== 1} title={t("fmShortcutRename")} onClick={renameSelection}><Pencil size={14}/>{t("fmRename")}</button>
      <button className={buttonGhost} disabled={disabled} title={t("fmShortcutTrash")} onClick={trashSelection}><Trash2 size={14}/>{t("fmSafeDelete")}</button>
      <button className={cn(buttonGhost, "fm-more-trigger")} aria-expanded={panel === "more"} aria-controls={panelId} onClick={(event) => panel === "more" ? closePanel() : openPanel("more", event.currentTarget)}><MoreHorizontal size={16}/>{t("fmMore")}{state.basket.length > 0 && <span className="fm-count">{state.basket.length}</span>}</button>
      {!!state.results.length && <button className={cn(buttonGhost, "fm-result-trigger")} aria-expanded={panel === "results"} onClick={(event) => panel === "results" ? closePanel() : openPanel("results", event.currentTarget)}>{state.results.some((log) => log.status !== "success") ? <AlertTriangle size={14}/> : <Check size={14}/>}<span>{resultsSummary}</span></button>}
    </div>
    {focused && !transfer && (error || notice) && <div className="fm-notice" role={error ? "alert" : "status"}>{error || notice}<button className={buttonGhost} aria-label={t("fmDismiss")} onClick={() => { setError(""); setNotice(""); }}><X size={14}/></button></div>}
     {focused && transfer?.automatic && automaticTask && <div className="fm-notice fm-task" role="status" aria-live="polite">
       <span>{t(automaticTask === "preparing" ? "fmPrepare" : "fmRunning")}</span>
       {progress && <><progress max={progress.total || 1} value={progress.processed}/><span>{cancelling ? t("fmCancelPending") : `${progress.processed}/${progress.total}`} · {progress.currentPath}</span></>}
       <button className={buttonGhost} disabled={cancelling} onClick={cancelTask}>{t(progress ? "fmCancelRunning" : "fmCancel")}</button>
     </div>}
    {panelOpen && <div ref={panelRef} id={panelId} className="fm-panel" role="region" aria-label={panelTitle}>
      <div className="fm-panel-heading"><h2>{panelTitle}{state.basketOpen ? ` (${state.basket.length})` : ""}</h2><button className={buttonGhost} aria-label={t("fmDismiss")} onClick={() => closePanel()}><X size={16}/></button></div>
      <div className="fm-panel-body">
      {panel === "more" && <div className="fm-more-actions">
        <button className={buttonGhost} disabled={disabled || windowsDirectorySelection} title={windowsDirectorySelection ? t("fmWindowsDirectories") : undefined} onClick={() => collect("basket")}><PackagePlus size={16}/>{t("fmAddBasket")}</button>
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
        <div className="fm-toolbar"><button className={buttonSecondary} disabled={busy || !state.basket.length || windowsDirectoryBasket} title={windowsDirectoryBasket ? t("fmWindowsDirectories") : undefined} onClick={() => begin(state.basket, "copy", "basket")}>{t("fmCopyBasket")}</button><button className={buttonSecondary} disabled={busy || !state.basket.length || windowsDirectoryBasket} title={windowsDirectoryBasket ? t("fmWindowsDirectories") : undefined} onClick={() => begin(state.basket, "move", "basket")}>{t("fmMoveBasket")}</button><button className={buttonGhost} disabled={busy || !state.basket.length} onClick={() => void run(async () => { const ids = state.basket.map((ticket) => ticket.id); useFileManagementStore.setState({ basket: [] }); await releaseUnused(ids); })}>{t("fmClearBasket")}</button></div>
        <p className="fm-muted">{t("fmSessionOnly")}</p>
        {!state.basket.length && <p className="fm-empty">{t("fmEmptyBasket")}</p>}
        <ul className="fm-basket">{state.basket.slice(effectiveBasketPage * PAGE_SIZE, (effectiveBasketPage + 1) * PAGE_SIZE).map((ticket) => <li key={ticket.id}><File size={16}/><div className="min-w-0"><strong>{ticket.name}</strong><p className="fm-path" title={ticket.path}>{ticket.path}</p>{state.statuses[ticket.id] && <p className="fm-item-status">{state.statuses[ticket.id]}</p>}</div><button className={buttonGhost} disabled={busy} aria-label={`${t("fmRemove")} ${ticket.name}`} onClick={() => void run(async () => { useFileManagementStore.setState((current) => ({ basket: current.basket.filter((item) => item.id !== ticket.id) })); await releaseUnused([ticket.id]); })}><X size={14}/></button></li>)}</ul>
        {pageControls(state.basket.length, effectiveBasketPage, setBasketPage)}
      </>}
      {panel === "results" && <>
        <p className="fm-muted">{resultsSummary}</p>
        <ul className="fm-results">{state.results.slice(resultPage * PAGE_SIZE, (resultPage + 1) * PAGE_SIZE).map((log) => <li key={log.id}><strong>{log.name_after}</strong><span className="fm-item-status">{log.status === "success" ? t("fmSucceeded") : log.status === "skipped" ? t("fmSkipped") : t("fmFailed")}</span><p className="fm-path" title={log.path_after}>{log.path_after}</p>{log.error_message && <p role="alert" className="fm-item-error">{log.error_message}</p>}</li>)}</ul>
        {pageControls(state.results.length, resultPage, setResultPage)}
        <div className="fm-toolbar"><button className={buttonSecondary} onClick={() => setView("restore")}>{t("fmHistory")}</button><button className={buttonGhost} onClick={() => void go(state.results[0]!.path_after.replace(/[/\\][^/\\]+$/, ""))}>{t("fmOpenDestination")}</button><button className={buttonGhost} onClick={() => { useFileManagementStore.setState({ results: [] }); closePanel(); }}>{t("fmDismiss")}</button></div>
        <p className="fm-muted">{t("fmRestoreHelp")}</p>
      </>}
      </div>
    </div>}
    {transfer && !transfer.automatic && <ModalPortal initialFocusRef={transfer.action === "rename" || transfer.action === "create_directory" ? nameInputRef : destinationRef} restoreFocus={() => restoreRef.current?.isConnected ? restoreRef.current : panelTriggerRef.current} onEscape={() => { if (!progress) closeTransfer(); }}>
      <div className="fixed inset-0 grid place-items-center bg-[var(--zc-overlay)] p-4">
        <section className="fm-dialog" role="dialog" aria-modal="true" aria-labelledby={pane ? `fm-transfer-title-${pane.id}` : "fm-transfer-title"} aria-busy={busy}>
          <header className="fm-dialog-header">
            <div className="fm-dialog-title">
              {transfer.action === "copy" ? <Copy size={21}/> : transfer.action === "rename" ? <Pencil size={21}/> : transfer.action === "trash" ? <Trash2 size={21}/> : transfer.action === "create_directory" ? <FolderPlus size={21}/> : <FolderInput size={21}/>}
              <div><h2 id={pane ? `fm-transfer-title-${pane.id}` : "fm-transfer-title"}>{t(transferTitle)}</h2><p className="fm-muted">{previewTickets.length} {t("fmFiles")} · {t("fmNoOverwrite")}</p></div>
            </div>
            {(transfer.action === "copy" || transfer.action === "move") ? <>
              <label className="fm-destination">{t("fmDestination")}<input ref={destinationRef} value={destination} disabled={busy} onChange={(event) => { preparing.current++; setDestination(event.target.value); setPlan(null); }} placeholder={t("fmPathHint")}/></label>
              <div className="fm-toolbar"><button className={buttonSecondary} disabled={busy} onClick={() => void run(chooseDestination)}><FolderOpen size={14}/>{t("fmChooseFolder")}</button><select aria-label={t("fmFavoriteDestination")} value="" disabled={busy} onChange={(event) => { preparing.current++; setDestination(event.target.value); setPlan(null); }}><option value="">{t("fmFavoriteDestination")}</option>{[true, false].map((favorite) => <optgroup key={String(favorite)} label={t(favorite ? "fmFavorites" : "fmRecent")}>{places.filter((place) => place.favorite === favorite).map((place) => <option key={place.id} value={place.routingHint} disabled={place.unavailable}>{place.name}{place.unavailable ? ` · ${t("fmUnavailable")}` : ""}</option>)}</optgroup>)}</select></div>
            </> : <p className="fm-muted">{transfer.action === "trash" ? t("fmSafeTrashNote") : `${t("fmDestination")} · ${destination}`}</p>}
            {(transfer.action === "rename" || transfer.action === "create_directory") && <label className="fm-destination">{t(transfer.action === "rename" ? "fmNewName" : "fmFolderName")}<input ref={nameInputRef} value={newName} disabled={busy} onChange={(event) => { preparing.current++; setNewName(event.target.value); setPlan(null); }} /></label>}
            <div className="fm-toolbar"><button className={buttonSecondary} disabled={busy || !destination.trim() || ((transfer.action === "rename" || transfer.action === "create_directory") && !newName.trim())} onClick={() => void prepare()}>{t("fmPrepare")}</button></div>
            {error && <p role="alert" className="fm-item-error">{error}</p>}
          </header>
          <div className="fm-dialog-body">
            <ul className="fm-preview">{previewTickets.slice(previewPage * PAGE_SIZE, (previewPage + 1) * PAGE_SIZE).map((ticket) => {
              const row = plan?.rows.find((item) => item.ticketId === ticket.id);
              return <li key={ticket.id} data-skipped={skip.includes(ticket.id)}>
                <div className="fm-preview-file"><File size={17}/><div><strong>{ticket.name}</strong><p className="fm-path" title={ticket.path}>{t("fmSource")} {ticket.path.replace(/[/\\][^/\\]+$/, "")}</p>{row?.preview && <p className="fm-path" title={row.preview.target_path}>{t("fmDestinationName")} · {row.preview.new_name}</p>}</div></div>
                {(transfer.action === "copy" || transfer.action === "move") && <select aria-label={`${ticket.name} · ${t("fmResolution")}`} disabled={busy} value={skip.includes(ticket.id) ? "skip" : keepBoth.includes(ticket.id) ? "both" : "original"} onChange={(event) => resolution(ticket.id, event.target.value)}><option value="original">{t("fmDefaultResolution")}</option><option value="skip">{t("fmSkip")}</option><option value="both">{t("fmKeepBothShort")}</option></select>}
                {row?.error && !row.skipped && <p role="alert" className="fm-item-error">{describeError(row.error)}</p>}
                <details className="fm-path-details"><summary>{t("fmDetails")}</summary><p>{ticket.path}</p>{row?.preview && <p>→ {row.preview.target_path}</p>}</details>
              </li>;
            })}</ul>
            {pageControls(previewTickets.length, previewPage, setPreviewPage)}
          </div>
          <footer className="fm-dialog-footer">
            {progress && <div role="status" className="fm-progress"><progress max={progress.total || 1} value={progress.processed}/><span>{cancelling ? t("fmCancelPending") : `${progress.processed}/${progress.total}`} · {progress.currentPath}</span></div>}
            <div className="fm-toolbar fm-confirm-bar"><p role="status" className="fm-transfer-summary">{plan ? `${t("fmReadyCount")} ${readyCount} / ${previewTickets.length} · ${t("fmSkipped")} ${skippedCount}` : t(skip.length || keepBoth.length ? "fmReviewAgain" : "fmNeedsPreview")}</p><button className={buttonSecondary} disabled={cancelling} onClick={cancelTask}>{t(progress ? "fmCancelRunning" : "fmCancel")}</button><button className={buttonPrimary} disabled={busy || !rowsReady} onClick={() => void execute()}>{t("fmConfirm")}</button></div>
          </footer>
        </section>
      </div>
    </ModalPortal>}
  </section>;
}
