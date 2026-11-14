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
  id: ZcAnimationId,
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
