import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import {
  decideManagedScanEvent,
  indexIsIncompleteForStatus,
  isCurrentDedupeEvent,
  scanStatusForBackendStatus
} from "../src/store/useScanManagerStore";
import type { ManagedScanEvent } from "../src/api/tauriApi";

describe("scan manager progress callbacks", () => {
  it("accepts dedupe events only for the current parent scan and dedupe job", () => {
    const current = { dedupeJobId: "dedupe-a", parentScanJobId: "scan-a" };

    expect(isCurrentDedupeEvent(current, "scan-a", null)).toBe(true);
    expect(isCurrentDedupeEvent(current, "scan-a", "dedupe-a")).toBe(true);
    expect(isCurrentDedupeEvent(current, "scan-b", null)).toBe(false);
    expect(isCurrentDedupeEvent(current, "scan-a", "dedupe-b")).toBe(false);
  });

  // Acceptance (BRIEF 裁决 2.3): job outcome and index health are separate axes.
  // A scan that ran to the end must never be presented as an error just because its
  // coverage was incomplete — but the health signal must still be readable somewhere.
  it("presents a finished scan as completed regardless of index health, without losing the health signal", () => {
    expect(scanStatusForBackendStatus("completed")).toBe("completed");
    expect(scanStatusForBackendStatus("completed_with_warnings")).toBe("completed");
    expect(scanStatusForBackendStatus("requires_reconciliation")).toBe("completed");

    expect(scanStatusForBackendStatus("failed")).toBe("error");
    expect(scanStatusForBackendStatus("interrupted")).toBe("error");
    expect(scanStatusForBackendStatus("cancelled")).toBe("canceled");
    expect(scanStatusForBackendStatus("cancelled_not_started")).toBe("canceled");
    expect(scanStatusForBackendStatus("running")).toBe("scanning");

    // Axis 2 is preserved rather than dropped.
    expect(indexIsIncompleteForStatus("requires_reconciliation")).toBe(true);
    expect(indexIsIncompleteForStatus("completed")).toBe(false);
    expect(indexIsIncompleteForStatus("completed_with_warnings")).toBe(false);
    expect(indexIsIncompleteForStatus(undefined)).toBe(false);
  });

  it("refreshes the library scope for roots that finished with incomplete coverage", () => {
    const storeSource = readFileSync(resolve("src/store/useScanManagerStore.ts"), "utf8");
    const scopeFilter = storeSource.slice(
      storeSource.indexOf("const completedScanRoots"),
      storeSource.indexOf("const files = session.scannedFiles")
    );

    expect(scopeFilter).toContain("requires_reconciliation");
  });

  it("rejects duplicate, stale-generation, and terminal-regression events", () => {
    const event = managedEvent({ eventId: "event-2", generation: 2, runRevision: 8, sessionRevision: 9, status: "completed" });

    expect(decideManagedScanEvent(event, "session-a", 7, 2, 8, "running", [])).toBe("accept");
    expect(decideManagedScanEvent(event, "session-a", 8, 2, 9, "completed", ["event-2"])).toBe("ignore");
    expect(decideManagedScanEvent({ ...event, eventId: "event-old-generation", generation: 1 }, "session-a", 7, 2, 8, "running", [])).toBe("ignore");
    expect(decideManagedScanEvent({ ...event, eventId: "event-regression", status: "running", runRevision: 9, sessionRevision: 10 }, "session-a", 8, 2, 9, "completed", [])).toBe("ignore");
  });

  it("refetches on a durable revision gap or same revision with a new event ID", () => {
    const event = managedEvent({ eventId: "event-gap", runRevision: 12, sessionRevision: 14 });

    expect(decideManagedScanEvent(event, "session-a", 9, 1, 10, "running", [])).toBe("refresh");
    expect(decideManagedScanEvent({ ...event, eventId: "event-same-revision", runRevision: 9, sessionRevision: 10 }, "session-a", 9, 1, 10, "running", [])).toBe("refresh");
    expect(decideManagedScanEvent({ ...event, eventId: "event-contiguous", runRevision: 10, sessionRevision: 11 }, "session-a", 9, 1, 10, "running", [])).toBe("accept");
    expect(decideManagedScanEvent(event, "session-b", 9, 1, 10, "running", [])).toBe("ignore");
  });
  it("does not refresh or reset scope from scan event callbacks", () => {
    const storeSource = readFileSync(
      resolve("src/store/useScanManagerStore.ts"),
      "utf8"
    );
    const progressHandler = storeSource.slice(
      storeSource.indexOf("tauriApi.onScanProgress"),
      storeSource.indexOf("tauriApi.onScanBatch")
    );
    const completeHandler = storeSource.slice(
      storeSource.indexOf("tauriApi.onScanComplete"),
      storeSource.indexOf("tauriApi.onScanError")
    );

    expect(progressHandler).not.toContain("useFileLibraryStore.getState().refresh");
    expect(progressHandler).not.toContain("useFileLibraryStore.getState().setCurrentScanScope");
    expect(completeHandler).not.toContain("useFileLibraryStore.getState().refresh");
    expect(completeHandler).not.toContain("useFileLibraryStore.getState().setCurrentScanScope");
  });

  it("hydrates session mappings from the durable snapshot and preserves finalizing phase in the renderer", () => {
    const storeSource = readFileSync(
      resolve("src/store/useScanManagerStore.ts"),
      "utf8"
    );
    expect(storeSource).toContain("tauriApi.getManagedScanSnapshot(sessionId)");
    expect(storeSource).not.toContain("function sessionFromRunList");
    const projection = storeSource.slice(
      storeSource.indexOf("function sessionStatusFromMappings"),
      storeSource.indexOf("function applyManagedStartSnapshot")
    );
    expect(projection).toContain('session.phase === "finalizing"');
    expect(projection).toContain('session.phase === "completed"');
  });

  it("updates scope and refreshes once from scanPaths after all roots finish", () => {
    const storeSource = readFileSync(
      resolve("src/store/useScanManagerStore.ts"),
      "utf8"
    );
    const scanPaths = storeSource.slice(
      storeSource.indexOf("scanPaths: async"),
      storeSource.indexOf("handleScan: async")
    );

    expect(scanPaths).toContain("useFileLibraryStore.getState().setCurrentScanScope(completedScanRoots, session.id)");
    expect(scanPaths).toContain("useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery)");
    expect(scanPaths.indexOf("useFileLibraryStore.getState().setCurrentScanScope(completedScanRoots, session.id"))
      .toBeGreaterThan(scanPaths.indexOf("waitForManagedSession"));
    expect(scanPaths.indexOf("useFileLibraryStore.getState().refresh(useAppStore.getState().searchQuery)"))
      .toBeGreaterThan(scanPaths.indexOf("for (const path of scanRoots)"));
  });

  it("stops a batch scan before starting the next root when cancellation is requested", () => {
    const storeSource = readFileSync(
      resolve("src/store/useScanManagerStore.ts"),
      "utf8"
    );
    const scanPaths = storeSource.slice(
      storeSource.indexOf("scanPaths: async"),
      storeSource.indexOf("handleScan: async")
    );
    const cancelScan = storeSource.slice(
      storeSource.lastIndexOf("cancelScan: async"),
      storeSource.length
    );

    expect(storeSource).toContain("let scanJobCanceled = false");
    expect(storeSource).toContain("activeManagedSessionId");
    expect(storeSource).toContain("event.parentSessionId !== activeManagedSessionId");
    expect(scanPaths).toContain("scanJobCanceled = false");
    expect(scanPaths).toContain("activeManagedRequest");
    expect(scanPaths).toContain("await tauriApi.startManagedScan(activeManagedRequest)");
    expect(scanPaths).toContain("waitForManagedSession(start.session.id)");
    expect(cancelScan).toContain("scanJobCanceled = true");
    expect(cancelScan).toContain("tauriApi.cancelScanRun(activeRunId)");
    expect(cancelScan).toContain("isCancelingScan: true");
    expect(cancelScan).not.toContain("isScanning: false");
    expect(cancelScan).toContain('status: "scanning"');
    expect(cancelScan).not.toContain('status: "canceled"');
  });

  it("keeps scanning locked while cancellation is still settling", () => {
    const storeSource = readFileSync(
      resolve("src/store/useScanManagerStore.ts"),
      "utf8"
    );
    const scanPaths = storeSource.slice(
      storeSource.indexOf("scanPaths: async"),
      storeSource.indexOf("handleScan: async")
    );
    const finallyBlock = scanPaths.slice(
      scanPaths.indexOf("finally"),
      scanPaths.length
    );

    expect(storeSource).toContain("isCancelingScan: boolean");
    expect(scanPaths).toContain("if (get().isScanning) return");
    expect(scanPaths.indexOf("if (get().isScanning) return"))
      .toBeLessThan(scanPaths.indexOf("scanJobCanceled = false"));
    expect(finallyBlock).toContain("isScanning: false");
    expect(finallyBlock).toContain("isCancelingScan: false");
  });

  it("reports canceled scans without showing a success file count or refreshing unscanned roots", () => {
    const storeSource = readFileSync(
      resolve("src/store/useScanManagerStore.ts"),
      "utf8"
    );
    const scanPaths = storeSource.slice(
      storeSource.indexOf("scanPaths: async"),
      storeSource.indexOf("handleScan: async")
    );
    const canceledBranch = scanPaths.slice(
      scanPaths.indexOf('if (finalStatus === "canceled")'),
      scanPaths.indexOf('} else if (finalStatus === "completed")')
    );

    expect(canceledBranch).toContain('finalStatus === "canceled"');
    expect(canceledBranch).toContain('showSuccess(t("scanCanceled"))');
    expect(canceledBranch).not.toContain("setCurrentScanScope(completedScanRoots");
    expect(canceledBranch).not.toContain('`${t("success")}:');
  });

  it("treats scan-error events as warnings instead of fatal scan failures", () => {
    const storeSource = readFileSync(
      resolve("src/store/useScanManagerStore.ts"),
      "utf8"
    );
    const start = storeSource.indexOf("tauriApi.onScanError");
    const scanErrorHandler = storeSource.slice(
      start,
      storeSource.indexOf("])", start)
    );

    expect(scanErrorHandler).not.toContain('status: "error"');
    expect(scanErrorHandler).toContain("progress.errors");
  });

  it("marks scanState as error only when the scan command rejects", () => {
    const storeSource = readFileSync(
      resolve("src/store/useScanManagerStore.ts"),
      "utf8"
    );
    const scanPaths = storeSource.slice(
      storeSource.indexOf("scanPaths: async"),
      storeSource.indexOf("handleScan: async")
    );

    expect(scanPaths).toContain('status: "error"');
    expect(scanPaths).toContain("readableError(error)");
  });
});

function managedEvent(overrides: Partial<ManagedScanEvent> = {}): ManagedScanEvent {
  return {
    eventId: "event-1",
    runId: "run-a",
    scanRootId: "root-a",
    parentSessionId: "session-a",
    generation: 1,
    runRevision: 2,
    sessionRevision: 3,
    status: "running",
    runPhase: "discovering",
    sessionPhase: "running",
    scannedFiles: 1,
    scannedDirectories: 1,
    processedBytes: 1,
    warningsCount: 0,
    errorsCount: 0,
    currentPath: null,
    errorCode: null,
    errorMessage: null,
    timestamp: 1,
    ...overrides
  };
}


describe("M1 scan admission occupancy", () => {
  it.each([
    "Scan root already has an active run: F:/Library",
    "Scan root overlaps an active root lease: F:/Library overlaps F:/Library/child (run-1)"
  ])("keeps foreground occupancy informational without cancelling another owner: %s", async (message) => {
    const { tauriApi } = await import("../src/api/tauriApi");
    const { useScanManagerStore } = await import("../src/store/useScanManagerStore");
    const { useAppStore } = await import("../src/store/useAppStore");
    useScanManagerStore.getState().reset();
    const initialize = useScanManagerStore.getState().initializeScanListeners;
    useScanManagerStore.setState({ initializeScanListeners: async () => {} });
    const start = vi.spyOn(tauriApi, "startManagedScan").mockRejectedValue(new Error(message));
    const cancel = vi.spyOn(tauriApi, "cancelScanRun");
    try {
      await useScanManagerStore.getState().scanPaths(["F:/Library"]);
      expect(useScanManagerStore.getState().isScanning).toBe(false);
      expect(useScanManagerStore.getState().scanState).toMatchObject({ status: "idle", error: null });
      expect(useAppStore.getState().toast?.type).toBe("info");
      expect(cancel).not.toHaveBeenCalled();
    } finally {
      start.mockRestore();
      cancel.mockRestore();
      useScanManagerStore.setState({ initializeScanListeners: initialize });
      useScanManagerStore.getState().reset();
      useAppStore.getState().clearToast();
    }
  });
});


it("does not reinterpret genuine scan errors as normal occupancy", async () => {
  const { isScanAdmissionBusy } = await import("../src/utils/scanAdmission");
  for (const error of ["sqlite error: database is locked", "permission denied", "Scan finalization lost root lease", "Scan root has an invalid path"]) {
    expect(isScanAdmissionBusy(new Error(error))).toBe(false);
  }
});
