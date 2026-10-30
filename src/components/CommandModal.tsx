import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import type * as React from "react";
import { Activity, ChevronRight, CornerDownLeft, File as FileIcon, Folder, LayoutGrid, Radar, Search, X } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import { searchRuntimeApi } from "../api/searchRuntimeApi";
import type { SearchWindowSnapshot } from "../api/types";
import type { GlobalIndexStatus, GlobalSearchResult } from "../types/domain";
import type { Translator, View } from "../types/ui";
import { formatCount } from "../i18n";
import { cn, focusVisibleState, focusWithinSurface, selectedFocusSurface } from "../utils/tw";
import { compactPath, formatDisplayPath, readableError } from "../utils/viewHelpers";
import { IconButton } from "./ui/Button";
import { StateBlock } from "./ui/Notice";
import { quietText } from "./ui/surfaces";
import { ModalPortal } from "./modal/ModalPortal";
import { createCommandRegistry, executeSpotlightCommand, queryCommandRegistry, requestSettingsSection, type SpotlightCommand } from "./spotlight/commandRegistry";
import { completedSpotlightComposition, committedSpotlightInput } from "./spotlight/spotlightComposition";
import { groupSpotlightResults, mergeSpotlightResults, type SpotlightResult } from "./spotlight/spotlightModel";
import { globalSearchFeedback } from "./spotlight/globalSearchFeedback";
import { SpotlightQueryController } from "./spotlight/spotlightQueryController";
import { settingsTargetForSection, type SearchSettingsTarget } from "../utils/searchNavigation";

const tauriApi = searchRuntimeApi;

const keyBadge =
  "flex items-center justify-center rounded border border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] px-1.5 py-0.5 font-mono text-[10px] font-medium text-[var(--zc-text-tertiary)] shadow-sm";
const commandShellBase =
  "w-full overflow-hidden border border-[var(--zc-border-strong)] bg-[var(--zc-surface-overlay)] text-[var(--zc-text-primary)] shadow-[var(--zc-shadow-spotlight)]";
const commandShellCollapsed =
  "h-16 w-full max-w-[720px] rounded-full";
const commandShellExpanded =
  "rounded-2xl";
const commandShellDialogWidth = "max-w-[720px]";
const commandShellStandaloneExpanded = "w-full max-w-[720px]";

const commandInputRowBase =
  "relative flex h-16 min-h-16 items-center gap-3 border-b border-[var(--zc-divider)] px-4 transition-colors";
const commandInputRowCollapsed =
  "relative flex h-16 min-h-16 items-center gap-3 border-b-0 px-4 transition-colors";
const commandInputRowFocused = focusWithinSurface;

const commandSearchIcon =
  "grid h-5 w-5 shrink-0 place-items-center text-[var(--zc-primary)]";

const commandInput =
  "command-input h-full min-w-0 flex-1 bg-transparent text-lg text-[var(--zc-text-primary)] outline-none placeholder:text-[var(--zc-text-tertiary)] focus:outline-none focus-visible:outline-none";

const commandResultsShell = "grid min-h-0 gap-0";
const commandResultsBody = "max-h-[50vh] overflow-y-auto p-2";
const commandResultsHeader = "flex items-center justify-between px-3 py-2 text-xs font-semibold text-[var(--zc-text-tertiary)]";

const commandResultsList = "flex flex-col gap-1";
const commandResultItemBase =
  cn("grid w-full grid-cols-[40px_minmax(0,1fr)_auto] items-center gap-4 rounded-[var(--zc-radius-field)] px-3 py-3 text-left transition-[background,box-shadow] duration-[var(--zc-duration-fast)]", focusVisibleState);
const commandResultItemActive =
  selectedFocusSurface;
const commandResultItemInactive =
  "hover:bg-[var(--zc-surface-hover)]";

const commandFileIcon =
  "flex shrink-0 items-center justify-center w-10 h-10 rounded-lg border";
const commandFileName = "truncate text-sm font-medium text-[var(--zc-text-primary)] transition-colors";
const commandFileMeta = "truncate text-xs text-[var(--zc-text-secondary)]";

const commandFooter =
  "flex items-center justify-between border-t border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] px-4 py-3 text-xs text-[var(--zc-text-secondary)]";
const shortcutHints = "flex min-w-0 flex-wrap items-center justify-end gap-x-2 gap-y-1";
const shortcutHint = "inline-flex min-w-0 items-center gap-1 whitespace-nowrap";
const shortcutHintLabel = "hidden max-w-24 truncate text-[var(--zc-text-secondary)] sm:inline";
const highlightMark =
  "bg-transparent font-semibold text-[var(--zc-primary-text)]";
const commandIdleGroups = "grid gap-3 px-4 py-3";
const commandIdleGroup = "grid gap-1 border-b border-[var(--zc-divider)] pb-3 last:border-b-0 last:pb-0";
const commandIdleAction = cn("flex min-h-10 items-center gap-3 rounded-[var(--zc-radius-control)] px-2.5 text-left text-sm text-[var(--zc-text-secondary)] transition-[background,color] hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]", focusVisibleState);
const commandBackgroundStatus = "flex min-h-9 items-center gap-2 border-t border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] px-4 text-xs text-[var(--zc-text-secondary)]";
const SEARCH_RESULT_LIMIT = 80;
const standaloneSearchWindowCollapsedHeight = 160;
const standaloneSearchWindowExpandedHeight = 660;

export function findSpotlightSettingsRestoreTarget(requestedSection: string | null, fallback: HTMLElement | null = null) {
  if (!requestedSection) return fallback;
  const sectionId = requestedSection === "settings-search-scope" ? "settings-search" : requestedSection;
  return document.querySelector<HTMLElement>(`#${sectionId} [data-settings-section-heading]`)
    ?? document.querySelector<HTMLElement>(`#${sectionId}`)
    ?? fallback;
}

export function filesForCurrentQuery<T>(currentQuery: string, resultQuery: string, files: T[]) {
  return currentQuery === resultQuery ? files : [];
}

export async function activateCommandNavigation({
  standalone,
  windowSnapshot,
  view,
  fileId,
  settingsTarget,
  setView,
  setSelectedFileId,
  activateFileLibraryFile,
  onClose,
  activateSearchResult = tauriApi.activateSearchResult,
  onStandaloneActivationStateChange
}: {
  standalone: boolean;
  windowSnapshot?: SearchWindowSnapshot | null;
  view: View;
  fileId: string | null;
  settingsTarget?: SearchSettingsTarget | null;
  setView: (view: View) => void;
  setSelectedFileId: (id: string) => void;
  activateFileLibraryFile?: (id: string) => void;
  onClose: () => void;
  onStandaloneActivationStateChange?: (pending: boolean) => void;
  activateSearchResult?: (
    view: View,
    fileId: string | null,
    snapshot?: Pick<SearchWindowSnapshot, "sessionId" | "revision">,
    settingsTarget?: SearchSettingsTarget | null
  ) => Promise<void>;
}) {
  if (standalone) {
    onStandaloneActivationStateChange?.(true);
    try {
      if (settingsTarget) {
        await activateSearchResult(view, fileId, windowSnapshot ?? undefined, settingsTarget);
      } else {
        await activateSearchResult(view, fileId, windowSnapshot ?? undefined);
      }
    } finally {
      onStandaloneActivationStateChange?.(false);
    }
    return;
  }

  if (fileId) {
    setSelectedFileId(fileId);
    activateFileLibraryFile?.(fileId);
  }
  setView(view);
  onClose();
}

export function isSortingPreviewShortcut(
  event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">
) {
  if (!event.ctrlKey && !event.metaKey) return false;
  if (event.altKey || event.shiftKey) return false;
  const key = event.key.toLowerCase();
  return key === "enter" || key === "p";
}

export function CommandModal({
  inputRef,
  setView,
  setSelectedFileId,
  activateFileLibraryFile,
  openGlobalEntry,
  onClose,
  platform,
  t,
  onError,
  standalone = false,
  restoreFocusRef,
  backgroundStatus
}: {
  inputRef: RefObject<HTMLInputElement | null>;
  setView: (view: View) => void;
  setSelectedFileId: (id: string) => void;
  activateFileLibraryFile?: (id: string) => void;
  openGlobalEntry?: (id: string, accept: () => boolean | Promise<boolean>) => Promise<boolean>;
  onClose: () => void;
  platform: NodeJS.Platform | "browser";
  t: Translator;
  onError?: (message: string) => void;
  standalone?: boolean;
  restoreFocusRef?: React.RefObject<HTMLElement | null>;
  backgroundStatus?: {
    isRunning: boolean;
    currentRoot: string | null;
    pending: number;
  };
}) {
  const [search, setSearch] = useState("");
  const [committedSearch, setCommittedSearch] = useState("");
  const [globalResultState, setGlobalResultState] = useState<{ query: string; results: GlobalSearchResult[] }>({ query: "", results: [] });
  const [feedback, setFeedback] = useState<ReturnType<typeof globalSearchFeedback> | null>(null);
  const [queryState, setQueryState] = useState<"idle" | "pending" | "complete" | "partial" | "empty" | "failed" | "no_source">("idle");
  const [commandError, setCommandError] = useState("");
  const [globalIndexStatus, setGlobalIndexStatus] = useState<GlobalIndexStatus | null>(null);
  const [searchWindowSnapshot, setSearchWindowSnapshot] = useState<SearchWindowSnapshot | null>(null);
  const [revisionRefetchNonce, setRevisionRefetchNonce] = useState(0);
  const [activeIndex, setActiveIndex] = useState(0);
  const [inputFocused, setInputFocused] = useState(false);
  const [isComposing, setIsComposing] = useState(false);
  const isComposingRef = useRef(false);
  const queryIntentRef = useRef(search);
  queryIntentRef.current = search;
  const libraryActionRef = useRef(0);
  const [libraryActionPending, setLibraryActionPending] = useState(false);
  const libraryActionButtonRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => () => { libraryActionRef.current += 1; }, []);
  const standaloneActivationRef = useRef({ pending: 0, epoch: 0 });
  const trackStandaloneActivation = useCallback((pending: boolean) => {
    if (pending) {
      standaloneActivationRef.current.pending += 1;
      standaloneActivationRef.current.epoch += 1;
    } else {
      standaloneActivationRef.current.pending = Math.max(0, standaloneActivationRef.current.pending - 1);
    }
  }, []);
  const settingsCommandSectionRef = useRef<string | null>(null);
  const queryControllerRef = useRef(new SpotlightQueryController());
  const searchWindowSnapshotRef = useRef<SearchWindowSnapshot | null>(null);
  const prefersReducedMotion = useReducedMotion();
  const trimmedSearch = search.trim();
  const searchableQuery = isComposing ? "" : trimmedSearch;
  const currentGlobalResults = filesForCurrentQuery(trimmedSearch, globalResultState.query, globalResultState.results);
  const commandRegistry = useMemo(
    () => createCommandRegistry(t, platform === "browser" ? "browser" : standalone ? "standalone" : "main"),
    [platform, standalone, t]
  );
  const commandResults = useMemo(
    () => queryCommandRegistry(searchableQuery, commandRegistry),
    [commandRegistry, searchableQuery]
  );
  const visibleResults = useMemo(
    () => mergeSpotlightResults(currentGlobalResults, commandResults),
    [commandResults, currentGlobalResults]
  );
  const resultGroups = useMemo(() => groupSpotlightResults(visibleResults, t), [t, visibleResults]);
  const showResults = !isComposing && trimmedSearch.length > 0 && visibleResults.length > 0;
  const activeResultId = showResults ? `command-result-${activeIndex}` : undefined;
  const statusTitle =
    feedback && queryState !== "pending" && queryState !== "failed" ? feedback.title :
    queryState === "pending"
      ? t("commandTypingTitle")
      : queryState === "no_source"
        ? t("globalSearchNoSourcesTitle")
      : queryState === "partial"
        ? t("globalIndexStatusPartial")
      : queryState === "failed"
        ? feedback?.title ?? t("commandFailedTitle")
        : trimmedSearch
          ? t("commandNoResultsTitle")
          : t("commandIdleTitle");
  const statusDescription =
    feedback && queryState !== "pending" && queryState !== "failed" ? feedback.description :
    queryState === "pending"
      ? t("commandSearching")
      : queryState === "no_source"
        ? t("globalSearchNoSourcesDesc")
      : queryState === "partial"
        ? t("globalSearchIndexMeta")
      : queryState === "failed"
        ? commandError || feedback?.description || t("commandSearchFailed")
        : trimmedSearch
          ? t("commandNoResults")
          : t("commandIdleDesc");
  const isStandaloneCollapsed =
    standalone
    && !trimmedSearch
    && queryState === "idle";
  const shouldShowIdleState = !standalone && !trimmedSearch;
  const shouldShowStateBlock = !showResults && trimmedSearch.length > 0 && queryState !== "idle";
  const showGlobalIndexMeta = !isStandaloneCollapsed && Boolean(
    globalIndexStatus
  );
  const globalSearchIndexMeta = globalIndexStatus
    ? `${t("globalSearchIndexMeta")} · ${t("globalIndexStatus")}: ${globalIndexStatusLabel(globalIndexStatus.status, t)}`
    : t("globalSearchIndexMeta");

  const updateSearchWindowSnapshot = useCallback((next: SearchWindowSnapshot) => {
    const current = searchWindowSnapshotRef.current;
    if (
      current
      && (next.sessionId < current.sessionId
        || (next.sessionId === current.sessionId && next.revision <= current.revision))
    ) return;
    if (!current || next.sessionId !== current.sessionId) {
      queryControllerRef.current.openSession(next.sessionId);
      setSearch("");
      setCommittedSearch("");
      setGlobalResultState({ query: "", results: [] });
      setQueryState("idle");
      setActiveIndex(0);
    }
    searchWindowSnapshotRef.current = next;
    setSearchWindowSnapshot(next);
  }, []);

  const requestSearchWindowHide = useCallback((snapshot: SearchWindowSnapshot) => {
    void tauriApi.hideSearchWindow(snapshot)
      .then(updateSearchWindowSnapshot)
      .catch((error) => {
        const message = readableError(error);
        setCommandError(message);
        onError?.(message);
      });
  }, [onError, updateSearchWindowSnapshot]);

  const closeSpotlight = useCallback(() => {
    if (!standalone) {
      onClose();
      return;
    }
    const snapshot = searchWindowSnapshotRef.current;
    if (snapshot) requestSearchWindowHide(snapshot);
  }, [onClose, requestSearchWindowHide, standalone]);

  useEffect(() => {
    if (!standalone) {
      queryControllerRef.current.openSession();
      return () => queryControllerRef.current.closeSession();
    }
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriApi.onSearchWindowState(updateSearchWindowSnapshot).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    }).catch((error) => {
      if (!disposed) onError?.(readableError(error));
    });
    void tauriApi.getSearchWindowState()
      .then(async (snapshot) => {
        if (disposed) return;
        updateSearchWindowSnapshot(snapshot);
        if (snapshot.phase === "showing") {
          updateSearchWindowSnapshot(await tauriApi.searchWindowReady(snapshot));
        }
      })
      .catch((error) => {
        if (!disposed) onError?.(readableError(error));
      });
    return () => {
      disposed = true;
      unlisten?.();
      queryControllerRef.current.closeSession();
    };
  }, [onError, standalone, updateSearchWindowSnapshot]);

  useEffect(() => {
    if (!standalone || !searchWindowSnapshot) return;
    if (!searchWindowSnapshot.phase.startsWith("visible_")) return;
    const expanded = !isStandaloneCollapsed;
    const alreadySized = expanded
      ? searchWindowSnapshot.phase === "visible_expanded"
      : searchWindowSnapshot.phase === "visible_collapsed";
    if (alreadySized) return;
    void tauriApi.resizeSearchWindow(searchWindowSnapshot, expanded)
      .then(updateSearchWindowSnapshot)
      .catch(() => undefined);
  }, [isStandaloneCollapsed, searchWindowSnapshot, standalone, updateSearchWindowSnapshot]);

  useEffect(() => {
    if (!standalone) return;
    if (!searchWindowSnapshot?.phase.startsWith("visible_")) return;
    const focusFrame = requestAnimationFrame(() => inputRef.current?.focus());
    return () => cancelAnimationFrame(focusFrame);
  }, [inputRef, searchWindowSnapshot, standalone]);

  useEffect(() => {
    if (!standalone) return;
    let blurTimer: number | undefined;
    const handleBlur = () => {
      if (isComposingRef.current || standaloneActivationRef.current.pending > 0) return;
      const activationEpoch = standaloneActivationRef.current.epoch;
      const blurredSnapshot = searchWindowSnapshotRef.current;
      if (!blurredSnapshot) return;
      window.clearTimeout(blurTimer);
      blurTimer = window.setTimeout(() => {
        // Main intentionally takes focus during handoff. Neither that blur nor
        // a pre-handoff timer may close a rejected/timed-out Search afterward.
        if (isComposingRef.current || standaloneActivationRef.current.pending > 0
          || standaloneActivationRef.current.epoch !== activationEpoch) return;
        if (!document.hasFocus()) requestSearchWindowHide(blurredSnapshot);
      }, 120);
    };
    window.addEventListener("blur", handleBlur);
    return () => {
      window.clearTimeout(blurTimer);
      window.removeEventListener("blur", handleBlur);
    };
  }, [isComposing, requestSearchWindowHide, standalone]);

  useEffect(() => {
    const committedTrimmedSearch = committedSearch.trim();
    if (!committedTrimmedSearch) {
      setGlobalResultState({ query: "", results: [] });
      setQueryState("idle");
      setFeedback(null);
      setCommandError("");
      setActiveIndex(0);
      return;
    }

    let cancelled = false;
    setCommandError("");
    setGlobalResultState({ query: committedTrimmedSearch, results: [] });
    setQueryState("pending");
    setFeedback(null);
    const timer = window.setTimeout(() => {
      const request = queryControllerRef.current.nextRequest(committedTrimmedSearch, SEARCH_RESULT_LIMIT);
      tauriApi.searchGlobalEntries(request)
        .then((response) => {
          if (cancelled || !queryControllerRef.current.accepts(response)) return;
          setGlobalResultState({ query: response.normalizedQuery, results: response.results });
          setGlobalIndexStatus(response.indexStatus);
          setFeedback(response.indexStatus.platform === "windows" ? globalSearchFeedback(response, t) : null);
          setQueryState(
            response.resultState === "complete"
              ? "complete"
              : response.resultState === "pending"
                ? "partial"
                : response.resultState
          );
          setActiveIndex(0);
          if (queryControllerRef.current.acceptSourceRevision(response.sourceRevision)) {
            setRevisionRefetchNonce((value) => value + 1);
          }
        })
        .catch((error) => {
          if (cancelled) return;
          setGlobalResultState({ query: committedTrimmedSearch, results: [] });
          setQueryState("failed");
          const message = readableError(error);
          setCommandError(message === "global_index_database_busy"
            ? t("globalIndexErrorDatabaseBusy")
            : message === "global_index_service_busy"
              ? t("globalIndexErrorServiceBusy")
              : message === "global_index_permission_required"
                ? t("globalIndexErrorPermissionRequired")
                : t("commandSearchFailed"));
        });
    }, 50);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [committedSearch, revisionRefetchNonce, t]);

  useEffect(() => {
    setActiveIndex((index) => Math.min(index, Math.max(0, visibleResults.length - 1)));
  }, [visibleResults.length]);

  useEffect(() => {
    if (!showResults || !activeResultId) return;
    document.getElementById(activeResultId)?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, activeResultId, showResults]);

  async function chooseGlobalEntry(entry: GlobalSearchResult) {
    if (libraryActionPending) return;
    try {
      await tauriApi.openGlobalSearchResult(entry.id);
      closeSpotlight();
    } catch (error) {
      const message = readableError(error);
      setCommandError(message);
      onError?.(message);
    }
  }

  async function showInFileLibrary(entry: GlobalSearchResult) {
    if (isComposingRef.current || libraryActionPending) return;
    const action = ++libraryActionRef.current;
    const query = queryIntentRef.current;
    const accept = () => libraryActionRef.current === action && queryIntentRef.current === query;
    setLibraryActionPending(true);
    trackStandaloneActivation(true);
    setCommandError("");
    try {
      if (standalone) {
        await tauriApi.activateSearchResult("library", null, searchWindowSnapshot ?? undefined, null, entry.id);
      } else if (await openGlobalEntry?.(entry.id, accept)) {
        setView("library");
        closeSpotlight();
      } else if (accept()) {
        throw new Error("search_navigation_rejected");
      }
    } catch (error) {
      if (accept()) {
        const message = `${t("globalSearchLibraryFailed")} ${readableError(error)}`;
        setCommandError(message);
        onError?.(message);
      }
    } finally {
      trackStandaloneActivation(false);
      if (libraryActionRef.current === action) setLibraryActionPending(false);
    }
  }

  async function openSortingPreview() {
    try {
      await activateCommandNavigation({
        standalone,
        windowSnapshot: searchWindowSnapshotRef.current,
        view: "preview",
        fileId: null,
        setView,
        setSelectedFileId,
        activateFileLibraryFile,
        onClose,
        onStandaloneActivationStateChange: trackStandaloneActivation
      });
    } catch (error) {
      const message = readableError(error);
      setCommandError(message);
      onError?.(message);
    }
  }

  function clearSearch() {
    if (standalone && libraryActionPending) return;
    setSearch("");
    setCommittedSearch("");
    setActiveIndex(0);
    window.setTimeout(() => inputRef.current?.focus(), 0);
  }

  async function chooseCommand(command: SpotlightCommand) {
    if (!command.enabled) {
      setCommandError(command.disabledReason ?? "command_unavailable");
      return;
    }
    try {
      if (standalone) {
        await activateCommandNavigation({
          standalone,
          windowSnapshot: searchWindowSnapshotRef.current,
          view: command.view,
          fileId: null,
          settingsTarget: settingsTargetForSection(command.settingsSection),
          setView,
          setSelectedFileId,
          activateFileLibraryFile,
          onClose,
          onStandaloneActivationStateChange: trackStandaloneActivation
        });
        return;
      }
      settingsCommandSectionRef.current = command.settingsSection ?? null;
      executeSpotlightCommand(command, { setView, requestSettingsSection, onClose });
    } catch (error) {
      const message = readableError(error);
      setCommandError(message);
      onError?.(message);
    }
  }

  function chooseResult(result: SpotlightResult) {
    if (libraryActionPending) return;
    if (result.kind === "global") void chooseGlobalEntry(result.entry);
    else void chooseCommand(result);
  }

  function openIdleDestination(view: View) {
    void activateCommandNavigation({
      standalone,
      windowSnapshot: searchWindowSnapshotRef.current,
      view,
      fileId: null,
      setView,
      setSelectedFileId,
      activateFileLibraryFile,
      onClose,
      onStandaloneActivationStateChange: trackStandaloneActivation
    }).catch((error) => {
      const message = readableError(error);
      setCommandError(message);
      onError?.(message);
    });
  }

  function openGlobalIndexSettings() {
    const command = commandRegistry.find((item) => item.id === "global-index-settings");
    if (command) void chooseCommand(command);
  }

  const content = (
    <div
      className={cn(
        standalone
          ? "relative z-10 flex h-full w-full items-start justify-center bg-transparent pt-8 px-8"
          : "fixed inset-0 z-40 flex items-start justify-center bg-[var(--zc-overlay)] px-5 pt-[15vh] sm:pt-[20vh]"
      )}
      onMouseDown={(event) => event.target === event.currentTarget && closeSpotlight()}
    >
      <motion.div
        layout
        className={cn(
          commandShellBase,
          isStandaloneCollapsed ? commandShellCollapsed : commandShellExpanded,
          !isStandaloneCollapsed && (standalone ? commandShellStandaloneExpanded : commandShellDialogWidth)
        )}
        initial={prefersReducedMotion ? false : { opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: 8 }}
        transition={prefersReducedMotion ? { duration: 0 } : { duration: 0.18, ease: [0.2, 0.8, 0.2, 1] }}
        role={standalone ? "search" : "dialog"}
        aria-modal={standalone ? undefined : true}
        aria-label={t("globalSearch")}
        aria-busy={queryState === "pending"}
        onKeyDown={(event) => {
          if (isComposingRef.current || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
          if (event.key === "Escape") {
            event.preventDefault();
            closeSpotlight();
            return;
          }
          // Buttons retain native activation/focus behavior; Enter on Refresh
          // must never open the currently highlighted file.
          if (event.target instanceof Element && event.target.closest("button")) return;
          const tabLeavesResults = event.shiftKey ? activeIndex === 0 : activeIndex === visibleResults.length - 1;
          if (event.key === "Tab" && visibleResults.length > 0 && !tabLeavesResults) {
            event.preventDefault();
            setActiveIndex((index) => event.shiftKey
              ? Math.max(index - 1, 0)
              : Math.min(index + 1, visibleResults.length - 1));
            inputRef.current?.focus();
            return;
          }
          if ((event.metaKey && event.key === "Backspace") || (event.ctrlKey && event.key === "Backspace")) {
            event.preventDefault();
            clearSearch();
          }
          if (event.key === "Home") {
            event.preventDefault();
            setActiveIndex(0);
          }
          if (event.key === "End") {
            event.preventDefault();
            setActiveIndex(Math.max(0, visibleResults.length - 1));
          }
          if (event.key === "PageDown") {
            event.preventDefault();
            setActiveIndex((index) => Math.min(index + 5, Math.max(0, visibleResults.length - 1)));
          }
          if (event.key === "PageUp") {
            event.preventDefault();
            setActiveIndex((index) => Math.max(index - 5, 0));
          }
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setActiveIndex((index) => Math.min(index + 1, Math.max(0, visibleResults.length - 1)));
          }
          if (event.key === "ArrowUp") {
            event.preventDefault();
            setActiveIndex((index) => Math.max(index - 1, 0));
          }
          if (libraryActionPending) return;
          const activeResult = visibleResults[activeIndex];
          if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && activeResult?.kind === "global") {
            event.preventDefault();
            void tauriApi.revealGlobalSearchResult(activeResult.entry.id)
              .then(closeSpotlight)
              .catch((error) => {
                const message = readableError(error);
                setCommandError(message);
                onError?.(message);
              });
            return;
          }
          if (isSortingPreviewShortcut(event) && activeResult?.kind !== "global") {
            event.preventDefault();
            void openSortingPreview();
            return;
          }
          if (event.key === "Enter" && activeResult) {
            event.preventDefault();
            chooseResult(activeResult);
          }
        }}
      >
        <div
          className={cn(
            isStandaloneCollapsed ? commandInputRowCollapsed : commandInputRowBase,
            inputFocused && commandInputRowFocused
          )}
        >
          <span className={commandSearchIcon}>
            <Search size={18} strokeWidth={2.2} />
          </span>
          <input
            ref={inputRef}
            role="combobox"
            aria-expanded={showResults}
            aria-controls="command-results"
            aria-activedescendant={activeResultId}
            value={search}
            readOnly={standalone && libraryActionPending}
            aria-busy={libraryActionPending}
            placeholder={t("commandPlaceholder")}
            onChange={(event) => {
              const value = event.target.value;
              const nativeEvent = event.nativeEvent as Event & {
                isComposing?: boolean;
                keyCode?: number;
              };
              setSearch(value);
              const committed = committedSpotlightInput(
                value,
                isComposingRef.current,
                nativeEvent.isComposing === true,
                nativeEvent.keyCode ?? 0
              );
              if (committed !== null) setCommittedSearch(committed);
            }}
            onCompositionStart={() => {
              isComposingRef.current = true;
              setIsComposing(true);
            }}
            onCompositionEnd={(event) => {
              isComposingRef.current = false;
              setIsComposing(false);
              const value = completedSpotlightComposition(event.currentTarget.value);
              setSearch(value);
              setCommittedSearch(value);
            }}
            onClick={() => inputRef.current?.focus()}
            onFocus={() => setInputFocused(true)}
            onBlur={() => setInputFocused(false)}
            className={commandInput}
          />
          {search && (
            <IconButton
              className="h-8 w-8 rounded-lg border-transparent bg-transparent text-[var(--zc-text-secondary)] shadow-none hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]"
              onClick={clearSearch}
              disabled={standalone && libraryActionPending}
              aria-label={t("commandClearSearch")}
              title={t("commandClearSearch")}
            >
              <X size={16} strokeWidth={2.5} />
            </IconButton>
          )}
          <kbd className={cn(keyBadge, "hidden sm:inline-flex")}>ESC</kbd>
        </div>
        {commandError && queryState !== "failed" && (
          <div className="px-4 py-2 text-sm text-[var(--zc-text-primary)]" role="alert">{commandError}</div>
        )}
        {showGlobalIndexMeta && (
          <div className="flex items-center justify-between gap-3 border-b border-[var(--zc-divider)] px-4 py-2 text-[11px] leading-tight text-[var(--zc-text-secondary)]">
            <span className="min-w-0 truncate">{globalSearchIndexMeta}</span>
            {trimmedSearch ? <button
              className={cn("shrink-0 rounded-md px-2 py-1 font-medium text-[var(--zc-primary-text)] hover:bg-[var(--zc-primary-soft)] disabled:opacity-50", focusVisibleState)}
              disabled={isComposing || queryState === "pending" || libraryActionPending}
              onClick={() => setRevisionRefetchNonce((value) => value + 1)}
            >{t("globalSearchRefreshResults")}</button> : null}
            <button
              className={cn("shrink-0 rounded-md px-2 py-1 font-medium text-[var(--zc-primary-text)] hover:bg-[var(--zc-primary-soft)]", focusVisibleState)}
              disabled={libraryActionPending}
              onClick={openGlobalIndexSettings}
              aria-label={t("globalSearchManage")}
            >
              {t("globalSearchManage")}
            </button>
          </div>
        )}
        {showResults && (
          <div className={commandResultsShell}>
            <div className={commandResultsBody}>
              <div className={commandResultsHeader}>
                <span>{t("smartMatches")}</span>
                <span className={cn(quietText, "hidden sm:inline")}>{t("commandKeyboardHint")}</span>
              </div>
              <SpotlightResultGroups
                groups={resultGroups}
                results={visibleResults}
                activeIndex={activeIndex}
                highlight={trimmedSearch}
                t={t}
                onChoose={chooseResult}
                onActivate={setActiveIndex}
                onContextAction={(index) => {
                  setActiveIndex(index);
                  requestAnimationFrame(() => libraryActionButtonRef.current?.focus());
                }}
              />
              {visibleResults[activeIndex]?.kind === "global" && (
                <button ref={libraryActionButtonRef} type="button"
                  className={cn("mt-2 rounded-md px-3 py-2 text-xs text-[var(--zc-primary-text)] hover:bg-[var(--zc-primary-soft)] disabled:opacity-50", focusVisibleState)}
                  disabled={isComposing || libraryActionPending}
                  onClick={() => {
                    const result = visibleResults[activeIndex];
                    if (result?.kind === "global") void showInFileLibrary(result.entry);
                  }}
                ><Folder size={14} className="mr-1 inline" aria-hidden="true" />{t("globalSearchShowInLibrary")}</button>
              )}
            </div>
            <div className={commandFooter}>
              <span>{formatCount(t, visibleResults.length, { zero: "matchesFoundZero", one: "matchesFoundOne", other: "matchesFoundOther" })}</span>
              <div className={shortcutHints}>
                <ShortcutHint badge={<CornerDownLeft className="w-3 h-3" />} label={t("commandOpenHint")} />
                <ShortcutHint badge="↑↓" label={t("commandNavigateHint")} />
                <ShortcutHint badge="ESC" label={t("commandCloseHint")} />
              </div>
            </div>
          </div>
        )}
        {shouldShowIdleState && (
          <CommandIdleGroups
            t={t}
            backgroundStatus={backgroundStatus}
            onOpen={openIdleDestination}
          />
        )}
        {shouldShowStateBlock && (
          <div className="px-4 py-4" aria-live={queryState === "failed" ? "assertive" : "polite"} role={queryState === "failed" ? "alert" : "status"}>
            <StateBlock
              tone={queryState === "failed" ? "error" : queryState === "pending" || queryState === "partial" || queryState === "no_source" ? "info" : "neutral"}
              title={statusTitle}
              description={statusDescription}
              density="compact"
            />
          </div>
        )}
      </motion.div>
    </div>
  );

  function restoreSpotlightFocus() {
    return findSpotlightSettingsRestoreTarget(settingsCommandSectionRef.current, restoreFocusRef?.current ?? null);
  }

  const spotlight = standalone
    ? content
    : <ModalPortal initialFocusRef={inputRef} restoreFocus={restoreSpotlightFocus} onEscape={closeSpotlight}>{content}</ModalPortal>;

  return spotlight;
}

function SpotlightResultGroups({
  groups,
  results,
  activeIndex,
  highlight,
  t,
  onChoose,
  onActivate,
  onContextAction
}: {
  groups: ReturnType<typeof groupSpotlightResults>;
  results: SpotlightResult[];
  activeIndex: number;
  highlight: string;
  t: Translator;
  onChoose: (result: SpotlightResult) => void;
  onActivate: (index: number) => void;
  onContextAction: (index: number) => void;
}) {
  return (
    <div id="command-results" role="listbox" className="grid gap-2">
      {groups.map((group) => (
        <section className="grid gap-1" key={group.type} aria-label={group.label}>
          <h3 className={commandResultsHeader}>{group.label}</h3>
          <div className={commandResultsList}>
            {group.items.map((result) => {
              const index = results.indexOf(result);
              const active = index === activeIndex;
              if (result.kind === "command") {
                return (
                  <button
                    key={`command:${result.id}`}
                    id={`command-result-${index}`}
                    role="option"
                    aria-selected={active}
                    data-result-kind="command"
                    className={cn(commandResultItemBase, active ? commandResultItemActive : commandResultItemInactive)}
                    onClick={() => onChoose(result)}
                    onMouseMove={() => onActivate(index)}
                  >
                    <span className="grid h-10 w-10 place-items-center rounded-lg bg-[var(--zc-primary-soft)] text-[var(--zc-primary-text)]">
                      <Search size={18} />
                    </span>
                    <span className="grid min-w-0 gap-1">
                      <strong className={commandFileName}><HighlightText text={result.label} highlight={highlight} /></strong>
                      <span className={commandFileMeta}>{result.description}</span>
                    </span>
                    <ChevronRight className={active ? "text-[var(--zc-primary)]" : "text-[var(--zc-text-tertiary)]"} size={16} />
                  </button>
                );
              }

              const entry = result.entry;
              const extension = entry.extension ? entry.extension.replace(".", "").toUpperCase() : t("spotlightFolders");
              return (
                <button
                  key={result.id}
                  id={`command-result-${index}`}
                  role="option"
                  aria-selected={active}
                  data-result-kind="global"
                  onContextMenu={(event) => { event.preventDefault(); onContextAction(index); }}
                  onKeyDown={(event) => {
                    if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
                      event.preventDefault(); onContextAction(index);
                    }
                  }}
                  className={cn(commandResultItemBase, active ? commandResultItemActive : commandResultItemInactive)}
                  onMouseMove={() => onActivate(index)}
                  onClick={() => onChoose(result)}
                >
                  <span className={cn(commandFileIcon, "bg-[var(--zc-surface-subtle)] text-[var(--zc-primary)]")}>
                    {entry.isDirectory ? <Folder size={20} /> : <FileIcon size={20} />}
                  </span>
                  <span className="grid min-w-0 gap-1.5 text-left">
                    <strong className={commandFileName}><HighlightText text={entry.name} highlight={highlight} /></strong>
                    <span className={commandFileMeta} title={formatDisplayPath(entry.path)}>{compactPath(formatDisplayPath(entry.path), 74)}</span>
                    <span className="flex min-w-0 flex-wrap items-center gap-1.5">
                      <span className="text-[10px] font-medium uppercase tracking-[0.08em] text-[var(--zc-text-tertiary)]">{extension}</span>
                      {entry.managed ? <span className="text-[10px] font-medium text-[var(--zc-primary-text)]">{t("globalSearchManaged")}</span> : null}
                    </span>
                  </span>
                  <span className="flex items-center gap-1">
                    <ChevronRight className={active ? "text-[var(--zc-primary)]" : "text-[var(--zc-text-tertiary)]"} size={16} />
                  </span>
                </button>
              );
            })}
          </div>
        </section>
      ))}
    </div>
  );
}

function CommandIdleGroups({
  t,
  backgroundStatus,
  onOpen
}: {
  t: Translator;
  backgroundStatus?: {
    isRunning: boolean;
    currentRoot: string | null;
    pending: number;
  };
  onOpen: (view: View) => void;
}) {
  const isBackgroundIndexing = backgroundStatus?.isRunning ?? false;
  const currentBackgroundRoot = backgroundStatus?.currentRoot ?? null;
  const pendingBackgroundRoots = backgroundStatus?.pending ?? 0;
  const backgroundDescription = isBackgroundIndexing && currentBackgroundRoot
    ? compactPath(formatDisplayPath(currentBackgroundRoot), 42)
    : pendingBackgroundRoots > 0
      ? t("spotlightPendingTasks").replace("{count}", String(pendingBackgroundRoots))
      : t("spotlightNoBackgroundTasks");

  return (
    <>
      <div className={commandIdleGroups} aria-label={t("commandIdleTitle")}>
        <IdleGroup title={t("spotlightCommonTasks")}>
          <IdleAction icon={<Radar size={17} className="text-[var(--zc-primary)]" aria-hidden="true" />} label={t("overview")} onClick={() => onOpen("scanner")} />
          <IdleAction icon={<LayoutGrid size={17} className="text-[var(--zc-primary)]" aria-hidden="true" />} label={t("organizeFiles")} onClick={() => onOpen("organize")} />
        </IdleGroup>
      </div>
      {backgroundStatus ? (
        <div className={commandBackgroundStatus} role="status" aria-label={t("spotlightBackgroundTasks")}>
          <Activity size={15} className={isBackgroundIndexing ? "animate-pulse text-[var(--zc-primary)]" : "text-[var(--zc-text-tertiary)]"} />
          <span className="min-w-0 truncate">{backgroundDescription}</span>
        </div>
      ) : null}
    </>
  );
}

function IdleGroup({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className={commandIdleGroup}>
      <h3 className="px-2 text-xs font-semibold text-[var(--zc-text-tertiary)]">{title}</h3>
      {children}
    </section>
  );
}

function IdleAction({
  icon,
  label,
  onClick
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button className={commandIdleAction} onClick={onClick}>
      {icon}
      <span>{label}</span>
      <ChevronRight size={15} className="ml-auto text-[var(--zc-text-tertiary)]" />
    </button>
  );
}

function ShortcutHint({ badge, label }: { badge: React.ReactNode; label: string }) {
  return (
    <span className={shortcutHint}>
      <kbd className={keyBadge}>{badge}</kbd>
      <span className={shortcutHintLabel}>{label}</span>
    </span>
  );
}

function globalIndexStatusLabel(status: string, t: Translator) {
  if (status === "ready") return t("globalIndexStatusReady");
  if (status === "indexing") return t("globalIndexStatusIndexing");
  if (status === "syncing") return t("globalIndexStatusSyncing");
  if (status === "paused") return t("globalIndexStatusPaused");
  if (status === "partial") return t("globalIndexStatusPartial");
  if (status === "rebuild_required") return t("globalIndexStatusRebuildRequired");
  if (status === "permission_required") return t("globalIndexStatusPermissionRequired");
  if (status === "unavailable") return t("globalIndexStatusUnavailable");
  if (status === "error") return t("globalIndexStatusError");
  return t("globalIndexStatusUnknown");
}

function HighlightText({ text, highlight }: { text: string; highlight: string }) {
  const value = highlight.trim();
  if (!value) return <>{text}</>;
  const escaped = value.replace(/[-/\\^$*+?.()|[\]{}]/g, "\\$&");
  const matcher = new RegExp(`(${escaped})`, "ig");
  return (
    <>
      {text.split(matcher).map((part, index) => (
        part.toLowerCase() === value.toLowerCase()
          ? <mark className={highlightMark} key={`${part}-${index}`}>{part}</mark>
          : <span key={`${part}-${index}`}>{part}</span>
      ))}
    </>
  );
}
