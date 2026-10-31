import type { EntryRef } from "../../../types/fileWorkspace";
import type { LibrarySelectionV1 } from "../../../types/domain";

export const WORKBENCH_POINTER_DRAG_THRESHOLD = 6;

export interface WorkbenchPointerSourceRow {
  selected: boolean;
  managedFileId?: string;
  browseSessionId?: string;
  browseEntryId?: string;
}

export type WorkbenchPointerCaptureRequest =
  | { selection: LibrarySelectionV1 }
  | { refs: EntryRef[] };

/** Preserve a multi-selection when dragging a selected row; otherwise drag the row itself. */
export function workbenchPointerCaptureRequest(
  row: WorkbenchPointerSourceRow,
  surface: { selection?: LibrarySelectionV1; refs?: EntryRef[] },
): WorkbenchPointerCaptureRequest | null {
  if (row.selected) {
    if (surface.selection) return { selection: surface.selection };
    if (surface.refs?.length) return { refs: [...surface.refs] };
  }

  if (row.managedFileId) {
    return { refs: [{ kind: "managed", fileId: row.managedFileId }] };
  }
  if (row.browseSessionId && row.browseEntryId) {
    return {
      refs: [{ kind: "ephemeral", browseSessionId: row.browseSessionId, entryId: row.browseEntryId }],
    };
  }
  return null;
}

export interface WorkbenchPointerSession {
  token: string;
  pointerId: number;
  tabId: string;
  sourcePaneId: string;
  peerPaneId: string;
  sourceSignature: string;
  peerSignature: string;
  sourcePath: string;
  peerPath: string;
  startX: number;
  startY: number;
  moved: boolean;
  ended: boolean;
  cancelled: boolean;
}

export class WorkbenchPointerSessionRegistry {
  private active: WorkbenchPointerSession | null = null;

  get current() {
    return this.active;
  }

  begin(session: WorkbenchPointerSession) {
    if (this.active) return null;
    this.active = session;
    return session;
  }

  markMoved(pointerId: number, x: number, y: number) {
    const session = this.active;
    if (!session || session.pointerId !== pointerId || session.cancelled || session.ended) return false;
    if (!session.moved && Math.hypot(x - session.startX, y - session.startY) < WORKBENCH_POINTER_DRAG_THRESHOLD) return false;
    session.moved = true;
    return true;
  }

  finish(pointerId: number, token: string) {
    const session = this.active;
    if (!session || session.pointerId !== pointerId || session.token !== token || session.cancelled) return null;
    this.active = null;
    return session;
  }

  cancel(pointerId?: number, token?: string) {
    const session = this.active;
    if (!session || (pointerId !== undefined && session.pointerId !== pointerId) || (token !== undefined && session.token !== token)) return null;
    session.cancelled = true;
    this.active = null;
    return session;
  }

  dispose() {
    return this.cancel();
  }
}

export function workbenchPointerDropDisposition(
  sourcePaneId: string,
  peerPaneId: string,
  targetPaneId: string | null,
): "same-pane" | "peer-pane" | "unsupported" {
  if (targetPaneId === sourcePaneId) return "same-pane";
  if (targetPaneId === peerPaneId) return "peer-pane";
  return "unsupported";
}

export interface WorkbenchPointerSessionState {
  currentToken: string | null;
  activeTabId: string | null;
  sourceSignature: string | null;
  peerSignature: string | null;
  sourcePath: string | null;
  peerPath: string | null;
  sourceVisible: boolean;
  peerVisible: boolean;
  workspaceActive: boolean;
  transferOpen: boolean;
  modalOpen: boolean;
}

export function workbenchPointerSessionIsCurrent(
  session: WorkbenchPointerSession,
  state: WorkbenchPointerSessionState,
) {
  return !session.cancelled &&
    session.moved &&
    state.currentToken === session.token &&
    state.activeTabId === session.tabId &&
    state.sourceSignature === session.sourceSignature &&
    state.peerSignature === session.peerSignature &&
    state.sourcePath === session.sourcePath &&
    state.peerPath === session.peerPath &&
    state.sourceVisible &&
    state.peerVisible &&
    state.workspaceActive &&
    !state.transferOpen &&
    !state.modalOpen;
}
