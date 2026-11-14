import {
  Brush,
  Clock3,
  Cloud,
  Cpu,
  House,
  Library,
  LoaderCircle,
  LockKeyhole,
  Minus,
  PanelLeft,
  PanelLeftClose,
  PanelLeftOpen,
  Radar,
  Search,
  Settings,
  Sparkles,
  Square,
  TriangleAlert,
  X
} from "lucide-react";
import { lazy, memo, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { tauriApi } from "../api/tauriApi";
import { CommandModal } from "./CommandModal";
import { OnboardingDialog } from "./OnboardingDialog";
import { ViewErrorBoundary } from "./ErrorBoundary";
import { AmbientMesh, CloseChoiceDialog, WorkspaceTopActions } from "./ShellChrome";
import { BrandMark } from "./ui/BrandMark";
import { PressMotion } from "./motion/PressMotion";
import { playPageEnter } from "../utils/motion";
import { requestSettingsSection } from "./spotlight/commandRegistry";
import { useCommandContext, useI18nContext, useNavigationContext, useWindowContext } from "../contexts/AppContexts";
import { useAppStore } from "../store/useAppStore";
import { useWatcherStatusStore } from "../store/useWatcherStatusStore";
import { deriveWatcherPresentation, watcherPresentationNeedsAttention } from "../utils/watcherPresentation";
import { useFileLibraryStore } from "../store/useFileLibraryStore";
import { useFileLibraryInspectorStore, useFileLibrarySelectionStore } from "../store/useFileLibraryV2Store";
import { organizationPlanPendingReview, useOrganizationPlanStore } from "../store/useOrganizationPlanStore";
import { useOperationQueueStore } from "../store/useOperationQueueStore";
import { useBackgroundIndexerStore } from "../store/useBackgroundIndexerStore";
import { resolveAIProcessingMode, useAIProcessingModeStore, type AIProcessingModeState } from "../store/useAIProcessingModeStore";
import type { DashboardStats, LibraryScope } from "../types/domain";
import type { Translator, View } from "../types/ui";
import { formatDate } from "../utils/format";
import { cn, focusVisibleState, statusToast, toastTone } from "../utils/tw";
import { libraryScopeLabel, readableError } from "../utils/viewHelpers";
import { projectAcceptedFileLibraryActivation } from "../utils/fileLibraryActivation";
import { useSearchNavigationHandoff } from "../hooks/useSearchNavigationHandoff";
import { FileLibraryExperienceProvider, useFileLibraryExperience } from "../views/fileLibrary/FileLibraryExperienceProvider";
import { PageHeader, pageFrame, softPanel, viewStage } from "../views/shared/ui";
import { APP_SHELL_CONTENT_ID, ModalHost } from "./modal/ModalPortal";

const ScannerView = lazy(() => import("../views/scanner/ScannerView").then((module) => ({ default: module.ScannerView })));
const StorageCleanupView = lazy(() => import("../views/cleanup/StorageCleanupView").then((module) => ({ default: module.StorageCleanupView })));
const OrganizeSuggestionsView = lazy(() => import("../views/organize/OrganizeSuggestionsView").then((module) => ({ default: module.OrganizeSuggestionsView })));
const FileLibraryWorkspace = lazy(() => import("../views/fileLibrary/FileLibraryWorkspace").then((module) => ({ default: module.FileLibraryWorkspace })));
const TimelineView = lazy(() => import("../views/timeline/TimelineView").then((module) => ({ default: module.TimelineView })));
const AutomationWorkspace = lazy(() => import("../views/automation/AutomationWorkspace").then((module) => ({ default: module.AutomationWorkspace })));
const RestoreView = lazy(() => import("../views/restore/RestoreView").then((module) => ({ default: module.RestoreView })));
const SettingsView = lazy(() => import("../views/settings/SettingsView").then((module) => ({ default: module.SettingsView })));

const appRoot = cn(pageFrame, "zc-app-frame");
const searchWindowRoot =
  "relative h-full w-full overflow-hidden bg-transparent text-[var(--zc-text-primary)]";
const noDrag = "[-webkit-app-region:no-drag]";
const viewStageClass = viewStage;

/**
 * Shared presentation hooks. Kept as named constants so the shell retheming
 * stays a one-line change and the design contract remains inspectable.
 */
const spotlightButton = "zc-titlebar-search";
export const windowsControlButton = "zc-window-control";
export const windowsCloseButton = "zc-window-control zc-window-control-close";

function sidebarWindowAction(action: "close" | "minimize" | "maximize") {
  return action === "close" ? windowsCloseButton : windowsControlButton;
}

type NavItemId = View | "global-search";
type NavItem = { id: NavItemId; label: string; icon: typeof Radar; action?: "open_search" };
type NavGroup = { id: "primary" | "workflow" | "advanced"; label: string; items: NavItem[] };

/**
 * Three-state sidebar: expanded (216), icon rail (64) and hidden.
 *
 * The toggle walks expanded → rail → hidden → expanded so every state stays
 * reachable from one predictable control, and its label always states what
 * the next click does.
 */
export type SidebarMode = "expanded" | "rail" | "hidden";

export function resolveSidebarMode(hidden: boolean, rail: boolean): SidebarMode {
  if (hidden) return "hidden";
  return rail ? "rail" : "expanded";
}

export function nextSidebarMode(mode: SidebarMode): SidebarMode {
  if (mode === "expanded") return "rail";
  if (mode === "rail") return "hidden";
  return "expanded";
}

export function sidebarToggleLabel(mode: SidebarMode, t: Translator) {
  if (mode === "expanded") return t("shellCollapseSidebar");
  if (mode === "rail") return t("shellHideSidebar");
  return t("shellShowSidebar");
}

/**
 * Narrow windows converge to the icon rail: the global navigation stays
 * reachable without eating the file list's width. The stored preference is not
 * overwritten, so widening the window restores the user's own choice.
 */
const NARROW_RAIL_QUERY = "(max-width: 1180px)";

export function useNarrowRail() {
  const [narrow, setNarrow] = useState(() => {
    try { return window.matchMedia(NARROW_RAIL_QUERY).matches; } catch { return false; }
  });
  useEffect(() => {
    let query: MediaQueryList;
    try { query = window.matchMedia(NARROW_RAIL_QUERY); } catch { return; }
    const update = () => setNarrow(query.matches);
    update();
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, []);
  return narrow;
}

export function AppShell() {
  const { isSearchMode, setIsCommandOpen, hotkeyLabel, isCommandOpen } = useCommandContext();
  const { isCloseChoiceOpen, onCancelCloseChoice, requestClose, resolveCloseChoice } = useWindowContext();
  const { view } = useNavigationContext();
  const [hasOpenedLibrary, setHasOpenedLibrary] = useState(view === "library");
  useEffect(() => { if (view === "library") setHasOpenedLibrary(true); }, [view]);
  const viewStageRef = useRef<HTMLDivElement | null>(null);
  const viewMotionReadyRef = useRef(false);
  useLayoutEffect(() => {
    if (!viewMotionReadyRef.current) {
      viewMotionReadyRef.current = true;
      return;
    }
    playPageEnter(viewStageRef.current);
  }, [view]);
  const { t } = useI18nContext();
  const stats = useFileLibraryStore((state) => state.stats);
  const scope = useFileLibraryStore((state) => state.scope);
  const sidebarHidden = useAppStore((state) => state.sidebarHidden);
  const sidebarRail = useAppStore((state) => state.sidebarRail);
  const setSidebarHidden = useAppStore((state) => state.setSidebarHidden);
  const setSidebarRail = useAppStore((state) => state.setSidebarRail);
  const density = useAppStore((state) => state.density);
  const previewActionCount = useOrganizationPlanStore((state) => organizationPlanPendingReview(state.plans, state.activePlan));
  const executionIntent = useOperationQueueStore((state) => state.executionIntent);
  const spotlightTriggerRef = useRef<HTMLButtonElement | null>(null);
  const requestCloseRef = useRef(requestClose);
  requestCloseRef.current = requestClose;

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriApi.onMainWindowCloseRequested(() => requestCloseRef.current()).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch((error) => {
      if (!disposed) console.error("Main window close listener failed", error);
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const groups = useMemo(() => navGroups(t), [t]);
  const activeLabel = view === "automation"
    ? t("automationIntentWorkspaceTitle")
    : groups.flatMap((group) => group.items).find((item) => item.id === view)?.label ?? viewLabel(view, t);
  const scopeText = libraryScopeLabel(scope, t("allIndexedFiles"), t("noFolderSelected"));
  const headingDescription = viewDescription(view, stats, scope, scopeText, view === "preview" && executionIntent?.source === "organize" ? executionIntent.allowedPreviewIds.size : previewActionCount, t);
  const storedSidebarMode = resolveSidebarMode(sidebarHidden, sidebarRail);
  const narrow = useNarrowRail();
  const sidebarMode: SidebarMode = storedSidebarMode === "hidden"
    ? "hidden"
    : narrow ? "rail" : storedSidebarMode;

  const cycleSidebar = useCallback(() => {
    // Narrow windows only swap between the rail and full hiding; the rail is
    // already the widest presentation that fits.
    const current = resolveSidebarMode(sidebarHidden, sidebarRail);
    const next = narrow
      ? (current === "hidden" ? "rail" : "hidden")
      : nextSidebarMode(current);
    setSidebarHidden(next === "hidden");
    setSidebarRail(next === "rail");
  }, [narrow, setSidebarHidden, setSidebarRail, sidebarHidden, sidebarRail]);

  return (
    <FileLibraryExperienceProvider active={!isSearchMode && (view === "library" || hasOpenedLibrary)}>
      {!isSearchMode && <SearchLibraryHandoff />}
      {isSearchMode ? <SearchWindow /> : (
        <div className={appRoot} data-density={density}>
          <PressMotion />
          <AmbientMesh />
          <div id={APP_SHELL_CONTENT_ID} className="zc-shell">
            <Titlebar
              mode={sidebarMode}
              onCycleSidebar={cycleSidebar}
              activeLabel={activeLabel}
              context={topbarContext(view, t)}
              spotlightTriggerRef={spotlightTriggerRef}
              hotkeyLabel={hotkeyLabel}
              onOpenSearch={() => setIsCommandOpen(true)}
              t={t}
            />
            <div className="zc-shell-body" data-sidebar-mode={sidebarMode}>
              <Sidebar
                groups={groups}
                mode={sidebarMode}
                canExpand={!narrow}
                onExpand={() => { setSidebarHidden(false); setSidebarRail(false); }}
              />
              <div className="zc-app-workspace">
                <main data-flush={view === "library" || view === "settings" ? "true" : undefined}>
                  {view !== "library" && view !== "settings" ? (
                    <ShellViewHeading view={view} activeLabel={activeLabel} headingDescription={headingDescription} />
                  ) : null}
                  <ToastContainer />
                  <div className={viewStageClass} ref={viewStageRef}>
                    {(hasOpenedLibrary || view === "library") && <div hidden={view !== "library"} className="h-full min-h-0 min-w-0">
                      <ViewErrorBoundary><Suspense fallback={<div className={softPanel}>{t("loading")}</div>}><FileLibraryWorkspace active={view === "library"}/></Suspense></ViewErrorBoundary>
                    </div>}
                    {view !== "library" && <ViewErrorBoundary key={view}>
                      <AppViewContent />
                    </ViewErrorBoundary>}
                  </div>
                </main>
              </div>
            </div>
            {isCommandOpen && <CommandLauncher restoreFocusRef={spotlightTriggerRef} />}
            {isCloseChoiceOpen && (
              <CloseChoiceDialog t={t} onCancel={onCancelCloseChoice} onChoose={resolveCloseChoice} />
            )}
            <OnboardingDialog />
          </div>
          <ModalHost />
        </div>
      )}
    </FileLibraryExperienceProvider>
  );
}

/**
 * One 44px glass titlebar replaces the previous two stacked bars (a 38px
 * window chrome plus a 46px workspace header). It carries the app mark, the
 * current route breadcrumb, the only global search entry, display controls and
 * the platform window cluster.
 */
export function Titlebar({
  mode,
  onCycleSidebar,
  activeLabel,
  context,
  spotlightTriggerRef,
  hotkeyLabel,
  onOpenSearch,
  t
}: {
  mode: SidebarMode;
  onCycleSidebar: () => void;
  activeLabel: string;
  context: string;
  spotlightTriggerRef: React.RefObject<HTMLButtonElement | null>;
  hotkeyLabel: string;
  onOpenSearch: () => void;
  t: Translator;
}) {
  const { isWindows } = useWindowContext();
  const label = sidebarToggleLabel(mode, t);

  return (
    <header className="zc-titlebar" aria-label={t("appName")}>
      <div className="zc-titlebar-lead">
        {!isWindows && <MacWindowControls />}
        <button
          type="button"
          className={cn("zc-titlebar-toggle", noDrag)}
          data-sidebar-mode={mode}
          aria-label={label}
          title={label}
          aria-expanded={mode !== "hidden"}
          aria-controls={mode === "hidden" ? undefined : "zc-main-sidebar"}
          onClick={onCycleSidebar}
        >
          {mode === "expanded" ? <PanelLeftClose size={16} /> : mode === "rail" ? <PanelLeft size={16} /> : <PanelLeftOpen size={16} />}
        </button>
        <span className="zc-titlebar-mark" aria-hidden="true"><BrandMark size="micro" decorative /></span>
        <div className="zc-titlebar-crumb">
          <strong>{activeLabel}</strong>
          <span aria-hidden="true">/</span>
          <span>{context}</span>
        </div>
      </div>
      <div className="flex min-w-0 items-center justify-center">
        <button ref={spotlightTriggerRef} className={cn(spotlightButton, noDrag)} onClick={onOpenSearch}>
          <Search size={15} aria-hidden="true" />
          <span className="min-w-0 truncate text-left">{t("topbarSearch")}</span>
          <kbd>{hotkeyLabel}</kbd>
        </button>
      </div>
      <div className="zc-titlebar-trail">
        <WorkspaceTopActions />
        {isWindows && <WindowsControls />}
      </div>
    </header>
  );
}

function SearchWindow() {
  return (
    <div className={searchWindowRoot}>
      <CommandLauncher standalone />
    </div>
  );
}

function CommandLauncher({
  standalone = false,
  restoreFocusRef
}: {
  standalone?: boolean;
  restoreFocusRef?: React.RefObject<HTMLElement | null>;
}) {
  const { commandInputRef, setIsCommandOpen, platform } = useCommandContext();
  const { setView, onError } = useNavigationContext();
  const { t } = useI18nContext();
  const isBackgroundIndexing = useBackgroundIndexerStore((state) => state.isBackgroundIndexing);
  const currentBackgroundRoot = useBackgroundIndexerStore((state) => state.currentRoot);
  const pendingBackgroundRoots = useBackgroundIndexerStore((state) => state.pendingRoots.length);
  const setSelectedFileId = useFileLibraryStore((state) => state.setSelectedFileId);
  const openGlobalEntry = useOpenSearchParent();
  const activateFileLibraryFile = useCallback(
    (fileId: string) => projectAcceptedFileLibraryActivation(fileId, {
      setExplicitSelection: useFileLibrarySelectionStore.getState().setExplicit,
      loadDetail: useFileLibraryInspectorStore.getState().loadDetail
    }),
    []
  );

  function closeCommand() {
    if (!standalone) setIsCommandOpen(false);
  }

  return (
    <CommandModal
      inputRef={commandInputRef}
      setView={setView}
      setSelectedFileId={setSelectedFileId}
      openGlobalEntry={openGlobalEntry}
      activateFileLibraryFile={activateFileLibraryFile}
      onClose={closeCommand}
      platform={platform}
      t={t}
      onError={onError}
      standalone={standalone}
      restoreFocusRef={restoreFocusRef}
      backgroundStatus={{
        isRunning: isBackgroundIndexing,
        currentRoot: currentBackgroundRoot,
        pending: pendingBackgroundRoots
      }}
    />
  );
}

function MacWindowControls() {
  const { handleWindowAction } = useWindowContext();
  const { t } = useI18nContext();

  return (
    <div className={cn("zc-mac-controls", noDrag)} aria-label={t("windowControls")}>
      <button type="button" data-action="close" onClick={() => handleWindowAction("close")} aria-label={t("close")} />
      <button type="button" data-action="minimize" onClick={() => handleWindowAction("minimize")} aria-label={t("minimize")} />
      <button type="button" data-action="maximize" onClick={() => handleWindowAction("maximize")} aria-label={t("maximize")} />
    </div>
  );
}

function WindowsControls() {
  const { handleWindowAction } = useWindowContext();
  const { t } = useI18nContext();

  return (
    <div className={cn("zc-window-controls", noDrag)} aria-label={t("windowControls")}>
      <button type="button" className={sidebarWindowAction("minimize")} data-action="minimize" onClick={() => handleWindowAction("minimize")} aria-label={t("minimize")}>
        <Minus size={15} strokeWidth={1.6} />
      </button>
      <button type="button" className={sidebarWindowAction("maximize")} data-action="maximize" onClick={() => handleWindowAction("maximize")} aria-label={t("maximize")}>
        <Square size={11} strokeWidth={1.6} />
      </button>
      <button type="button" className={sidebarWindowAction("close")} data-action="close" onClick={() => handleWindowAction("close")} aria-label={t("close")}>
        <X size={15} strokeWidth={1.6} />
      </button>
    </div>
  );
}

export const Sidebar = memo(function Sidebar({
  groups,
  mode = "expanded",
  canExpand = true,
  onExpand = () => undefined
}: {
  groups: NavGroup[];
  mode?: SidebarMode;
  canExpand?: boolean;
  onExpand?: () => void;
}) {
  const { view, setView } = useNavigationContext();
  const { t } = useI18nContext();
  const previewActionCount = useOrganizationPlanStore((state) => organizationPlanPendingReview(state.plans, state.activePlan));
  const aiModeStatus = useAIProcessingModeStore((state) => state.status);
  const aiModeSettings = useAIProcessingModeStore((state) => state.settings);
  const aiModeError = useAIProcessingModeStore((state) => state.error);
  const loadAIProcessingMode = useAIProcessingModeStore((state) => state.load);
  const rail = mode === "rail";

  useEffect(() => {
    void loadAIProcessingMode();
  }, [loadAIProcessingMode]);

  function openAISettings() {
    openAIProcessingModeSettings(setView, requestSettingsSection);
  }

  return (
    <aside
      id="zc-main-sidebar"
      className="zc-sidebar"
      aria-label={t("appName")}
      aria-hidden={mode === "hidden" ? true : undefined}
      inert={mode === "hidden" ? true : undefined}
      data-sidebar-mode={mode}
    >
      <div className="zc-sidebar-brand">
        <BrandMark size="sidebar" decorative />
        <div className="zc-sidebar-brand-text">
          <strong>{t("appName")}</strong>
          <span>{t("appTagline")}</span>
        </div>
        {rail && canExpand && (
          <button type="button" className="zc-titlebar-tool" onClick={onExpand} aria-label={t("shellShowSidebar")} title={t("shellShowSidebar")}>
            <PanelLeftOpen size={15} />
          </button>
        )}
      </div>
      <nav className="zc-sidebar-nav" aria-label={t("mainNavigationLabel")}>
        {groups.map((group) => (
          <section className="zc-nav-group" key={group.id}>
            {group.label ? <span className="zc-nav-group-label">{group.label}</span> : null}
            {group.items.map((item) => (
              <button
                key={item.id}
                className="zc-nav-item"
                title={rail ? item.label : undefined}
                onClick={() => { if (item.action !== "open_search") setView(item.id as View); }}
                aria-current={view === item.id ? "page" : undefined}
                data-nav-action={item.action ?? undefined}
              >
                <item.icon size={16} strokeWidth={1.7} aria-hidden="true" />
                <span className="zc-nav-text">{item.label}</span>
                {item.id === "organize" && previewActionCount > 0 && (
                  <span className="zc-nav-badge" aria-label={t("organizePendingBadge").replace("{count}", previewActionCount.toLocaleString())}>
                    {previewActionCount}
                  </span>
                )}
              </button>
            ))}
          </section>
        ))}
      </nav>
      <div className="zc-sidebar-footer">
        <div className="zc-sidebar-account">
          <span className="zc-sidebar-avatar" aria-hidden="true">ZS</span>
          <div className="zc-sidebar-account-text">
            <strong>{t("localWorkspace")}</strong>
            {/* The subtitle states the mode the app actually reports, so it can
                never contradict the AI status block below it. */}
            <span>{t("localOnly")} · {sidebarMode({ status: aiModeStatus, settings: aiModeSettings, error: aiModeError }, t).title}</span>
          </div>
        </div>
        <AIProcessingModeStatus
          state={{ status: aiModeStatus, settings: aiModeSettings, error: aiModeError }}
          t={t}
          onCheckSettings={openAISettings}
        />
      </div>
    </aside>
  );
});

export function ShellViewHeading({
  view,
  activeLabel,
  headingDescription,
  actions
}: {
  view: View;
  activeLabel: string;
  headingDescription: string;
  actions?: React.ReactNode;
}) {
  return <PageHeader title={activeLabel} description={headingDescription} actions={actions} />;
}

export const ToastContainer = memo(function ToastContainer() {
  const toast = useAppStore((state) => state.toast);
  const watcherRoots = useWatcherStatusStore((state) => state.roots);
  const { t } = useI18nContext();
  const clearToast = useAppStore((state) => state.clearToast);
  const { view } = useNavigationContext();
  const previousViewRef = useRef(view);

  useEffect(() => {
    if (!toast || toast.type === "error") return;
    const timeout = window.setTimeout(clearToast, toast.type === "success" ? 2200 : 3200);
    return () => window.clearTimeout(timeout);
  }, [clearToast, toast]);

  useEffect(() => {
    if (previousViewRef.current !== view) {
      previousViewRef.current = view;
      if (toast?.type === "success") clearToast();
    }
  }, [clearToast, toast, view]);

  // Durable warnings are a projection, not one-shot global errors. A newer
  // healthy snapshot removes only its root's warning; unrelated errors retain
  // their existing lifetime and take priority over the watcher projection.
  const watcherWarning = Object.values(watcherRoots)
    .map(deriveWatcherPresentation)
    .find(watcherPresentationNeedsAttention);
  const visibleToast = toast ?? (watcherWarning ? { message: t(watcherWarning.messageKey), type: "error" as const } : null);
  if (!visibleToast) return null;

  if (visibleToast.type === "success") {
    return (
      <div className="fixed bottom-5 right-5 z-50 max-w-sm rounded-[var(--zc-radius-field)] border border-[var(--zc-success-border)] bg-[var(--zc-success-soft)] px-3 py-2 text-xs font-medium text-[var(--zc-success-text)] shadow-[var(--zc-shadow-raised)]" role="status">
        {visibleToast.message}
      </div>
    );
  }

  return (
    <div className={cn(statusToast, toastTone(visibleToast.type))} role={visibleToast.type === "error" ? "alert" : "status"}>
      {visibleToast.message}
    </div>
  );
});

const AppViewContent = memo(function AppViewContent() {
  const { view } = useNavigationContext();
  const { t } = useI18nContext();

  let content;
  if (view === "scanner") content = <ScannerView />;
  else if (view === "cleanup") content = <StorageCleanupView />;
  else if (view === "organize") content = <OrganizeSuggestionsView />;
  else if (view === "preview") content = <TimelineView />;
  else if (view === "automation") content = <AutomationWorkspace />;
  else if (view === "restore") content = <RestoreView />;
  else content = <SettingsView />;
  return <Suspense fallback={<div className={softPanel}>{t("loading")}</div>}>{content}</Suspense>;
});

export function shouldShowAIProcessingModeStatus(state: AIProcessingModeState) {
  const mode = resolveAIProcessingMode(state);
  return mode === "failed" || mode === "local" || mode === "cloud";
}

export function AIProcessingModeStatus({
  state,
  t,
  onCheckSettings
}: {
  state: AIProcessingModeState;
  t: Translator;
  onCheckSettings?: () => void;
}) {
  const resolvedMode = resolveAIProcessingMode(state);
  if (!shouldShowAIProcessingModeStatus(state)) return null;
  const mode = sidebarMode(state, t);
  return (
    <div className={cn(softPanel, "mt-1 flex items-start gap-2 p-2 text-xs")} data-ai-processing-mode={resolvedMode}>
      <AIProcessingModeIcon mode={mode.id} />
      <div className="min-w-0 flex-1">
        <strong className="block text-xs text-[var(--zc-text-primary)]">{mode.title}</strong>
        <span className="block text-[11px] leading-4 text-[var(--zc-text-secondary)]">{mode.description}</span>
        {mode.id === "failed" && onCheckSettings ? (
          <button
            className={cn("mt-1 rounded px-1 py-0.5 text-[11px] font-semibold text-[var(--zc-warning-text)] hover:bg-[var(--zc-warning-soft)]", focusVisibleState)}
            onClick={onCheckSettings}
          >
            {t("modeCheckSettings")}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function AIProcessingModeIcon({ mode }: { mode: ReturnType<typeof resolveAIProcessingMode> }) {
  const iconClass = "mt-0.5 shrink-0";
  if (mode === "loading") {
    return <LoaderCircle data-ai-mode-icon="loader" size={15} className={cn(iconClass, "animate-spin text-[var(--zc-info-text)]")} />;
  }
  if (mode === "failed") {
    return <TriangleAlert data-ai-mode-icon="warning" size={15} className={cn(iconClass, "text-[var(--zc-warning-text)]")} />;
  }
  if (mode === "disabled") {
    return <LockKeyhole data-ai-mode-icon="disabled" size={15} className={cn(iconClass, "text-[var(--zc-neutral-text)]")} />;
  }
  if (mode === "local") {
    return <Cpu data-ai-mode-icon="local" size={15} className={cn(iconClass, "text-[var(--zc-success-text)]")} />;
  }
  return <Cloud data-ai-mode-icon="cloud" size={15} className={cn(iconClass, "text-[var(--zc-info-text)]")} />;
}

export function openAIProcessingModeSettings(
  setView: (view: View) => void,
  openSection: (sectionId: string) => void
) {
  setView("settings");
  openSection("settings-ai");
}

function viewDescription(
  view: View,
  stats: DashboardStats,
  scope: LibraryScope,
  scopeText: string,
  previewActionCount: number,
  t: Translator
): string {
  switch (view) {
    case "scanner":
      return `${t("lastScan")}: ${stats.lastScannedAt ? formatDate(stats.lastScannedAt) : t("notScannedYet")}`;
    case "cleanup":
      return t("viewDescCleanup");
    case "organize":
      return `${t("currentOrganizeScope")}: ${scopeText} · ${t("viewDescOrganize")}`;
    case "library":
      return fileLibraryHeadingDescription(scope, stats.totalFiles, scopeText, t);
    case "preview":
      return previewActionCount > 0
        ? `${previewActionCount.toLocaleString()} ${t("items")} · ${t("viewDescPreview")}`
        : t("viewDescPreview");
    case "automation":
      return t("automationIntentDescription");
    case "restore":
      return t("viewDescRestore");
    case "settings":
      return t("viewDescSettings");
  }
}

export function navGroups(t: Translator): NavGroup[] {
  return [
    {
      id: "primary",
      label: "",
      items: [
        // ADR-0016 four core jobs. Files is the default destination and opens
        // in Browse without any Managed Library scope or index authorization.
        // Global search deliberately has no rail entry: its only entry point is
        // the titlebar field plus Ctrl/⌘ K (DESIGN-SPEC §4.3).
        { id: "library", label: t("filesWorkspace"), icon: Library },
        { id: "organize", label: t("organizeFiles"), icon: Sparkles },
        { id: "cleanup", label: t("storageCleanup"), icon: Brush }
      ]
    },
    {
      id: "workflow",
      label: t("navWorkflow"),
      items: [
        { id: "restore", label: t("history"), icon: Clock3 }
      ]
    },
    {
      id: "advanced",
      label: t("navSystem"),
      items: [
        // Managed Library setup stays available, but it is no longer the entry
        // point to the ordinary file manager.
        { id: "scanner", label: t("overview"), icon: House },
        { id: "settings", label: t("settingsPageTitle"), icon: Settings }
      ]
    }
  ];
}

export function fileLibraryHeadingDescription(
  scope: LibraryScope,
  scopedTotal: number,
  scopeText: string,
  t: Translator
) {
  const prefix = `${t("currentScope")}: ${scopeText}`;
  return scope.kind === "current_scan" && scope.roots.length === 0
    ? prefix
    : `${prefix} · ${scopedTotal.toLocaleString()} ${t("files")}`;
}

function viewLabel(view: View, t: Translator) {
  if (view === "cleanup") return t("storageCleanup");
  if (view === "organize") return t("organizeFiles");
  if (view === "preview") return t("previewExecute");
  return t("overview");
}

function topbarContext(view: View, t: Translator) {
  switch (view) {
    case "scanner": return t("topbarOverviewContext");
    case "library": return t("topbarLibraryContext");
    case "organize": return t("topbarOrganizeContext");
    case "cleanup": return t("topbarCleanupContext");
    case "restore": return t("topbarHistoryContext");
    case "automation": return t("topbarAutomationContext");
    case "preview": return t("topbarPreviewContext");
    case "settings": return t("topbarSettingsContext");
  }
}

function sidebarMode(
  state: AIProcessingModeState,
  t: Translator
) {
  const mode = resolveAIProcessingMode(state);
  if (mode === "loading") return { id: mode, title: t("modeLoading"), description: t("modeLoadingDesc") };
  if (mode === "failed") return { id: mode, title: t("modeFailed"), description: t("modeFailedDesc") };
  if (mode === "disabled") return { id: mode, title: t("modeAIDisabled"), description: t("modeAIDisabledDesc") };
  if (mode === "local") return { id: mode, title: t("modeAILocal"), description: t("modeAILocalDesc") };
  return { id: mode, title: t("modeAICloud"), description: t("modeAICloudDesc") };
}

function useOpenSearchParent() {
  const { controller } = useFileLibraryExperience();
  const { platform } = useCommandContext();
  return useCallback(async (id: string, accept: () => boolean | Promise<boolean>) => {
    if (platform !== "win32" && platform !== "darwin" && platform !== "browser") return false;
    return (await controller.openBrowse({
      platform: platform === "darwin" ? "macos" : "windows",
      routingHint: "", globalEntryId: id
    }, accept)) !== null;
  }, [controller, platform]);
}

function SearchLibraryHandoff() {
  const setView = useAppStore((state) => state.setView);
  const showError = useAppStore((state) => state.showError);
  const openGlobalEntry = useOpenSearchParent();
  const activateFile = useCallback((fileId: string) => projectAcceptedFileLibraryActivation(fileId, {
    setExplicitSelection: useFileLibrarySelectionStore.getState().setExplicit,
    loadDetail: useFileLibraryInspectorStore.getState().loadDetail
  }), []);
  useSearchNavigationHandoff(false, setView, activateFile, showError, openGlobalEntry);
  return null;
}
