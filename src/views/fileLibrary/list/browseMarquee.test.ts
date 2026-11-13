import { describe, expect, it } from "vitest";
import { intersectSelectionRects, selectionRectFromPoints, selectionRectIntersects } from "./browseMarquee";
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
});
