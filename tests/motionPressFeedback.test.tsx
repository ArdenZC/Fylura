// @vitest-environment happy-dom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PressMotion } from "../src/components/motion/PressMotion";

let container: HTMLDivElement;
let root: Root;

function pointer(type: string, button = 0) {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button });
  return event;
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function render() {
  await act(async () => root.render(
    <>
      <PressMotion />
      <button type="button" id="pressable">Press</button>
      <button type="button" id="second">Second</button>
      <button type="button" id="disabled" disabled>Disabled</button>
      <div role="option" id="option"><button type="button" id="option-button">Option</button></div>
      <div data-file-row><button type="button" id="file-row-button">Row action</button></div>
    </>
  ));
}

describe("press feedback director", () => {
  it("marks pointer presses and releases them on pointerup", async () => {
    await render();
    const button = container.querySelector<HTMLButtonElement>("#pressable")!;
    await act(async () => { button.dispatchEvent(pointer("pointerdown")); });
    expect(button.getAttribute("data-zc-press")).toBe("active");
    await act(async () => { button.dispatchEvent(pointer("pointerup")); });
    expect(button.getAttribute("data-zc-press")).toBe("release");
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 320)); });
    expect(button.hasAttribute("data-zc-press")).toBe(false);
  });

  it("releases when the pointer leaves or the window loses focus", async () => {
    await render();
    const button = container.querySelector<HTMLButtonElement>("#pressable")!;
    await act(async () => { button.dispatchEvent(pointer("pointerdown")); });
    await act(async () => {
      button.dispatchEvent(new MouseEvent("pointerout", { bubbles: true, relatedTarget: document.body }));
    });
    expect(button.getAttribute("data-zc-press")).toBe("release");
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 320)); });
    expect(button.hasAttribute("data-zc-press")).toBe(false);

    await act(async () => { button.dispatchEvent(pointer("pointerdown")); });
    await act(async () => { window.dispatchEvent(new Event("blur")); });
    expect(button.getAttribute("data-zc-press")).toBe("release");
  });

  it("supports keyboard activation and ignores disabled, option and file-row controls", async () => {
    await render();
    const button = container.querySelector<HTMLButtonElement>("#pressable")!;
    await act(async () => {
      button.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true }));
    });
    expect(button.getAttribute("data-zc-press")).toBe("active");
    await act(async () => {
      button.dispatchEvent(new KeyboardEvent("keyup", { key: " ", bubbles: true }));
    });
    expect(button.getAttribute("data-zc-press")).toBe("release");

    for (const id of ["#disabled", "#option-button", "#file-row-button"]) {
      const target = container.querySelector<HTMLButtonElement>(id)!;
      await act(async () => { target.dispatchEvent(pointer("pointerdown")); });
      expect(target.hasAttribute("data-zc-press"), id).toBe(false);
    }
  });

  it("does not leave a stuck press when a second control takes over", async () => {
    await render();
    const first = container.querySelector<HTMLButtonElement>("#pressable")!;
    const second = container.querySelector<HTMLButtonElement>("#second")!;
    await act(async () => { first.dispatchEvent(pointer("pointerdown")); });
    await act(async () => { second.dispatchEvent(pointer("pointerdown")); });
    expect(first.getAttribute("data-zc-press")).toBe("release");
  });
});
