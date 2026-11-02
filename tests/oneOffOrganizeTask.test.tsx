// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import { useOrganizationPlanStore } from "../src/store/useOrganizationPlanStore";
import type { OrganizationPlan } from "../src/types/domain";
import { useOneOffOrganizeTask } from "../src/views/organize/useOneOffOrganizeTask";
const api = vi.hoisted(() => ({ queryOrganizationPlanItems: vi.fn() }));
vi.mock("../src/api/tauriApi", () => ({ tauriApi: api }));
const initial = useOrganizationPlanStore.getState();
const plan = { id: "one-off-a", sourceKind: "one_off", revision: 1 } as OrganizationPlan;
let root: Root;
let task: ReturnType<typeof useOneOffOrganizeTask>;
function Harness() { task = useOneOffOrganizeTask(makeTranslator("en"), "win32"); return null; }
beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  useOrganizationPlanStore.setState({ ...initial, activePlan: plan, analyzeMissing: vi.fn().mockResolvedValue({ applied: true, value: 1 }), refreshPlan: vi.fn().mockResolvedValue({ applied: true }) });
  api.queryOrganizationPlanItems.mockResolvedValue({ hasMore: false, items: ["a", "b"].map(id => ({ id, validity: "needs_analysis", sourceIsDirSnapshot: false, decision: "undecided" })) });
  root = createRoot(document.createElement("div"));
  await act(async () => { root.render(createElement(Harness)); });
});
afterEach(async () => { await act(async () => root.unmount()); useOrganizationPlanStore.setState(initial); });
describe("one-off task authorization and lifecycle", () => {
  it("does not analyze before task consent and does not reuse consent for another Plan", async () => {
    await act(async () => { await task.analyze(); });
    expect(api.queryOrganizationPlanItems).not.toHaveBeenCalled();
    await act(async () => { task.setConsent(true); });
    expect(task.consent).toBe(true);
    await act(async () => { useOrganizationPlanStore.setState({ activePlan: { ...plan, id: "one-off-b" } }); });
    expect(task.consent).toBe(false);
    await act(async () => { await task.analyze(); });
    expect(api.queryOrganizationPlanItems).not.toHaveBeenCalled();
  });
  it("stops after the in-flight file and prevents a duplicate analysis loop", async () => {
    let resolve!: (value: { applied: true; value: number }) => void;
    const pending = new Promise<{ applied: true; value: number }>(done => { resolve = done; });
    const analyze = vi.fn(() => pending);
    useOrganizationPlanStore.setState({ analyzeMissing: analyze });
    await act(async () => { task.setConsent(true); });
    let run!: Promise<void>;
    await act(async () => { run = task.analyze(); await task.analyze(); });
    expect(analyze).toHaveBeenCalledTimes(1);
    await act(async () => { task.cancel(); resolve({ applied: true, value: 1 }); await run; });
    expect(analyze).toHaveBeenCalledTimes(1);
    expect(analyze).toHaveBeenCalledWith(["a"], true);
    expect(useOrganizationPlanStore.getState().refreshPlan).toHaveBeenCalledOnce();
    expect(task.progress).toBeNull();
  });
});
