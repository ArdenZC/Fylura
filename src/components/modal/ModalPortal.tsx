import { createPortal } from "react-dom";
import { createContext, Fragment, useContext, useEffect, useId, useRef, useState, type ReactNode, type RefObject } from "react";

export const APP_SHELL_CONTENT_ID = "app-shell-content";
export const MODAL_HOST_ID = "modal-host";

export interface ModalStackEntry {
  id: string;
  element: HTMLElement | null;
  restoreTarget: HTMLElement | null;
  openedAt: number;
  nestingDepth?: number;
  onEscape?: () => void;
  preferredRestore?: () => HTMLElement | null;
}

type ModalSnapshot = {
  element: HTMLElement;
  inert: boolean;
  ariaHidden: string | null;
};

let modalCount = 0;
let mainDepthCount = 0;
let modalSequence = 0;
let modalSnapshots: ModalSnapshot[] = [];
let observer: MutationObserver | null = null;
let modalStack: ModalStackEntry[] = [];
let globalKeydownAttached = false;
let globalFocusinAttached = false;
const ModalNestingContext = createContext(0);

export function ensureModalHost(): HTMLElement | null {
  if (typeof document === "undefined") return null;
  const existing = document.getElementById(MODAL_HOST_ID);
  const host = existing instanceof HTMLElement ? existing : document.createElement("div");
  host.id = MODAL_HOST_ID;
  host.dataset.modalHost = "true";
  host.style.position = "fixed";
  host.style.inset = "0";
  host.style.pointerEvents = "none";
  host.style.isolation = "isolate";
  host.style.zIndex = "1000";
  if (!host.isConnected) document.body.appendChild(host);
  return host;
}

function findContentRoot() {
  if (typeof document === "undefined") return null;
  const root = document.getElementById(APP_SHELL_CONTENT_ID);
  return root instanceof HTMLElement ? root : null;
}

function isolateContentRoot() {
  const root = findContentRoot();
  if (!root) return;
  if (!modalSnapshots.some((snapshot) => snapshot.element === root)) {
    modalSnapshots.push({
      element: root,
      inert: root.inert,
      ariaHidden: root.getAttribute("aria-hidden")
    });
  }
  root.inert = true;
  root.setAttribute("aria-hidden", "true");
}

function startIsolationObserver() {
  if (typeof MutationObserver === "undefined" || observer) return;
  observer = new MutationObserver(() => {
    if (modalCount > 0) isolateContentRoot();
  });
  observer.observe(document.body, { childList: true, subtree: true });
}

function stopIsolationObserver() {
  observer?.disconnect();
  observer = null;
}

/**
 * Depth cue from MOTION.md §3.3: while a modal layer is present the main
 * workspace recedes by 0.6%. The counter is shared with non-ModalPortal
 * layers (the command spotlight) so overlapping layers cannot clear it early.
 */
export function acquireMainDepth() {
  if (typeof document === "undefined") return () => undefined;
  mainDepthCount += 1;
  document.documentElement.classList.add("zc-modal-depth");
  let released = false;
  return () => {
    if (released) return;
    released = true;
    mainDepthCount = Math.max(0, mainDepthCount - 1);
    if (mainDepthCount === 0) document.documentElement.classList.remove("zc-modal-depth");
  };
}

export function acquireModalIsolation() {
  modalCount += 1;
  if (modalCount === 1) {
    isolateContentRoot();
    startIsolationObserver();
  } else {
    isolateContentRoot();
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    modalCount = Math.max(0, modalCount - 1);
    if (modalCount !== 0) return;
    stopIsolationObserver();
    for (const snapshot of modalSnapshots) {
      if (!snapshot.element.isConnected) continue;
      snapshot.element.inert = snapshot.inert;
      if (snapshot.ariaHidden === null) snapshot.element.removeAttribute("aria-hidden");
      else snapshot.element.setAttribute("aria-hidden", snapshot.ariaHidden);
    }
    modalSnapshots = [];
  };
}

function isHiddenByAncestor(element: HTMLElement) {
  let current: HTMLElement | null = element;
  while (current) {
    if (current.hidden || current.inert || current.getAttribute("aria-hidden") === "true") return true;
    const style = window.getComputedStyle(current);
    if (style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse") return true;
    current = current.parentElement;
  }
  return false;
}

export function isUsableFocusTarget(element: HTMLElement | null, allowZeroGeometry = false): element is HTMLElement {
  if (typeof document === "undefined" || !element || !element.isConnected || element === document.body || element === document.documentElement) return false;
  if (element.matches(":disabled, [disabled], [hidden], [aria-disabled='true']")) return false;
  if (isHiddenByAncestor(element)) return false;
  const style = window.getComputedStyle(element);
  if (style.display === "none" || style.visibility === "hidden" || style.visibility === "collapse") return false;
  const rects = Array.from(element.getClientRects());
  if (!rects.length && !allowZeroGeometry) return false;
  if (!rects.length && allowZeroGeometry) return isNaturallyFocusable(element);
  const hasVisibleRect = rects.some((rect) => rect.width > 0 || rect.height > 0) || element.clientWidth > 0 || element.clientHeight > 0;
  return hasVisibleRect || (allowZeroGeometry && isNaturallyFocusable(element));
}

function isNaturallyFocusable(element: HTMLElement) {
  return element.matches("a[href], area[href], button, input, select, textarea, iframe, object, summary, [contenteditable='true']");
}

/** Focuses a semantic fallback without leaving a permanent tabindex on it. */
export function focusWithTemporaryTabIndex(element: HTMLElement | null, allowZeroGeometry = false) {
  if (!isUsableFocusTarget(element, allowZeroGeometry)) return false;
  const shouldTemporarilyTab = !isNaturallyFocusable(element) && element.tabIndex < 0;
  const hadTabIndex = element.hasAttribute("tabindex");
  const originalTabIndex = element.getAttribute("tabindex");
  if (shouldTemporarilyTab) element.setAttribute("tabindex", "-1");
  element.focus();
  const focused = document.activeElement === element;
  if (shouldTemporarilyTab) {
    if (hadTabIndex) element.setAttribute("tabindex", originalTabIndex ?? "");
    else element.removeAttribute("tabindex");
  }
  return focused;
}

function focusableIn(element: HTMLElement | null, allowZeroGeometry = false) {
  if (!element) return [];
  return Array.from(element.querySelectorAll<HTMLElement>(
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [contenteditable="true"], [tabindex]:not([tabindex="-1"])'
  )).filter((candidate) => isUsableFocusTarget(candidate, allowZeroGeometry));
}

function focusModalEntry(entry: ModalStackEntry | undefined, preferred: HTMLElement | null = null) {
  if (!entry?.element) return false;
  if (preferred && entry.element.contains(preferred) && focusWithTemporaryTabIndex(preferred, true)) return true;
  const autofocus = entry.element.querySelector<HTMLElement>("[autofocus]");
  if (focusWithTemporaryTabIndex(autofocus, true)) return true;
  const first = focusableIn(entry.element)[0] ?? entry.element.querySelector<HTMLElement>(
    'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [contenteditable="true"], [tabindex]:not([tabindex="-1"])'
  );
  if (focusWithTemporaryTabIndex(first, true)) return true;
  return focusWithTemporaryTabIndex(entry.element, true);
}

export function restoreDialogFocus(
  previous: Element | null,
  preferred?: HTMLElement | null,
  fallbackSelector = "[data-dialog-focus-fallback]"
) {
  const candidates: HTMLElement[] = [];
  if (preferred) candidates.push(preferred);
  if (previous instanceof HTMLElement) candidates.push(previous);
  if (typeof document !== "undefined") {
    candidates.push(...Array.from(document.querySelectorAll<HTMLElement>(fallbackSelector)));
    candidates.push(...Array.from(document.querySelectorAll<HTMLElement>("main button:not([disabled]), [role='main'] button:not([disabled])")));
    candidates.push(...Array.from(document.querySelectorAll<HTMLElement>("main h1, main h2, [role='main'] h1, [role='main'] h2")));
    const main = document.querySelector<HTMLElement>("main, [role='main']");
    if (main) candidates.push(main);
    candidates.push(...Array.from(document.querySelectorAll<HTMLElement>("#app-shell-content nav a[href], #app-shell-content nav button:not([disabled]), #app-shell-content [data-nav-item]")));
  }
  const seen = new Set<HTMLElement>();
  for (const candidate of candidates) {
    if (seen.has(candidate)) continue;
    seen.add(candidate);
    if (focusWithTemporaryTabIndex(candidate)) return candidate;
  }
  return null;
}

function syncModalStack() {
  const topIndex = modalStack.length - 1;
  modalStack.forEach((entry, index) => {
    const element = entry.element;
    if (!element) return;
    const top = index === topIndex;
    element.dataset.modalIndex = String(index);
    element.dataset.modalTop = top ? "true" : "false";
    element.dataset.modalDepth = String(entry.nestingDepth ?? 0);
    element.style.zIndex = String(1000 + index);
    element.style.position = "fixed";
    element.style.inset = "0";
    element.style.isolation = "isolate";
    element.style.pointerEvents = top ? "auto" : "none";
    const content = element.querySelector<HTMLElement>("[data-modal-content]");
    if (content) content.style.pointerEvents = top ? "auto" : "none";
    element.inert = !top;
    if (top) element.removeAttribute("aria-hidden");
    else element.setAttribute("aria-hidden", "true");
  });
}

function handleModalKeyDown(event: KeyboardEvent) {
  const top = modalStack[modalStack.length - 1];
  if (!top?.element) return;
  if (event.key === "Escape") {
    if (!top.onEscape) return;
    event.preventDefault();
    top.onEscape();
    return;
  }
  if (event.key !== "Tab") return;
  const focusable = focusableIn(top.element, true);
  if (!focusable.length) {
    event.preventDefault();
    focusWithTemporaryTabIndex(top.element, true);
    return;
  }
  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  const active = document.activeElement as HTMLElement | null;
  if (!top.element.contains(active)) {
    event.preventDefault();
    focusWithTemporaryTabIndex(event.shiftKey ? last : first, true);
  } else if (event.shiftKey && active === first) {
    event.preventDefault();
    focusWithTemporaryTabIndex(last, true);
  } else if (!event.shiftKey && active === last) {
    event.preventDefault();
    focusWithTemporaryTabIndex(first, true);
  }
}

function startModalKeydownListener() {
  if (globalKeydownAttached || typeof document === "undefined") return;
  document.addEventListener("keydown", handleModalKeyDown);
  globalKeydownAttached = true;
}

function handleModalFocusIn(event: FocusEvent) {
  const top = modalStack[modalStack.length - 1];
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (!top?.element || !target || top.element.contains(target)) return;
  event.preventDefault();
  requestAnimationFrame(() => focusModalEntry(top));
}

function startModalFocusinListener() {
  if (globalFocusinAttached || typeof document === "undefined") return;
  document.addEventListener("focusin", handleModalFocusIn, true);
  globalFocusinAttached = true;
}

function stopModalFocusinListener() {
  if (!globalFocusinAttached || typeof document === "undefined") return;
  document.removeEventListener("focusin", handleModalFocusIn, true);
  globalFocusinAttached = false;
}

function stopModalKeydownListener() {
  if (!globalKeydownAttached || typeof document === "undefined") return;
  document.removeEventListener("keydown", handleModalKeyDown);
  globalKeydownAttached = false;
}

export function registerModal(
  id: string,
  options: Omit<ModalStackEntry, "id" | "openedAt"> = { element: null, restoreTarget: null }
) {
  modalStack = modalStack.filter((entry) => entry.id !== id);
  const entry: ModalStackEntry = { id, openedAt: ++modalSequence, ...options };
  modalStack.push(entry);
  modalStack.sort((left, right) => (left.nestingDepth ?? 0) - (right.nestingDepth ?? 0) || left.openedAt - right.openedAt);
  syncModalStack();
  startModalKeydownListener();
  startModalFocusinListener();
  return () => unregisterModal(id);
}

export function unregisterModal(id: string) {
  const index = modalStack.findIndex((entry) => entry.id === id);
  if (index < 0) return;
  const [removed] = modalStack.splice(index, 1);
  const removedWasTop = index === modalStack.length;
  const nextTop = modalStack[modalStack.length - 1];
  syncModalStack();
  if (nextTop && removedWasTop) {
    requestAnimationFrame(() => focusModalEntry(nextTop, removed.restoreTarget));
  } else if (!nextTop) {
    requestAnimationFrame(() => restoreDialogFocus(removed.restoreTarget, removed.preferredRestore?.() ?? null));
    stopModalKeydownListener();
    stopModalFocusinListener();
  }
  if (removed.element?.isConnected) {
    removed.element.inert = false;
    removed.element.removeAttribute("aria-hidden");
    delete removed.element.dataset.modalIndex;
    delete removed.element.dataset.modalTop;
    delete removed.element.dataset.modalDepth;
    removed.element.style.zIndex = "";
    removed.element.style.position = "";
    removed.element.style.inset = "";
    removed.element.style.isolation = "";
    removed.element.style.pointerEvents = "";
    const content = removed.element.querySelector<HTMLElement>("[data-modal-content]");
    if (content) content.style.pointerEvents = "";
  }
}

export function isTopModal(id: string) {
  return modalStack[modalStack.length - 1]?.id === id;
}

export function getModalIndex(id: string) {
  return modalStack.findIndex((entry) => entry.id === id);
}

export function getTopModal() {
  return modalStack[modalStack.length - 1] ?? null;
}

export function getModalStack() {
  return [...modalStack];
}

export interface ModalPortalProps {
  children: ReactNode;
  modalId?: string;
  onEscape?: () => void;
  initialFocusRef?: RefObject<HTMLElement | null>;
  restoreFocus?: () => HTMLElement | null;
}

export function ModalHost() {
  const [host] = useState(() => ensureModalHost());
  return host ? null : null;
}

export function ModalPortal({ children, modalId, onEscape, initialFocusRef, restoreFocus }: ModalPortalProps) {
  const [host] = useState(() => ensureModalHost());
  const [inline, setInline] = useState(() => typeof document !== "undefined" && !findContentRoot());
  const parentNestingDepth = useContext(ModalNestingContext);
  const generatedId = useId();
  const id = modalId ?? `modal-${generatedId.replace(/:/g, "-")}`;
  const layerRef = useRef<HTMLDivElement | null>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const onEscapeRef = useRef(onEscape);
  const preferredRestoreRef = useRef(restoreFocus);
  const initialFocusRefValue = useRef(initialFocusRef);
  onEscapeRef.current = onEscape;
  preferredRestoreRef.current = restoreFocus;
  initialFocusRefValue.current = initialFocusRef;

  useEffect(() => {
    if (!inline && findContentRoot()) return;
    if (findContentRoot()) setInline(false);
  }, [inline]);

  useEffect(() => {
    const layer = layerRef.current;
    if (!layer) return;
    previousFocusRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const unregister = registerModal(id, {
      element: layer,
      restoreTarget: previousFocusRef.current,
      nestingDepth: parentNestingDepth + 1,
      onEscape: () => onEscapeRef.current?.(),
      preferredRestore: () => preferredRestoreRef.current?.() ?? null
    });
    const releaseIsolation = host && !inline ? acquireModalIsolation() : () => undefined;
    const releaseDepth = host && !inline ? acquireMainDepth() : () => undefined;
    const focusFrame = requestAnimationFrame(() => {
      if (isTopModal(id)) focusModalEntry(getTopModal(), initialFocusRefValue.current?.current ?? null);
    });
    return () => {
      cancelAnimationFrame(focusFrame);
      unregister();
      releaseIsolation();
      releaseDepth();
    };
  }, [host, id, inline]);

  const layer = <div ref={layerRef} data-modal-layer="true" data-modal-id={id} style={{ position: "fixed", inset: 0, isolation: "isolate", pointerEvents: "auto" }}><div data-modal-content="true" style={{ pointerEvents: "auto" }}><ModalNestingContext.Provider value={parentNestingDepth + 1}>{children}</ModalNestingContext.Provider></div></div>;
  if (typeof document === "undefined" || inline) return <Fragment>{layer}</Fragment>;
  return host ? createPortal(layer, host) : <Fragment>{layer}</Fragment>;
}

export function resetModalInfrastructureForTests() {
  for (const entry of modalStack) {
    if (!entry.element?.isConnected) continue;
    entry.element.inert = false;
    entry.element.removeAttribute("aria-hidden");
    delete entry.element.dataset.modalIndex;
    delete entry.element.dataset.modalTop;
    delete entry.element.dataset.modalDepth;
    entry.element.style.zIndex = "";
    entry.element.style.position = "";
    entry.element.style.inset = "";
    entry.element.style.isolation = "";
    entry.element.style.pointerEvents = "";
    const content = entry.element.querySelector<HTMLElement>("[data-modal-content]");
    if (content) content.style.pointerEvents = "";
  }
  modalStack = [];
  modalSequence = 0;
  stopModalKeydownListener();
  stopModalFocusinListener();
  modalCount = 0;
  mainDepthCount = 0;
  document.documentElement.classList.remove("zc-modal-depth");
  stopIsolationObserver();
  for (const snapshot of modalSnapshots) {
    if (!snapshot.element.isConnected) continue;
    snapshot.element.inert = snapshot.inert;
    if (snapshot.ariaHidden === null) snapshot.element.removeAttribute("aria-hidden");
    else snapshot.element.setAttribute("aria-hidden", snapshot.ariaHidden);
  }
  modalSnapshots = [];
  document.getElementById(MODAL_HOST_ID)?.remove();
}
