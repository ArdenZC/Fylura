import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import type { PresentationInteractionProjection } from "./interactionContracts";
import { intersectSelectionRects, selectionRectFromPoints, selectionRectIntersects, type SelectionRect } from "./browseMarquee";

const EDGE_ZONE = 28;
const MAX_SCROLL_STEP = 22;
const MIN_DRAG_DISTANCE = 4;

interface ActiveMarquee {
  pointerId: number;
  startX: number;
  startY: number;
  x: number;
  y: number;
  additive: boolean;
  moved: boolean;
  selectionRect: SelectionRect | null;
  hitIds: Set<string>;
  frame: number | null;
}

function idsInRect(root: HTMLElement, rect: SelectionRect) {
  const viewport = root.getBoundingClientRect();
  const clipped = intersectSelectionRects(rect, viewport);
  if (!clipped) return [];
  const ids = new Set<string>();
  for (const element of root.querySelectorAll<HTMLElement>("[data-browse-entry-id], [data-browse-grid-entry-id]")) {
    const bounds = element.getBoundingClientRect();
    if (!selectionRectIntersects(clipped, bounds)) continue;
    const id = element.dataset.browseEntryId ?? element.dataset.browseGridEntryId;
    if (id) ids.add(id);
  }
  return [...ids];
}

/** Marquee selection is only an input method; BrowseSourceOwner remains the selection authority. */
export function useBrowseMarqueeSelection(
  rootRef: RefObject<HTMLElement | null>,
  interaction: PresentationInteractionProjection
) {
  const interactionRef = useRef(interaction);
  interactionRef.current = interaction;
  const activeRef = useRef<ActiveMarquee | null>(null);
  const [rectangle, setRectangle] = useState<SelectionRect | null>(null);

  const stop = useCallback((commit: boolean, pointerId?: number, finalX?: number, finalY?: number) => {
    const active = activeRef.current;
    if (!active || (pointerId !== undefined && active.pointerId !== pointerId)) return;
    if (commit && finalX !== undefined && finalY !== undefined) {
      active.x = finalX;
      active.y = finalY;
      active.moved ||= Math.hypot(active.x - active.startX, active.y - active.startY) >= MIN_DRAG_DISTANCE;
      if (active.moved) {
        active.selectionRect = selectionRectFromPoints(active.startX, active.startY, active.x, active.y);
        const root = rootRef.current;
        if (root) idsInRect(root, active.selectionRect).forEach((id) => active.hitIds.add(id));
      }
    }
    activeRef.current = null;
    if (active.frame !== null) cancelAnimationFrame(active.frame);
    const root = rootRef.current;
    if (root) {
      try {
        if (root.hasPointerCapture(active.pointerId)) root.releasePointerCapture(active.pointerId);
      } catch {
        // A browser may release pointer capture before pointerup is delivered.
      }
      if (commit && active.moved && active.selectionRect) {
        interactionRef.current.actions.selectIds?.(
          [...active.hitIds],
          active.additive
        );
      }
    }
    setRectangle(null);
  }, [rootRef]);

  const tick = useCallback(() => {
    const active = activeRef.current;
    const root = rootRef.current;
    if (!active || !root) return;
    const bounds = root.getBoundingClientRect();
    const distance = Math.hypot(active.x - active.startX, active.y - active.startY);
    active.moved ||= distance >= MIN_DRAG_DISTANCE;
    if (active.moved) {
      const scroll = (position: number, min: number, max: number) => {
        if (position < min + EDGE_ZONE) return -Math.min(MAX_SCROLL_STEP, Math.max(1, min + EDGE_ZONE - position));
        if (position > max - EDGE_ZONE) return Math.min(MAX_SCROLL_STEP, Math.max(1, position - (max - EDGE_ZONE)));
        return 0;
      };
      root.scrollTop += scroll(active.y, bounds.top, bounds.bottom);
      root.scrollLeft += scroll(active.x, bounds.left, bounds.right);
      active.selectionRect = selectionRectFromPoints(active.startX, active.startY, active.x, active.y);
      idsInRect(root, active.selectionRect).forEach((id) => active.hitIds.add(id));
      setRectangle(active.selectionRect);
    }
    active.frame = requestAnimationFrame(tick);
  }, [rootRef]);

  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    const current = interactionRef.current;
    if (current.source !== "browse" || !current.actions.selectIds || event.button !== 0 || !event.isPrimary) return;
    const target = event.target;
    if (!(target instanceof Element) || target.closest("[data-file-entry-kind],button,input,select,textarea,a,[role=button],[role=menuitem]")) return;
    if (!event.currentTarget.contains(target)) return;
    const root = rootRef.current;
    if (!root) return;
    const active: ActiveMarquee = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      x: event.clientX,
      y: event.clientY,
      additive: event.ctrlKey || event.metaKey || event.shiftKey,
      moved: false,
      selectionRect: null,
      hitIds: new Set(),
      frame: null
    };
    activeRef.current = active;
    try {
      root.setPointerCapture(event.pointerId);
    } catch {
      activeRef.current = null;
      return;
    }
    active.frame = requestAnimationFrame(tick);
  }, [rootRef, tick]);

  const onPointerMove = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    const active = activeRef.current;
    if (!active || active.pointerId !== event.pointerId) return;
    active.x = event.clientX;
    active.y = event.clientY;
    if (Math.hypot(active.x - active.startX, active.y - active.startY) >= MIN_DRAG_DISTANCE) event.preventDefault();
  }, []);

  const onPointerUp = useCallback((event: ReactPointerEvent<HTMLElement>) => stop(true, event.pointerId, event.clientX, event.clientY), [stop]);
  const onPointerCancel = useCallback((event: ReactPointerEvent<HTMLElement>) => stop(false, event.pointerId), [stop]);

  useEffect(() => () => stop(false), [stop]);

  return {
    rectangle,
    onPointerDown,
    onPointerMove,
    onPointerUp,
    onPointerCancel
  };
}
