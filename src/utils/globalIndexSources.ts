import type { GlobalIndexSource } from "../types/domain";

/**
 * Windows NTFS fixed volumes are indexed as a whole volume. Enabling one is a
 * distinct, explicit full-volume authorization decision rather than an
 * ordinary source toggle, so both Settings and first-run consent must use the
 * same predicate.
 */
export function isFullVolumeAuthorizationTarget(source: GlobalIndexSource): boolean {
  return source.volume.platform === "windows"
    && source.volume.driveKind === "fixed"
    && source.volume.filesystemType.trim().toLowerCase() === "ntfs";
}

export function isGlobalIndexSourceAuthorized(source: GlobalIndexSource): boolean {
  return source.volume.platform !== "windows"
    || source.volume.scopeAuthorizationState === "authorized";
}
