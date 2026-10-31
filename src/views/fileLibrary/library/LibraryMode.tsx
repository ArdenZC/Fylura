import { isFileLibraryFocusTarget } from "../fileLibraryInteraction";
import { usePaneQuerySelector, usePane } from "../workbench/PaneContext";
import { Bookmark, ChevronDown, FolderSearch, Layers, SlidersHorizontal, Tag } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { tauriApi } from "../../../api/tauriApi";
import { useChromeContext, useI18nContext, useNavigationContext, useRuntimeCapabilitiesContext } from "../../../contexts/AppContexts";
import { cloneFileQuerySpec, explicitSingleSelectionId } from "../../../store/useFileLibraryV2Store";
import type { FileLibrarySummary, FileLibraryDetail } from "../../../types/domain";
import { libraryScopeLabel, readableError } from "../../../utils/viewHelpers";
import { buttonGhost, buttonSecondary, buttonSubtle, cn, buttonPrimary, raisedSurface } from "../../../utils/tw";
import { NoticeBanner, StateBlock, pageFrame } from "../../shared/ui";
import { FileLibraryFilterPopover } from "../../vault/components/FileLibraryFilterPopover";
import { ContentUnderstandingSheet } from "../../vault/components/ContentUnderstandingSheet";
import { LibraryMetadataManagerDialog } from "../../vault/components/LibraryMetadataManagerDialog";
import { ManualRenameDialog } from "./ManualRenameDialog";
import { BatchRenameAction } from "./BatchRenameAction";
import { useBatchRenameAction } from "./useBatchRenameAction";
import { LibraryContextMenu, libraryTransferMenuItems } from "./LibraryContextMenu";
import { useLibrarySourceOwner } from "./librarySourceOwner";
import { useLibraryContextMenu } from "./useLibraryContextMenu";
import { useLibraryContentCompatibility } from "./useLibraryContentCompatibility";
import { createLibraryInteractionProjection } from "../list/interactionAdapters";
import { SharedFileList } from "../list/SharedFileList";
import { SharedFileGrid } from "../list/SharedFileGrid";
import type { LibraryPresentationEntry, PresentationEntry } from "../presentation/contracts";
import { useFileLibraryExperience } from "../FileLibraryExperienceProvider";
import { PersonalFiles } from "../context/FilePersonalMetadata";
import { ContextPanel } from "../context/ContextPanel";
import { createLibraryContextProjection } from "../context/contextPanelProjection";
import { usePreviewExperience } from "../preview/PreviewExperienceProvider";
import { SELECTED_PREVIEW_LIMIT, useSelectedPreview } from "../preview/useSelectedPreview";
import { previewSourceFromEntry } from "../preview/previewSource";
import { createPreviewSiblingNavigation } from "../preview/previewSiblingNavigation";
import { adaptLibrarySummary } from "../presentation/adapters";
import {
  useApplyLibraryNavigationTarget,
  useRegisterLibraryNavigationSurface
} from "./libraryNavigationSurface";
import { useFileLibraryLibrarySearchSurface } from "../fileLibraryCommandBarSurface";
import { useFileManagementStore } from "../management/fileManagementStore";
import { useLibraryManagementSurface } from "../management/useManagementSurfaces";
import { countActiveFilters, scopeHealthLabel, replaceCopy } from "./libraryModeCopy";
import "./libraryMode.css";

/**
 * The concrete Library source projection for the W2 workspace. It composes
 * existing Query V2, LibrarySelectionV1, Inspector, operation and metadata
 * authorities through the source owner; it does not create a second store.
 */
export function LibraryMode() {
  const query = usePaneQuerySelector();
  const pane = usePane();
  const { controller, state: experienceState } = useFileLibraryExperience();
  const { t, language } = useI18nContext();
  const { platform } = useChromeContext();
  const { onError, setView } = useNavigationContext();
  const { capabilities } = useRuntimeCapabilitiesContext();
  const { controller: previewController, state: previewState } = usePreviewExperience();
  const handleQueryError = useCallback((error: unknown) => onError(readableError(error)), [onError]);
  const source = useLibrarySourceOwner({ onError: handleQueryError });
  const clipboardIncludesDirectory = useFileManagementStore((store) => store.clipboard?.tickets.some((ticket) => ticket.isDirectory) ?? false);
  useLibraryManagementSurface(source);

  const currentTarget = experienceState.workspace.session.currentTarget;
  useRegisterLibraryNavigationSurface({
    controller,
    currentTarget,
    source,
    t,
    locations: experienceState.workspace.locations
  });
  useApplyLibraryNavigationTarget({ currentTarget, source });
  const [personalOpen, setPersonalOpen] = useState(false);
  const personalTrigger = useRef<HTMLButtonElement | null>(null);
  const [renameFile, setRenameFile] = useState<FileLibrarySummary | null>(null);
  const canonicalSingleSelectionId = explicitSingleSelectionId(source.selection);
  const [isFilterOpen, setIsFilterOpen] = useState(false);
  const [isSortOpen, setIsSortOpen] = useState(false);
  const [metadataManager, setMetadataManager] = useState<"tags" | "saved_views" | null>(null);
  const filterButtonRef = useRef<HTMLButtonElement | null>(null);
  const sortButtonRef = useRef<HTMLButtonElement | null>(null);

  const remainingCount = source.totalCount === null ? 0 : Math.max(0, source.totalCount - source.files.length);
  const activeFilterCount = countActiveFilters(source.querySpec.filters);
  const scopeText = libraryScopeLabel(source.scope, t("allIndexedFiles"), t("noFolderSelected"));
  const isNoIndexState = !source.stats.lastScannedAt
    && source.scope.kind === "all"
    && source.totalCount === 0
    && !source.isLoading
    && !source.error;
  const showLibraryControls = !isNoIndexState;
  const sortOptions = useMemo(() => [
    { key: "modified" as const, label: t("librarySortModified") },
    { key: "created" as const, label: t("librarySortCreated") },
    { key: "name" as const, label: t("librarySortName") },
    { key: "size" as const, label: t("librarySortSize") },
    { key: "confidence" as const, label: t("librarySortConfidence") },
    { key: "relevance" as const, label: t("librarySortRelevance") }
  ], [t]);
  const currentSortLabel = sortOptions.find((option) => option.key === source.querySpec.sort.kind)?.label ?? t("librarySortModified");
  const { source: batchRenameSource, onPreview: onBatchRenamePreview } = useBatchRenameAction(source, setView);
  const commandBarActions = useMemo(() => showLibraryControls ? <>
    <div className="relative"><button ref={filterButtonRef} className={cn(buttonSubtle, "min-h-9 px-3 py-1.5 text-xs")} aria-expanded={isFilterOpen} aria-controls={pane ? `library-filter-popover-${pane.id}` : "library-filter-popover"} aria-haspopup="dialog" onClick={() => { setIsFilterOpen((value) => !value); setIsSortOpen(false); }}><SlidersHorizontal size={15} />{t("libraryFilterButton")}{activeFilterCount ? <span className="tabular-nums text-[var(--zc-primary)]">{activeFilterCount}</span> : null}</button>{isFilterOpen ? <div id={pane ? `library-filter-popover-${pane.id}` : "library-filter-popover"}><FileLibraryFilterPopover filters={source.querySpec.filters} tags={source.tags} t={t} onFiltersChange={source.updateFilters} onClear={source.clearFilters} onClose={() => { setIsFilterOpen(false); requestAnimationFrame(() => filterButtonRef.current?.focus()); }} /></div> : null}</div>
    <div className="relative"><button ref={sortButtonRef} className={cn(buttonSubtle, "min-h-9 px-3 py-1.5 text-xs")} aria-expanded={isSortOpen} aria-controls={pane ? `library-sort-menu-${pane.id}` : "library-sort-menu"} aria-haspopup="menu" onClick={() => { setIsSortOpen((value) => !value); setIsFilterOpen(false); }}><span>{currentSortLabel}</span><ChevronDown size={14} /></button>{isSortOpen ? <div id={pane ? `library-sort-menu-${pane.id}` : "library-sort-menu"} className="absolute right-0 top-[calc(100%+8px)] z-30 grid min-w-48 rounded-[var(--zc-radius-floating)] border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] p-2 shadow-[var(--zc-shadow-menu)]" role="menu" aria-label={t("librarySortMenuLabel")} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); setIsSortOpen(false); requestAnimationFrame(() => sortButtonRef.current?.focus()); } }}><div className="grid gap-1">{sortOptions.map((option) => <button key={option.key} role="menuitemradio" aria-checked={source.querySpec.sort.kind === option.key} className={cn("flex min-h-9 items-center justify-between rounded-[var(--zc-radius-control)] px-3 text-left text-sm", source.querySpec.sort.kind === option.key ? "bg-[var(--zc-surface-selected)] text-[var(--zc-text-primary)]" : "text-[var(--zc-text-secondary)] hover:bg-[var(--zc-surface-hover)]")} onClick={() => { source.setSort(option.key); setIsSortOpen(false); requestAnimationFrame(() => sortButtonRef.current?.focus()); }}>{option.label}<span className="text-xs">{source.querySpec.sort.kind === option.key ? source.querySpec.sort.direction === "desc" ? "↓" : "↑" : ""}</span></button>)}</div></div> : null}</div>
    <select className={cn(buttonSubtle, "min-h-9 max-w-48 px-2 text-xs")} value={source.activeViewId ?? ""} onChange={(event) => source.applySavedView(source.savedViews.find((view) => view.id === event.target.value) ?? null)} aria-label={t("librarySavedViewsLabel")}><option value="">{t("librarySavedViewsPlaceholder")}</option>{source.savedViews.map((view) => <option key={view.id} value={view.id}>{view.displayName}{view.invalidReferences.length ? ` · ${t("librarySavedViewInvalid")}` : ""}</option>)}</select>
    <BatchRenameAction
      source={batchRenameSource}
      renameAvailable={capabilities?.renameAvailable === true}
      paneVisible={pane?.visible ?? true}
      t={t}
      onPreview={onBatchRenamePreview}
    />
  </> : null, [
    activeFilterCount, batchRenameSource, capabilities?.renameAvailable, currentSortLabel,
    isFilterOpen, isSortOpen, onBatchRenamePreview, pane?.id, pane?.visible,
    showLibraryControls, sortOptions, source.activeViewId, source.applySavedView,
    source.clearFilters, source.querySpec.filters, source.querySpec.sort.direction,
    source.querySpec.sort.kind, source.savedViews, source.setSort, source.tags,
    source.updateFilters, t
  ]);
  useFileLibraryLibrarySearchSurface({
    enabled: showLibraryControls,
    value: source.librarySearch,
    onChange: source.handleLibrarySearchChange,
    placeholder: source.scope.kind === "all" ? t("librarySearchPlaceholder") : t("librarySearchPlaceholderScoped"),
    actions: commandBarActions,
    t
  });

  const restoreLibraryFocus = useCallback((target: HTMLElement | null) => {
    if (target && isFileLibraryFocusTarget(target)) {
      target.focus();
      if (document.activeElement === target) return;
    }
    query('[data-library-source-owner="query-v2"] [role="listbox"]')?.focus();
  }, [query]);
  const {
    contextMenu,
    openContextMenu,
    closeContextMenu,
    openFocusedContextMenu,
    handleRowContextMenu
  } = useLibraryContextMenu({ source, restoreFocus: restoreLibraryFocus });
  const content = useLibraryContentCompatibility({ source, t, onError, restoreFocus: restoreLibraryFocus });
  const { isContentOpenPending } = content;

  useEffect(() => {
    void source.loadTags().catch(() => undefined);
    void source.loadSavedViews().catch(() => undefined);
  }, [source.loadSavedViews, source.loadTags]);

  useEffect(() => {
    if (!source.selection) {
      source.clearInspector();
    } else if (canonicalSingleSelectionId !== null) {
      if (!isContentOpenPending(canonicalSingleSelectionId)) void source.loadDetail(canonicalSingleSelectionId);
    } else {
      void source.loadSelectionSummary(source.selection).catch(() => undefined);
    }
  }, [canonicalSingleSelectionId, isContentOpenPending, source.clearInspector, source.loadDetail, source.loadSelectionSummary, source.selection]);

  const interaction = useMemo(() => createLibraryInteractionProjection(source), [source]);
  const viewMode = experienceState.workspace.session.presentation.viewMode ?? "list";

  const focusedPreviewSource = useMemo(() => {
    const file = source.files.find((item) => item.id === source.focusedId);
    return previewSourceFromEntry(file === undefined ? undefined : adaptLibrarySummary(file), source.collection);
  }, [source.collection, source.files, source.focusedId]);

  const siblingNavigation = useMemo(() => {
    const currentIndex = source.files.findIndex((file) => file.id === source.focusedId);
    return focusedPreviewSource === null || currentIndex < 0 ? null : createPreviewSiblingNavigation({
      source: "library", generation: focusedPreviewSource.generation, currentKey: focusedPreviewSource.key,
      currentIndex, loadedCount: source.files.length, hasMore: source.hasMore, move: source.moveFocus
    });
  }, [focusedPreviewSource, source.files, source.focusedId, source.hasMore, source.moveFocus]);

  useEffect(() => {
    previewController.observeSource(focusedPreviewSource);
  }, [focusedPreviewSource, previewController]);

  useEffect(() => { previewController.setSiblingNavigation(siblingNavigation); return () => previewController.setSiblingNavigation(null); }, [previewController, siblingNavigation]);

  const previewSelected = useSelectedPreview(previewController, source.selectedFiles.slice(0, SELECTED_PREVIEW_LIMIT).filter((file) => !file.isStale).flatMap((file) => {
    const projected = previewSourceFromEntry(adaptLibrarySummary(file), source.collection);
    return projected ? [projected] : [];
  }));

  const openLibraryPreview = useCallback((fileId: string, trigger: HTMLElement | null) => {
    const file = source.files.find((item) => item.id === fileId) ?? (source.detail?.id === fileId ? { ...source.detail, displayDirectory: source.detail.directory, tagCount: source.detail.tags.length } : undefined);
    const nextSource = previewSourceFromEntry(file === undefined ? undefined : adaptLibrarySummary(file), source.collection);
    if (nextSource === null) return false;
    if (contextMenu) closeContextMenu("dialog-handoff");
    return previewController.open(nextSource, trigger);
  }, [closeContextMenu, contextMenu, previewController, source.collection, source.files, source.detail]);

  const handleLibraryPreviewKey = useCallback((entry: PresentationEntry, trigger: HTMLElement, event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (entry.source !== "library") return false;
    return previewController.handleSpace(
      previewSourceFromEntry(entry, source.collection),
      trigger,
      event
    );
  }, [previewController, source.collection]);

  function activateLibraryEntry(entry: LibraryPresentationEntry, trigger: HTMLElement) {
    if (entry.source === "library") openLibraryPreview(entry.entryRef.fileId, trigger);
  }

  function handleSharedContextMenu(
    event: React.MouseEvent<HTMLDivElement>,
    entry: LibraryPresentationEntry,
    index: number
  ) {
    if (entry.source !== "library") return;
    handleRowContextMenu(event, index);
  }

  function handleListEscape() {
    if (contextMenu) {
      closeContextMenu("escape");
      return true;
    }
    if (previewState.visible) {
      previewController.close("escape");
      return true;
    }
    if (contextOpen && contextProjection.kind !== "none") {
      closeContextPanel();
      return true;
    }
    return false;
  }

  async function revealFile(fileId: string) {
    try {
      await tauriApi.revealFileLibraryEntry(fileId);
    } catch (error) {
      onError(readableError(error));
    }
  }

  async function openContentFromContext(fileId: string) {
    const context = contextMenu;
    closeContextMenu("dialog-handoff");
    await content.openContentForFile(fileId, context?.restoreFocusElement ?? undefined);
  }

  async function openOperationsPreview(targetFileIds?: ReadonlySet<string>) {
    const currentSelection = source.selection;
    if (!targetFileIds && !currentSelection) {
      onError(t("libraryOperationPreviewRequiresSelection"));
      return;
    }
    source.clearExecutionIntent();
    try {
      const result = targetFileIds
        ? await source.refreshPreviewsForFiles(source.scope, new Set(targetFileIds))
        : currentSelection?.kind === "all_matching"
          ? await source.refreshPreviewsForSelection(source.scope, currentSelection)
          : await source.refreshPreviewsForFiles(source.scope, new Set(source.selectedIdList));
      if (!result?.previews.length) {
        onError(t("libraryNoOperationsForSelection"));
        return;
      }
      setView("preview");
    } catch (error) {
      onError(readableError(error));
    }
  }

  async function openPermanentDeletePreview(file: FileLibraryDetail) {
    if (capabilities?.permanentDeleteAvailable !== true || file.isStale) return;
    try {
      const preview = await tauriApi.getPermanentDeleteOperationPreview(file.id);
      source.clearExecutionIntent();
      source.setPreviewResult({ previews: [preview], total: 1, limit: 1, offset: 0, truncated: false, hasMore: false }, source.scope);
      setView("preview");
    } catch (error) {
      onError(readableError(error));
    }
  }

  async function toggleTag(tagId: string, operation: "add" | "remove") {
    if (!source.selection) return;
    try {
      await source.mutateTags({ selection: source.selection, tagIds: [tagId], operation, expectedCount: source.selectionSummary?.count ?? null });
      await source.refreshResults();
      if (canonicalSingleSelectionId !== null) await source.loadDetail(canonicalSingleSelectionId);
    } catch (error) {
      onError(readableError(error));
    }
  }

  function libraryState() {
    if (source.resultState === "snapshot_expired" || source.error === "library_snapshot_expired") return null;
    if (source.error || source.resultState === "failed") {
      return {
        tone: "error" as const,
        title: t("libraryLoadFailedTitle"),
        description: source.error ?? t("libraryLoadFailedDesc"),
        primaryAction: <button className={buttonSecondary} onClick={() => void source.refreshResults().catch(() => undefined)}>{t("libraryRetry")}</button>
      };
    }
    if (source.isLoading && source.totalCount === 0) return { tone: "info" as const, title: t("libraryLoadingResults"), description: t("libraryScopeHint") };
    if (isNoIndexState) return { tone: "info" as const, title: t("libraryNoScanTitle"), description: t("libraryNoScanDesc"), primaryAction: <button className={buttonPrimary} onClick={() => setView("scanner")}><Layers size={16} />{t("libraryGoToOverview")}</button> };
    if (source.isEmptyCurrentScanScope) return {
      tone: "info" as const,
      title: t("noCurrentScanTitle"),
      description: t("noCurrentScanDesc"),
      primaryAction: <button className={buttonPrimary} onClick={() => setView("scanner")}><Layers size={16} />{t("libraryGoToOverview")}</button>,
      secondaryAction: <button className={buttonSecondary} onClick={() => source.setScope({ kind: "all" })}>{t("viewAllIndexedFiles")}</button>
    };
    if (source.totalCount !== null && source.totalCount > 0) return null;
    if (source.librarySearch.trim()) return { tone: "neutral" as const, title: t("libraryNoSearchTitle"), description: t("libraryNoSearchDesc") };
    if (activeFilterCount) return { tone: "neutral" as const, title: t("libraryNoFilterTitle"), description: t("libraryNoFilterDesc") };
    return { tone: "neutral" as const, title: t("libraryNoScopeFilesTitle"), description: t("libraryNoScopeFilesDesc"), secondaryAction: <button className={buttonSecondary} onClick={() => source.setScope({ kind: "all" })}>{t("viewAllIndexedFiles")}</button> };
  }

  const state = libraryState();
  const selectionLabel = source.selection?.kind === "all_matching"
    ? replaceCopy(t("librarySelectionAll"), { count: source.totalCount === null ? t("libraryCountPending") : source.totalCount.toLocaleString(), excluded: source.selection.excludedFileIds.length.toLocaleString() })
    : source.selectedIds.size ? replaceCopy(t("librarySelectionLoaded"), { count: source.selectedIds.size.toLocaleString() }) : t("librarySelectionNone");
  const resultCountLabel = source.isLoading
    ? t("loading")
    : source.isCountLoading || source.totalCount === null
      ? replaceCopy(t("libraryResultCountDeferred"), { loaded: source.files.length.toLocaleString() })
      : replaceCopy(t("libraryResultCountExact"), { loaded: source.files.length.toLocaleString(), total: source.totalCount.toLocaleString() });
  const contextOpen = experienceState.workspace.session.presentation.contextOpen === true;
  const contextProjection = createLibraryContextProjection(source.selection, {
    selectedIds: source.selectedIds,
    selectedFiles: source.selectedFiles,
    detail: source.detail,
    selectionKind: source.selection?.kind ?? null,
    selectedCount: source.selection === null
      ? null
      : source.selection.kind === "all_matching"
        ? source.selectionSummary?.count ?? null
        : source.selection.fileIds.length,
    selectionSummary: source.selectionSummary,
    isLoading: source.isInspectorLoading,
    error: source.inspectorError,
    language,
    t,
    onPreviewSelection: (id, trigger) => { previewSelected(source.selectedFiles.slice(0, SELECTED_PREVIEW_LIMIT).filter((file) => !file.isStale).findIndex((file) => file.id === id), query("[data-file-library-context-toggle]") ?? trigger); },
    onPreview: (event, file) => { openLibraryPreview(file.id, query("[data-file-library-context-toggle]") ?? event.currentTarget); },
    onReveal: (fileId) => void revealFile(fileId).catch(() => undefined),
    onViewSuggestions: () => setView("organize"),
    onViewOperations: () => void openOperationsPreview().catch(() => undefined),
    onPermanentDelete: capabilities?.permanentDeleteAvailable === true ? openPermanentDeletePreview : undefined,
    onOpenContentUnderstanding: (file, trigger) => void content.openContentForFile(file.id, trigger, file),
    onClearSelection: source.clearSelection,
    onRetryDetail: () => {
      if (canonicalSingleSelectionId !== null) void source.loadDetail(canonicalSingleSelectionId);
    },
    availableTags: source.tags,
    onToggleTag: (tagId, operation) => void toggleTag(tagId, operation).catch(() => undefined)
  });

  function closeContextPanel() {
    controller.setContextOpen(false);
  }

  const restoreContextFocus = () => query("[data-file-library-context-toggle]");

  return (
    <div
      className={cn(pageFrame, "file-library-library-mode gap-3 overflow-x-hidden")}
      data-library-source-owner="query-v2"
      data-library-provenance={source.collection ? "query-v2-snapshot" : "pending"}
      data-library-selection-authority="library-selection-v1"
      data-library-selection-kind={source.selection?.kind ?? "none"}
      data-library-query-file-types={source.querySpec.filters.fileTypes.join(",")}
      data-library-query-tags={source.querySpec.filters.tagsAllOf.join(",")}
      data-library-query-scope={source.querySpec.scope.kind === "roots" ? source.querySpec.scope.scanRootIds.join(",") : source.querySpec.scope.kind}
    >
      <div className="file-library-library-mode-chrome">
        <section className={cn(raisedSurface, "relative z-20 grid shrink-0 gap-2 px-3 py-2")}>
          <div className="flex min-w-0 flex-wrap items-center justify-between gap-2" aria-label={scopeText}>
            <span className="truncate text-xs text-[var(--zc-text-secondary)]">{scopeText}{source.scopeHealth && source.scopeHealth.state !== "healthy" ? ` · ${scopeHealthLabel(source.scopeHealth.state, t)}` : ""}</span>
            <div className="flex flex-wrap items-center gap-2">
              {source.scope.kind !== "all" && !source.isEmptyCurrentScanScope ? <button className={cn(buttonGhost, "min-h-8 px-2.5 py-1.5 text-xs")} onClick={() => source.setScope({ kind: "all" })}><Layers size={15} />{t("viewAllIndexedFiles")}</button> : null}
              <button className={cn(buttonGhost, "min-h-8 px-2.5 py-1.5 text-xs")} onClick={() => void source.chooseFolders().catch(() => undefined)}><FolderSearch size={15} />{t("switchScanDirectory")}</button>
            </div>
          </div>
          <div className="flex flex-wrap items-center gap-2"><button ref={personalTrigger} className={cn(buttonSubtle, "min-h-8 px-2 text-xs")} onClick={() => setPersonalOpen(true)}>{t("u4MyFiles")}</button>{showLibraryControls ? <><button data-library-manager="saved_views" className={cn(buttonSubtle, "min-h-8 px-2 text-xs")} onClick={() => setMetadataManager("saved_views")}><Bookmark size={14} />{t("libraryManageSavedViews")}</button><button data-library-manager="tags" className={cn(buttonSubtle, "min-h-8 px-2 text-xs")} onClick={() => setMetadataManager("tags")}><Tag size={14} />{t("libraryManageTags")}{source.tags.length ? ` · ${source.tags.length}` : ""}</button></> : null}</div>
          {showLibraryControls ? <div className="flex min-h-0 flex-wrap items-center gap-1.5" aria-label={t("libraryAppliedFilters")}><span className="text-xs text-[var(--zc-text-tertiary)]">{activeFilterCount ? replaceCopy(t("libraryFiltersAppliedCount"), { count: activeFilterCount }) : t("libraryFilterAllOptions")}</span>{activeFilterCount ? <button className="text-xs text-[var(--zc-primary)] underline" onClick={source.clearFilters}>{t("libraryFilterClear")}</button> : null}</div> : null}
          {showLibraryControls ? <p className="text-xs text-[var(--zc-text-tertiary)]" data-library-result-status aria-live="polite">{t("libraryResultCountLabel")}: {resultCountLabel} · {t("librarySelectionLabel")}: {selectionLabel}</p> : null}
          {showLibraryControls && source.selection?.kind === "explicit" && source.selectedIds.size === source.files.length && source.totalCount !== null && source.totalCount > source.files.length ? <button className="justify-self-start text-xs text-[var(--zc-primary)] underline" onClick={source.selectAllMatching}>{t("librarySelectAllMatching")}</button> : null}
        </section>
        {source.resultState === "snapshot_expired" || source.error === "library_snapshot_expired" ? <NoticeBanner tone="warning" title={t("librarySnapshotExpiredTitle")} action={<button className={buttonSecondary} onClick={() => void source.refreshResults().catch(() => undefined)}>{t("librarySnapshotRefresh")}</button>}>{t("librarySnapshotExpiredDesc")}</NoticeBanner> : null}
      </div>
      <PersonalFiles open={personalOpen} t={t} onClose={() => setPersonalOpen(false)} restoreFocus={() => personalTrigger.current} onSelect={(id) => {
        source.setExplicitSelection([id], id);
        controller.setContextOpen(true);
        setPersonalOpen(false);
      }} />
      <div className={cn("file-library-library-mode-result", contextOpen && contextProjection.kind !== "none" && "has-context") }>
        <section className={cn(raisedSurface, "h-full min-h-0 max-[1100px]:min-h-[340px] overflow-hidden")} aria-label={t("fileLibrary")}>{state ? <StateBlock tone={state.tone} title={state.title} description={state.description} primaryAction={state.primaryAction} secondaryAction={state.secondaryAction} /> : viewMode === "grid" ? <SharedFileGrid
          interaction={interaction}
          language={language}
          t={t}
          controller={controller.workspace}
          ariaLabel={t("fileLibrary")}
          emptyLabel={t("libraryNoSearchTitle")}
          loadMoreLabel={t("loadMoreFiles").replace("{count}", String(Math.min(32, remainingCount)))}
          loadingMoreLabel={t("libraryLoadingMore")}
          onPreview={handleLibraryPreviewKey}
          onActivate={(entry, trigger) => {
            if (entry.source === "library") activateLibraryEntry(entry, trigger);
          }}
          onContextMenu={(event, entry, index) => {
            if (entry.source === "library") handleSharedContextMenu(event, entry, index);
          }}
          onOpenContextMenu={() => openFocusedContextMenu()}
          onEscape={handleListEscape}
        /> : <SharedFileList
          interaction={interaction}
          language={language}
          t={t}
          ariaLabel={t("fileLibrary")}
          emptyLabel={t("libraryNoSearchTitle")}
          loadMoreLabel={t("loadMoreFiles").replace("{count}", String(Math.min(32, remainingCount)))}
          loadingMoreLabel={t("libraryLoadingMore")}
          loadedAllLabel={t("libraryLoadedAll")}
          onPreview={handleLibraryPreviewKey}
          onActivate={(entry, trigger) => {
            if (entry.source === "library") activateLibraryEntry(entry, trigger);
          }}
          onContextMenu={(event, entry, index) => {
            if (entry.source === "library") handleSharedContextMenu(event, entry, index);
          }}
          onOpenContextMenu={() => openFocusedContextMenu()}
          onEscape={handleListEscape}
        />}</section>
          <ContextPanel
            projection={contextProjection}
            open={contextOpen && (pane?.visible ?? true) && (!isNoIndexState || previewState.host === "pinned")}
            onClose={closeContextPanel}
            restoreFocus={restoreContextFocus}
          />
      </div>
      {renameFile && (pane?.visible ?? true) ? <ManualRenameDialog
        key={`rename-${renameFile.id}`}
        mode="rename"
        file={renameFile}
        t={t}
        onClose={() => setRenameFile(null)}
        restoreFocus={() => query('[data-library-source-owner="query-v2"] [role="listbox"]')}
        onPreview={(preview) => {
          source.clearExecutionIntent();
          source.setPreviewResult({ previews: [preview], total: 1, limit: 1, offset: 0, truncated: false, hasMore: false }, source.scope);
          setRenameFile(null);
          setView("preview");
        }}
      /> : null}
      {contextMenu && (pane?.visible ?? true) ? (
        <LibraryContextMenu
          context={contextMenu}
          t={t}
          onClose={() => closeContextMenu("action")}
          onPreview={(trigger) => {
            const fileId = contextMenu.file.id;
            closeContextMenu("dialog-handoff");
            openLibraryPreview(fileId, trigger);
          }}
          onMove={platform === "win32" && contextMenu.file.isDirectory ? undefined : () => { closeContextMenu("dialog-handoff"); useFileManagementStore.getState().requestAction("move"); }}
          transferItems={libraryTransferMenuItems({t,windows:platform==="win32",isDirectory:contextMenu.file.isDirectory,clipboardIncludesDirectory,closeContextMenu,requestAction:(action)=>useFileManagementStore.getState().requestAction(action)})}
          onRename={capabilities?.renameAvailable && !contextMenu.file.isDirectory && !contextMenu.file.isStale ? () => {
            closeContextMenu("dialog-handoff"); source.setExplicitSelection([contextMenu.file.id], contextMenu.file.id);
            setRenameFile(contextMenu.file);
          } : undefined}
          onReveal={() => void revealFile(contextMenu.file.id).catch(() => undefined)}
          onOpenContent={() => void openContentFromContext(contextMenu.file.id).catch(() => undefined)}
          onViewOperations={() => void openOperationsPreview(new Set([contextMenu.file.id])).catch(() => undefined)}
          onViewSuggestions={() => setView("organize")}
          onClearSelection={source.clearSelection}
        />
      ) : null}
      {content.contentDetail && (pane?.visible ?? true) ? <ContentUnderstandingSheet open detail={content.contentDetail} t={t} restoreFocus={() => content.contentRestoreTargetRef.current ?? content.contentTriggerRef.current} onClose={content.closeContentUnderstanding} onRefreshAuthoritativeContentState={content.refreshOpenContentDetail} /> : null}
      <LibraryMetadataManagerDialog
        kind={pane && !pane.visible ? null : metadataManager}
        query={cloneFileQuerySpec({ ...source.querySpec, text: source.debouncedSearchQuery.trim() || null })}
        selection={source.selection}
        selectionCount={source.selectionSummary?.count ?? (source.selection?.kind === "all_matching" ? source.totalCount : source.selectedIds.size)}
        activeViewId={source.activeViewId}
        t={t}
        onApplyView={(view) => { source.applySavedView(view); setMetadataManager(null); }}
        onMutated={async () => {
          await source.refreshResults();
          if (canonicalSingleSelectionId !== null) await source.loadDetail(canonicalSingleSelectionId);
          else if (source.selection) await source.loadSelectionSummary(source.selection);
        }}
        onClose={() => setMetadataManager(null)}
      />
    </div>
  );
}
