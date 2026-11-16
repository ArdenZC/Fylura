import { describe, expect, it } from "vitest";
import { idsIntersectingSelection, intersectSelectionRects, selectionEntryRectToContent, selectionRectFromPoints, selectionRectIntersects, selectionRectToContent } from "./browseMarquee";
import { invertLoadedSelection, mergeMarqueeSelection } from "../browse/browseSelection";

describe("Browse selection helpers", () => {
  it("normalizes a marquee drawn in any direction and clips it to the list viewport", () => {
    const rect = selectionRectFromPoints(90, 70, 10, 20);
    expect(rect).toEqual({ left: 10, top: 20, right: 90, bottom: 70 });
    expect(intersectSelectionRects(rect, { left: 30, top: 0, right: 100, bottom: 50 }))
      .toEqual({ left: 30, top: 20, right: 90, bottom: 50 });
    expect(selectionRectIntersects(rect, { left: 89, top: 69, right: 100, bottom: 80 })).toBe(true);
    expect(selectionRectIntersects(rect, { left: 90, top: 20, right: 100, bottom: 30 })).toBe(false);
  });

  it("inverts all loaded pages without dropping selected IDs outside the loaded set", () => {
    expect(invertLoadedSelection(["a", "b", "c"], new Set(["a", "outside"])))
      .toEqual(new Set(["outside", "b", "c"]));
  });

  it("adds marquee hits when a selection modifier is held and replaces otherwise", () => {
    const selected = new Set(["a", "outside"]);
    expect(mergeMarqueeSelection(selected, ["b", "c"], true)).toEqual(new Set(["a", "outside", "b", "c"]));
    expect(mergeMarqueeSelection(selected, ["b", "c"], false)).toEqual(new Set(["b", "c"]));
  });

  it("uses final content-space geometry after scrolling and dragging the rectangle back", () => {
    const viewport = { left: 20, top: 40 };
    const itemRects = new Map([
      ["first", selectionEntryRectToContent({ left: 25, top: 60, right: 85, bottom: 90 }, viewport, 0, 0)],
      ["second", selectionEntryRectToContent({ left: 25, top: 120, right: 85, bottom: 150 }, viewport, 0, 0)],
      ["third", selectionEntryRectToContent({ left: 25, top: 180, right: 85, bottom: 210 }, viewport, 0, 0)]
    ]);
    const expanded = selectionRectToContent({ left: 20, top: 50, right: 100, bottom: 180 }, viewport, 0, 0);
    expect(idsIntersectingSelection(itemRects, expanded)).toEqual(["first", "second"]);

    // The final geometry is recomputed, so an item hit only by the earlier,
    // larger rectangle is not left in the selection after the pointer returns.
    const contracted = selectionRectToContent({ left: 20, top: 50, right: 100, bottom: 115 }, viewport, 0, 0);
    expect(idsIntersectingSelection(itemRects, contracted)).toEqual(["first"]);

    const scrolled = selectionRectToContent({ left: 20, top: 50, right: 100, bottom: 115 }, viewport, 0, 300);
    expect(scrolled.top).toBe(contracted.top + 300);
    expect(scrolled.bottom).toBe(contracted.bottom + 300);
  });
});
