export function invertLoadedSelection(loadedIds: readonly string[], selectedIds: ReadonlySet<string>) {
  const loaded = new Set(loadedIds);
  return new Set([
    ...[...selectedIds].filter((id) => !loaded.has(id)),
    ...loadedIds.filter((id) => !selectedIds.has(id))
  ]);
}

export function mergeMarqueeSelection(
  selectedIds: ReadonlySet<string>,
  intersectingIds: readonly string[],
  additive: boolean
) {
  return additive ? new Set([...selectedIds, ...intersectingIds]) : new Set(intersectingIds);
}
