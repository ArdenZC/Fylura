import type { View } from "../types/ui";

const CANONICAL_VIEWS: ReadonlySet<string> = new Set([
  "scanner",
  "cleanup",
  "organize",
  "library",
  "preview",
  "automation",
  "restore",
  "settings"
]);

/** Normalize external and historical view input into the current app route model. */
export function normalizeViewInput(value: unknown): View | null {
  if (value === "rules") return "automation";
  return typeof value === "string" && CANONICAL_VIEWS.has(value)
    ? value as View
    : null;
}

/**
 * ADR-0016: the ordinary launch destination is the Files workspace. Overview
 * and Managed Library setup remain routable (`?view=scanner`) but are never
 * the mandatory first screen.
 */
export function initialViewFromSearch(search: string): View {
  return normalizeViewInput(new URLSearchParams(search).get("view")) ?? "library";
}
