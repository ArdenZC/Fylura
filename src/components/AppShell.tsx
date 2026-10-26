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
import { lazy, memo, Suspense, useCallback, useEffect, useMemo, useRef } from "react";
import { tauriApi } from "../api/tauriApi";
import { CommandModal } from "./CommandModal";
import { OnboardingDialog } from "./OnboardingDialog";
import { ViewErrorBoundary } from "./ErrorBoundary";
import { AmbientMesh, CloseChoiceDialog, WorkspaceTopActions } from "./ShellChrome";
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
import { cn, focusVisibleState, selectedFocusVisibleState, selectedSurface, statusToast, toastTone } from "../utils/tw";
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

const appRoot =
  cn(pageFrame, "relative h-screen min-h-0 min-w-0 bg-[var(--zc-canvas)] text-[var(--zc-text-primary)]");
const searchWindowRoot =
  "relative h-full w-full overflow-hidden bg-transparent text-[var(--zc-text-primary)]";
const titlebar =
  "relative z-30 grid h-12 grid-cols-[228px_minmax(0,1fr)_228px] items-center border-b border-[var(--zc-divider)] bg-[var(--zc-titlebar)] px-4 [-webkit-app-region:drag] max-[1100px]:grid-cols-[176px_minmax(0,1fr)_176px] max-[720px]:grid-cols-[0_minmax(0,1fr)_auto] max-[720px]:px-2";
const windowChrome =
  "relative z-40 flex h-[38px] min-h-[38px] shrink-0 items-center border-b border-[var(--zc-divider)] bg-[var(--zc-surface)] px-3 text-[11px] text-[var(--zc-text-tertiary)] [-webkit-app-region:drag]";
const windowTitle = "absolute left-3 top-1/2 -translate-y-1/2 font-semibold tracking-[0.01em] text-[var(--zc-text-secondary)]";
const noDrag = "[-webkit-app-region:no-drag]";
const spotlightButton =
  cn("mx-auto grid h-8 w-[min(42vw,440px)] min-w-64 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2.5 rounded-[var(--zc-radius-control)] border border-[var(--zc-control-border)] bg-[var(--zc-surface-subtle)] px-3 text-xs text-[var(--zc-text-secondary)] shadow-none transition-[background,border-color,box-shadow,color] duration-[var(--zc-duration-fast)] hover:border-[var(--zc-control-border-hover)] hover:bg-[var(--zc-surface-hover)]", focusVisibleState, "[&_kbd]:rounded-md [&_kbd]:border [&_kbd]:border-[var(--zc-divider)] [&_kbd]:bg-[var(--zc-surface)] [&_kbd]:px-1.5 [&_kbd]:py-0.5 [&_kbd]:text-[11px] [&_kbd]:font-medium [&_kbd]:text-[var(--zc-text-tertiary)]");
const workspaceShell = "relative z-10 grid min-h-0 flex-1 grid-cols-[228px_minmax(0,1fr)] max-[1100px]:grid-cols-[176px_minmax(0,1fr)]";
const sidebarClass =
  "flex min-h-0 flex-col gap-5 overflow-x-hidden overflow-y-auto overscroll-contain border-r border-[var(--zc-divider)] bg-[var(--zc-sidebar)] px-4 py-5";
const navItemBase =
  cn("relative flex min-h-10 w-full items-center gap-3 rounded-[var(--zc-radius-control)] border border-transparent px-3 py-2 text-left text-sm font-medium text-[var(--zc-text-secondary)] transition-[background,border-color,color] duration-[var(--zc-duration-fast)] hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]", focusVisibleState);
const navItemActive = selectedSurface;
const workspaceClass = "flex min-h-0 min-w-[720px] flex-col overflow-hidden px-5 py-5 max-[1100px]:min-w-0 max-[1100px]:px-3";
const libraryWorkspaceClass = "flex min-h-0 min-w-[720px] flex-col overflow-hidden max-[1100px]:min-w-0";
const viewStageClass = viewStage;
const windowsControlButton =
  cn("grid h-[38px] w-[46px] place-items-center text-[var(--zc-text-secondary)] transition-[background,color] hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]", focusVisibleState);
const windowsCloseButton =
  cn("grid h-[38px] w-[46px] place-items-center text-[var(--zc-text-secondary)] transition-[background,color] hover:bg-[var(--zc-window-close-hover)] hover:text-[var(--zc-window-close-text)]", focusVisibleState);
const macControlButton = "grid h-6 w-6 place-items-center rounded-full";
const navGroupTitle = "px-3 pt-2 text-[11px] font-semibold text-[var(--zc-text-tertiary)]";

type NavItem = { id: View; label: string; icon: typeof Radar };
type NavGroup = { id: "primary" | "workflow" | "advanced"; label: string; items: NavItem[] };

export function AppShell() {
  const { isSearchMode, setIsCommandOpen, hotkeyLabel, isCommandOpen } = useCommandContext();
  const { isCloseChoiceOpen, onCancelCloseChoice, requestClose, resolveCloseChoice } = useWindowContext();
  const { view } = useNavigationContext();
  const { t } = useI18nContext();
  const stats = useFileLibraryStore((state) => state.stats);
  const scope = useFileLibraryStore((state) => state.scope);
  const sidebarHidden = useAppStore((state) => state.sidebarHidden);
  const setSidebarHidden = useAppStore((state) => state.setSidebarHidden);
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

  return (
    <FileLibraryExperienceProvider active={!isSearchMode && view === "library"}>
      {!isSearchMode && <SearchLibraryHandoff />}
      {isSearchMode ? <SearchWindow /> : (
        <div className={appRoot} data-density={density}>
          <AmbientMesh />
          <WindowChrome />
          <div id={APP_SHELL_CONTENT_ID} className={cn(workspaceShell, "zc-app-shell-content")} data-sidebar-hidden={sidebarHidden}>
            {!sidebarHidden && <Sidebar groups={groups} />}
            <div className="zc-app-workspace">
              <header className={titlebar}>
                <div className="flex min-w-0 items-center gap-1.5 truncate text-[var(--zc-text-secondary)]">
                  <button type="button" className={cn("zc-sidebar-toggle grid h-8 w-8 shrink-0 place-items-center rounded-md hover:bg-[var(--zc-surface-hover)]", noDrag, focusVisibleState)} aria-label={t(sidebarHidden ? "shellShowSidebar" : "shellHideSidebar")} title={t(sidebarHidden ? "shellShowSidebar" : "shellHideSidebar")} aria-expanded={!sidebarHidden} aria-controls="zc-main-sidebar" onClick={() => setSidebarHidden(!sidebarHidden)}>
                    {sidebarHidden ? <PanelLeftOpen size={17} /> : <PanelLeftClose size={17} />}
                  </button>
                  <strong className="truncate text-[13px]">{activeLabel}</strong>
                  <span aria-hidden="true" className="text-[var(--zc-text-tertiary)]">/</span>
                  <span className="truncate text-[11px] text-[var(--zc-text-tertiary)]">{topbarContext(view, t)}</span>
                </div>
                <div className="flex items-center justify-center">
                  <button ref={spotlightTriggerRef} className={cn(spotlightButton, noDrag)} onClick={() => setIsCommandOpen(true)}>
                    <Search size={15} className="text-[var(--zc-primary)]" />
                    <span className="min-w-0 truncate text-left">{t("topbarSearch")}</span>
                    <kbd>{hotkeyLabel}</kbd>
                  </button>
                </div>
                <div className="flex items-center justify-end">
                  <WorkspaceTopActions />
                </div>
              </header>
              <main className={view === "library" ? libraryWorkspaceClass : workspaceClass}>
                {view !== "library" ? (
                  <ShellViewHeading view={view} activeLabel={activeLabel} headingDescription={headingDescription} />
                ) : null}
                <ToastContainer />
                <div className={viewStageClass}>
                  <ViewErrorBoundary key={view}>
                    <AppViewContent />
                  </ViewErrorBoundary>
                </div>
              </main>
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

function WindowChrome() {
  const { isWindows } = useWindowContext();
  const { t } = useI18nContext();

  return (
    <header className={windowChrome} aria-label={t("windowControls")}>
      <span className={windowTitle}>{t("appName")}</span>
      <div className="ml-auto">
        {isWindows ? <WindowsControls /> : <MacWindowControls />}
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
    <div className={cn("flex items-center gap-1", noDrag)} aria-label={t("windowControls")}>
      <button className={macControlButton} onClick={() => handleWindowAction("close")} aria-label={t("close")}>
        <span className="h-3 w-3 rounded-full bg-[var(--zc-window-mac-close)] shadow-sm" />
      </button>
      <button className={macControlButton} onClick={() => handleWindowAction("minimize")} aria-label={t("minimize")}>
        <span className="h-3 w-3 rounded-full bg-[var(--zc-window-mac-minimize)] shadow-sm" />
      </button>
      <button className={macControlButton} onClick={() => handleWindowAction("maximize")} aria-label={t("maximize")}>
        <span className="h-3 w-3 rounded-full bg-[var(--zc-window-mac-maximize)] shadow-sm" />
      </button>
    </div>
  );
}

function WindowsControls() {
  const { handleWindowAction } = useWindowContext();
  const { t } = useI18nContext();

  return (
    <div className={cn("flex h-[38px] items-center", noDrag)} aria-label={t("windowControls")}>
      <button className={windowsControlButton} onClick={() => handleWindowAction("minimize")} aria-label={t("minimize")}>
        <Minus size={15} strokeWidth={1.6} />
      </button>
      <button className={windowsControlButton} onClick={() => handleWindowAction("maximize")} aria-label={t("maximize")}>
        <Square size={12} strokeWidth={1.6} />
      </button>
      <button className={windowsCloseButton} onClick={() => handleWindowAction("close")} aria-label={t("close")}>
        <X size={16} strokeWidth={1.6} />
      </button>
    </div>
  );
}

export const Sidebar = memo(function Sidebar({ groups }: { groups: NavGroup[] }) {
  const { view, setView } = useNavigationContext();
  const { t } = useI18nContext();
  const previewActionCount = useOrganizationPlanStore((state) => organizationPlanPendingReview(state.plans, state.activePlan));
  const aiModeStatus = useAIProcessingModeStore((state) => state.status);
  const aiModeSettings = useAIProcessingModeStore((state) => state.settings);
  const aiModeError = useAIProcessingModeStore((state) => state.error);
  const loadAIProcessingMode = useAIProcessingModeStore((state) => state.load);

  useEffect(() => {
    void loadAIProcessingMode();
  }, [loadAIProcessingMode]);

  function openAISettings() {
    openAIProcessingModeSettings(setView, requestSettingsSection);
  }

  return (
    <aside id="zc-main-sidebar" className={sidebarClass}>
      <div className="zc-sidebar-brand flex items-center gap-2.5">
        <span className="zc-sidebar-brand-mark" aria-hidden="true">Z</span>
        <strong className="block text-sm font-semibold">{t("appName")}</strong>
      </div>
      <nav className="flex flex-1 flex-col gap-3">
        {groups.map((group) => (
          <section className="grid gap-1 border-t border-[var(--zc-divider)] pt-3 first:border-t-0 first:pt-0" key={group.id}>
            {group.label ? <span className={navGroupTitle}>{group.label}</span> : null}
            {group.items.map((item) => (
              <button
                key={item.id}
                className={cn(navItemBase, view === item.id && navItemActive, view === item.id && selectedFocusVisibleState)}
                onClick={() => setView(item.id)}
                aria-current={view === item.id ? "page" : undefined}
              >
                <item.icon size={18} strokeWidth={1.7} className="shrink-0" />
                <span>{item.label}</span>
                {item.id === "organize" && previewActionCount > 0 && (
                  <span className="ml-auto inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-[var(--zc-warning-soft)] px-1 text-[11px] font-medium text-[var(--zc-warning-text)]" aria-label={t("organizePendingBadge").replace("{count}", previewActionCount.toLocaleString())}>
                    {previewActionCount}
                  </span>
                )}
              </button>
            ))}
          </section>
        ))}
      </nav>
      <div className="zc-sidebar-account border-t border-[var(--zc-divider)] pt-2">
        <div className="flex items-center gap-2 px-2 py-1.5">
          <span className="zc-sidebar-avatar" aria-hidden="true">ZS</span>
          <div className="min-w-0">
            <strong className="block truncate text-xs font-semibold text-[var(--zc-text-primary)]">{t("localWorkspace")}</strong>
            <span className="block truncate text-[11px] leading-4 text-[var(--zc-text-tertiary)]">{t("localOnly")} · {t("modeAIDisabled")}</span>
          </div>
        </div>
      </div>
      <AIProcessingModeStatus
        state={{ status: aiModeStatus, settings: aiModeSettings, error: aiModeError }}
        t={t}
        onCheckSettings={openAISettings}
      />
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
  else if (view === "library") content = <FileLibraryWorkspace />;
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
    <div className={cn(softPanel, "mt-auto flex items-start gap-3 p-3 text-sm")} data-ai-processing-mode={resolvedMode}>
      <AIProcessingModeIcon mode={mode.id} />
      <div className="min-w-0 flex-1">
        <strong className="block text-sm text-[var(--zc-text-primary)]">{mode.title}</strong>
        <span className="block text-xs leading-5 text-[var(--zc-text-secondary)]">{mode.description}</span>
        {mode.id === "failed" && onCheckSettings ? (
          <button
            className={cn("mt-1 rounded px-1 py-0.5 text-xs font-semibold text-[var(--zc-warning-text)] hover:bg-[var(--zc-warning-soft)]", focusVisibleState)}
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
    return <LoaderCircle data-ai-mode-icon="loader" size={17} className={cn(iconClass, "animate-spin text-[var(--zc-info-text)]")} />;
  }
  if (mode === "failed") {
    return <TriangleAlert data-ai-mode-icon="warning" size={17} className={cn(iconClass, "text-[var(--zc-warning-text)]")} />;
  }
  if (mode === "disabled") {
    return <LockKeyhole data-ai-mode-icon="disabled" size={17} className={cn(iconClass, "text-[var(--zc-neutral-text)]")} />;
  }
  if (mode === "local") {
    return <Cpu data-ai-mode-icon="local" size={17} className={cn(iconClass, "text-[var(--zc-success-text)]")} />;
  }
  return <Cloud data-ai-mode-icon="cloud" size={17} className={cn(iconClass, "text-[var(--zc-info-text)]")} />;
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
        { id: "scanner", label: t("overview"), icon: House },
        { id: "library", label: t("filesWorkspace"), icon: Library }
      ]
    },
    {
      id: "workflow",
      label: t("navWorkflow"),
      items: [
        { id: "organize", label: t("organizeFiles"), icon: Sparkles },
        { id: "cleanup", label: t("storageCleanup"), icon: Brush },
        { id: "restore", label: t("history"), icon: Clock3 }
      ]
    },
    {
      id: "advanced",
      label: t("navSystem"),
      items: [
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
