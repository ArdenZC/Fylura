import { FileLibraryExperienceController } from "../fileLibraryExperience";
import type { NavigationTarget } from "../../../types/fileWorkspace";
import {
  createFileLibraryProjection,
  defaultFileLibraryProjection,
} from "../../../store/useFileLibraryV2Store";

export interface WorkbenchPane {
  id: string;
  controller: FileLibraryExperienceController;
  projection: typeof defaultFileLibraryProjection;
  path: string;
  pathTargetKey: string | null;
}

export function workbenchTargetKey(target: NavigationTarget | null) {
  if (!target) return null;
  if (target.kind === "library") return `library:${target.source}:${target.key}`;
  const locationKey = target.location.kind === "managed"
    ? `managed:${target.location.scanRootId}`
    : `ephemeral:${target.location.browseSessionId}:${target.location.locationId}`;
  return `browse:${locationKey}:${target.pathRef.id}`;
}

export function currentWorkbenchBrowsePath(
  path: string,
  pathTargetKey: string | null | undefined,
  target: NavigationTarget | null,
) {
  if (
    target?.kind !== "browse"
    || path.trim().length === 0
    || pathTargetKey !== workbenchTargetKey(target)
  ) return null;
  return path;
}

export interface WorkbenchTab {
  id: string;
  panes: WorkbenchPane[];
  activePaneId: string;
  dual: boolean;
}
export interface WorkbenchSnapshot {
  tabs: WorkbenchTab[];
  activeTabId: string;
}
const MAX_TABS = 8;
let serial = 0;
/** Owns disposable UI instances. Browse/Query/Preview authorities remain in their existing controllers. */
export class FileWorkbenchModel {
  private listeners = new Set<() => void>();
  private snapshot: WorkbenchSnapshot;
  private readonly root: FileLibraryExperienceController;
  constructor(
    root: FileLibraryExperienceController,
    private makeController = () => new FileLibraryExperienceController(),
  ) {
    this.root = root;
    const pane: WorkbenchPane = {
      id: `pane-${++serial}`,
      controller: root,
      projection: defaultFileLibraryProjection,
      path: "",
      pathTargetKey: null,
    };
    const tab = {
      id: `tab-${++serial}`,
      panes: [pane],
      activePaneId: pane.id,
      dual: false,
    };
    this.snapshot = { tabs: [tab], activeTabId: tab.id };
  }
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  private publish(
    tabs = [...this.snapshot.tabs],
    activeTabId = this.snapshot.activeTabId,
  ) {
    this.snapshot = { tabs, activeTabId };
    this.listeners.forEach((listener) => listener());
  }
  private pane(): WorkbenchPane {
    const controller = this.makeController();
    void controller.switchMode("browse");
    return {
      id: `pane-${++serial}`,
      controller,
      projection: createFileLibraryProjection(),
      path: "",
      pathTargetKey: null,
    };
  }
  newTab() {
    if (this.snapshot.tabs.length >= MAX_TABS) return null;
    const pane = this.pane();
    const tab = {
      id: `tab-${++serial}`,
      panes: [pane],
      activePaneId: pane.id,
      dual: false,
    };
    this.publish([...this.snapshot.tabs, tab], tab.id);
    return tab;
  }
  activate(tabId: string, paneId?: string) {
    const tab = this.snapshot.tabs.find((item) => item.id === tabId);
    if (!tab) return;
    if (paneId && tab.panes.some((pane) => pane.id === paneId))
      tab.activePaneId = paneId;
    this.publish([...this.snapshot.tabs], tabId);
  }
  toggleDual(tabId: string) {
    const tab = this.snapshot.tabs.find((item) => item.id === tabId);
    if (!tab) return;
    if (tab.panes.length === 1) tab.panes.push(this.pane());
    tab.dual = !tab.dual;
    if (!tab.dual) tab.activePaneId = tab.panes[0].id;
    this.publish();
  }
  updatePath(paneId: string, path: string, pathTargetKey: string | null = null) {
    const pane = this.snapshot.tabs
      .flatMap((tab) => tab.panes)
      .find((item) => item.id === paneId);
    if (!pane || (pane.path === path && pane.pathTargetKey === pathTargetKey)) return;
    pane.path = path;
    pane.pathTargetKey = pathTargetKey;
    this.publish();
  }
  async close(tabId: string) {
    const index = this.snapshot.tabs.findIndex((tab) => tab.id === tabId);
    if (index < 0) return;
    const removed = this.snapshot.tabs[index];
    let tabs = this.snapshot.tabs.filter((tab) => tab.id !== tabId);
    if (!tabs.length) {
      const pane = this.pane();
      tabs = [
        {
          id: `tab-${++serial}`,
          panes: [pane],
          activePaneId: pane.id,
          dual: false,
        },
      ];
    }
    const active =
      this.snapshot.activeTabId === tabId
        ? tabs[Math.min(index, tabs.length - 1)].id
        : this.snapshot.activeTabId;
    this.publish(tabs, active);
    await Promise.all(
      removed.panes.map((pane) =>
        pane.controller === this.root
          ? pane.controller.suspend()
          : pane.controller.dispose(),
      ),
    );
  }
  revealRoot() {
    let tab = this.snapshot.tabs.find((tab) =>
      tab.panes.some((pane) => pane.controller === this.root),
    );
    if (!tab) {
      const pane: WorkbenchPane = {
        id: `pane-${++serial}`,
        controller: this.root,
        projection: defaultFileLibraryProjection,
        path: "",
        pathTargetKey: null,
      };
      tab = {
        id: `tab-${++serial}`,
        panes: [pane],
        activePaneId: pane.id,
        dual: false,
      };
      this.publish([...this.snapshot.tabs, tab], tab.id);
    } else
      this.activate(
        tab.id,
        tab.panes.find((pane) => pane.controller === this.root)!.id,
      );
  }
  async dispose() {
    await Promise.all(
      this.snapshot.tabs
        .flatMap((tab) => tab.panes)
        .filter((pane) => pane.controller !== this.root)
        .map((pane) => pane.controller.dispose()),
    );
    this.listeners.clear();
  }
}
