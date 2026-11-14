import { FileLibraryWorkbench } from "./workbench/FileLibraryWorkbench";
import { PathBar } from "./workbench/PathBar";
import { usePane, usePaneQuerySelector } from "./workbench/PaneContext";
import { usePreviewExperience } from "./preview/PreviewExperienceProvider";
import { ArrowLeft, ArrowRight, ChevronRight, Columns3, Grid2X2, List, PanelLeftClose, PanelLeftOpen, PanelRightClose, PanelRightOpen, SlidersHorizontal } from "lucide-react";
import { lazy, useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { useChromeContext, useI18nContext } from "../../contexts/AppContexts";
import type { WorkspaceViewMode } from "../../fileWorkspace";
import { useFileLibraryExperience } from "./FileLibraryExperienceProvider";
import { isManagedLibraryBrowserFixture, type FileLibraryMode } from "./fileLibraryExperience";
import { ContextPanelPresentationProvider } from "./context/contextPanelPresentation";
import { LibraryNavigationSurfaceProvider } from "./library/libraryNavigationSurface";
import { FileLibraryNavigation } from "./navigation/FileLibraryNavigation";
import { isFileLibraryShortcutExcludedTarget } from "./fileLibraryInteraction";
import { PreviewExperienceProvider } from "./preview/PreviewExperienceProvider";
import { ZenFloatingQuickPreview } from "./preview/ZenFloatingQuickPreview";
import { ZenPinnedPreview } from "./preview/ZenPinnedPreview";
import {
  FileLibraryCommandBarSurfaceProvider,
  type FileLibraryCommandBarSurface,
  type FileLibraryCommandBarSurfaceOwner
} from "./fileLibraryCommandBarSurface";
import { FileManagementToolbar } from "./management/FileManagementToolbar";
import "./fileLibraryWorkspace.css";
import "./fileLibraryV340.css";

const LibraryMode = lazy(() => import("./library/LibraryMode").then((module) => ({ default: module.LibraryMode })));
const BrowseMode = lazy(() => import("./browse/BrowseMode").then((module) => ({ default: module.BrowseMode })));

export type FileLibraryLayout = "large" | "medium" | "compact";

export const layoutForWidth = (width: number): FileLibraryLayout => {
  if (width >= 1120) return "large";
  if (width >= 820) return "medium";
  return "compact";
};

const LIBRARY_TREE_PREFERENCE_KEY = "fylura:file-tree-open";
const LIBRARY_TREE_WIDTH_KEY = "fylura:file-tree-width";
const TREE_WIDTH_DEFAULT = 196;
const TREE_WIDTH_MIN = 168;
const TREE_WIDTH_MAX = 340;

export function clampTreeWidth(width: number) {
  if (!Number.isFinite(width)) return TREE_WIDTH_DEFAULT;
  return Math.min(TREE_WIDTH_MAX, Math.max(TREE_WIDTH_MIN, Math.round(width)));
}

function readTreeWidthPreference() {
  try {
    const stored = Number(window.localStorage.getItem(LIBRARY_TREE_WIDTH_KEY));
    return stored > 0 ? clampTreeWidth(stored) : TREE_WIDTH_DEFAULT;
  } catch {
    return TREE_WIDTH_DEFAULT;
  }
}

function readTreeOpenPreference() {
  try {
    return window.localStorage.getItem(LIBRARY_TREE_PREFERENCE_KEY) !== "false";
  } catch {
    return true;
  }
}

export function FileLibraryWorkspace({ active = true }: { active?: boolean }) { return <FileLibraryWorkbench Pane={FileLibraryPane} active={active}/>; }

function FileLibraryPane() {
  const pane = usePane();
  const query = usePaneQuerySelector();
  const { controller, state } = useFileLibraryExperience();
  const { t } = useI18nContext();
  const { platform } = useChromeContext();
  const workspaceRef = useRef<HTMLDivElement | null>(null);
  const navigationToggleRef = useRef<HTMLButtonElement | null>(null);
  const [layout, setLayout] = useState<FileLibraryLayout>("compact");
  const [navigationOpen, setNavigationOpen] = useState(false);
  const [treeOpen, setTreeOpen] = useState(readTreeOpenPreference);
  const [treeWidth, setTreeWidth] = useState(readTreeWidthPreference);
  const [commandBarSurface, setCommandBarSurface] = useState<FileLibraryCommandBarSurface | null>(null);

  const registerSurface = useCallback((surface: FileLibraryCommandBarSurface) => {
    setCommandBarSurface((current) => current?.owner === surface.owner
      && current.search === surface.search
      && current.actions === surface.actions
      && current.enabled === surface.enabled
      ? current
      : surface);
  }, []);
  const clearSurface = useCallback((owner: FileLibraryCommandBarSurfaceOwner) => {
    setCommandBarSurface((current) => current?.owner === owner ? null : current);
  }, []);

  // Measured before paint: the first frame must already know whether the tree
  // is an inline column or a drawer, otherwise the toggle briefly reports the
  // wrong state and can be clicked into the wrong branch.
  useLayoutEffect(() => {
    const element = workspaceRef.current;
    if (!element) return;

    const updateLayout = () => setLayout(layoutForWidth(element.clientWidth));
    updateLayout();
    if (typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver(updateLayout);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const history = state.workspace.session;
  const contextOpen = history.presentation.contextOpen === true;
  const viewMode = history.presentation.viewMode ?? "list";
  const targetLabel = state.mode === "library"
    ? t("fileLibraryModeLibrary")
    : state.workspace.browse?.location.displayName ?? t("fileLibraryNoFolderOpen");

  // Finder-style columns are a browsing mode, not a per-folder preference:
  // after navigating into a folder the new history entry starts empty, so the
  // pane re-applies the mode the user last chose.
  const columnModeSticky = useRef(false);
  const changeViewMode = useCallback((nextViewMode: WorkspaceViewMode) => {
    columnModeSticky.current = nextViewMode === "columns";
    controller.setViewMode(nextViewMode);
  }, [controller]);
  useEffect(() => {
    if (columnModeSticky.current && viewMode !== "columns") controller.setViewMode("columns");
  }, [controller, viewMode]);

  const closeNavigation = useCallback(() => setNavigationOpen(false), []);
  /**
   * The directory tree has two presentations: an inline 196px column on wide
   * layouts and a drawer below that. They keep separate state so resizing does
   * not silently open a drawer, and the inline column's choice is remembered.
   */
  const treeInline = layout === "large" && treeOpen;
  const navigationVisible = layout === "large" ? treeOpen : navigationOpen;
  const toggleNavigation = useCallback(() => {
    if (layout === "large") {
      setTreeOpen((open) => {
        const next = !open;
        try { window.localStorage.setItem(LIBRARY_TREE_PREFERENCE_KEY, String(next)); } catch { /* optional preference */ }
        return next;
      });
      return;
    }
    const nextOpen = !navigationOpen;
    // Drawer presentation may only own one overlay: opening the tree gives the
    // inspector back to the layout rather than stacking two overlays.
    if (nextOpen && contextOpen) controller.setContextOpen(false);
    setNavigationOpen(nextOpen);
  }, [contextOpen, controller, layout, navigationOpen]);
  /** Live width during a drag: state only, so pointer moves stay cheap. */
  const previewTreeWidth = useCallback((width: number) => {
    setTreeWidth(clampTreeWidth(width));
  }, []);
  /** Committed width: the release, the keyboard step or the reset. */
  const commitTreeWidth = useCallback((width: number) => {
    const next = clampTreeWidth(width);
    setTreeWidth(next);
    try { window.localStorage.setItem(LIBRARY_TREE_WIDTH_KEY, String(next)); } catch { /* optional preference */ }
  }, []);
  const toggleContext = useCallback(() => {
    const nextOpen = !contextOpen;
    if (nextOpen && layout !== "large") setNavigationOpen(false);
    controller.setContextOpen(nextOpen);
  }, [contextOpen, controller, layout]);

  const handoffPinnedPreview = useCallback(() => {
    setNavigationOpen(false);
    controller.setContextOpen(false);
    return true;
  }, [controller]);

  useEffect(() => {
    // A resize can move both inline surfaces into modal presentation without
    // a new click. Keep one File Library overlay owner in that transition.
    if (layout !== "large" && navigationOpen && contextOpen) controller.setContextOpen(false);
  }, [contextOpen, controller, layout, navigationOpen]);

  useEffect(() => {
    const handleLocalSearchShortcut = (event: globalThis.KeyboardEvent) => {
      if (!pane || pane.focused && pane.visible) focusLocalFileLibrarySearch(event, commandBarSurface);
    };
    window.addEventListener("keydown", handleLocalSearchShortcut);
    return () => window.removeEventListener("keydown", handleLocalSearchShortcut);
  }, [commandBarSurface, pane?.focused, pane?.visible]);

  return (
    <LibraryNavigationSurfaceProvider>
      <FileLibraryCommandBarSurfaceProvider value={{ registerSurface, clearSurface }}>
        <PreviewExperienceProvider
          workspace={controller.workspace}
          onPinHandoff={handoffPinnedPreview}
          prepareOpen={() => {
            setNavigationOpen(false);
            controller.setContextOpen(false);
          }}
        >
          <div
            ref={workspaceRef}
            className="file-library-workspace"
            data-layout={layout}
            data-mode={state.mode}
            data-detached-browse={state.detachedBrowse ? "true" : "false"}
            data-context-open={contextOpen ? "true" : "false"}
            data-file-library-navigation-open={navigationVisible ? "true" : "false"}
            data-tree-inline={treeInline ? "true" : "false"}
            style={treeInline ? { "--file-tree-width": `${treeWidth}px` } as React.CSSProperties : undefined}
          >
            <div className="file-library-location-row">
            <PathBar />
            <WorkspaceCommandBar
              fixtureControls={isManagedLibraryBrowserFixture() && <div className="file-library-mode-switch" role="tablist" aria-label="Managed Library compatibility fixture">
              {(["library", "browse"] as const).map((mode) => <button className="file-library-mode-button" key={mode} role="tab" aria-selected={state.mode === mode} data-file-library-mode={mode} onClick={() => void controller.switchMode(mode)}>{mode === "library" ? t("fileLibraryModeLibrary") : t("fileLibraryModeBrowse")}</button>)}
            </div>}
              mode={state.mode}
              targetLabel={targetLabel}
              canGoBack={history.historyIndex > 0}
              canGoForward={history.historyIndex >= 0 && history.historyIndex < history.history.length - 1}
              onBack={() => void controller.back()}
              onForward={() => void controller.forward()}
              navigationOpen={navigationVisible}
              onNavigationToggle={toggleNavigation}
              navigationToggleRef={navigationToggleRef}
              localSearch={commandBarSurface?.search}
              sourceActions={commandBarSurface?.actions}
              contextOpen={contextOpen}
              onContextToggle={toggleContext}
              viewMode={viewMode}
              onViewModeChange={changeViewMode}
              platform={platform}
              t={t}
            />
            </div>

            <FileManagementToolbar />
            <ContextPanelPresentationProvider
              layout={layout}
              paneOverlay={pane?.wideMultiPane === true}
            >
              <div className="file-library-workspace-body">
                <aside
                  id={pane ? `file-library-navigation-slot-${pane.id}` : "file-library-navigation-slot"}
                  className="file-library-navigation-slot"
                  data-workspace-slot="navigation"
                  aria-hidden={!navigationVisible}
                >
                  {navigationVisible && (pane?.visible ?? true) ? (
                    <FileLibraryNavigation
                      controller={controller}
                      state={state}
                      layout={layout === "large" ? "large" : "drawer"}
                      platform={platform}
                      t={t}
                      onClose={closeNavigation}
                    />
                  ) : null}
                  {treeInline ? (
                    <TreeResizeHandle
                      width={treeWidth}
                      onPreview={previewTreeWidth}
                      onCommit={commitTreeWidth}
                      label={t("fileLibraryTreeResize")}
                    />
                  ) : null}
                </aside>
                {layout === "large" && !treeOpen ? (
                  <button
                    type="button"
                    className="file-library-tree-handle"
                    onClick={toggleNavigation}
                    aria-label={t("fileLibraryNavigationOpen")}
                    title={t("fileLibraryNavigationOpen")}
                  >
                    <ChevronRight size={12} aria-hidden="true" />
                  </button>
                ) : null}

                <main className="file-library-content-slot" data-workspace-slot="content">
                  {state.mode === "library"
                    ? <LibrarySourceSlot />
                    : <BrowseMode />}
                </main>

                <aside
                  className="file-library-context-slot"
                  data-workspace-slot="context"
                  aria-hidden="true"
                />
              </div>
            </ContextPanelPresentationProvider>
          </div>
          <PanePreviewLifecycle />
          <ZenFloatingQuickPreview />
          <ZenPinnedPreview />
        </PreviewExperienceProvider>
      </FileLibraryCommandBarSurfaceProvider>
    </LibraryNavigationSurfaceProvider>
  );
}

/**
 * Pointer-captured column resizer. Dragging updates the width live, releasing
 * commits it to the stored preference; keyboard and double-click provide the
 * same reach without a pointer.
 */
function TreeResizeHandle({
  width,
  onPreview,
  onCommit,
  label
}: {
  width: number;
  onPreview: (width: number) => void;
  onCommit: (width: number) => void;
  label: string;
}) {
  const drag = useRef<{ x: number; width: number } | null>(null);
  // The last previewed width also lives in a ref so a pointerup that arrives in
  // the same task as the final pointermove still commits the newest value.
  const latest = useRef(width);
  latest.current = width;
  const endDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    drag.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={TREE_WIDTH_MIN}
      aria-valuemax={TREE_WIDTH_MAX}
      tabIndex={0}
      className="file-library-tree-resizer"
      onPointerDown={(event) => {
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { x: event.clientX, width };
      }}
      onPointerMove={(event) => {
        if (!drag.current) return;
        const next = drag.current.width + (event.clientX - drag.current.x);
        latest.current = next;
        onPreview(next);
      }}
      onPointerUp={(event) => {
        if (!drag.current) return;
        const committed = latest.current;
        endDrag(event);
        onCommit(committed);
      }}
      // A cancelled gesture (Escape, OS interruption) must not keep the
      // previewed width: restore the width the drag started from.
      onPointerCancel={(event) => {
        if (!drag.current) return;
        const original = drag.current.width;
        endDrag(event);
        onPreview(original);
      }}
      onLostPointerCapture={() => {
        if (!drag.current) return;
        const original = drag.current.width;
        drag.current = null;
        onPreview(original);
      }}
      onDoubleClick={() => onCommit(TREE_WIDTH_DEFAULT)}
      onKeyDown={(event) => {
        const step = event.shiftKey ? 24 : 8;
        if (event.key === "ArrowLeft") { event.preventDefault(); onCommit(width - step); }
        else if (event.key === "ArrowRight") { event.preventDefault(); onCommit(width + step); }
        else if (event.key === "Home") { event.preventDefault(); onCommit(TREE_WIDTH_DEFAULT); }
      }}
    />
  );
}

export function focusLocalFileLibrarySearch(
  event: globalThis.KeyboardEvent,
  surface: Pick<FileLibraryCommandBarSurface, "enabled" | "searchInputRef"> | null
) {
  if (event.isComposing || event.defaultPrevented) return false;
  if (event.altKey || isFileLibraryShortcutExcludedTarget(event.target)) return false;
  if (!(event.metaKey || event.ctrlKey) || event.key.toLowerCase() !== "f") return false;
  const input = surface?.enabled ? surface.searchInputRef.current : null;
  if (!input) return false;
  event.preventDefault();
  input.focus();
  input.select();
  return true;
}

type WorkspaceCommandBarProps = {
  fixtureControls?: ReactNode;
  /** Kept for the internal Library/Browse projection; no user-visible switch. */
  mode: FileLibraryMode;
  targetLabel: string;
  canGoBack: boolean;
  canGoForward: boolean;
  onBack: () => void;
  onForward: () => void;
  navigationOpen?: boolean;
  onNavigationToggle?: () => void;
  navigationToggleRef?: RefObject<HTMLButtonElement | null>;
  localSearch?: ReactNode;
  sourceActions?: ReactNode;
  viewMode?: WorkspaceViewMode;
  onViewModeChange?: (viewMode: WorkspaceViewMode) => void;
  /** Passed in rather than read from context so the bar stays renderable in
   * isolation; the column mode is a macOS browsing form. */
  platform?: string;
  contextOpen?: boolean;
  onContextToggle?: () => void;
  t: ReturnType<typeof useI18nContext>["t"];
};

/**
 * ADR-0016: the ordinary Files workspace exposes one user-visible mode.
 * Browse is the only presented content surface, so the previous Library /
 * Browse Folder tablist is intentionally absent. Managed Library keeps its
 * components, backend query authority, selection, tags and Saved Views; it is
 * simply no longer a first-class switch in the normal file manager chrome.
 */
export function WorkspaceCommandBar({
  fixtureControls,
  mode,
  targetLabel,
  canGoBack,
  canGoForward,
  onBack,
  onForward,
  navigationOpen = false,
  onNavigationToggle = () => undefined,
  navigationToggleRef,
  localSearch,
  sourceActions,
  viewMode = "list",
  onViewModeChange,
  platform,
  contextOpen = false,
  onContextToggle = () => undefined,
  t
}: WorkspaceCommandBarProps) {
  const pane = usePane();

  return (
    <div className="file-library-command-bar" role="toolbar" aria-label={t("fileLibraryToolbarLabel")} data-file-library-presented-mode={mode}>
      {fixtureControls}
      <div className="file-library-command-group file-library-history-controls">
        <button
          className="file-library-command-button"
          type="button"
          aria-label={t("fileLibraryBack")}
          disabled={!canGoBack}
          onClick={onBack}
        >
          <ArrowLeft size={15} aria-hidden="true" />
          <span>{t("fileLibraryBack")}</span>
        </button>
        <button
          className="file-library-command-button"
          type="button"
          aria-label={t("fileLibraryForward")}
          disabled={!canGoForward}
          onClick={onForward}
        >
          <ArrowRight size={15} aria-hidden="true" />
          <span>{t("fileLibraryForward")}</span>
        </button>
      </div>

      <button
        ref={navigationToggleRef}
        className="file-library-command-button file-library-nav-toggle"
        type="button"
        aria-label={navigationOpen ? t("fileLibraryNavigationClose") : t("fileLibraryNavigationOpen")}
        aria-expanded={navigationOpen}
        aria-controls={pane ? `file-library-navigation-slot-${pane.id}` : "file-library-navigation-slot"}
        data-file-library-nav-toggle="true"
        onClick={onNavigationToggle}
      >
        {navigationOpen ? <PanelLeftClose size={15} aria-hidden="true" /> : <PanelLeftOpen size={15} aria-hidden="true" />}
        <span>{t("fileLibraryNavigationLabel")}</span>
      </button>
      <div className="file-library-command-target" title={targetLabel} aria-label={targetLabel}>
        <span className="file-library-command-target-label">{targetLabel}</span>
      </div>

      {localSearch ? <div className="file-library-command-search" data-file-library-command-search="true">{localSearch}</div> : null}
      {sourceActions ? mode === "library" ? <div className="file-library-command-actions" data-file-library-source-actions="true">{sourceActions}</div> : <details className="file-library-query-menu" data-file-library-query-menu onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
      }} onKeyDown={(event) => {
        if (event.key === "Escape" && !event.defaultPrevented) {
          event.preventDefault(); event.currentTarget.open = false;
          event.currentTarget.querySelector("summary")?.focus();
        }
      }}>
        <summary className="file-library-command-button" aria-label={t("fileLibrarySortFilter")} title={t("fileLibrarySortFilter")}><SlidersHorizontal size={15} aria-hidden="true" /></summary>
        <div className="file-library-command-actions" data-file-library-source-actions="true">{sourceActions}</div>
      </details> : null}

      <div className="file-library-view-switch" role="group" aria-label={t("fileLibraryViewModeLabel")} data-file-library-view-mode={viewMode}>
        <button
          className="file-library-command-button"
          type="button"
          aria-label={t("fileLibraryViewList")}
          aria-pressed={viewMode === "list"}
          data-file-library-view="list"
          onClick={() => onViewModeChange?.("list")}
        >
          <List size={15} aria-hidden="true" />
          <span>{t("fileLibraryViewList")}</span>
        </button>
        <button
          className="file-library-command-button"
          type="button"
          aria-label={t("fileLibraryViewGrid")}
          aria-pressed={viewMode === "grid"}
          data-file-library-view="grid"
          onClick={() => onViewModeChange?.("grid")}
        >
          <Grid2X2 size={15} aria-hidden="true" />
          <span>{t("fileLibraryViewGrid")}</span>
        </button>
        {/* Finder-style columns are a macOS browsing form, not a replacement
         * for independent panes: the pane keeps its own navigation stack. */}
        {platform === "darwin" ? (
          <button
            className="file-library-command-button"
            type="button"
            aria-label={t("fileLibraryViewColumns")}
            aria-pressed={viewMode === "columns"}
            data-file-library-view="columns"
            onClick={() => onViewModeChange?.("columns")}
          >
            <Columns3 size={15} aria-hidden="true" />
            <span>{t("fileLibraryViewColumns")}</span>
          </button>
        ) : null}
      </div>

      <button
        className="file-library-command-button file-library-context-toggle"
        type="button"
        aria-label={contextOpen ? t("fileLibraryContextClose") : t("fileLibraryContextOpen")}
        aria-expanded={contextOpen}
        aria-pressed={contextOpen}
        data-file-library-context-toggle="true"
        onClick={onContextToggle}
      >
        {contextOpen ? <PanelRightClose size={15} aria-hidden="true" /> : <PanelRightOpen size={15} aria-hidden="true" />}
        <span>{t("fileLibraryContextLabel")}</span>
      </button>
    </div>
  );
}

function LibrarySourceSlot() {
  return (
    <div className="file-library-library-adapter" data-library-migration-adapter="library-source-owner" data-library-source-slot="library">
      <LibraryMode />
    </div>
  );
}

function PanePreviewLifecycle() {
  const pane = usePane(); const { controller } = usePreviewExperience();
  const register = pane?.registerPreview;
  useEffect(() => { register?.(controller); return () => register?.(null); }, [controller, register]);
  useEffect(() => { if (pane && !pane.focused) controller.close("escape"); }, [controller, pane?.focused]);
  return null;
}
