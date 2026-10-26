import { useCallback, useEffect, useRef, useState } from "react";
import { Copy, Scissors, ClipboardPaste, FolderInput, PackagePlus, Star, X, FolderOpen } from "lucide-react";
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
export function FileManagementToolbar() {
  const { t } = useI18nContext(); const { platform } = useChromeContext(); const { setView } = useNavigationContext();
  const { controller } = useFileLibraryExperience(); const { controller: previewController } = usePreviewExperience();
  const state = useFileManagementStore(); const surface = state.surface?.();
  const [previewPage, setPreviewPage] = useState(0); const [basketPage, setBasketPage] = useState(0); const [resultPage, setResultPage] = useState(0);
  const [busy, setBusy] = useState(false); const [error, setError] = useState(""); const [notice, setNotice] = useState("");
  const [places, setPlaces] = useState<SavedPlace[]>([]); const [currentPath, setCurrentPath] = useState("");
  const [locationInput, setLocationInput] = useState(""); const [placesOpen, setPlacesOpen] = useState(false);
  const [transfer, setTransfer] = useState<{ tickets: TransferTicket[]; action: TransferAction; origin: "selection" | "clipboard" | "basket" } | null>(null);
  const [destination, setDestination] = useState(""); const [plan, setPlan] = useState<TransferPreview | null>(null);
  const [skip, setSkip] = useState<string[]>([]); const [keepBoth, setKeepBoth] = useState<string[]>([]);
  const [progress, setProgress] = useState<OperationProgressPayload | null>(null);
  const destinationRef = useRef<HTMLInputElement | null>(null); const restoreRef = useRef<HTMLElement | null>(null);
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
    const current = useFileManagementStore.getState().surface?.();
    if (!current || (current.count === 0 && current.selection?.kind !== "all_matching")) return [];
    const captured = await api.captureTransfer(current.selection ? { selection: current.selection } : { refs: current.refs ?? [] });
    if (useFileManagementStore.getState().surface?.().signature !== current.signature) { await releaseUnused(captured.map((ticket) => ticket.id)); throw Error("transfer_selection_changed"); }
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
      const old = useFileManagementStore.getState();
      if (action === "basket") {
        const merged = new Map(old.basket.map((ticket) => [ticket.id, ticket])); tickets.forEach((ticket) => merged.set(ticket.id, ticket));
        useFileManagementStore.setState({ basket: [...merged.values()], basketOpen: true });
      } else {
        useFileManagementStore.setState({ clipboard: { action: action === "cut" ? "move" : "copy", tickets } });
        await releaseUnused(old.clipboard?.tickets.map((ticket) => ticket.id) ?? []);
      }
      setNotice(t(action === "basket" ? "fmAdded" : "fmClipboardReady"));
    });
  }
  function begin(tickets: TransferTicket[], action: TransferAction, origin: "selection" | "clipboard" | "basket") {
    if (!tickets.length) return;
    restoreRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    useFileManagementStore.setState({ results: [] }); setPreviewPage(0); setResultPage(0);
    setTransfer({ tickets, action, origin }); setDestination(currentPath); setPlan(null); setSkip([]); setKeepBoth([]); setError("");
  }
  function closeTransfer() {
    preparing.current++; setTransfer(null); setPlan(null);
    if (transfer?.origin === "selection") void releaseUnused(transfer.tickets.map((ticket) => ticket.id)).catch((value) => { if (mounted.current) setError(describeError(value)); });
  }
  function moveSelection() { void run(async () => { begin(await capturedSelection(), "move", "selection"); }); }
  function paste() { const clipboard = useFileManagementStore.getState().clipboard; if (clipboard) begin(clipboard.tickets, clipboard.action, "clipboard"); }
  const actionsRef = useRef({ collect, paste, moveSelection }); actionsRef.current = { collect, paste, moveSelection };
  useEffect(() => {
    const handle = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.altKey || !(event.ctrlKey || event.metaKey) || isFileLibraryShortcutExcludedTarget(event.target) || busyRef.current) return;
      const key = event.key.toLowerCase(); if (!["c", "x", "v"].includes(key)) return;
      if (key !== "v" && (!useFileManagementStore.getState().surface || useFileManagementStore.getState().surface?.()?.count === 0)) return;
      if (key === "v" && !useFileManagementStore.getState().clipboard?.tickets.length) return;
      event.preventDefault(); if (key === "v") actionsRef.current.paste(); else actionsRef.current.collect(key === "x" ? "cut" : "copy");
    };
    window.addEventListener("keydown", handle); return () => window.removeEventListener("keydown", handle);
  }, []);
  useEffect(() => {
    const action = state.pendingAction; if (!action) return;
    useFileManagementStore.setState({ pendingAction: null });
    if (action === "paste") paste(); else if (action === "move") moveSelection(); else collect(action);
  }, [state.pendingAction]);
  async function chooseDestination() {
    const result = await open({ directory: true, multiple: false, title: t("fmDestination") });
    const path = Array.isArray(result) ? result[0] : result;
    if (path) { setDestination(path); setPlan(null); }
  }
  async function prepare(nextSkip = skip, nextBoth = keepBoth) {
    if (!transfer) return;
    const epoch = ++preparing.current; setPlan(null);
    await run(async () => {
      const next = await api.previewTransfer({ ticketIds: transfer.tickets.map((ticket) => ticket.id), action: transfer.action, destinationDirectory: destination, skip: nextSkip, keepBoth: nextBoth });
      if (mounted.current && preparing.current === epoch) {
        setPlan(next);
        const firstProblem = next.rows.find((row) => row.error && !row.skipped);
        if (firstProblem) setPreviewPage(Math.floor(Math.max(0, transfer.tickets.findIndex((ticket) => ticket.id === firstProblem.ticketId)) / PAGE_SIZE));
        const statuses = { ...useFileManagementStore.getState().statuses };
        next.rows.forEach((row) => { if (row.error) statuses[row.ticketId] = describeError(row.error); });
        useFileManagementStore.setState({ statuses });
      }
    });
  }
  async function execute() {
    if (!plan || !transfer) return;
    const acceptedPlan = plan; const acceptedTransfer = transfer;
    await run(async () => {
      cancelRequested.current = false; executionSent.current = false;
      setProgress({ kind: "execute", batchId: "", processed: 0, total: acceptedPlan.rows.filter((row) => !row.skipped).length, currentPath: acceptedPlan.rows.find((row) => !row.skipped && row.preview)?.preview?.source_path ?? "" });
      let dispose: (() => void) | undefined;
      try {
        dispose = await tauriApi.onOperationProgress((update) => { if (mounted.current && update.kind === "execute") setProgress(update); });
        await previewController.closeForFileOperation();
        if (cancelRequested.current) { setTransfer(null); return; }
        executionSent.current = true;
        let result;
        try { result = await api.executeTransfer({ previewId: acceptedPlan.id, confirmed: true }); }
        catch (value) { setError(`${t("fmExecutionUncertain")} ${readableError(value)}`); return; }
        const rows = acceptedPlan.rows.filter((row) => !row.skipped && row.preview);
        const succeeded = new Set<string>(); const statuses = { ...useFileManagementStore.getState().statuses };
        rows.forEach((row, index) => { const log = result.logs[index]; statuses[row.ticketId] = log?.status === "success" ? t("fmSucceeded") : log?.status === "skipped" ? t("fmSkipped") : log?.error_message || t("fmFailed"); if (log?.status === "success") succeeded.add(row.ticketId); });
        const current = useFileManagementStore.getState();
        useFileManagementStore.setState({ results: result.logs, statuses, basketOpen: false,
          ...(acceptedTransfer.action === "move" ? { basket: current.basket.filter((ticket) => !succeeded.has(ticket.id)), clipboard: current.clipboard ? { ...current.clipboard, tickets: current.clipboard.tickets.filter((ticket) => !succeeded.has(ticket.id)) } : null } : {}) });
        setTransfer(null); setPlan(null); setNotice(t("fmFinished"));
        previewController.close("escape");
        const currentSurface = useFileManagementStore.getState().surface?.(); currentSurface?.clear();
        try { await currentSurface?.refresh(); await useOperationQueueStore.getState().refreshOperationLogs(); } catch (value) { setError(`${t("fmRefreshFailed")} ${readableError(value)}`); }
        if (acceptedTransfer.action === "move") {
          try { await releaseUnused(acceptedTransfer.origin === "selection" ? acceptedTransfer.tickets.map((ticket) => ticket.id) : [...succeeded]); } catch (value) { setError(`${t("fmRefreshFailed")} ${readableError(value)}`); }
        }
      } finally { dispose?.(); executionSent.current = false; setProgress(null); setPlan(null); }
    });
  }
  async function go(path: string) {
    await run(async () => {
      if (platform !== "win32" && platform !== "darwin") throw Error("workspace_platform_not_supported");
      previewController.close("escape");
      const admitted = await controller.openBrowse({ platform: platform === "win32" ? "windows" : "macos", routingHint: path, displayHint: path });
      if (!admitted) throw Error("browse_admission_superseded"); setPlacesOpen(false);
    });
  }
  const effectiveBasketPage = Math.min(basketPage, Math.max(0, Math.ceil(state.basket.length / PAGE_SIZE) - 1));
  const pageControls = (count: number, page: number, change: (page: number) => void) => <PageControls count={count} page={page} change={change} previous={t("fmPrevious")} next={t("fmNext")}/>;
  const selected = surface?.count === null ? t("fmAllResults") : String(surface?.count ?? 0);
  const disabled = busy || !surface || surface.count === 0;
  const rowsReady = plan && plan.rows.some((row) => !row.skipped && row.preview) && plan.rows.every((row) => row.skipped || !row.error);
  return <section className="fm-workflow" aria-label={t("fmActions")} data-file-management>
    <div className="fm-toolbar">
      <span className="text-xs text-[var(--zc-text-secondary)]" aria-live="polite">{t("fmSelection")} {selected}</span>
      <button className={buttonGhost} disabled={busy || !surface?.selectAll} onClick={() => surface?.selectAll?.()}>{t(surface?.selectionScope === "loaded" ? "fmSelectLoaded" : "fmSelectAll")}</button>
      <button className={buttonGhost} disabled={disabled} onClick={() => collect("copy")}><Copy size={14}/>{t("fmCopy")}</button>
      <button className={buttonGhost} disabled={disabled} onClick={() => collect("cut")}><Scissors size={14}/>{t("fmCut")}</button>
      <button className={buttonGhost} disabled={busy || !state.clipboard?.tickets.length} onClick={paste}><ClipboardPaste size={14}/>{t("fmPaste")}{state.clipboard?.tickets.length ? ` (${state.clipboard.tickets.length})` : ""}</button>
      <button className={buttonGhost} disabled={disabled} onClick={moveSelection}><FolderInput size={14}/>{t("fmMove")}</button>
      <button className={buttonGhost} disabled={disabled} onClick={() => collect("basket")}><PackagePlus size={14}/>{t("fmAddBasket")}</button>
      <button className={buttonGhost} aria-expanded={state.basketOpen} onClick={() => useFileManagementStore.setState({ basketOpen: !state.basketOpen })}>{t("fmBasket")} ({state.basket.length})</button>
      <button className={buttonGhost} aria-expanded={placesOpen} onClick={() => setPlacesOpen(!placesOpen)}><Star size={14}/>{t("fmPlaces")}</button>
    </div>
    {(error || notice) && <div className="fm-notice" role={error ? "alert" : "status"}>{error || notice}<button className={buttonGhost} aria-label={t("fmDismiss")} onClick={() => { setError(""); setNotice(""); }}><X size={14}/></button></div>}
    {placesOpen && <div className="fm-panel" aria-label={t("fmPlaces")}>
      <form className="fm-toolbar" onSubmit={(event) => { event.preventDefault(); void go(locationInput); }}><label className="text-sm">{t("fmGoFolder")}<input value={locationInput} onChange={(event) => setLocationInput(event.target.value)} placeholder={t("fmPathHint")} /></label><button className={buttonSecondary} disabled={busy || !locationInput.trim()}>{t("fmGo")}</button></form>
      <button className={buttonSecondary} disabled={busy || !surface?.browse} onClick={() => void run(async () => { setPlaces(await api.fileLibraryPlaces({ ...surface!.browse!, favorite: true })); })}>{t("fmFavoriteCurrent")}</button>
      <p className="text-xs text-[var(--zc-text-secondary)]">{t("fmRecentDesc")}</p>
      <ul className="fm-places">{places.map((place) => <li key={place.id}><button className={buttonGhost} disabled={busy || place.unavailable} onClick={() => void go(place.routingHint)}><Star size={12} fill={place.favorite ? "currentColor" : "none"}/><span title={place.routingHint}>{place.name}{place.unavailable ? ` · ${t("fmUnavailable")}` : ""}</span></button><button className={buttonGhost} onClick={() => void run(async () => setPlaces(await api.fileLibraryPlaces({ removeId: place.id })))}>{t(place.favorite ? "fmUnfavorite" : "fmRemove")}</button></li>)}</ul>
    </div>}
    {state.basketOpen && <div className="fm-panel" aria-label={t("fmBasket")}>
      <p className="text-xs text-[var(--zc-text-secondary)]">{t("fmSessionOnly")}</p>
      <div className="fm-toolbar"><button className={buttonSecondary} disabled={busy || !state.basket.length} onClick={() => begin(state.basket, "copy", "basket")}>{t("fmCopyBasket")}</button><button className={buttonSecondary} disabled={busy || !state.basket.length} onClick={() => begin(state.basket, "move", "basket")}>{t("fmMoveBasket")}</button><button className={buttonGhost} disabled={busy || !state.basket.length} onClick={() => void run(async () => { const ids = state.basket.map((ticket) => ticket.id); useFileManagementStore.setState({ basket: [] }); await releaseUnused(ids); })}>{t("fmClearBasket")}</button></div>
      <ul className="fm-basket">{state.basket.slice(effectiveBasketPage * PAGE_SIZE, (effectiveBasketPage + 1) * PAGE_SIZE).map((ticket) => <li key={ticket.id}><div className="min-w-0"><strong>{ticket.name}</strong><p className="truncate text-xs text-[var(--zc-text-secondary)]" title={ticket.path}>{ticket.path}</p>{state.statuses[ticket.id] && <p className="text-xs">{state.statuses[ticket.id]}</p>}</div><button className={buttonGhost} disabled={busy} onClick={() => void run(async () => { useFileManagementStore.setState((current) => ({ basket: current.basket.filter((item) => item.id !== ticket.id) })); await releaseUnused([ticket.id]); })}>{t("fmRemove")}</button></li>)}</ul>
      {pageControls(state.basket.length, effectiveBasketPage, setBasketPage)}
    </div>}
    {!!state.results.length && <details className="fm-panel" open={state.results.some((log) => log.status !== "success")}><summary>{t("fmResults")} · {state.results.filter((log) => log.status === "success").length}/{state.results.length}</summary><ul className="fm-results">{state.results.slice(resultPage * PAGE_SIZE, (resultPage + 1) * PAGE_SIZE).map((log) => <li key={log.id}><strong>{log.name_after}</strong> · {log.status === "success" ? t("fmSucceeded") : log.status === "skipped" ? t("fmSkipped") : t("fmFailed")}<p className="break-all text-xs">{log.path_after}</p>{log.error_message && <p role="alert" className="text-sm">{log.error_message}</p>}</li>)}</ul>{pageControls(state.results.length, resultPage, setResultPage)}<div className="fm-toolbar"><button className={buttonSecondary} onClick={() => setView("restore")}>{t("fmHistory")}</button><button className={buttonGhost} onClick={() => void go(state.results[0]!.path_after.replace(/[/\\][^/\\]+$/, ""))}>{t("fmOpenDestination")}</button><button className={buttonGhost} onClick={() => useFileManagementStore.setState({ results: [] })}>{t("fmDismiss")}</button></div><p className="text-xs text-[var(--zc-text-secondary)]">{t("fmRestoreHelp")}</p></details>}
    {transfer && <ModalPortal initialFocusRef={destinationRef} restoreFocus={() => restoreRef.current} onEscape={() => { if (!progress) closeTransfer(); }}><div className="fixed inset-0 grid place-items-center bg-[var(--zc-overlay)] p-4"><section className="fm-dialog" role="dialog" aria-modal="true" aria-labelledby="fm-transfer-title" aria-busy={busy}>
      <h2 id="fm-transfer-title" className="text-lg font-semibold">{t(transfer.action === "copy" ? "fmCopyPreview" : "fmMovePreview")} ({transfer.tickets.length})</h2>
      <p className="text-sm text-[var(--zc-text-secondary)]">{t("fmPreviewHelp")}</p>
      <label className="grid gap-1 text-sm">{t("fmDestination")}<input ref={destinationRef} value={destination} disabled={busy} onChange={(event) => { preparing.current++; setDestination(event.target.value); setPlan(null); }} placeholder={t("fmPathHint")}/></label>
      <div className="fm-toolbar"><button className={buttonSecondary} disabled={busy} onClick={() => void run(chooseDestination)}><FolderOpen size={14}/>{t("fmChooseFolder")}</button><select aria-label={t("fmFavoriteDestination")} value="" disabled={busy} onChange={(event) => { preparing.current++; setDestination(event.target.value); setPlan(null); }}><option value="">{t("fmFavoriteDestination")}</option>{places.filter((place) => place.favorite).map((place) => <option key={place.id} value={place.routingHint} disabled={place.unavailable}>{place.name}{place.unavailable ? ` · ${t("fmUnavailable")}` : ""}</option>)}</select><button className={buttonSecondary} disabled={busy || !destination.trim()} onClick={() => void prepare()}>{t("fmPrepare")}</button></div>
      {error && <p role="alert" className="text-sm">{error}</p>}
      <ul className="fm-preview">{transfer.tickets.slice(previewPage * PAGE_SIZE, (previewPage + 1) * PAGE_SIZE).map((ticket) => { const row = plan?.rows.find((item) => item.ticketId === ticket.id); return <li key={ticket.id}><strong>{ticket.name}</strong><p className="break-all text-xs text-[var(--zc-text-secondary)]">{ticket.path}</p>{row?.preview && <p className="break-all text-xs">→ {row.preview.target_path}</p>}{row?.error && !row.skipped && <p role="alert" className="text-sm">{describeError(row.error)}</p>}<div className="fm-toolbar"><label className="text-xs"><input type="checkbox" disabled={busy} checked={skip.includes(ticket.id)} onChange={(event) => { const next = event.target.checked ? [...skip, ticket.id] : skip.filter((id) => id !== ticket.id); setSkip(next); setPlan(null); }}/>{t("fmSkip")}</label><label className="text-xs"><input type="checkbox" disabled={busy} checked={keepBoth.includes(ticket.id)} onChange={(event) => { const next = event.target.checked ? [...keepBoth, ticket.id] : keepBoth.filter((id) => id !== ticket.id); setKeepBoth(next); setPlan(null); }}/>{t("fmKeepBoth")}</label></div></li>; })}</ul>
      {pageControls(transfer.tickets.length, previewPage, setPreviewPage)}
      {progress && <div role="status" className="grid gap-1 text-sm"><progress max={progress.total || 1} value={progress.processed}/><span>{progress.processed}/{progress.total} · {progress.currentPath}</span></div>}
      <div className="fm-toolbar justify-end"><button className={buttonSecondary} onClick={() => { if (progress) { cancelRequested.current = true; if (executionSent.current) void tauriApi.cancelOperations().catch((value) => setError(describeError(value))); } else closeTransfer(); }}>{t(progress ? "fmCancelRunning" : "fmCancel")}</button><button className={cn(buttonPrimary)} disabled={busy || !rowsReady} onClick={() => void execute()}>{t("fmConfirm")}</button></div>
    </section></div></ModalPortal>}
  </section>;
}
