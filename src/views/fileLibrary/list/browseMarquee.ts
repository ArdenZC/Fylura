export interface SelectionRect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export function selectionRectFromPoints(x1: number, y1: number, x2: number, y2: number): SelectionRect {
  return {
    left: Math.min(x1, x2),
    top: Math.min(y1, y2),
    right: Math.max(x1, x2),
    bottom: Math.max(y1, y2)
  };
}

export function intersectSelectionRects(a: SelectionRect, b: SelectionRect): SelectionRect | null {
  const rect = {
    left: Math.max(a.left, b.left),
    top: Math.max(a.top, b.top),
    right: Math.min(a.right, b.right),
    bottom: Math.min(a.bottom, b.bottom)
  };
  return rect.right > rect.left && rect.bottom > rect.top ? rect : null;
}

export function selectionRectIntersects(a: SelectionRect, b: SelectionRect) {
  return a.right > b.left && a.left < b.right && a.bottom > b.top && a.top < b.bottom;
}
