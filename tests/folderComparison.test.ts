import { describe, expect, it } from "vitest";
import { compareFolders } from "../src/views/fileLibrary/compare/compareFolders";
import type { BrowsePresentationEntry } from "../src/views/fileLibrary/presentation/contracts";
const entry = (displayName: string, size = 10, modifiedAt = 1): BrowsePresentationEntry => ({
  source: "browse", displayName, entryKind: "file", size, modifiedAt,
  renderKey: displayName as BrowsePresentationEntry["renderKey"],
  entryRef: { kind: "ephemeral", browseSessionId: "session", entryId: displayName }
});
describe("current-level metadata comparison", () => {
  it("pairs exact names and reports both one-sided sets, size/time differences, matching metadata and unvisited subdirectories", () => {
    const left = [entry("left"), entry("size"), entry("time"), entry("same"), { ...entry("folder"), entryKind: "directory" as const }];
    const right = [entry("right"), entry("size", 11), entry("time", 10, 2), entry("same"), { ...entry("folder"), entryKind: "directory" as const }];
    expect(Object.fromEntries(compareFolders(left, right).map((r) => [r.name, r.status]))).toEqual({ left: "left", right: "right", size: "different", time: "different", same: "same", folder: "directory" });
    expect(compareFolders(left, right, false).find((r) => r.name === "time")?.status).toBe("same");
  });
  it("does not assume case folding or Unicode normalization from the host OS", () => {
    expect(compareFolders([entry("Case.txt"), entry("é")], [entry("case.txt"), entry("e\u0301")]).map((r) => r.status)).toEqual(["left", "left", "right", "right"]);
  });
  it("refuses matching conclusions for missing metadata or mismatched types", () => {
    expect(compareFolders([{ ...entry("a"), size: undefined }, entry("b"), entry("c")], [entry("a"), { ...entry("b"), modifiedAt: undefined }, { ...entry("c"), entryKind: "directory" }]).map((r) => r.status)).toEqual(["unknown", "unknown", "different"]);
  });
  it("keeps every ref when native names have colliding display spellings", () => {
    const a = entry("lossy-name"), b = { ...entry("lossy-name"), entryRef: { ...entry("lossy-name").entryRef, entryId: "other" } };
    const rows = compareFolders([a, b], [entry("lossy-name")]);
    expect(rows).toHaveLength(3);
    expect(rows.every((r) => r.status === "unknown")).toBe(true);
    expect(new Set(rows.map((r) => r.key)).size).toBe(3);
  });
  it("preserves source-owned opaque references across more than 100 rows", () => {
    const left = Array.from({ length: 137 }, (_, i) => entry(`file-${i}`, i));
    const result = compareFolders(left, left.map((e) => ({ ...e })));
    expect(result).toHaveLength(137);
    expect(result.every((r, i) => r.left === left[i] && r.status === "same")).toBe(true);
  });
});
