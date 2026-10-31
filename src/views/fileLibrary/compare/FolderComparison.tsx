import { LocateFixed } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useChromeContext, useI18nContext } from "../../../contexts/AppContexts";
import { useFileManagementStore } from "../management/fileManagementStore";
import type { BrowseSourceOwner } from "../browse/browseSourceOwner";
import { compareFolders, type Difference } from "./compareFolders";
import "./folderComparison.css";

const LIMIT = 10_000;
const PAGE = 40;
const statuses: Difference[] = ["left", "right", "different", "same", "directory", "unknown"];
function hasDirectorySelection(source: BrowseSourceOwner | undefined) {
  return source?.entries.some((entry) => source.selectedIds.has(entry.entryRef.entryId) && entry.entryKind === "directory") ?? false;
}
export function FolderComparison({ paneIds, activate, compact = false }: { paneIds: string[]; activate: (id: string) => void; compact?: boolean }) {
  const { t } = useI18nContext();
  const { platform } = useChromeContext();
  const surfaces = useFileManagementStore((s) => s.surfaces);
  const locked = useFileManagementStore((s) => s.transferOpen || s.transferActive);
  const sources = paneIds.map((id) => surfaces[id]?.read().browseSource);
  const [open, setOpen] = useState(false);
  const [running, setRunning] = useState(false);
  const [filter, setFilter] = useState<Difference | "all">("all");
  const [modified, setModified] = useState(true);
  const [page, setPage] = useState(0);
  const [batchExpanded, setBatchExpanded] = useState(false);
  useEffect(() => setBatchExpanded(false), [compact]);
  const inFlight = useRef(false);
  const [left, right] = sources;
  const ready = !!left?.browse && !!right?.browse && !left.showLocationPicker && !right.showLocationPicker;
  const filteredSource = sources.some((s) => s?.query.text || s?.query.entryKind !== "all");
  const overflow = sources.some((s) => (s?.entries.length ?? 0) > LIMIT);
  const failed = sources.some((s) => s?.enumerationState === "failed");
  const pending = sources.some((s) => !!s?.pendingChange || s?.changeState === "refreshing");
  const complete = ready && sources.every((s) => s?.enumerationState === "complete");
  const incomplete = sources.some((s) => (s?.skippedEntries ?? 0) > 0);
  // Source owners retain all opaque refs and enforce generation/cursor checks. One bounded page per turn.
  useEffect(() => {
    if (!open || !running || locked || inFlight.current || !ready || filteredSource || overflow || failed || pending || complete) return;
    const next = sources.find((s) => s?.enumerationState === "partial");
    if (!next) return;
    const timer = window.setTimeout(() => {
      inFlight.current = true;
      void next.loadNextPage().finally(() => { inFlight.current = false; });
    }, 0);
    return () => clearTimeout(timer);
  }, [open, running, locked, ready, filteredSource, overflow, failed, pending, complete, surfaces]);
  const usable = open && running && complete && !filteredSource && !overflow && !failed && !pending;
  const rows = useMemo(() => usable ? compareFolders(left!.entries, right!.entries, modified) : [], [usable, left?.entries, right?.entries, modified]);
  const counts = useMemo(() => {
    const counts = { left: 0, right: 0, different: 0, same: 0, directory: 0, unknown: 0 };
    rows.forEach((row) => counts[row.status]++); return counts;
  }, [rows]);
  const visible = filter === "all" ? rows : rows.filter((r) => r.status === filter);
  const maxPage = Math.max(0, Math.ceil(visible.length / PAGE) - 1);
  const currentPage = Math.min(page, maxPage);
  function select(side: number, ids: string[]) {
    activate(paneIds[side]); sources[side]?.selectIds(ids);
  }
  function action(side: number, kind: "basket" | "copy_other" | "move_other") {
    activate(paneIds[side]); useFileManagementStore.getState().requestAction(kind);
  }
  return <section className="u3-comparison" aria-label={t("u3Compare")}>
    <div className="u3-compare-controls">
      <button className="wb-cross-button" aria-expanded={open} disabled={locked}
        onClick={() => { setOpen(!open); if (!open) setRunning(true); }}>{t(open ? "u3CloseCompare" : "u3Compare")}</button>
      {open && <>
        <button className="wb-cross-button" disabled={locked || !ready} onClick={() => {
          setRunning(true); setPage(0); void Promise.all(sources.map((s) => s?.refreshEnumeration()));
        }}>{t("browseRefresh")}</button>
        <button className="wb-cross-button" disabled={locked} onClick={() => setRunning(!running)}>{t(running ? "u3Pause" : "u3Resume")}</button>
        <label><input type="checkbox" checked={modified} disabled={locked} onChange={(e) => { setModified(e.target.checked); setPage(0); }}/>{t("u3CompareModified")}</label>
      </>}
    </div>
    {open && <>
      <p className="u3-compare-help">{t("u3MetadataOnly")}</p>
      <details className="u3-compare-help"><summary>{t("u3Conditions")}</summary>{t("u3CompareHelp")}</details>
      <div role="status" aria-live="polite">{!ready ? t("u3NeedFolders") : filteredSource ? t("u3ClearFilters") : overflow ? t("u3CompareLimit") : failed ? t("u3CompareFailed") : pending ? t("u3Changed") : !running ? t("u3Paused") : !complete ? `${t("u3Comparing")} ${sources.map((s) => s?.loadedCount ?? 0).join(" / ")}` : `${t("u3Compared")} ${rows.length}`}</div>
      {incomplete && <p role="alert">{t("u3Incomplete")} {t("u3CompareCoverage")}</p>}
      {usable && <>
        <div className="u3-compare-filters" role="group" aria-label={t("u3Filter")}>
          <button aria-pressed={filter === "all"} onClick={() => { setFilter("all"); setPage(0); }}>{t("u3All")} {rows.length}</button>
          {statuses.map((status) => <button key={status} aria-pressed={filter === status} onClick={() => { setFilter(status); setPage(0); }}>
            {t(`u3Status_${status}`)} {counts[status]}
          </button>)}
        </div>
        <details className="u3-batch-details" open={batchExpanded} onToggle={(event) => setBatchExpanded(event.currentTarget.open)}>
        <summary>{t("u3BatchActions")} · {t("wbLeftPane")} {left?.selectedCount ?? 0} / {t("wbRightPane")} {right?.selectedCount ?? 0}</summary>
        <div className="u3-batch-actions">{sources.map((source, side) => <div key={paneIds[side]}>
          <strong>{t(side ? "wbRightPane" : "wbLeftPane")} · {source?.selectedCount ?? 0}</strong>
          <button disabled={locked} onClick={() => select(side, visible.flatMap((r) => {
            const entry = side ? r.right : r.left; return entry ? [entry.entryRef.entryId] : [];
          }))}>{t("u3SelectFiltered")}</button>
          <button disabled={locked} onClick={() => select(side, [])}>{t("libraryClearSelection")}</button>
          <button disabled={locked || !source?.selectedCount || source.selectedCount > 1000} onClick={() => action(side, "basket")}>{t("fmAddBasket")}</button>
          <button disabled={locked || !source?.selectedCount || source.selectedCount > 1000} onClick={() => action(side, "copy_other")}>{t("wbCopyOther")}</button>
          <button disabled={locked || !source?.selectedCount || source.selectedCount > 1000 || platform === "win32" && hasDirectorySelection(source)} title={platform === "win32" && hasDirectorySelection(source) ? t("fmDirectoryMoveUnsupported") : undefined} onClick={() => action(side, "move_other")}>{t("wbMoveOther")}</button>
        </div>)}</div>
        <details className="u3-compare-help"><summary>{t("u3BatchLimits")}</summary>{t("u3BatchHelp")}</details>
        </details>
        <div className="u3-compare-table" role="table" aria-label={t("u3CompareResults")}>
          <div role="row" className="u3-compare-row"><span role="columnheader">{t("wbLeftPane")}</span><span role="columnheader">{t("u3Filter")}</span><span role="columnheader">{t("wbRightPane")}</span></div>
          {visible.slice(currentPage * PAGE, (currentPage + 1) * PAGE).map((row) => <div role="row" className="u3-compare-row" key={row.key}>
            {[0, 1, 2].map((cell) => cell === 1 ? <span role="cell" key={cell} className={`u3-status-${row.status}`}>{t(`u3Status_${row.status}`)}</span> : (() => {
              const side = cell === 0 ? 0 : 1; const entry = side ? row.right : row.left;
              return <span role="cell" key={cell}>{entry ? <><label title={`${entry.displayName} · ${entry.size ?? "—"} B · ${entry.modifiedAt === undefined ? "—" : new Date(entry.modifiedAt * 1000).toLocaleString()}`}>
                <input type="checkbox" disabled={locked} aria-label={`${t(side ? "wbRightPane" : "wbLeftPane")}: ${entry.displayName}`} checked={sources[side]!.selectedIds.has(entry.entryRef.entryId)} onChange={(event) => {
                  const ids = new Set(sources[side]!.selectedIds); if (event.target.checked) ids.add(entry.entryRef.entryId); else ids.delete(entry.entryRef.entryId); select(side, [...ids]);
                }}/><span>{entry.entryKind === "directory" ? "▸ " : ""}{entry.displayName}</span>
              </label><button className="u3-locate" disabled={locked} title={t("u3Locate")} aria-label={`${t("u3Locate")}: ${t(side ? "wbRightPane" : "wbLeftPane")} ${entry.displayName}`}
                onClick={() => {
                  activate(paneIds[side]); sources[side]?.setFocusedId(entry.entryRef.entryId);
                  requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-pane-id="${paneIds[side]}"] [data-shared-file-list-source], [data-pane-id="${paneIds[side]}"] [data-shared-file-grid-source]`)?.focus({ preventScroll: true }));
                }}><LocateFixed size={13}/></button></> : "—"}</span>;
            })())}
          </div>)}
        </div>
        <div className="u3-compare-controls u3-compare-pagination" role="navigation" aria-label={t("u3CompareResults")}><button disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>{t("fileLibraryBack")}</button><span>{currentPage + 1} / {maxPage + 1}</span><button disabled={currentPage >= maxPage} onClick={() => setPage(currentPage + 1)}>{t("fileLibraryForward")}</button></div>
      </>}
    </>}
  </section>;
}
