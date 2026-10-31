// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { makeTranslator } from "../src/i18n";
import { beforeEach, afterEach, describe, it, expect, vi } from "vitest";
import { useSearchNavigationHandoff } from "../src/hooks/useSearchNavigationHandoff";
import { CommandModal, activateCommandNavigation } from "../src/components/CommandModal";
import { searchRuntimeApi } from "../src/api/searchRuntimeApi";
import { useAppStore } from "../src/store/useAppStore";
import { useFileLibraryStore } from "../src/store/useFileLibraryStore";
import { useFileLibrarySelectionStore, useFileLibraryInspectorStore } from "../src/store/useFileLibraryV2Store";
import { projectAcceptedFileLibraryActivation } from "../src/utils/fileLibraryActivation";
import { AutomationWorkspace } from "../src/views/automation/AutomationWorkspace";
import type { View } from "../src/types/ui";
import type { SearchNavigationAcknowledgement } from "../src/api/windowApi";

const transport = vi.hoisted(() => ({
  invoke: vi.fn(), listeners: new Map<string, (payload: any) => void>()
}));
// Only the native transport is substituted. Both production window APIs,
// the Main bridge, navigation stores and mounted Automation surface are real.
vi.mock("../src/api/core", () => ({
  invokeCommand: (...args: unknown[]) => transport.invoke(...args),
  listenTo: async (event: string, handler: (payload: any) => void) => {
    transport.listeners.set(event, handler);
    return () => { transport.listeners.delete(event); };
  }
}));
vi.mock("../src/contexts/AppContexts", async () => {
  const { makeTranslator } = await import("../src/i18n");
  const { useAppStore } = await import("../src/store/useAppStore");
  return { useI18nContext: () => ({ t: makeTranslator("en") }),
    useNavigationContext: () => ({ setView: useAppStore.getState().setView }) };
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
const activateFile = (id: string) => { projectAcceptedFileLibraryActivation(id, {
  setExplicitSelection: useFileLibrarySelectionStore.getState().setExplicit,
  loadDetail: useFileLibraryInspectorStore.getState().loadDetail
}); };
const reportError = vi.fn();
const openGlobalEntry = vi.fn(async (_id: string, accept: () => boolean | Promise<boolean>) => await accept());
function Main() {
  const view = useAppStore((state) => state.view);
  useSearchNavigationHandoff(false, useAppStore.getState().setView, activateFile, reportError, openGlobalEntry);
  return <main data-view={view}>{view === "automation" ? <AutomationWorkspace /> : <div>{view}</div>}</main>;
}

describe("standalone Search to mounted Main commit handoff", () => {
  let root: Root;
  let searchVisible: boolean;
  let nonce: number;
  let readyAck: ReturnType<typeof deferred<void>>;
  let navigationAck: ReturnType<typeof deferred<SearchNavigationAcknowledgement>>;
  let releaseCommit: ReturnType<typeof deferred<void>>;
  let afterReady: () => void;
  let observedAck: SearchNavigationAcknowledgement | undefined;
  const snapshot = { sessionId: 4, revision: 9, phase: "visible_collapsed" as const };
  const choose = (view: View, fileId: string | null = null, settingsTarget?: "appearance") => activateCommandNavigation({
    standalone: true, windowSnapshot: snapshot, view, fileId, settingsTarget,
    setView: vi.fn(), setSelectedFileId: vi.fn(), onClose: vi.fn(),
    activateSearchResult: searchRuntimeApi.activateSearchResult
  });
  beforeEach(async () => {
    (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
    window.history.replaceState({}, "", "/?mainGeneration=7");
    document.body.innerHTML = '<div id="root"></div>';
    root = createRoot(document.getElementById("root")!);
    searchVisible = true; nonce = 0; observedAck = undefined;
    readyAck = deferred<void>(); navigationAck = deferred<SearchNavigationAcknowledgement>(); releaseCommit = deferred<void>();
    afterReady = () => undefined;
    reportError.mockReset(); openGlobalEntry.mockClear(); transport.listeners.clear(); transport.invoke.mockReset();
    useAppStore.setState({ view: "scanner", automationSurface: "advanced-policies" });
    useFileLibraryStore.setState({ selectedFileId: "old" });
    useFileLibrarySelectionStore.getState().setExplicit(["old"], "old");
    vi.spyOn(useFileLibraryInspectorStore.getState(), "loadDetail").mockResolvedValue({ status: "superseded", requestEpoch: 1 });
    transport.invoke.mockImplementation(async (command, args) => {
      if (command === "mark_main_window_ready") {
        if (args.ready) {
          expect(transport.listeners.has("search-navigate")).toBe(true);
          expect(transport.listeners.has("search-main-ready-request")).toBe(true);
        }
        return;
      }
      if (command === "acknowledge_main_window_ready") { readyAck.resolve(); return; }
      if (command === "acknowledge_search_navigation") {
        observedAck = args.request;
        expect(args.request).toMatchObject({ generation: 7, nonce, sessionId: 4, revision: 9 });
        expect(searchVisible).toBe(true);
        if (args.request.applied) {
          expect(document.querySelector("main")?.getAttribute("data-view")).toBe(useAppStore.getState().view);
          if (useAppStore.getState().view === "automation") {
            expect(useAppStore.getState().automationSurface).toBe("intents");
            expect(document.querySelector("#automation-workspace-panel")?.getAttribute("aria-label")).toBe("Intents");
          }
        }
        await releaseCommit.promise;
        navigationAck.resolve(args.request);
        return;
      }
      if (command === "activate_search_result") {
        const request = args.request;
        expect(request).toMatchObject({ sessionId: 4, expectedRevision: 9 });
        const binding = { generation: 7, nonce: ++nonce, sessionId: 4, revision: 9 };
        transport.listeners.get("search-main-ready-request")! (binding);
        await readyAck.promise;
        afterReady();
        transport.listeners.get("search-navigate")! ({ ...binding, view: request.view, fileId: request.fileId, settingsTarget: request.settingsTarget, globalEntryId: request.globalEntryId, expiresAt: Date.now() + 3000 });
        const ack = await navigationAck.promise;
        if (!ack.applied) throw new Error("search_navigation_rejected");
        searchVisible = false;
        return;
      }
      if (command === "get_search_window_state") return snapshot;
      if (command === "hide_search_window_command") { searchVisible = false; return { ...snapshot, phase: "hidden" }; }
      if (command === "list_automation_intents" || command === "list_automation_runs") return [];
      throw new Error(`unexpected command ${command}`);
    });
    await act(async () => { root.render(<Main />); });
  });
  afterEach(() => { act(() => root.unmount()); vi.restoreAllMocks(); });

  it("routes a global result by ID and ACKs only after Browse admission", async () => {
    let resolve!: (value: boolean) => void;
    openGlobalEntry.mockImplementationOnce(async (_id, accept) => {
      await new Promise<boolean>((r) => { resolve = r; });
      return await accept();
    });
    let activation!: Promise<void>;
    await act(async () => { activation = searchRuntimeApi.activateSearchResult("library", null, snapshot, null, "global-id"); await Promise.resolve(); });
    expect(openGlobalEntry).toHaveBeenCalledWith("global-id", expect.any(Function));
    expect(observedAck).toBeUndefined();
    expect(searchVisible).toBe(true);
    await act(async () => { resolve(true); await Promise.resolve(); });
    expect(observedAck?.applied).toBe(true);
    expect(useAppStore.getState().view).toBe("library");
    expect(useFileLibraryStore.getState().selectedFileId).toBe("old");
    releaseCommit.resolve(); await activation;
    expect(searchVisible).toBe(false);
  });

  it("rejects a delayed global admission after newer Main navigation", async () => {
    let resolve!: () => void;
    openGlobalEntry.mockImplementationOnce(async (_id, accept) => {
      await new Promise<void>((r) => { resolve = r; });
      return await accept();
    });
    let activation!: Promise<void>;
    await act(async () => { activation = searchRuntimeApi.activateSearchResult("library", null, snapshot, null, "global-id"); await Promise.resolve(); });
    await act(async () => { useAppStore.getState().setView("settings"); resolve(); await Promise.resolve(); });
    expect(observedAck?.applied).toBe(false);
    releaseCommit.resolve(); await expect(activation).rejects.toThrow("search_navigation_rejected");
    expect(searchVisible).toBe(true);
    expect(useAppStore.getState().view).toBe("settings");
  });

  it("commits Overview to Automation Intents despite unrelated selection churn, then closes Search", async () => {
    afterReady = () => {
      useFileLibraryStore.setState({ selectedFileId: "unrelated" });
      useFileLibrarySelectionStore.getState().setExplicit(["unrelated"], "unrelated");
    };
    let activation!: Promise<void>;
    await act(async () => { activation = choose("automation"); await Promise.resolve(); });
    expect(observedAck?.applied).toBe(true);
    expect(useAppStore.getState().view).toBe("automation");
    expect(useAppStore.getState().automationSurface).toBe("intents");
    expect(searchVisible).toBe(true); // emission/commit alone cannot close Search
    releaseCommit.resolve(); await activation;
    expect(searchVisible).toBe(false);
  });

  it("keeps fixed Settings navigation independent of file selection churn", async () => {
    const sections: string[] = [];
    const listener = (event: Event) => { sections.push((event as CustomEvent).detail); };
    window.addEventListener("fylura:settings-section", listener);
    afterReady = () => { useFileLibrarySelectionStore.getState().setExplicit(["other"], "other"); };
    let activation!: Promise<void>;
    await act(async () => { activation = choose("settings", null, "appearance"); await Promise.resolve(); });
    expect(observedAck?.applied).toBe(true);
    expect(useAppStore.getState().view).toBe("settings");
    expect(sections).toEqual(["settings-general"]);
    expect(searchVisible).toBe(true);
    releaseCommit.resolve(); await activation;
    expect(searchVisible).toBe(false);
    window.removeEventListener("fylura:settings-section", listener);
  });

  it.each(["selected", "focused", "selection", "view"])("rejects stale file-result %s continuity and leaves Search visible", async (field) => {
    afterReady = () => {
      if (field === "selected") useFileLibraryStore.setState({ selectedFileId: "other" });
      if (field === "focused") useFileLibrarySelectionStore.setState({ focusedId: "other" });
      if (field === "selection") useFileLibrarySelectionStore.getState().setExplicit(["old"], "old");
      if (field === "view") useAppStore.getState().setView("settings");
    };
    let activation!: Promise<void>;
    await act(async () => { activation = choose("library", "file-id"); await Promise.resolve(); });
    expect(observedAck?.applied).toBe(false);
    const failure = expect(activation).rejects.toThrow("search_navigation_rejected");
    releaseCommit.resolve(); await failure;
    expect(searchVisible).toBe(true);
    expect(useFileLibraryInspectorStore.getState().loadDetail).not.toHaveBeenCalled();
  });

  it.each(["changed-view", "invalid-fixed-target"])("rejects file-less %s without success ACK", async (caseName) => {
    if (caseName === "changed-view") afterReady = () => useAppStore.getState().setView("preview");
    let activation!: Promise<void>;
    await act(async () => {
      activation = caseName === "changed-view" ? choose("automation") : choose("settings", null, "arbitrary-selector" as "appearance");
      await Promise.resolve();
    });
    expect(observedAck?.applied).toBe(false);
    const failure = expect(activation).rejects.toThrow("search_navigation_rejected");
    releaseCommit.resolve(); await failure;
    expect(searchVisible).toBe(true);
  });

  it("accepts a valid ID-only file result and commits explicit selection/detail before ACK", async () => {
    let activation!: Promise<void>;
    await act(async () => { activation = choose("library", "file-id"); await Promise.resolve(); });
    expect(observedAck?.applied).toBe(true);
    expect(useFileLibraryStore.getState().selectedFileId).toBe("file-id");
    expect(useFileLibrarySelectionStore.getState().selection).toEqual({ kind: "explicit", fileIds: ["file-id"] });
    expect(useFileLibrarySelectionStore.getState().focusedId).toBe("file-id");
    expect(useFileLibraryInspectorStore.getState().loadDetail).toHaveBeenCalledWith("file-id");
    releaseCommit.resolve(); await activation;
    expect(searchVisible).toBe(false);
  });

  it.each([true, false])("mounted Search blur cannot hide while command handoff awaits commit (applied=%s)", async (accepted) => {
    const searchContainer = document.createElement("div");
    document.body.appendChild(searchContainer);
    const searchRoot = createRoot(searchContainer);
    const inputRef = { current: null as HTMLInputElement | null };
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    try {
      await act(async () => { searchRoot.render(<CommandModal inputRef={inputRef}
        setView={() => undefined} setSelectedFileId={() => undefined} onClose={() => undefined}
        platform="win32" t={makeTranslator("en")} standalone />); });
      // A timer started just before selection must also be invalidated.
      window.dispatchEvent(new Event("blur"));
      afterReady = () => {
        window.dispatchEvent(new Event("blur")); // ensure Main took focus
        if (!accepted) useAppStore.getState().setView("settings");
      };
      await act(async () => {
        inputRef.current!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true }));
        await new Promise((done) => window.setTimeout(done, 160));
      });
      expect(observedAck?.applied).toBe(accepted);
      expect(searchVisible).toBe(true);
      expect(transport.invoke.mock.calls.some(([command]) => command === "hide_search_window_command")).toBe(false);
      await act(async () => { releaseCommit.resolve(); await new Promise((done) => window.setTimeout(done, 160)); });
      expect(searchVisible).toBe(!accepted);
      expect(transport.invoke.mock.calls.some(([command]) => command === "hide_search_window_command")).toBe(false);
    } finally { act(() => searchRoot.unmount()); searchContainer.remove(); }
  });

  it("ignores unmatched generation/session/revision/nonce without sending any success ACK", () => {
    transport.listeners.get("search-main-ready-request")! ({ generation: 7, nonce: 1, sessionId: 4, revision: 9 });
    const payload = { generation: 7, nonce: 1, sessionId: 4, revision: 9, view: "automation", fileId: null };
    for (const changed of [{ generation: 8 }, { nonce: 2 }, { sessionId: 5 }, { revision: 10 }]) {
      transport.listeners.get("search-navigate")! ({ ...payload, ...changed });
    }
    expect(observedAck).toBeUndefined();
    expect(useAppStore.getState().view).toBe("scanner");
    expect(searchVisible).toBe(true);
  });
});
