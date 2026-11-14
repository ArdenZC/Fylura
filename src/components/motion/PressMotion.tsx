import { useEffect } from "react";

/**
 * Delegated press feedback.
 *
 * CSS `:active` misses keyboard activation and can leave a button stuck when
 * the pointer is dragged out or the window loses focus. A single document-level
 * director marks the real pressed element, so every button/segment/nav row in
 * the app shares the 90ms press / 260ms release contract without per-control
 * listeners. Data rows and menu options are excluded on purpose: selection is
 * immediate, and macOS menus do not animate on hover.
 */

const PRESSABLE = [
  "button",
  "[role='button']",
  "[role='tab']",
  "[role='radio']",
  "[data-pressable]"
].join(",");

const EXCLUDED = [
  "[role='option']",
  "[role='menuitem']",
  "[role='menuitemradio']",
  "[role='menuitemcheckbox']",
  "[data-file-row]",
  "[data-no-press]",
  "[inert]",
  "[aria-hidden='true']"
].join(",");

function pressTarget(target: EventTarget | null) {
  if (!(target instanceof Element)) return null;
  const element = target.closest<HTMLElement>(PRESSABLE);
  if (!element || element.closest(EXCLUDED)) return null;
  if (element.matches(":disabled") || element.getAttribute("aria-disabled") === "true") return null;
  if (element.closest(":disabled")) return null;
  return element;
}

function setPressed(element: HTMLElement | null, state: "active" | "release" | null) {
  if (!element) return;
  if (state) element.setAttribute("data-zc-press", state);
  else element.removeAttribute("data-zc-press");
}

export function PressMotion() {
  useEffect(() => {
    let active: HTMLElement | null = null;
    const releaseTimers = new WeakMap<HTMLElement, number>();

    const begin = (target: EventTarget | null) => {
      const next = pressTarget(target);
      if (!next) return;
      if (active && active !== next) release();
      const pendingRelease = releaseTimers.get(next);
      if (pendingRelease !== undefined) {
        window.clearTimeout(pendingRelease);
        releaseTimers.delete(next);
      }
      active = next;
      setPressed(next, "active");
    };

    const release = () => {
      if (!active) return;
      const element = active;
      active = null;
      // Keep a release state long enough for the 260ms pop-back transition to
      // run; removing the attribute immediately can cancel the rebound.
      setPressed(element, "release");
      const timer = window.setTimeout(() => {
        releaseTimers.delete(element);
        if (element.getAttribute("data-zc-press") === "release") setPressed(element, null);
      }, 280);
      releaseTimers.set(element, timer);
    };

    const onPointerDown = (event: PointerEvent) => {
      if (event.button !== 0) return;
      begin(event.target);
    };
    const onPointerOut = (event: PointerEvent) => {
      if (!active) return;
      const next = event.relatedTarget;
      if (next instanceof Node && active.contains(next)) return;
      release();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.repeat || (event.key !== " " && event.key !== "Enter")) return;
      begin(event.target);
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.key !== " " && event.key !== "Enter") return;
      release();
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("pointerup", release, true);
    document.addEventListener("pointercancel", release, true);
    document.addEventListener("pointerout", onPointerOut, true);
    document.addEventListener("keydown", onKeyDown, true);
    document.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", release);
    document.addEventListener("visibilitychange", release);

    return () => {
      release();
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("pointerup", release, true);
      document.removeEventListener("pointercancel", release, true);
      document.removeEventListener("pointerout", onPointerOut, true);
      document.removeEventListener("keydown", onKeyDown, true);
      document.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", release);
      document.removeEventListener("visibilitychange", release);
    };
  }, []);

  return null;
}
