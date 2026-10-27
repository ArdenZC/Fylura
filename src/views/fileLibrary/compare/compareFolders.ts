import type { BrowsePresentationEntry } from "../presentation/contracts";
export type Difference = "left" | "right" | "different" | "same" | "directory" | "unknown";
export interface ComparisonRow {
  key: string;
  name: string;
  left?: BrowsePresentationEntry;
  right?: BrowsePresentationEntry;
  status: Difference;
}
/** Exact names are deliberate: neither OS name nor display path proves a volume's case rules. */
export function compareFolders(left: readonly BrowsePresentationEntry[], right: readonly BrowsePresentationEntry[], modified = true): ComparisonRow[] {
  const names = new Map<string, { left: BrowsePresentationEntry[]; right: BrowsePresentationEntry[] }>();
  for (const [side, entries] of [["left", left], ["right", right]] as const) for (const entry of entries) {
    let group = names.get(entry.displayName);
    if (!group) { group = { left: [], right: [] }; names.set(entry.displayName, group); }
    group[side].push(entry);
  }
  const rows: ComparisonRow[] = [];
  for (const [name, group] of names) {
    // Lossy native filename display can collide. Do not silently overwrite a ref or pair ambiguous names.
    if (group.left.length > 1 || group.right.length > 1) {
      for (const side of ["left", "right"] as const) for (const entry of group[side]) {
        rows.push({ key: `${side}:${entry.entryRef.entryId}`, name, [side]: entry, status: "unknown" });
      }
      continue;
    }
    const a = group.left[0], b = group.right[0];
    const status: Difference = !a ? "right" : !b ? "left"
      : a.entryKind !== b.entryKind ? "different"
      : a.entryKind === "directory" ? "directory"
      : a.size === undefined || b.size === undefined || modified && (a.modifiedAt === undefined || b.modifiedAt === undefined) ? "unknown"
      : a.size !== b.size || modified && a.modifiedAt !== b.modifiedAt ? "different" : "same";
    rows.push({ key: name, name, left: a, right: b, status });
  }
  return rows;
}
