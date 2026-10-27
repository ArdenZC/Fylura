import { Columns2, Copy, FolderInput, Plus, X } from "lucide-react";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ComponentType,
  type DragEvent,
} from "react";
import {
  FileLibraryExperienceProvider,
  useFileLibraryExperience,
} from "../FileLibraryExperienceProvider";
import { useI18nContext } from "../../../contexts/AppContexts";
import { defaultFileLibraryProjection } from "../../../store/useFileLibraryV2Store";
import { useFileLibraryStore } from "../../../store/useFileLibraryStore";
import { useAppStore } from "../../../store/useAppStore";
import { useFileManagementStore } from "../management/fileManagementStore";
import type { LibraryScope } from "../../../types/domain";
import type { PreviewExperienceController } from "../preview/previewExperienceController";
import { PaneContext, type PaneContextValue } from "./PaneContext";
import {
  FileWorkbenchModel,
  type WorkbenchPane,
  type WorkbenchTab,
} from "./workbenchModel";
import type { Translator } from "../../../types/ui";
import { useWorkbenchDrag } from "./useWorkbenchDrag";
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
  const { t } = useI18nContext();
  const [model] = useState(() => new FileWorkbenchModel(rootController));
  const state = useSyncExternalStore(model.subscribe, model.getSnapshot);
  const root = useRef<HTMLDivElement | null>(null);
  const [wide, setWide] = useState(false);
  const previews = useRef(new Map<string, PreviewExperienceController>());
  const [mutationEpochs, setMutationEpochs] = useState<Record<string, number>>(
    {},
  );
  const pendingDispose = useRef<ReturnType<typeof setTimeout> | null>(null);
  const active = state.tabs.find((tab) => tab.id === state.activeTabId)!;
  useLayoutEffect(() => {
    if (useFileManagementStore.getState().activePaneId !== active.activePaneId)
      useFileManagementStore.getState().activatePane(active.activePaneId);
  }, [active.activePaneId]);
  const locked = useFileManagementStore((s) => s.transferOpen);
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
    !!peer?.path && !!focusedSurface && focusedSurface.count !== 0 && !locked;
  const { startDrag, drop, endDrag } = useWorkbenchDrag({ focus, locked, t });
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
                onClick={() => void model.close(tab.id)}
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
                peerId={
                  tab.dual
                    ? tab.panes.find((other) => other.id !== pane.id)?.id
                    : undefined
                }
                mutationEpoch={mutationEpochs[pane.id] ?? 0}
                activate={() => focus(tab.id, pane.id)}
                activatePeer={() => {
                  const other = tab.panes.find((other) => other.id !== pane.id);
                  if (other) focus(tab.id, other.id);
                }}
                updateLocation={(path) => model.updatePath(pane.id, path)}
                registerPreview={(preview) => {
                  if (preview) previews.current.set(pane.id, preview);
                  else previews.current.delete(pane.id);
                }}
                beforeMutation={beforeMutation}
                afterMutation={afterMutation}
                onDragStart={(event) => void startDrag(event, pane)}
                onDragEnd={endDrag}
                onDrop={(event) => void drop(event, pane, tab.id)}
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
  updateLocation: (path: string) => void;
  registerPreview: (preview: PreviewExperienceController | null) => void;
  onDragStart: (event: DragEvent) => void;
  onDragEnd: () => void;
  onDrop: (event: DragEvent) => void;
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
    (path: string) => pathUpdater.current(path),
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
    updateLocation,
    registerPreview,
  };
  return (
    <div
      ref={rootRef}
      className="wb-pane"
      data-pane-id={pane.id}
      data-focused={props.focused}
      hidden={!props.visible}
      onPointerDownCapture={props.activate}
      onFocusCapture={props.activate}
      onDragStartCapture={props.onDragStart}
      onDragEnd={props.onDragEnd}
      onDragOver={(event) => {
        event.preventDefault();
        event.dataTransfer.dropEffect = event.shiftKey ? "move" : "copy";
      }}
      onDrop={props.onDrop}
    >
      <PaneContext.Provider value={value}>
        <FileLibraryExperienceProvider
          active
          controller={pane.controller}
          disposeOnUnmount={false}
        >
          <Pane />
        </FileLibraryExperienceProvider>
      </PaneContext.Provider>
    </div>
  );
}
