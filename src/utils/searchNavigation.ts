import type { View } from "../types/ui";
import type { LibrarySelectionV1 } from "../types/domain";
import { normalizeViewInput } from "./viewRoutes";

export type SearchSettingsTarget = "search-scope" | "global-index" | "appearance" | "ai";

const SETTINGS_SECTION_BY_TARGET: Record<SearchSettingsTarget, string> = {
  "search-scope": "settings-search",
  "global-index": "settings-global-index",
  // Keep the standalone command target as a compatibility alias, but route
  // theme/language/density to the one canonical General settings section.
  appearance: "settings-general",
  ai: "settings-ai"
};

const SETTINGS_TARGET_BY_SECTION: Record<string, SearchSettingsTarget> = {
  "settings-search-scope": "search-scope",
  "settings-search": "search-scope",
  "settings-global-index": "global-index",
  "settings-general": "appearance",
  "settings-ai": "ai"
};

export interface SearchNavigatePayload {
  view: unknown;
  fileId: unknown;
  nonce?: unknown;
  generation?: unknown;
  sessionId?: unknown;
  revision?: unknown;
  settingsTarget?: unknown;
  globalEntryId?: unknown;
  expiresAt?: unknown;
}

export interface PendingSearchNavigation {
  nonce: number;
  generation: number;
  view: View;
  selectedFileId: string;
  librarySelection: LibrarySelectionV1 | null;
  libraryFocusedId: string;
  sessionId: number | null;
  revision: number | null;
}

export function settingsTargetForSection(sectionId: string | null | undefined): SearchSettingsTarget | null {
  if (!sectionId) return null;
  return SETTINGS_TARGET_BY_SECTION[sectionId] ?? null;
}

export function settingsSectionForTarget(target: unknown): string | null {
  return isSearchSettingsTarget(target)
    ? SETTINGS_SECTION_BY_TARGET[target]
    : null;
}

export function isSearchSettingsTarget(value: unknown): value is SearchSettingsTarget {
  return value === "search-scope"
    || value === "global-index"
    || value === "appearance"
    || value === "ai";
}

function isOptionalRevision(value: unknown) {
  return value === undefined
    || value === null
    || (typeof value === "number" && Number.isSafeInteger(value) && value >= 0);
}

function isOptionalFileId(value: unknown) {
  return value === undefined || value === null || (typeof value === "string" && value.length > 0);
}

function isValidSearchNavigatePayload(payload: SearchNavigatePayload) {
  const view = normalizeViewInput(payload.view);
  if (!isOptionalRevision(payload.sessionId)
    || !isOptionalRevision(payload.revision)
    || !isOptionalFileId(payload.fileId)
    || view === null) return false;
  if (payload.settingsTarget !== undefined
    && payload.settingsTarget !== null
    && !isSearchSettingsTarget(payload.settingsTarget)) return false;
  if (payload.globalEntryId != null) {
    return view === "library" && payload.fileId == null && payload.settingsTarget == null
      && typeof payload.globalEntryId === "string" && payload.globalEntryId.length > 0
      && payload.globalEntryId.length <= 256 && typeof payload.expiresAt === "number"
      && Number.isSafeInteger(payload.expiresAt) && Date.now() + 100 < payload.expiresAt;
  }
  return payload.settingsTarget == null
    ? true
    : view === "settings" && payload.fileId == null;
}

function matchesOptionalContext(payloadValue: unknown, pendingValue: number | null | undefined) {
  const normalized = payloadValue == null ? null : payloadValue;
  return normalized === pendingValue;
}

export function matchesSearchNavigationBinding(payload: SearchNavigatePayload, pending: PendingSearchNavigation | null) {
  return Boolean(pending
    && Number.isSafeInteger(pending.generation) && pending.generation > 0
    && Number.isSafeInteger(pending.nonce) && pending.nonce > 0
    && payload.generation === pending.generation
    && payload.nonce === pending.nonce
    && matchesOptionalContext(payload.sessionId, pending.sessionId)
    && matchesOptionalContext(payload.revision, pending.revision));
}

export function shouldApplySearchNavigation(
  payload: SearchNavigatePayload,
  pending: PendingSearchNavigation | null,
  current: Pick<PendingSearchNavigation, "view" | "selectedFileId" | "librarySelection" | "libraryFocusedId">
) {
  return Boolean(
    pending
    && matchesSearchNavigationBinding(payload, pending)
    && isValidSearchNavigatePayload(payload)
    && current.view === pending.view
    && ((payload.fileId == null && payload.globalEntryId == null) || (
      current.selectedFileId === pending.selectedFileId
      && current.librarySelection === pending.librarySelection
      && current.libraryFocusedId === pending.libraryFocusedId
    ))
  );
}

export function applySearchNavigation(
  payload: SearchNavigatePayload,
  setView: (view: View) => void,
  setSelectedFileId: (id: string) => void,
  requestSettingsSection?: (sectionId: string) => void,
  activateFileLibraryFile?: (id: string) => void
) {
  if (!isValidSearchNavigatePayload(payload) || payload.globalEntryId != null) return false;
  const view = normalizeViewInput(payload.view);
  if (!view) return false;
  setView(view);
  if (typeof payload.fileId === "string" && payload.fileId) {
    setSelectedFileId(payload.fileId);
    activateFileLibraryFile?.(payload.fileId);
  }
  const settingsSection = settingsSectionForTarget(payload.settingsTarget);
  if (settingsSection) requestSettingsSection?.(settingsSection);
  return true;
}
