// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import { IndexCoverageSettings } from "../src/views/settings/sections/IndexCoverageSettings";
import type { IndexCoverageRequest, IndexCoverageResponse } from "../src/types/indexCoverage";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const { call } = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock("../src/api/tauriApi", () => ({ tauriApi: { globalIndexCoverage: call } }));
let root: Root | undefined;
let host: HTMLDivElement;
afterEach(async () => { await act(async () => root?.unmount()); host?.remove(); call.mockReset(); vi.useRealTimers(); });
function response(): IndexCoverageResponse { return { snapshot: { policy: { version: 1, revision: 7, mode: "smart_lean", rules: [] }, sources: [] }, preview: null }; }
async function mount() {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<IndexCoverageSettings t={makeTranslator("en")} sources={[]} disabled={false} />));
}
async function click(text: string) { const b = [...host.querySelectorAll("button")].find((b) => b.textContent === text)!; await act(async () => b.click()); }
describe("Index Coverage settings workflow", () => {
  it("invalidates an old preview when polling observes a concurrent policy change", async () => {
    vi.useFakeTimers();
    let revision = 7;
    call.mockImplementation(async (r: IndexCoverageRequest) => ({
      snapshot: {
        policy: { version: 1, revision, mode: revision === 7 ? "smart_lean" : "custom", rules: [] },
        sources: [{ sourceId: "source", enabled: true, status: "indexing", pending: true, hasUndetermined: true, included: 0, excluded: 0, undetermined: 0 }]
      },
      preview: r.action === "preview" ? { sampled: 1, included: 1, excluded: 0, undetermined: 0, sampleLimit: 1000, isComplete: false } : null
    }));
    await mount();
    await click("Preview coverage changes");
    expect(call).toHaveBeenCalledWith({ action: "preview", expectedRevision: 7, mode: "smart_lean", rules: [] });
    revision = 8;
    await act(async () => vi.advanceTimersByTimeAsync(2500));
    expect(host.textContent).toContain("Coverage policy changed elsewhere");
    expect(host.textContent).not.toContain("not the complete impact");
    expect([...host.querySelectorAll("button")].find((b) => b.textContent === "Apply and rebuild")!.disabled).toBe(true);
    await click("Apply and rebuild");
    expect(call.mock.calls.filter(([r]) => r.action === "save")).toHaveLength(0);
    await click("Refresh coverage status");
    expect(host.querySelector("select")!.value).toBe("custom");
    expect(host.textContent).not.toContain("Coverage policy changed elsewhere");
    await click("Preview coverage changes");
    expect(call).toHaveBeenCalledWith({ action: "preview", expectedRevision: 8, mode: "custom", rules: [] });
    await click("Apply and rebuild");
    expect(call).toHaveBeenCalledWith({ action: "save", expectedRevision: 8, mode: "custom", rules: [] });
  });

  it("rejects a preview immediately when another window changes policy before Apply", async () => {
    let revision = 7;
    let persistedMode: IndexCoverageResponse["snapshot"]["policy"]["mode"] = "smart_lean";
    const current = (preview: IndexCoverageResponse["preview"] = null): IndexCoverageResponse => ({
      snapshot: { policy: { version: 1, revision, mode: persistedMode, rules: [] }, sources: [] },
      preview
    });
    call.mockImplementation(async (request: IndexCoverageRequest) => {
      if (request.action === "read") return current();
      if (request.expectedRevision !== revision) throw new Error("coverage_revision_conflict");
      if (request.action === "preview") return current({ sampled: 1, included: 1, excluded: 0, undetermined: 0, sampleLimit: 1000, isComplete: false });
      revision += 1;
      persistedMode = request.mode!;
      return current();
    });
    await mount();
    await click("Preview coverage changes");
    revision = 8;
    persistedMode = "broad_coverage";

    await click("Apply and rebuild");

    expect(call).toHaveBeenCalledWith({ action: "save", expectedRevision: 7, mode: "smart_lean", rules: [] });
    expect(host.textContent).toContain("Coverage policy changed elsewhere");
    expect([...host.querySelectorAll("button")].find((b) => b.textContent === "Apply and rebuild")!.disabled).toBe(true);
    await click("Refresh coverage status");
    expect(host.querySelector("select")!.value).toBe("broad_coverage");
    await click("Preview coverage changes");
    await click("Apply and rebuild");
    expect(call).toHaveBeenCalledWith({ action: "save", expectedRevision: 8, mode: "broad_coverage", rules: [] });
  });

  it("retains an existing Broad Coverage policy when the Settings view remounts", async () => {
    const saved: IndexCoverageResponse = {
      snapshot: { policy: { version: 1, revision: 9, mode: "broad_coverage", rules: [] }, sources: [] },
      preview: null
    };
    call.mockResolvedValue(saved);
    await mount();
    expect(host.querySelector("select")!.value).toBe("broad_coverage");
    await act(async () => root?.unmount());
    root = undefined;
    host.remove();
    call.mockClear();

    await mount();

    expect(host.querySelector("select")!.value).toBe("broad_coverage");
    expect(call.mock.calls.some(([request]) => request.action === "save")).toBe(false);
  });

  it("keeps the last saved policy after a failed write and clears the failed preview", async () => {
    const persisted: IndexCoverageResponse = {
      snapshot: { policy: { version: 1, revision: 7, mode: "smart_lean", rules: [] }, sources: [] },
      preview: null
    };
    call.mockImplementation(async (request: IndexCoverageRequest) => {
      if (request.action === "save") throw new Error("controlled SQLite write failure");
      return { ...persisted, preview: request.action === "preview" ? { sampled: 2, included: 2, excluded: 0, undetermined: 0, sampleLimit: 1000, isComplete: false } : null };
    });
    await mount();
    const mode = host.querySelector("select")!;
    await act(async () => { mode.value = "broad_coverage"; mode.dispatchEvent(new Event("change", { bubbles: true })); });
    await click("Preview coverage changes");
    await click("Apply and rebuild");

    expect(host.querySelector("select")!.value).toBe("broad_coverage");
    expect(host.textContent).toContain("controlled SQLite write failure");
    expect([...host.querySelectorAll("button")].find((b) => b.textContent === "Apply and rebuild")!.disabled).toBe(true);
    await click("Refresh coverage status");
    expect(host.querySelector("select")!.value).toBe("smart_lean");
    expect(host.textContent).not.toContain("controlled SQLite write failure");
  });

  it("reloads a committed policy after backend recovery is partial and clears the notice when indexing completes", async () => {
    vi.useFakeTimers();
    let committed = false;
    let pending = false;
    const current = (preview: IndexCoverageResponse["preview"] = null): IndexCoverageResponse => ({
      snapshot: {
        policy: { version: 1, revision: committed ? 8 : 7, mode: committed ? "broad_coverage" : "smart_lean", rules: [] },
        sources: [{ sourceId: "source", enabled: true, status: pending ? "rebuild_required" : "ready", pending, hasUndetermined: false, included: 0, excluded: 0, undetermined: 0 }]
      },
      preview
    });
    call.mockImplementation(async (request: IndexCoverageRequest) => {
      if (request.action === "read") return current();
      if (request.action === "preview") return current({ sampled: 1, included: 1, excluded: 0, undetermined: 0, sampleLimit: 1000, isComplete: false });
      committed = true;
      pending = true;
      throw new Error("global_index_change_committed_recovery_pending: controlled worker start failure");
    });
    await mount();
    await click("Preview coverage changes");
    const mode = host.querySelector("select")!;
    await act(async () => { mode.value = "broad_coverage"; mode.dispatchEvent(new Event("change", { bubbles: true })); });
    await click("Preview coverage changes");
    await click("Apply and rebuild");

    expect(host.querySelector("select")!.value).toBe("broad_coverage");
    expect(host.textContent).toContain("safe recovery is still pending");
    expect(host.textContent).toContain("Reconfiguring coverage. Old-policy results are hidden");
    expect([...host.querySelectorAll("button")].find((b) => b.textContent === "Apply and rebuild")!.disabled).toBe(true);

    pending = false;
    await act(async () => vi.advanceTimersByTimeAsync(2500));

    expect(host.querySelector("select")!.value).toBe("broad_coverage");
    expect(host.textContent).not.toContain("recovery is pending");
  });

  it("shows observation categories separately from a source that is still indexing", async () => {
    call.mockResolvedValue({
      snapshot: {
        policy: { version: 1, revision: 7, mode: "smart_lean", rules: [] },
        sources: [{ sourceId: "source", enabled: true, status: "indexing", pending: false, hasUndetermined: true, included: 4, excluded: 2, undetermined: 1 }]
      },
      preview: null
    });
    await act(async () => {
      host = document.createElement("div"); document.body.append(host); root = createRoot(host);
      root.render(<IndexCoverageSettings t={makeTranslator("en")} sources={[]} disabled={false} statusText={(status) => status === "indexing" ? "Indexing" : status} />);
    });

    expect(host.textContent).toContain("Included: 4");
    expect(host.textContent).toContain("Excluded: 2");
    expect(host.textContent).toContain("Undetermined: 1");
    expect(host.textContent).toContain("not file totals");
    expect(host.textContent).toContain("Indexing");
    expect(host.textContent).toContain("Partial coverage: entries with uncertain identity");
  });

  it("requires a preview, binds CAS revision, and keeps source authorization separate", async () => {
    call.mockImplementation(async (r: IndexCoverageRequest) => ({ ...response(), preview: r.action === "preview" ? { sampled: 0, included: 0, excluded: 0, undetermined: 0, sampleLimit: 1000, isComplete: false } : null }));
    await mount();
    expect(host.querySelector("select")!.value).toBe("smart_lean");
    expect([...host.querySelectorAll("button")].find((b) => b.textContent === "Apply and rebuild")!.disabled).toBe(true);
    await click("Preview coverage changes");
    expect(host.textContent).toContain("not the complete impact");
    await click("Apply and rebuild");
    expect(call).toHaveBeenCalledWith({ action: "save", expectedRevision: 7, mode: "smart_lean", rules: [] });
    expect(host.textContent).toContain("never authorizes a new disk");
  });
  it("requires refresh after the backend rejects a stale preview revision", async () => {
    call.mockImplementation(async (r: IndexCoverageRequest) => {
      if (r.action === "preview") throw new Error("coverage_revision_conflict");
      return response();
    });
    await mount();
    const select = host.querySelector("select")!;
    await act(async () => { select.value = "custom"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(host.textContent).toContain("Enable a source first");
    await click("Preview coverage changes");
    expect(host.textContent).toContain("Coverage policy changed elsewhere");
    expect([...host.querySelectorAll("button")].find((b) => b.textContent === "Apply and rebuild")!.disabled).toBe(true);
    expect([...host.querySelectorAll("button")].find((b) => b.textContent === "Preview coverage changes")!.disabled).toBe(true);
    expect(call.mock.calls.filter(([r]) => r.action === "save")).toHaveLength(0);
  });
});
