// @vitest-environment happy-dom

import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { makeTranslator } from "../src/i18n";
import { ToastContainer } from "../src/components/AppShell";
import { CountBadge } from "../src/components/motion/CountBadge";
import { ChromeProvider, type ChromeContextValue } from "../src/contexts/AppContexts";
import { useAppStore } from "../src/store/useAppStore";
import { useWatcherStatusStore } from "../src/store/useWatcherStatusStore";

const t = makeTranslator("zh");
const chrome = { view: "library", t, language: "zh" } as ChromeContextValue;

let container: HTMLDivElement;
let root: Root;
let animateSpy: ReturnType<typeof vi.fn>;

function installAnimate() {
  animateSpy = vi.fn(() => ({
    id: "",
    cancel: vi.fn(),
    finished: Promise.resolve()
  }));
  (HTMLElement.prototype as unknown as { animate: typeof animateSpy }).animate = animateSpy;
}

beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  useAppStore.setState({ toast: null, language: "zh" });
  useWatcherStatusStore.getState().reset();
  installAnimate();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  reflectDeleteAnimate();
  vi.restoreAllMocks();
});

function reflectDeleteAnimate() {
  delete (HTMLElement.prototype as unknown as { animate?: unknown }).animate;
}

async function renderToast() {
  await act(async () => {
    root.render(createElement(ChromeProvider, { value: chrome, children: createElement(ToastContainer) }));
  });
}

describe("overlay presence feedback", () => {
  it("enters a transient toast with the 320ms bounce and removes it when cleared", async () => {
    await renderToast();
    await act(async () => {
      useAppStore.getState().showSuccess("已保存");
    });
    expect(container.querySelector("[data-toast='success']")?.textContent).toBe("已保存");
    const enter = animateSpy.mock.calls.find(([keyframes]) =>
      Array.isArray(keyframes) && keyframes.some((frame: Keyframe) => String(frame.transform).includes("scale(0.94)"))
    );
    expect(enter?.[1]?.duration).toBe(320);
    expect(enter?.[1]?.easing).toContain("0.2, 1.62, 0.36, 1");

    await act(async () => {
      useAppStore.getState().clearToast();
    });
    expect(container.querySelector("[data-toast]")).toBeNull();
  });

  it("does not hold a durable watcher warning behind an exit animation", async () => {
    useWatcherStatusStore.setState({
      roots: {
        "scan-root-a": {
          scanRootId: "scan-root-a",
          path: "C:/Managed",
          rootRevision: 1,
          watcherRevision: 1,
          watcherAppliedRevision: 0,
          pending: false,
          needsReconciliation: false,
          healthStatus: "permission_required",
          activeRunId: null,
          lastEventAt: 1,
          lastAppliedAt: 1,
          lastErrorCode: null,
          lastErrorMessage: null,
          timestamp: 1
        }
      }
    } as never);
    await renderToast();
    const alert = container.querySelector("[data-toast='error']");
    expect(alert?.getAttribute("role")).toBe("alert");
    expect(alert?.getAttribute("data-closing")).toBeNull();

    await act(async () => {
      useWatcherStatusStore.getState().reset();
    });
    // Durable errors are a projection: they resolve in the same commit.
    expect(container.querySelector("[data-toast]")).toBeNull();
  });

  it("pulses a count badge once per value change", async () => {
    await act(async () => {
      root.render(createElement(CountBadge, { value: 3, className: "badge", label: "3 pending" }));
    });
    expect(animateSpy).not.toHaveBeenCalled();
    expect(container.querySelector(".badge")?.textContent).toBe("3");

    await act(async () => {
      root.render(createElement(CountBadge, { value: 4, className: "badge", label: "4 pending" }));
    });
    expect(container.querySelector(".badge")?.textContent).toBe("4");
    const pulse = animateSpy.mock.calls.find(([keyframes]) =>
      Array.isArray(keyframes) && keyframes.some((frame: Keyframe) => String(frame.transform).includes("scale(1.22)"))
    );
    expect(pulse?.[1]?.duration).toBe(320);
    expect(pulse?.[1]?.id).toBe("zc-badge-pop");
    expect(animateSpy).toHaveBeenCalledTimes(1);

    await act(async () => {
      root.render(createElement(CountBadge, { value: 4, className: "badge", label: "4 pending" }));
    });
    expect(animateSpy).toHaveBeenCalledTimes(1);
  });
});
