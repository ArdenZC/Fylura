import { describe, expect, it } from "vitest";
import {
  WorkbenchPointerSessionRegistry,
  workbenchPointerCaptureRequest,
  workbenchPointerDropDisposition,
  workbenchPointerSessionIsCurrent,
  type WorkbenchPointerSession,
  type WorkbenchPointerSessionState,
} from "./workbenchPointerSession";

function session(): WorkbenchPointerSession {
  return {
    token: "gesture-1",
    pointerId: 4,
    tabId: "tab-1",
    sourcePaneId: "pane-a",
    peerPaneId: "pane-b",
    sourceSignature: "source-signature",
    peerSignature: "peer-signature",
    sourcePath: "E:/fixture/source",
    peerPath: "E:/fixture/target",
    startX: 10,
    startY: 10,
    moved: false,
    ended: false,
    cancelled: false,
  };
}

function currentState(): WorkbenchPointerSessionState {
  return {
    currentToken: "gesture-1",
    activeTabId: "tab-1",
    sourceSignature: "source-signature",
    peerSignature: "peer-signature",
    sourcePath: "E:/fixture/source",
    peerPath: "E:/fixture/target",
    sourceVisible: true,
    peerVisible: true,
    workspaceActive: true,
    transferOpen: false,
    modalOpen: false,
  };
}

describe("controlled workbench pointer sessions", () => {
  it("captures the dragged row when it was not already selected", () => {
    expect(workbenchPointerCaptureRequest({
      selected: false,
      browseSessionId: "browse-1",
      browseEntryId: "entry-1",
    }, { refs: [] })).toEqual({ refs: [{ kind: "ephemeral", browseSessionId: "browse-1", entryId: "entry-1" }] });
  });

  it("preserves the full selection when a selected row is dragged", () => {
    const selection = { kind: "explicit" as const, fileIds: ["file-1", "file-2"] };
    expect(workbenchPointerCaptureRequest({ selected: true, managedFileId: "file-1" }, { selection }))
      .toEqual({ selection });
  });

  it("rejects rows without a trustworthy managed or browse identity", () => {
    expect(workbenchPointerCaptureRequest({ selected: false }, { refs: [] })).toBeNull();
  });

  it("opens only after the movement threshold and finishes the matching pointer", () => {
    const registry = new WorkbenchPointerSessionRegistry();
    const opened = registry.begin(session());
    expect(opened).not.toBeNull();
    expect(registry.markMoved(4, 14, 14)).toBe(false);
    expect(registry.markMoved(4, 16, 10)).toBe(true);
    expect(registry.finish(3, "gesture-1")).toBeNull();
    expect(registry.finish(4, "gesture-1")?.moved).toBe(true);
    expect(registry.current).toBeNull();
  });

  it("cancels and disposes an active session", () => {
    const registry = new WorkbenchPointerSessionRegistry();
    registry.begin(session());
    expect(registry.cancel(4, "gesture-1")?.cancelled).toBe(true);
    expect(registry.current).toBeNull();
    registry.begin(session());
    expect(registry.dispose()?.cancelled).toBe(true);
    expect(registry.current).toBeNull();
  });

  it("treats the source pane as a no-op and accepts only its peer", () => {
    expect(workbenchPointerDropDisposition("pane-a", "pane-b", "pane-a")).toBe("same-pane");
    expect(workbenchPointerDropDisposition("pane-a", "pane-b", "pane-b")).toBe("peer-pane");
    expect(workbenchPointerDropDisposition("pane-a", "pane-b", "other")).toBe("unsupported");
  });

  it("revalidates both pane identities and cancels stale or obscured sessions", () => {
    const active = session();
    active.moved = true;
    expect(workbenchPointerSessionIsCurrent(active, currentState())).toBe(true);
    expect(workbenchPointerSessionIsCurrent(active, { ...currentState(), peerSignature: "changed" })).toBe(false);
    expect(workbenchPointerSessionIsCurrent(active, { ...currentState(), sourcePath: "E:/fixture/other" })).toBe(false);
    expect(workbenchPointerSessionIsCurrent(active, { ...currentState(), modalOpen: true })).toBe(false);
    expect(workbenchPointerSessionIsCurrent(active, { ...currentState(), peerVisible: false })).toBe(false);
  });
});
