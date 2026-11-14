/**
 * v3.40 motion helpers.
 *
 * Colour changes stay short and linear (`--zc-duration-fast`); geometry uses
 * the four approved single-overshoot curves. WAAPI is only used where CSS
 * transitions cannot replay on a reused node (page swap, pane layout, view
 * swap), and every entry point returns early under reduced motion.
 */

export const motionEasing = {
  smooth: "cubic-bezier(0.32, 0.72, 0, 1)",
  pop: "cubic-bezier(0.34, 1.38, 0.64, 1)",
  bounce: "cubic-bezier(0.2, 1.62, 0.36, 1)",
  out: "cubic-bezier(0.4, 0, 1, 1)"
} as const;

export const motionDuration = {
  press: 90,
  release: 260,
  pop: 240,
  popOut: 130,
  panel: 200,
  page: 220,
  toast: 320
} as const;

function cssToken(name: string, fallback: string) {
  if (typeof document === "undefined") return fallback;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

function cssDuration(name: string, fallback: number) {
  const parsed = Number.parseFloat(cssToken(name, ""));
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

export function prefersReducedMotion() {
  try {
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}

type ZcAnimationId = "zc-page-in" | "zc-pane-in" | "zc-view-swap" | "zc-tree-in";

/**
 * Plays a named animation without stacking duplicates. `fill: "none"` keeps
 * the settled state owned by CSS, so a cancelled/aborted run cannot leave the
 * element stuck at a transform the stylesheet never declared.
 */
export function playMotion(
  element: Element | null | undefined,
  id: ZcAnimationId | OverlayAnimationId,
  keyframes: Keyframe[],
  options: KeyframeAnimationOptions
) {
  if (!element || prefersReducedMotion()) return null;
  for (const animation of element.getAnimations()) {
    if (animation.id === id) animation.cancel();
  }
  try {
    return element.animate(keyframes, { ...options, id, fill: "none" });
  } catch {
    return null;
  }
}

export function playPageEnter(element: Element | null | undefined, direction: "in" | "out" = "in") {
  if (!element) return null;
  return playMotion(
    element,
    "zc-page-in",
    direction === "in"
      ? [{ opacity: 0, transform: "translateY(4px)" }, { opacity: 1, transform: "translateY(0)" }]
      : [{ opacity: 1 }, { opacity: 0 }],
    { duration: cssDuration("--zc-dur-page", motionDuration.page), easing: cssToken("--zc-spring-smooth", motionEasing.smooth) }
  );
}

export function playPaneEnter(element: Element | null | undefined) {
  return playMotion(
    element,
    "zc-pane-in",
    [{ opacity: 0, transform: "scale(0.982)" }, { opacity: 1, transform: "scale(1)" }],
    { duration: cssDuration("--zc-dur-release", motionDuration.release), easing: cssToken("--zc-spring-smooth", motionEasing.smooth) }
  );
}

export function playViewSwap(element: Element | null | undefined) {
  return playMotion(
    element,
    "zc-view-swap",
    [{ opacity: 0.45, transform: "translateY(5px)" }, { opacity: 1, transform: "translateY(0)" }],
    { duration: cssDuration("--zc-dur-pop", motionDuration.pop), easing: cssToken("--zc-spring-smooth", motionEasing.smooth) }
  );
}

type OverlayAnimationId = "zc-menu-in" | "zc-menu-out" | "zc-modal-in" | "zc-modal-out" | "zc-toast-in" | "zc-toast-out" | "zc-badge-pop" | "zc-bar-in";

export function playOverlayEnter(
  element: Element | null | undefined,
  kind: "menu" | "select" | "modal" | "toast" | "bar",
  options: { above?: boolean; origin?: string } = {}
) {
  if (!element) return null;
  const y = kind === "menu" ? (options.above ? 6 : -6) : kind === "select" ? (options.above ? 6 : -6) : kind === "modal" ? 10 : kind === "toast" ? 14 : 8;
  const scale = kind === "menu" ? 0.96 : kind === "select" ? 0.98 : kind === "modal" ? 0.955 : kind === "toast" ? 0.94 : 1;
  const id: OverlayAnimationId = kind === "menu" || kind === "select" ? "zc-menu-in" : kind === "modal" ? "zc-modal-in" : kind === "toast" ? "zc-toast-in" : "zc-bar-in";
  if (element instanceof HTMLElement && options.origin) element.style.transformOrigin = options.origin;
  for (const animation of element.getAnimations()) {
    if (animation.id === "zc-menu-out" || animation.id === "zc-modal-out" || animation.id === "zc-toast-out") animation.cancel();
  }
  return playMotion(
    element,
    id,
    [
      { opacity: 0, transform: `translateY(${y}px) scale(${scale})` },
      { opacity: 1, transform: "translateY(0) scale(1)" }
    ],
    {
      duration: kind === "toast" ? cssDuration("--zc-dur-toast", motionDuration.toast) : kind === "bar" ? 140 : kind === "select" ? cssDuration("--zc-dur-panel", motionDuration.panel) : cssDuration("--zc-dur-pop", motionDuration.pop),
      easing: cssToken(kind === "toast" ? "--zc-spring-bounce" : "--zc-spring-pop", kind === "toast" ? motionEasing.bounce : motionEasing.pop)
    }
  );
}

export function playOverlayExit(element: Element | null | undefined, kind: "menu" | "modal" | "toast") {
  if (!element) return Promise.resolve();
  const id: OverlayAnimationId = kind === "menu" ? "zc-menu-out" : kind === "modal" ? "zc-modal-out" : "zc-toast-out";
  for (const running of element.getAnimations()) {
    if (running.id === "zc-menu-in" || running.id === "zc-modal-in" || running.id === "zc-toast-in") running.cancel();
  }
  const animation = playMotion(
    element,
    id,
    kind === "toast"
      ? [{ opacity: 1, transform: "translateY(0) scale(1)" }, { opacity: 0, transform: "translateY(8px) scale(.98)" }]
      : [{ opacity: 1, transform: "translateY(0) scale(1)" }, { opacity: 0, transform: "translateY(0) scale(.985)" }],
    { duration: cssDuration("--zc-dur-pop-out", motionDuration.popOut), easing: cssToken("--zc-spring-out", motionEasing.out) }
  );
  if (!animation) return Promise.resolve();
  return animation.finished.catch(() => undefined).then(() => undefined);
}

export function playBadgePop(element: Element | null | undefined) {
  return playMotion(
    element,
    "zc-badge-pop",
    [{ transform: "scale(1)" }, { transform: "scale(1.22)" }, { transform: "scale(1)" }],
    { duration: 320, easing: cssToken("--zc-spring-bounce", motionEasing.bounce) }
  );
}

/**
 * Non-interactive visual exit for a modal that must release its real DOM (and
 * whatever it owns) immediately. The caller clones the currently rendered
 * layer, marks it inert, and this helper fades the snapshot out. The clone has
 * no React handlers, no ids and no focusables, so an operation can run and
 * focus can return while the 130ms picture is still leaving.
 */
export function playExitGhost(element: Element | null | undefined): HTMLElement | null {
  if (!(element instanceof HTMLElement) || typeof document === "undefined") return null;
  if (prefersReducedMotion()) return null;
  const rect = element.getBoundingClientRect();
  if (rect.width < 1 || rect.height < 1) return null;
  const ghost = element.cloneNode(true) as HTMLElement;
  ghost.removeAttribute("id");
  ghost.querySelectorAll("[id]").forEach((node) => node.removeAttribute("id"));
  ghost.querySelectorAll("[data-confirm-card]").forEach((node) => node.removeAttribute("data-confirm-card"));
  ghost.querySelectorAll("[data-confirm-backdrop]").forEach((node) => node.removeAttribute("data-confirm-backdrop"));
  ghost.removeAttribute("data-confirm-backdrop");
  ghost.setAttribute("data-zc-exit-ghost", "true");
  ghost.setAttribute("aria-hidden", "true");
  ghost.inert = true;
  ghost.style.position = "fixed";
  ghost.style.left = `${rect.left}px`;
  ghost.style.top = `${rect.top}px`;
  ghost.style.width = `${rect.width}px`;
  ghost.style.height = `${rect.height}px`;
  ghost.style.margin = "0";
  ghost.style.pointerEvents = "none";
  ghost.style.zIndex = "1001";
  ghost.style.animation = "none";
  ghost.querySelectorAll<HTMLElement>("*").forEach((node) => {
    node.style.animation = "none";
  });
  document.body.appendChild(ghost);
  if (typeof ghost.animate !== "function") {
    ghost.remove();
    return null;
  }
  const card = ghost.querySelector<HTMLElement>("[role='dialog'], [role='alertdialog']");
  const animation = ghost.animate(
    [{ opacity: 1 }, { opacity: 0 }],
    { duration: cssDuration("--zc-dur-pop-out", motionDuration.popOut), easing: cssToken("--zc-spring-out", motionEasing.out), fill: "forwards" }
  );
  if (card) {
    card.animate(
      [{ transform: "none" }, { transform: "translateY(4px) scale(.985)" }],
      { duration: cssDuration("--zc-dur-pop-out", motionDuration.popOut), easing: cssToken("--zc-spring-out", motionEasing.out), fill: "forwards" }
    );
  }
  animation.finished.catch(() => undefined).then(() => ghost.remove());
  return ghost;
}
