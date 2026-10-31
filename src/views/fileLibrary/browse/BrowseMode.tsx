import { createPortal } from "react-dom";
import { usePaneQuerySelector, usePane } from "../workbench/PaneContext";
import { ChevronRight, Folder, FolderOpen, LoaderCircle, MapPin, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useLayoutEffect, useState } from "react";
import { NoticeBanner, SearchField, StateBlock } from "../../shared/ui";
import { useCommandContext, useI18nContext } from "../../../contexts/AppContexts";
import type { LocationDescriptor } from "../../../types/fileWorkspace";
import { revealBrowseEntry } from "../../../api/fileWorkspaceApi";
import { cn } from "../../../utils/tw";
import { useFileLibraryExperience } from "../FileLibraryExperienceProvider";
import {
  isActivatableLocation,
  locationAvailabilityLabel,
  useBrowseSourceOwner
} from "./browseSourceOwner";
import { createBrowseInteractionProjection } from "../list/interactionAdapters";
import { SharedFileGrid } from "../list/SharedFileGrid";
import { SharedFileList } from "../list/SharedFileList";
import { ContextPanel } from "../context/ContextPanel";
import { createBrowseContextProjection } from "../context/contextPanelProjection";
import { useRegisterFileLibraryCommandBarSurface } from "../fileLibraryCommandBarSurface";
import { usePreviewExperience } from "../preview/PreviewExperienceProvider";
import { SELECTED_PREVIEW_LIMIT, useSelectedPreview } from "../preview/useSelectedPreview";
import { previewSourceFromEntry } from "../preview/previewSource";
import { createPreviewSiblingNavigation } from "../preview/previewSiblingNavigation";
import { FileLibraryContextMenu } from "../library/LibraryContextMenu";
import { isFileLibraryFocusTarget } from "../fileLibraryInteraction";
import { useBrowseContextMenu } from "./useBrowseContextMenu";
import { useFileManagementStore } from "../management/fileManagementStore";
import { useBrowseManagementSurface } from "../management/useManagementSurfaces";
import "./browseMode.css";

export function BrowseMode() {
  const query = usePaneQuerySelector();
  const pane = usePane();
  const [breadcrumbHost, setBreadcrumbHost] = useState<HTMLElement | null>(null);
  useLayoutEffect(() => { setBreadcrumbHost(pane ? document.getElementById(`wb-crumbs-${pane.id}`) : null); }, [pane?.id]);
  const { controller, state } = useFileLibraryExperience();
  const { language, t } = useI18nContext();
  const [revealError, setRevealError] = useState<string | null>(null);
  const { platform } = useCommandContext();
  const source = useBrowseSourceOwner({ controller, state, t, platform });
  useBrowseManagementSurface(source);
  useEffect(() => setRevealError(null), [source.focusedId]);
  const { controller: previewController, state: previewState } = usePreviewExperience();
  const interaction = useMemo(() => createBrowseInteractionProjection(source), [source]);
  const focusedPreviewSource = useMemo(() => {
    const entry = source.entries.find((item) => item.entryRef.entryId === source.focusedId);
    return previewSourceFromEntry(entry, source.collection);
  }, [source.collection, source.entries, source.focusedId]);

  const siblingNavigation = useMemo(() => {
    if (focusedPreviewSource === null) return null;
    const currentIndex = source.entries.findIndex((entry) => entry.entryRef.entryId === source.focusedId);
    if (currentIndex < 0) return null;
    return createPreviewSiblingNavigation({
      source: "browse",
      generation: focusedPreviewSource.generation,
      currentKey: focusedPreviewSource.key,
      currentIndex,
      loadedCount: source.entries.length,
      hasMore: source.hasMore,
      move: source.moveFocus
    });
  }, [focusedPreviewSource, source.entries, source.focusedId, source.hasMore, source.moveFocus]);

  useEffect(() => {
    previewController.observeSource(focusedPreviewSource);
  }, [focusedPreviewSource, previewController]);

  useEffect(() => {
    previewController.setSiblingNavigation(siblingNavigation);
    return () => previewController.setSiblingNavigation(null);
  }, [previewController, siblingNavigation]);
  const restoreBrowseFocus = useCallback((target: HTMLElement | null) => {
    if (target && isFileLibraryFocusTarget(target)) {
      target.focus();
      if (document.activeElement === target) return;
    }
    query(
      '[data-shared-file-list-source="browse"], [data-shared-file-grid-source="browse"]'
    )?.focus();
  }, []);
  const {
    contextMenu,
    closeContextMenu,
    openFocusedContextMenu,
    handleRowContextMenu
  } = useBrowseContextMenu({ source, restoreFocus: restoreBrowseFocus });
  const viewMode = state.workspace.session.presentation.viewMode ?? "list";
  const browseSearchInputRef = useRef<HTMLInputElement | null>(null);
  const browseSearch = useMemo(() => (
    <SearchField
      value={source.queryText}
      onChange={(event) => source.setQueryText(event.currentTarget.value)}
      onClear={() => source.setQueryText("")}
      label={t("browseSearchLabel")}
      clearLabel={t("browseSearchClear")}
      placeholder={t("browseSearchPlaceholder")}
      inputRef={browseSearchInputRef}
      loading={source.enumerationState === "loading" || source.enumerationState === "loading_more"}
      className="file-library-command-search-field"
      data-file-library-local-search="true"
      data-file-library-local-search-state={source.isQueryActive ? "active" : "idle"}
    />
  ), [source.enumerationState, source.isQueryActive, source.queryText, source.setQueryText, t]);

  const browseActions = useMemo(() => (
    <div className="flex min-h-0 flex-wrap items-center gap-1.5" data-browse-query-controls="true">
      <label className="sr-only" htmlFor="browse-query-kind">{t("browseFilterLabel")}</label>
      <select
        id="browse-query-kind"
        className="file-library-command-button min-h-9 px-2 py-1.5 text-xs"
        value={source.queryEntryKind}
        aria-label={t("browseFilterLabel")}
        data-browse-query-kind={source.queryEntryKind}
        onChange={(event) => source.setQueryEntryKind(event.currentTarget.value as typeof source.queryEntryKind)}
      >
        <option value="all">{t("browseFilterAll")}</option>
        <option value="file">{t("browseFilterFiles")}</option>
        <option value="directory">{t("browseFilterDirectories")}</option>
      </select>
      <select className="file-library-command-button" aria-label={t("u3Sort")}
        value={source.query.sort?.field ?? "native"} data-browse-sort
        onChange={(event) => source.setSort(event.target.value === "native" ? undefined : {
          field: event.target.value as "name" | "type" | "size" | "modified",
          direction: source.query.sort?.direction ?? "asc", foldersFirst: source.query.sort?.foldersFirst ?? true
        })}>
        <option value="native">{t("u3NativeOrder")}</option>
        <option value="name">{t("u3Name")}</option><option value="type">{t("u3Type")}</option>
        <option value="size">{t("u3Size")}</option><option value="modified">{t("u3Modified")}</option>
      </select>
      <button className="file-library-command-button" disabled={!source.query.sort}
        aria-label={t("u3Direction")} onClick={() => source.query.sort && source.setSort({
          ...source.query.sort, direction: source.query.sort.direction === "asc" ? "desc" : "asc"
        })}>{source.query.sort?.direction === "desc" ? t("u3Desc") : t("u3Asc")}</button>
      <label className="u3-folders-first"><input type="checkbox" disabled={!source.query.sort}
        checked={source.query.sort?.foldersFirst ?? true} onChange={(event) => source.query.sort && source.setSort({
          ...source.query.sort, foldersFirst: event.target.checked
        })}/>{t("u3FoldersFirst")}</label>
    </div>
  ), [source.queryEntryKind, source.setQueryEntryKind, source.query.sort, source.setSort, t]);

  useRegisterFileLibraryCommandBarSurface("browse", browseSearch, browseSearchInputRef, true, browseActions);

  const selectedPreviewEntries = source.entries.filter((entry) => source.selectedIds.has(entry.entryRef.entryId)).slice(0, SELECTED_PREVIEW_LIMIT);
  const previewSelected = useSelectedPreview(previewController, selectedPreviewEntries.flatMap((entry) => {
    const projected = previewSourceFromEntry(entry, source.collection);
    return projected ? [projected] : [];
  }));

  const contextOpen = state.workspace.session.presentation.contextOpen === true;
  const contextProjection = createBrowseContextProjection({
    entries: source.entries,
    selectedIds: source.selectedIds,
    locationLabel: source.browse?.location.displayName ?? t("fileLibraryModeBrowse"),
    language,
    t,
    actionError: revealError,
    onReveal: (entry) => { setRevealError(null); void revealBrowseEntry(entry.entryRef).catch(() => setRevealError(t("u4RevealUnavailable"))); },
    onPreview: (entry, trigger) => { previewSelected(selectedPreviewEntries.findIndex((item) => item.entryRef.entryId === entry.entryRef.entryId), query("[data-file-library-context-toggle]") ?? trigger); }
  });

  function closeContextPanel() {
    controller.setContextOpen(false);
  }

  const restoreContextFocus = () => query("[data-file-library-context-toggle]");

  if (source.showLocationPicker || source.target === null || source.browse === null) {
    return (
      <div className="browse-mode" data-browse-state="location-picker">
        <BrowseLocationPicker detached={state.detachedBrowse} source={source} t={t} />
        <ContextPanel
          projection={contextProjection}
          open={contextOpen && (pane?.visible ?? true)}
          onClose={closeContextPanel}
          restoreFocus={restoreContextFocus}
        />
      </div>
    );
  }

  const locationUnavailable = source.browse.location.availability !== "available"
    || !source.browse.location.capabilities.canBrowse;
  if (locationUnavailable) {
    return (
      <div className="browse-mode" data-browse-source-owner="browse" data-browse-state="unavailable">
        <StateBlock
          tone="warning"
          title={locationAvailabilityLabel(source.browse.location.availability, t)}
          description={t("browseLocationsDesc")}
          primaryAction={(
            <button className="browse-action" type="button" onClick={source.openLocationPicker}>
              <MapPin size={15} aria-hidden="true" />
              {t("browseLocationsButton")}
            </button>
          )}
        />
        <ContextPanel
          projection={contextProjection}
          open={contextOpen && (pane?.visible ?? true)}
          onClose={closeContextPanel}
          restoreFocus={restoreContextFocus}
        />
      </div>
    );
  }

  const completion = source.collection?.provenance.completion ?? "pending";
  const emptyPartialQuery = source.isQueryActive
    && source.enumerationState === "partial"
    && source.entries.length === 0;
  const emptyLabel = emptyPartialQuery
    ? t("browseEnumerationSearching")
    : t("browseEnumerationEmptyTitle");
  const statusText = source.enumerationState === "loading"
    ? t("browseEnumerationLoading")
    : source.enumerationState === "loading_more"
      ? t("browseEnumerationLoadingMore")
      : source.enumerationState === "complete"
        ? t("browseEnumerationComplete").replace("{loaded}", String(source.loadedCount))
        : source.enumerationState === "partial"
          ? emptyPartialQuery
            ? t("browseEnumerationSearching")
            : t("browseEnumerationPartial").replace("{loaded}", String(source.loadedCount))
          : "";
  const changeStatusText = source.changeState === "checking"
    ? t("browseChangeChecking")
    : source.changeState === "refreshing"
      ? t("browseChangeRefreshing")
      : source.changeState === "failed" && source.changeError
        ? t("browseChangeFailed")
        : null;
  const selectionText = source.selectedCount === 0
    ? t("browseSelectionNone")
    : t("browseSelectionLoaded").replace("{count}", String(source.selectedCount));
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

  const contextIncludesDirectory = contextMenu !== null && (
    source.selectedIds.has(contextMenu.entry.entryRef.entryId)
      ? source.entries.filter((entry) => source.selectedIds.has(entry.entryRef.entryId))
      : [contextMenu.entry]
  ).some((entry) => entry.entryKind === "directory");
  const clipboardIncludesDirectory = useFileManagementStore.getState().clipboard?.tickets.some((ticket) => ticket.isDirectory) ?? false;
  const browseContextItems = contextMenu === null ? [] : [
    ...(["copy", "cut", "paste", "move", "basket"] as const).map((action) => ({
      label: t(action === "basket" ? "fmAddBasket" : action === "copy" ? "fmCopy" : action === "cut" ? "fmCut" : action === "paste" ? "fmPaste" : "fmMove"),
      disabled: platform === "win32" && (
        ["copy", "cut", "move", "basket"].includes(action) && contextIncludesDirectory
        || action === "paste" && clipboardIncludesDirectory
      ),
      title: platform === "win32" && (
        ["copy", "cut", "move", "basket"].includes(action) && contextIncludesDirectory
        || action === "paste" && clipboardIncludesDirectory
      ) ? t("fmWindowsDirectories") : undefined,
      action: () => { closeContextMenu(action === "paste" || action === "move" ? "dialog-handoff" : "action"); useFileManagementStore.getState().requestAction(action); }
    })),
    {
      label: t("fmRename"),
      action: () => {
        source.selectEntry(contextMenu.entry.entryRef.entryId, "replace");
        closeContextMenu("dialog-handoff");
        requestAnimationFrame(() => useFileManagementStore.getState().requestAction("rename"));
      }
    },
    {
      label: t("fmSafeDelete"),
      action: () => {
        closeContextMenu("dialog-handoff");
        requestAnimationFrame(() => useFileManagementStore.getState().requestAction("trash"));
      }
    },
    ...(contextMenu.entry.entryKind === "directory"
      ? [{
          label: t("browseOpenFolder"),
          action: () => {
            closeContextMenu("source-change");
            void source.navigateInto(contextMenu.entry);
          }
        }]
      : []),
    {
      label: t("fileLibraryContextOpen"),
      action: () => {
        closeContextMenu("action");
        controller.setContextOpen(true);
      }
    },
    {
      label: t("libraryClearSelection"),
      action: () => {
        source.clearSelection();
        closeContextMenu("action");
      }
    }
  ];

  const breadcrumbNavigation = (<nav className="browse-breadcrumbs" data-browse-breadcrumbs="true" aria-label={t("browseBreadcrumbLabel")}>
          {source.breadcrumbs.length === 0
            ? <span className="browse-breadcrumb" aria-current="page">{t("browseCurrentFolder")}</span>
            : source.breadcrumbs.map((breadcrumb, index) => {
              const current = index === source.breadcrumbs.length - 1;
              return (
                <span className="inline-flex min-w-0 items-center gap-1" key={`${breadcrumb.sessionId}:${breadcrumb.pathRef.id}`}>
                  {index > 0 ? <ChevronRight className="browse-breadcrumb-separator" size={14} aria-hidden="true" /> : null}
                  <button
                    className="browse-breadcrumb"
                    type="button"
                    disabled={current}
                    aria-current={current ? "page" : undefined}
                    onClick={() => source.navigateToBreadcrumb(breadcrumb)}
                  >
                    {breadcrumb.label}
                  </button>
                </span>
              );
            })}
        </nav>);

  return (
    <div
      className="browse-mode"
      data-browse-source-owner="browse"
      data-browse-state="current-folder"
      data-browse-provenance={source.collection === null ? "pending" : "browse-enumeration"}
      data-browse-enumeration-completion={completion}
      data-browse-known-count={source.knownCount === null ? undefined : source.knownCount}
      data-browse-change-state={source.changeState}
      data-browse-change-pending={source.pendingChange === null ? "false" : "true"}
      data-browse-selection-authority="browse-source-local"
      data-browse-selection-count={source.selectedCount}
      data-browse-query={source.queryText || undefined}
      data-browse-query-kind={source.queryEntryKind}
      data-browse-search-state={source.isQueryActive ? source.enumerationState : "inactive"}
      data-browse-query-empty-partial={emptyPartialQuery ? "true" : "false"}
      data-browse-sort-capability="backend-snapshot"
    >
      <header className="browse-mode-header">
        <div className="min-w-0">
          <details className="browse-mode-help">
            <summary>{t("fmBrowseHelp")}</summary>
            <p className="browse-mode-description">{t("fileLibraryBrowseTargetDesc")} {t("u3SortLimit")}</p>
          </details>
        </div>
        <div className="browse-mode-actions">
          <button className="browse-action" type="button" onClick={source.openLocationPicker}>
            <MapPin size={15} aria-hidden="true" />
            {t("browseLocationsButton")}
          </button>
          <button
            className="browse-action"
            type="button"
            disabled={source.enumerationState === "loading"
              || source.enumerationState === "loading_more"
              || source.changeState === "checking"
              || source.changeState === "refreshing"}
            aria-label={t("browseRefresh")}
            onClick={() => void source.refreshEnumeration()}
          >
            <RefreshCw size={15} aria-hidden="true" />
            {t("browseRefresh")}
          </button>
        </div>
      </header>

      {breadcrumbHost ? createPortal(breadcrumbNavigation, breadcrumbHost) : <div className="browse-mode-toolbar">
        {breadcrumbNavigation}
        <span className="browse-selection-summary">{selectionText}</span>
      </div>}

      <div className={cn("browse-mode-body", contextOpen && contextProjection.kind !== "none" && "has-context")}>
        <div className="file-library-browse-context-layout">
          {source.skippedEntries > 0 ? <NoticeBanner tone="warning" title={t("u3Incomplete")}>{t("u3Skipped").replace("{count}", String(source.skippedEntries))}</NoticeBanner> : null}
          {source.enumerationError ? (
            <StateBlock
              tone="error"
              title={t("browseEnumerationFailedTitle")}
              description={source.query.sort ? t("u3SortFailure") : t("browseEnumerationFailedDesc")}
              primaryAction={(
                <button className="browse-action" type="button" onClick={() => void source.refreshEnumeration()}>
                  <RefreshCw size={15} aria-hidden="true" />
                  {t("browseRefresh")}
                </button>
              )}
            />
          ) : source.enumerationState === "loading" && source.entries.length === 0 ? (
            <StateBlock
              tone="info"
              title={t("browseEnumerationLoading")}
              description={t("fileLibraryBrowseTargetDesc")}
              primaryAction={<LoaderCircle className="animate-spin" size={18} aria-label={t("browseEnumerationLoading")} />}
            />
          ) : source.enumerationState === "complete" && source.entries.length === 0 ? (
            <StateBlock
              title={t("browseEnumerationEmptyTitle")}
              description={t("browseEnumerationEmptyDesc")}
            />
          ) : (
            <section className="browse-results-panel" aria-label={t("browseCurrentFolder")}>
              {source.locationError ? (
                <NoticeBanner tone="warning" title={t("browseLocationUnavailable")}>
                  {t("browseLocationsDesc")}
                </NoticeBanner>
              ) : null}
              <div className="browse-results-status" role="status" aria-live="polite" data-browse-enumeration-status="true">
                {[changeStatusText ?? statusText, selectionText].filter(Boolean).join(" · ")}
              </div>
              {viewMode === "grid" ? <SharedFileGrid
                interaction={interaction}
                language={language}
                t={t}
                controller={controller.workspace}
                ariaLabel={t("browseCurrentFolder")}
                emptyLabel={emptyLabel}
                loadMoreLabel={t("browseLoadMore")}
                loadingMoreLabel={t("browseEnumerationLoadingMore")}
                onEnter={(entry) => {
                  if (entry.source === "browse") source.navigateInto(entry);
                }}
                onPreview={(entry, trigger, event) => entry.source === "browse"
                  ? previewController.handleSpace(
                    previewSourceFromEntry(entry, source.collection),
                    trigger,
                    event
                  )
                  : false}
                onActivate={(entry) => {
                  if (entry.source === "browse") source.navigateInto(entry);
                }}
                onContextMenu={(event, entry) => {
                  if (entry.source === "browse") handleRowContextMenu(event, entry);
                }}
                onOpenContextMenu={openFocusedContextMenu}
                onEscape={handleListEscape}
              /> : <SharedFileList
                interaction={interaction}
                language={language}
                t={t}
                ariaLabel={t("browseCurrentFolder")}
                emptyLabel={emptyLabel}
                loadMoreLabel={t("browseLoadMore")}
                loadingMoreLabel={t("browseEnumerationLoadingMore")}
                onEnter={(entry) => {
                  if (entry.source === "browse") source.navigateInto(entry);
                }}
                onPreview={(entry, trigger, event) => entry.source === "browse"
                  ? previewController.handleSpace(
                    previewSourceFromEntry(entry, source.collection),
                    trigger,
                    event
                  )
                  : false}
                onActivate={(entry) => {
                  if (entry.source === "browse") source.navigateInto(entry);
                }}
                onContextMenu={(event, entry) => {
                  if (entry.source === "browse") handleRowContextMenu(event, entry);
                }}
                onOpenContextMenu={openFocusedContextMenu}
                onEscape={handleListEscape}
              />}
            </section>
          )}
          <ContextPanel
            projection={contextProjection}
            open={contextOpen && (pane?.visible ?? true)}
            onClose={closeContextPanel}
            restoreFocus={restoreContextFocus}
          />
          {contextMenu ? (
            <FileLibraryContextMenu
              x={contextMenu.x}
              y={contextMenu.y}
              title={contextMenu.entry.displayName}
              ariaLabel={t("browseContextMenu")}
              items={browseContextItems}
              onClose={() => closeContextMenu("action")}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}

export function BrowseLocationPicker({
  detached,
  source,
  t
}: {
  detached: boolean;
  source: ReturnType<typeof useBrowseSourceOwner>;
  t: ReturnType<typeof useI18nContext>["t"];
}) {
  const showLoading = source.locationState === "loading" && source.locations.length === 0;
  const showFailed = source.locationState === "failed" && source.locations.length === 0;
  const showEmpty = source.locationState === "ready" && source.locations.length === 0;

  return (
    <div className="browse-mode" data-browse-source-owner="browse" data-browse-state="locations">
      <header className="browse-mode-header">
        <div className="browse-location-intro min-w-0">
          <h2 className="browse-mode-title truncate text-base font-semibold">{detached ? t("fileLibraryBrowseDetachedTitle") : t("browseLocationsTitle")}</h2>
          <p className="browse-mode-description">
            {detached ? t("fileLibraryBrowseDetachedDesc") : t("browseLocationsDesc")}
          </p>
        </div>
        <div className="flex flex-wrap items-center justify-end gap-2">
          <button
            className="browse-action"
            type="button"
            disabled={source.admissionLoading}
            data-browse-choose-folder="true"
            onClick={() => void source.chooseFolder()}
          >
            <FolderOpen size={15} aria-hidden="true" />
            {t("browseChooseFolder")}
          </button>
          <button
            className="browse-action"
            type="button"
            disabled={source.locationState === "loading" || source.admissionLoading}
            onClick={() => void source.loadLocations()}
          >
            <RefreshCw size={15} aria-hidden="true" />
            {t("browseLocationsRetry")}
          </button>
        </div>
      </header>

      {source.locationError ? (
        <NoticeBanner tone="warning" title={t("browseLocationUnavailable")}>
          {t("browseLocationsDesc")}
        </NoticeBanner>
      ) : null}
      {source.admissionError ? (
        <NoticeBanner tone="error" title={t("browseChooseFolderError")}>
          {t("browseChooseFolderErrorDesc")}
        </NoticeBanner>
      ) : null}

      <div className="browse-location-panel">
        {showLoading ? (
          <StateBlock
            tone="info"
            title={t("browseLocationsLoading")}
            primaryAction={<LoaderCircle className="animate-spin" size={18} aria-label={t("browseLocationsLoading")} />}
          />
        ) : showFailed ? (
          <StateBlock
            tone="error"
            title={t("browseLocationsEmptyTitle")}
            description={t("browseLocationsEmptyDesc")}
            primaryAction={(
              <button className="browse-action" type="button" onClick={() => void source.loadLocations()}>
                <RefreshCw size={15} aria-hidden="true" />
                {t("browseLocationsRetry")}
              </button>
            )}
          />
        ) : showEmpty ? (
          <StateBlock
            title={t("browseLocationsEmptyTitle")}
            description={t("browseLocationsEmptyDesc")}
          />
        ) : (
          <div className="browse-location-grid" data-browse-locations="true">
            {source.locations.map((location) => (
              <BrowseLocationCard key={locationKey(location)} location={location} source={source} t={t} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function BrowseLocationCard({
  location,
  source,
  t
}: {
  location: LocationDescriptor;
  source: ReturnType<typeof useBrowseSourceOwner>;
  t: ReturnType<typeof useI18nContext>["t"];
}) {
  const activatable = isActivatableLocation(location);
  const statusLabel = locationAvailabilityLabel(location.availability, t);
  const opening = source.admissionLoading && activatable;
  return (
    <article
      className={`browse-location-card${activatable ? "" : " is-unavailable"}`}
      data-browse-location="true"
      data-browse-location-availability={location.availability}
      data-browse-location-openable={activatable ? "true" : "false"}
    >
      <div className="browse-location-card-header">
        <span className="browse-location-card-icon" aria-hidden="true"><Folder size={17} /></span>
        <div className="min-w-0">
          <strong className="browse-location-title">{location.displayName}</strong>
          <span className={`browse-location-status${activatable ? " is-ready" : " is-unavailable"}`}>
            <span aria-hidden="true">•</span>
            {statusLabel}
          </span>
        </div>
      </div>
      <div className="browse-location-actions">
        <button
          className="browse-location-open"
          type="button"
          disabled={!activatable || source.admissionLoading}
          data-browse-location-action="open"
          onClick={() => void source.activateLocation(location)}
        >
          {opening ? <LoaderCircle className="animate-spin" size={15} aria-hidden="true" /> : null}
          {opening ? t("browseLocationOpening") : t("browseLocationOpen")}
        </button>
      </div>
    </article>
  );
}

function locationKey(location: LocationDescriptor): string {
  return location.ref.kind === "managed"
    ? `managed:${location.ref.scanRootId}`
    : `ephemeral:${location.ref.browseSessionId}:${location.ref.locationId}`;
}
