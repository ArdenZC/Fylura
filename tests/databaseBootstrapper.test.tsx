// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseBootstrapper, DATABASE_BOOTSTRAP_LOADING_DELAY_MS } from "../src/components/DatabaseBootstrapper";
import { useAppStore } from "../src/store/useAppStore";

const apiMocks = vi.hoisted(() => ({ initDatabase: vi.fn() }));
vi.mock("../src/api/tauriApi", () => ({ tauriApi: apiMocks }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("database bootstrap recovery", () => {
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    apiMocks.initDatabase.mockReset();
    useAppStore.setState({ language: "zh", toast: null });
    document.body.innerHTML = '<div id="test-root"></div>';
    root = createRoot(document.getElementById("test-root")!);
  });

  afterEach(() => {
    act(() => root.unmount());
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("keeps fast startup quiet and renders children when initialization finishes before the delay", async () => {
    apiMocks.initDatabase.mockResolvedValue(undefined);
    await act(async () => {
      root.render(createElement(DatabaseBootstrapper, null, createElement("div", { id: "ready" }, "ready")));
      await Promise.resolve();
    });

    expect(apiMocks.initDatabase).toHaveBeenCalledOnce();
    expect(document.getElementById("ready")?.textContent).toBe("ready");
    expect(document.body.textContent).not.toContain("正在准备 Fylura");
  });

  it("shows and announces an intentional loading state only after the startup delay", async () => {
    const gate = deferred<void>();
    apiMocks.initDatabase.mockReturnValue(gate.promise);
    act(() => root.render(createElement(DatabaseBootstrapper, null, createElement("div", null, "ready"))));

    act(() => vi.advanceTimersByTime(DATABASE_BOOTSTRAP_LOADING_DELAY_MS - 1));
    expect(document.body.textContent).not.toContain("正在准备 Fylura");

    act(() => vi.advanceTimersByTime(1));
    const status = document.querySelector<HTMLElement>('[role="status"]');
    expect(status?.getAttribute("aria-live")).toBe("polite");
    expect(status?.getAttribute("aria-atomic")).toBe("true");
    expect(status?.textContent).toContain("正在准备 Fylura");
    expect(status?.textContent).toContain("正在打开本地数据与文件空间");

    await act(async () => {
      gate.resolve();
      await gate.promise;
    });
    expect(document.body.textContent).toContain("ready");
  });

  it("keeps technical failure details disclosed and retries the authoritative initialization path", async () => {
    apiMocks.initDatabase
      .mockRejectedValueOnce(new Error("sqlite_locked_internal_detail"))
      .mockResolvedValueOnce(undefined);

    await act(async () => {
      root.render(createElement(DatabaseBootstrapper, null, createElement("div", { id: "ready" }, "ready")));
      await Promise.resolve();
    });

    expect(document.body.textContent).toContain("无法访问数据库");
    expect(document.body.textContent).toContain("Fylura 暂时无法打开本地数据");
    expect(document.querySelector("[data-database-technical-details]")?.textContent).toContain("sqlite_locked_internal_detail");
    const primaryDescription = [...document.querySelectorAll("p")].find((node) => node.textContent?.includes("Fylura 暂时无法打开本地数据"));
    expect(primaryDescription?.textContent).not.toContain("sqlite_locked_internal_detail");

    const retry = [...document.querySelectorAll<HTMLButtonElement>("button")].find((button) => button.textContent === "重试");
    await act(async () => {
      retry?.click();
      await Promise.resolve();
    });

    expect(apiMocks.initDatabase).toHaveBeenCalledTimes(2);
    expect(document.getElementById("ready")?.textContent).toBe("ready");
  });
});
