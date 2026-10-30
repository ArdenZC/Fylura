// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import { IndexCoverageSettings } from "../src/views/settings/sections/IndexCoverageSettings";
import type { IndexCoverageRequest, IndexCoverageResponse } from "../src/types/indexCoverage";
const { call } = vi.hoisted(() => ({ call: vi.fn() }));
vi.mock("../src/api/tauriApi", () => ({ tauriApi: { globalIndexCoverage: call } }));
let root: Root | undefined;
let host: HTMLDivElement;
afterEach(async () => { await act(async () => root?.unmount()); host?.remove(); call.mockReset(); });
function response(): IndexCoverageResponse { return { snapshot: { policy: { version: 1, revision: 7, mode: "smart_lean", rules: [] }, sources: [] }, preview: null }; }
async function mount() {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root!.render(<IndexCoverageSettings t={makeTranslator("en")} sources={[]} disabled={false} />));
}
async function click(text: string) { const b = [...host.querySelectorAll("button")].find((b) => b.textContent === text)!; await act(async () => b.click()); }
describe("Index Coverage settings workflow", () => {
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
  it("invalidates preview on edit and displays backend rejection without optimistic success", async () => {
    call.mockImplementation(async (r: IndexCoverageRequest) => {
      if (r.action === "preview") throw new Error("coverage_revision_conflict");
      return response();
    });
    await mount();
    const select = host.querySelector("select")!;
    await act(async () => { select.value = "custom"; select.dispatchEvent(new Event("change", { bubbles: true })); });
    expect(host.textContent).toContain("Enable a source first");
    await click("Preview coverage changes");
    expect(host.querySelector('[role="alert"]')!.textContent).toContain("coverage_revision_conflict");
    expect([...host.querySelectorAll("button")].find((b) => b.textContent === "Apply and rebuild")!.disabled).toBe(true);
    expect(call.mock.calls.filter(([r]) => r.action === "save")).toHaveLength(0);
  });
});
