import { FolderComparison } from "../compare/FolderComparison";
import { Columns2, Copy, FolderInput, Plus, X } from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ComponentType,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import {
  FileLibraryExperienceProvider,
  useFileLibraryExperience,
} from "../FileLibraryExperienceProvider";
import { useChromeContext, useI18nContext } from "../../../contexts/AppContexts";
import { defaultFileLibraryProjection } from "../../../store/useFileLibraryV2Store";
import { useFileLibraryStore } from "../../../store/useFileLibraryStore";
import { useAppStore } from "../../../store/useAppStore";
import { useFileManagementStore } from "../management/fileManagementStore";
import type { LibraryScope } from "../../../types/domain";
import type { WorkspacePlatform } from "../../../types/fileWorkspace";
import type { PreviewExperienceController } from "../preview/previewExperienceController";
import { PaneContext, type PaneContextValue } from "./PaneContext";
import {
  navigationTargetFromLibraryRestoreLocator,
} from "../../../fileWorkspace/workspaceSession";
import {
  FileWorkbenchModel,
  type WorkbenchPane,
  type WorkbenchTab,
} from "./workbenchModel";
import {
  clearExpandedTreePaths,
  readExpandedTreePaths,
  readPersistedWorkbench,
  serializeWorkbenchSnapshot,
  writeExpandedTreePaths,
  writePersistedWorkbench,
  type PersistedWorkbenchPane,
} from "./workbenchPersistence";
import type { Translator } from "../../../types/ui";
import { useWorkbenchDrag } from "./useWorkbenchDrag";
import { readableError } from "../../../utils/viewHelpers";
import { useNativeFileDrop } from "./useNativeFileDrop";
import "./workbench.css";
function tabLabel(tab: WorkbenchTab, t: Translator) {
  const pane = tab.panes.find((pane) => pane.id === tab.activePaneId)!;
  return (
    pane.path.split(/[/\\]/).filter(Boolean).at(-1) ||
    t(
      pane.controller.getState().mode === "library"
        ? "fileLibraryModeLibrary"
        : "wbNewFolderTab",
    )
  );
}

export function FileLibraryWorkbench({
  Pane,
  active: workspaceActive = true,
}: {
  Pane: ComponentType;
  active?: boolean;
}) {
  const { controller: rootController } = useFileLibraryExperience();
  const { platform } = useChromeContext();
  const { t } = useI18nContext();
  const tRef = useRef(t);
  tRef.current = t;
  const [model] = useState(() => new FileWorkbenchModel(rootController));
  const state = useSyncExternalStore(model.subscribe, model.getSnapshot);
  const root = useRef<HTMLDivElement | null>(null);
  const [wide, setWide] = useState(false);
  const previews = useRef(new Map<string, PreviewExperienceController>());
  const [mutationEpochs, setMutationEpochs] = useState<Record<string, number>>(
    {},
  );
  const [restoreErrors, setRestoreErrors] = useState<Record<string, string>>({});
  const [workspaceReady, setWorkspaceReady] = useState(false);
  const restoreStarted = useRef(false);
  const pendingDispose = useRef<ReturnType<typeof setTimeout> | null>(null);
  const active = state.tabs.find((tab) => tab.id === state.activeTabId)!;

  useEffect(() => {
    if (restoreStarted.current) return;
    let cancelled = false;
    const saved = readPersistedWorkbench();
    if (!saved) {
      restoreStarted.current = true;
      setWorkspaceReady(true);
      return () => {
        cancelled = true;
      };
    }

    const restoreWorkspace = async () => {
      const runtimeTabs: WorkbenchTab[] = [];
      const treeTransfers: Array<{
        sourcePaneId: string;
        runtimePaneId: string;
        paths: string[];
      }> = [];
      for (let tabIndex = 0; tabIndex < saved.tabs.length; tabIndex += 1) {
        const savedTab = saved.tabs[tabIndex];
        let runtimeTab =
          tabIndex === 0 ? model.getSnapshot().tabs[0] : model.newTab();
        if (!runtimeTab) break;
        if (savedTab.panes.length === 2) {
          model.toggleDual(runtimeTab.id);
          if (!savedTab.dual) model.toggleDual(runtimeTab.id);
        }
        runtimeTab = model
          .getSnapshot()
          .tabs.find((tab) => tab.id === runtimeTab!.id)!;
        const restoredActivePane = runtimeTab.panes[savedTab.activePaneIndex];
        if (restoredActivePane) model.activate(runtimeTab.id, restoredActivePane.id);
        runtimeTabs.push(runtimeTab);
        savedTab.panes.forEach((savedPane, paneIndex) => {
          const runtimePane = runtimeTab.panes[paneIndex];
          if (!runtimePane) return;
          if (savedPane.path) model.updatePath(runtimePane.id, savedPane.path);
          treeTransfers.push({
            sourcePaneId: savedPane.id,
            runtimePaneId: runtimePane.id,
            paths: readExpandedTreePaths(savedPane.id),
          });
        });
      }
      const runtimePaneIds = new Set(
        runtimeTabs.flatMap((tab) => tab.panes.map((pane) => pane.id)),
      );
      treeTransfers.forEach((transfer) =>
        writeExpandedTreePaths(transfer.runtimePaneId, transfer.paths),
      );
      treeTransfers.forEach((transfer) => {
        if (!runtimePaneIds.has(transfer.sourcePaneId))
          clearExpandedTreePaths(transfer.sourcePaneId);
      });

      const restorePane = async (
        runtimePane: WorkbenchPane,
        savedPane: PersistedWorkbenchPane,
        keepActive: boolean,
      ) => {
        if (cancelled) return;
        let ownedEpoch = runtimePane.controller.workspace.session.requestEpoch;
        try {
          // Hidden tab/pane providers suspend their controller on mount. Resume
          // before re-admitting the saved navigation locator; inactive routes
          // are suspended again after their navigation intent is restored.
          await runtimePane.controller.resume();
          if (cancelled) return;
          if (savedPane.restore?.locator.kind === "browse") {
            const pendingAdmission = runtimePane.controller.restoreBrowse(
              savedPane.restore.locator,
            );
            // restoreBrowse advances the request epoch synchronously before
            // awaiting path revalidation.  Record that request so its own
            // failure remains visible, while a newer user navigation still
            // suppresses this stale result.
            ownedEpoch = runtimePane.controller.workspace.session.requestEpoch;
            const admitted = await pendingAdmission;
            if (!admitted) throw new Error(tRef.current("wbNavigationFailed"));
          } else if (savedPane.restore?.locator.kind === "library") {
            const target = navigationTargetFromLibraryRestoreLocator(
              savedPane.restore.locator,
            );
            const navigated = target && runtimePane.controller.navigate(target);
            ownedEpoch = runtimePane.controller.workspace.session.requestEpoch;
            if (!navigated)
              throw new Error(tRef.current("wbNavigationFailed"));
          }
        } catch (cause) {
          if (
            cancelled ||
            runtimePane.controller.workspace.session.requestEpoch !== ownedEpoch
          )
            return;
          setRestoreErrors((current) => ({
            ...current,
            [runtimePane.id]: `${tRef.current("wbNavigationFailed")}: ${readableError(cause)}`,
          }));
        } finally {
          if (!keepActive) await runtimePane.controller.suspend();
        }
      };

      // Re-admit the root controller first, then restore other panes one at a
      // time. Every Browse route gets fresh backend authority on this launch.
      const rootSaved = saved.tabs[0]?.panes[0];
      const rootRuntime = runtimeTabs[0]?.panes[0];
      if (rootSaved && rootRuntime) {
        const keepActive = workspaceActive
          && saved.activeTabIndex === 0
          && (!saved.tabs[0]?.dual || wide || saved.tabs[0]?.activePaneIndex === 0);
        await restorePane(rootRuntime, rootSaved, keepActive);
      }
      for (let tabIndex = 0; tabIndex < runtimeTabs.length; tabIndex += 1) {
        for (let paneIndex = 0; paneIndex < runtimeTabs[tabIndex].panes.length; paneIndex += 1) {
          if (tabIndex === 0 && paneIndex === 0) continue;
          const runtimePane = runtimeTabs[tabIndex].panes[paneIndex];
          const savedPane = saved.tabs[tabIndex]?.panes[paneIndex];
          const savedTab = saved.tabs[tabIndex];
          const keepActive = workspaceActive
            && tabIndex === saved.activeTabIndex
            && (!savedTab?.dual || wide || paneIndex === savedTab?.activePaneIndex);
          if (runtimePane && savedPane) await restorePane(runtimePane, savedPane, keepActive);
        }
      }
      const selectedTab = runtimeTabs[saved.activeTabIndex] ?? runtimeTabs[0];
      const selectedPane =
        selectedTab?.panes[saved.tabs[saved.activeTabIndex]?.activePaneIndex ?? 0];
      if (selectedTab && selectedPane) model.activate(selectedTab.id, selectedPane.id);
      if (!cancelled) setWorkspaceReady(true);
    };
    let started = false;
    const startTimer = setTimeout(() => {
      started = true;
      restoreStarted.current = true;
      void restoreWorkspace();
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(startTimer);
      if (!started) restoreStarted.current = false;
    };
  }, [model]);

  useEffect(() => {
    if (!workspaceReady) return;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const persistNow = () => {
      const runtime: WorkspacePlatform | null =
        platform === "win32"
          ? "windows"
          : platform === "darwin"
            ? "macos"
            : null;
      const snapshot = serializeWorkbenchSnapshot(model.getSnapshot(), runtime);
      if (snapshot) writePersistedWorkbench(snapshot);
    };
    const persist = () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(persistNow, 180);
    };
    const flush = () => {
      if (timer) clearTimeout(timer);
      persistNow();
    };
    const onVisibilityChange = () => {
      if (document.visibilityState === "hidden") flush();
    };
    const unsubscribe = state.tabs.map((tab) =>
      tab.panes.map((pane) => pane.controller.subscribe(persist)),
    ).flat(2);
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", onVisibilityChange);
    persist();
    return () => {
      if (timer) clearTimeout(timer);
      flush();
      window.removeEventListener("pagehide", flush);
      document.removeEventListener("visibilitychange", onVisibilityChange);
      unsubscribe.forEach((stop) => stop());
    };
  }, [model, platform, state.activeTabId, state.tabs, workspaceReady]);
  useLayoutEffect(() => {
    if (useFileManagementStore.getState().activePaneId !== active.activePaneId)
      useFileManagementStore.getState().activatePane(active.activePaneId);
  }, [active.activePaneId]);
  const locked = useFileManagementStore((s) => s.transferOpen);
  const transferActive = useFileManagementStore((s) => s.transferActive);
  const transferLocked = locked || transferActive;
  const management = useFileManagementStore();
  const focus = useCallback(
    (tabId: string, paneId: string) => {
      if (useFileManagementStore.getState().transferOpen) return;
      const current = model.getSnapshot();
      if (
        current.activeTabId === tabId &&
        current.tabs.find((tab) => tab.id === tabId)?.activePaneId === paneId &&
        useFileManagementStore.getState().activePaneId === paneId
      )
        return;
      useFileManagementStore.getState().activatePane(paneId);
      model.activate(tabId, paneId);
    },
    [model],
  );
  const add = useCallback(() => {
    if (useFileManagementStore.getState().transferOpen) return;
    const tab = model.newTab();
    if (!tab) useAppStore.getState().showError(t("wbTabLimit"));
    else focus(tab.id, tab.activePaneId);
  }, [focus, model, t]);
  const closeTab = useCallback(
    (tabId: string) => {
      if (useFileManagementStore.getState().transferOpen) return;
      model
        .getSnapshot()
        .tabs.find((tab) => tab.id === tabId)
        ?.panes.forEach((pane) => clearExpandedTreePaths(pane.id));
      void model.close(tabId);
    },
    [model],
  );
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const measure = () => setWide(el.clientWidth >= 1000);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (pendingDispose.current) clearTimeout(pendingDispose.current);
    return () => {
      pendingDispose.current = setTimeout(() => {
        void model.dispose();
      }, 0);
    };
  }, [model]);
  useEffect(() => {
    let lastSearchSession =
      rootController.getState().workspace.browse?.initialPage?.sessionId;
    const unsubscribe = rootController.subscribe((next) => {
      const admission = next.workspace.browse;
      if (admission?.initialPage && admission.sessionId !== lastSearchSession) {
        lastSearchSession = admission.sessionId;
        const tab = model
          .getSnapshot()
          .tabs.find((tab) =>
            tab.panes.some((pane) => pane.controller === rootController),
          );
        const rootPane = tab?.panes.find(
          (pane) => pane.controller === rootController,
        );
        if (
          !tab ||
          model.getSnapshot().activeTabId !== tab.id ||
          tab.activePaneId !== rootPane?.id
        )
          model.revealRoot();
      }
    });
    return () => {
      unsubscribe();
    };
  }, [model, rootController]);
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (
        !workspaceActive ||
        event.defaultPrevented ||
        event.isComposing ||
        event.altKey ||
        !(event.ctrlKey || event.metaKey) ||
        document.querySelector('[aria-modal="true"]') ||
        useFileManagementStore.getState().transferOpen
      )
        return;
      const snapshot = model.getSnapshot();
      const tab = snapshot.tabs.find(
        (item) => item.id === snapshot.activeTabId,
      )!;
      if (event.key.toLowerCase() === "t") {
        event.preventDefault();
        add();
      } else if (event.key.toLowerCase() === "w") {
        event.preventDefault();
        void model.close(tab.id);
      } else if (event.key === "Tab") {
        event.preventDefault();
        const index = snapshot.tabs.indexOf(tab);
        const next =
          snapshot.tabs[
            (index + (event.shiftKey ? -1 : 1) + snapshot.tabs.length) %
              snapshot.tabs.length
          ];
        focus(next.id, next.activePaneId);
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [add, focus, model, workspaceActive]);
  const beforeMutation = useCallback(async () => {
    await Promise.all(
      [...previews.current.values()].map((preview) =>
        preview.closeForFileOperation(),
      ),
    );
  }, []);
  const afterMutation = useCallback(
    async (directories: string[]) => {
      // These strings only route projection refresh, never filesystem authorization.
      const key = (path: string) =>
        path
          .replaceAll("\\", "/")
          .replace(/^\/\/\?\//, "")
          .replace(/\/+$/, "");
      const touched = new Set(directories.map(key));
      const affected = model
        .getSnapshot()
        .tabs.flatMap((tab) => tab.panes)
        .filter(
          (pane) =>
            pane.controller.getState().mode === "library" ||
            touched.has(key(pane.path)),
        );
      setMutationEpochs((epochs) => {
        const next = { ...epochs };
        affected.forEach((pane) => {
          next[pane.id] = (next[pane.id] ?? 0) + 1;
        });
        return next;
      });
    },
    [model],
  );
  const focusedSurface = management.surfaces[active.activePaneId]?.read();
  const peer = active.dual
    ? active.panes.find((pane) => pane.id !== active.activePaneId)
    : undefined;
  const canTransfer =
    !!peer?.path && !!focusedSurface && focusedSurface.count !== 0 && !transferLocked;
  const {
    feedback: internalDrop,
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel,
    onLostPointerCapture,
    onClickCapture,
  } = useWorkbenchDrag({
    workbenchRef: root,
    model,
    wide,
    dual: active.dual,
    active: workspaceActive,
    locked: transferLocked,
    focus,
    t,
  });
  const externalDrop = useNativeFileDrop({
    workbenchRef: root,
    model,
    wide,
    active: workspaceActive,
    locked: transferLocked,
    focus,
    t,
  });
  return (
    <div
      ref={root}
      className="wb-workbench"
      data-workbench
      data-dual={active.dual}
      data-wide={wide}
    >
      <header className="wb-tabs-row">
        <div className="wb-tabs" role="tablist" aria-label={t("wbTabs")}>
          {state.tabs.map((tab, index) => (
            <div
              key={tab.id}
              className="wb-tab"
              data-active={tab.id === state.activeTabId}
            >
              <button
                role="tab"
                aria-label={`${t("wbTabPrefix")}: ${tabLabel(tab, t)}`}
                id={`wb-tab-${tab.id}`}
                aria-controls={`wb-panel-${tab.id}`}
                aria-selected={tab.id === state.activeTabId}
                tabIndex={tab.id === state.activeTabId ? 0 : -1}
                disabled={locked}
                title={
                  tab.panes.find((pane) => pane.id === tab.activePaneId)
                    ?.path || t("fileLibraryModeLibrary")
                }
                onClick={() => focus(tab.id, tab.activePaneId)}
                onKeyDown={(event) => {
                  if (
                    !["ArrowLeft", "ArrowRight", "Home", "End"].includes(
                      event.key,
                    )
                  )
                    return;
                  event.preventDefault();
                  const nextIndex =
                    event.key === "Home"
                      ? 0
                      : event.key === "End"
                        ? state.tabs.length - 1
                        : (index +
                            (event.key === "ArrowLeft" ? -1 : 1) +
                            state.tabs.length) %
                          state.tabs.length;
                  const next = state.tabs[nextIndex];
                  focus(next.id, next.activePaneId);
                  root.current
                    ?.querySelector<HTMLElement>(`#wb-tab-${next.id}`)
                    ?.focus();
                }}
              >
                <span className="wb-tab-label">{tabLabel(tab, t)}</span>
                {tab.dual && <Columns2 size={12} />}
              </button>
              <button
                className="wb-tab-close"
                disabled={locked}
                aria-label={`${t("wbCloseTab")} ${index + 1}`}
                onClick={() => closeTab(tab.id)}
              >
                <X size={12} />
              </button>
            </div>
          ))}
        </div>
        <button
          className="wb-icon"
          disabled={locked}
          title={t("wbNewTab")}
          aria-label={t("wbNewTab")}
          onClick={add}
        >
          <Plus size={17} />
        </button>
        <button
          className="wb-split-button"
          disabled={locked}
          aria-pressed={active.dual}
          onClick={() => model.toggleDual(active.id)}
        >
          <Columns2 size={16} />
          <span>{t(active.dual ? "wbSinglePane" : "wbDualPane")}</span>
        </button>
      </header>
      {active.dual && (
        <div className="wb-pane-actions">
          {active.panes.map((pane, index) => (
            <button
              key={pane.id}
              className="wb-side-button"
              aria-pressed={active.activePaneId === pane.id}
              disabled={locked}
              onClick={() => focus(active.id, pane.id)}
            >
              {t(index === 0 ? "wbLeftPane" : "wbRightPane")}
            </button>
          ))}
          <span className="wb-pane-hint">
            {!wide ? t("wbCompactPane") : t("wbDragHint")}
          </span>
          <button
            className="wb-cross-button"
            disabled={!canTransfer}
            onClick={() =>
              useFileManagementStore.getState().requestAction("copy_other")
            }
          >
            <Copy size={14} />
            {t("wbCopyOther")}
          </button>
          <button
            className="wb-cross-button"
            disabled={!canTransfer}
            onClick={() =>
              useFileManagementStore.getState().requestAction("move_other")
            }
          >
            <FolderInput size={14} />
            {t("wbMoveOther")}
          </button>
        </div>
      )}
      {active.dual && <FolderComparison key={active.id} compact={!wide} paneIds={active.panes.map((pane) => pane.id)} activate={(id) => focus(active.id, id)} />}
      {state.tabs.map((tab) => (
        <div
          key={tab.id}
          id={`wb-panel-${tab.id}`}
          role="tabpanel"
          aria-labelledby={`wb-tab-${tab.id}`}
          className="wb-tab-panel"
          hidden={state.activeTabId !== tab.id}
          data-dual={tab.dual && wide}
        >
          {tab.panes.map((pane, index) => {
            const visible =
              workspaceActive &&
              state.activeTabId === tab.id &&
              ((index === 0 && !tab.dual) ||
                (tab.dual && (wide || tab.activePaneId === pane.id)));
            return (
              <PaneHost
                key={pane.id}
                pane={pane}
                Pane={Pane}
                visible={visible}
                focused={visible && tab.activePaneId === pane.id}
                wideDualPane={tab.dual && wide}
                peerId={
                  tab.dual
                    ? tab.panes.find((other) => other.id !== pane.id)?.id
                    : undefined
                }
                mutationEpoch={mutationEpochs[pane.id] ?? 0}
                restoreError={restoreErrors[pane.id] ?? ""}
                activate={() => focus(tab.id, pane.id)}
                activatePeer={() => {
                  const other = tab.panes.find((other) => other.id !== pane.id);
                  if (other) focus(tab.id, other.id);
                }}
                updateLocation={(path, targetKey) => {
                  model.updatePath(pane.id, path, targetKey ?? null);
                  if (path) {
                    setRestoreErrors((current) => {
                      if (!current[pane.id]) return current;
                      const next = { ...current };
                      delete next[pane.id];
                      return next;
                    });
                  }
                }}
                registerPreview={(preview) => {
                  if (preview) previews.current.set(pane.id, preview);
                  else previews.current.delete(pane.id);
                }}
                beforeMutation={beforeMutation}
                afterMutation={afterMutation}
                onPointerDown={(event) => onPointerDown(event, tab.id, pane.id, tab.dual ? tab.panes.find((other) => other.id !== pane.id)?.id : undefined, visible)}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
                onPointerCancel={onPointerCancel}
                onLostPointerCapture={onLostPointerCapture}
                onClickCapture={onClickCapture}
                internalDrop={internalDrop?.paneId === pane.id ? `${t(internalDrop.action === "copy" ? "wbCopyOther" : "wbMoveOther")} → ${internalDrop.label}` : undefined}
                externalDrop={externalDrop?.paneId === pane.id ? `${externalDrop.count} ${t(externalDrop.count === 1 ? "wbDropItem" : "wbDropItems")} → ${externalDrop.label}` : undefined}
              />
            );
          })}
        </div>
      ))}
    </div>
  );
}
function PaneHost({
  pane,
  Pane,
  mutationEpoch,
  restoreError,
  ...props
}: Omit<
  PaneContextValue,
  | "id"
  | "rootRef"
  | "projection"
  | "scope"
  | "setScope"
  | "routingHint"
  | "updateLocation"
  | "registerPreview"
> & {
  pane: WorkbenchPane;
  Pane: ComponentType;
  mutationEpoch: number;
  restoreError: string;
  updateLocation: (path: string, targetKey?: string | null) => void;
  registerPreview: (preview: PreviewExperienceController | null) => void;
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onPointerMove: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onPointerUp: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onPointerCancel: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onLostPointerCapture: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onClickCapture: (event: ReactMouseEvent<HTMLDivElement>) => void;
  internalDrop?: string;
  externalDrop?: string;
}) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const globalScope = useFileLibraryStore((state) => state.scope);
  const [localScope, setLocalScope] = useState<LibraryScope>(() => globalScope);
  const scope =
    pane.projection === defaultFileLibraryProjection ? globalScope : localScope;
  const setScope =
    pane.projection === defaultFileLibraryProjection
      ? useFileLibraryStore.getState().setScope
      : setLocalScope;
  const seenMutation = useRef(mutationEpoch);
  const pathUpdater = useRef(props.updateLocation);
  pathUpdater.current = props.updateLocation;
  const previewUpdater = useRef(props.registerPreview);
  previewUpdater.current = props.registerPreview;
  const updateLocation = useCallback(
    (path: string, targetKey?: string | null) => pathUpdater.current(path, targetKey),
    [],
  );
  const registerPreview = useCallback(
    (preview: PreviewExperienceController | null) =>
      previewUpdater.current(preview),
    [],
  );
  useEffect(() => {
    if (!props.visible || seenMutation.current === mutationEpoch) return;
    seenMutation.current = mutationEpoch;
    const surface = useFileManagementStore.getState().surfaces[pane.id]?.read();
    void surface
      ?.refresh()
      .catch((cause) => useAppStore.getState().showError(String(cause)));
  }, [props.visible, mutationEpoch, pane.id]);
  const value: PaneContextValue = {
    ...props,
    id: pane.id,
    rootRef,
    projection: pane.projection,
    scope,
    setScope,
    routingHint: pane.path,
    pathTargetKey: pane.pathTargetKey,
    updateLocation,
    registerPreview,
  };
  return (
    <div
      ref={rootRef}
      className="wb-pane"
      data-pane-id={pane.id}
      data-focused={props.focused}
      data-external-drop={props.externalDrop ? "active" : undefined}
      data-external-drop-label={props.externalDrop}
      data-internal-drop={props.internalDrop ? "active" : undefined}
      data-internal-drop-label={props.internalDrop}
      hidden={!props.visible}
      onPointerDownCapture={props.activate}
      onFocusCapture={props.activate}
      onPointerDown={props.onPointerDown}
      onPointerMove={props.onPointerMove}
      onPointerUp={props.onPointerUp}
      onPointerCancel={props.onPointerCancel}
      onLostPointerCapture={props.onLostPointerCapture}
      onClickCapture={props.onClickCapture}
    >
      {restoreError && (
        <div className="wb-pane-hint" role="alert">
          {restoreError}
        </div>
      )}
      <PaneContext.Provider value={value}>
        <FileLibraryExperienceProvider
          active={props.visible}
          controller={pane.controller}
          disposeOnUnmount={false}
        >
          <Pane />
        </FileLibraryExperienceProvider>
      </PaneContext.Provider>
    </div>
  );
}
