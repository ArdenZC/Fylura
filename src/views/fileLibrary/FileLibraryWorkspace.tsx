import { FileLibraryWorkbench } from "./workbench/FileLibraryWorkbench";
import { PathBar } from "./workbench/PathBar";
import { usePane, usePaneQuerySelector } from "./workbench/PaneContext";
import { usePreviewExperience } from "./preview/PreviewExperienceProvider";
import { ArrowLeft, ArrowRight, Grid2X2, List, PanelLeftClose, PanelLeftOpen, PanelRightClose, PanelRightOpen } from "lucide-react";
import { lazy, useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
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

const LibraryMode = lazy(() => import("./library/LibraryMode").then((module) => ({ default: module.LibraryMode })));
const BrowseMode = lazy(() => import("./browse/BrowseMode").then((module) => ({ default: module.BrowseMode })));

export type FileLibraryLayout = "large" | "medium" | "compact";

export const layoutForWidth = (width: number): FileLibraryLayout => {
  if (width >= 1120) return "large";
  if (width >= 820) return "medium";
  return "compact";
};

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

  useEffect(() => {
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

  const closeNavigation = useCallback(() => setNavigationOpen(false), []);
  const toggleNavigation = useCallback(() => {
    const nextOpen = !navigationOpen;
    if (nextOpen && layout !== "large" && contextOpen) controller.setContextOpen(false);
    setNavigationOpen(nextOpen);
  }, [contextOpen, controller, layout, navigationOpen]);
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
            data-file-library-navigation-open={navigationOpen ? "true" : "false"}
          >
            {isManagedLibraryBrowserFixture() && <div role="tablist" aria-label="Managed Library compatibility fixture">
              {(["library", "browse"] as const).map((mode) => <button key={mode} role="tab" aria-selected={state.mode === mode} data-file-library-mode={mode} onClick={() => void controller.switchMode(mode)}>{mode === "library" ? t("fileLibraryModeLibrary") : t("fileLibraryModeBrowse")}</button>)}
            </div>}
            <PathBar />
            <WorkspaceCommandBar
              mode={state.mode}
              targetLabel={targetLabel}
              canGoBack={history.historyIndex > 0}
              canGoForward={history.historyIndex >= 0 && history.historyIndex < history.history.length - 1}
              onBack={() => void controller.back()}
              onForward={() => void controller.forward()}
              navigationOpen={navigationOpen}
              onNavigationToggle={toggleNavigation}
              navigationToggleRef={navigationToggleRef}
              localSearch={commandBarSurface?.search}
              sourceActions={commandBarSurface?.actions}
              contextOpen={contextOpen}
              onContextToggle={toggleContext}
              viewMode={viewMode}
              onViewModeChange={(nextViewMode) => controller.setViewMode(nextViewMode)}
              t={t}
            />

            <FileManagementToolbar />
            <ContextPanelPresentationProvider
              layout={layout}
              paneOverlay={pane?.wideDualPane === true}
            >
              <div className="file-library-workspace-body">
                <aside
                  id={pane ? `file-library-navigation-slot-${pane.id}` : "file-library-navigation-slot"}
                  className="file-library-navigation-slot"
                  data-workspace-slot="navigation"
                  aria-hidden={!navigationOpen}
                >
                  {navigationOpen && (pane?.visible ?? true) ? (
                    <FileLibraryNavigation
                      controller={controller}
                      state={state}
                      layout={layout === "large" ? "large" : "drawer"}
                      platform={platform}
                      t={t}
                      onClose={closeNavigation}
                    />
                  ) : null}
                </aside>

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
  contextOpen = false,
  onContextToggle = () => undefined,
  t
}: WorkspaceCommandBarProps) {
  const pane = usePane();

  return (
    <div className="file-library-command-bar" role="toolbar" aria-label={t("fileLibraryToolbarLabel")} data-file-library-presented-mode={mode}>
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
      {sourceActions ? <div className="file-library-command-actions" data-file-library-source-actions="true">{sourceActions}</div> : null}

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
