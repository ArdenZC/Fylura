import type { FileQueryFiltersV2 } from "../../../types/domain";
import type { Translator } from "../../../types/ui";

export function countActiveFilters(filters: FileQueryFiltersV2) {
  return filters.fileTypes.length
    + filters.purposes.length
    + filters.lifecycles.length
    + filters.risks.length
    + filters.tagsAllOf.length
    + filters.tagsAnyOf.length
    + filters.tagsNoneOf.length
    + Number(filters.sizeMin !== null || filters.sizeMax !== null)
    + Number(filters.modifiedFrom !== null || filters.modifiedTo !== null)
    + Number(filters.createdFrom !== null || filters.createdTo !== null)
    + Number(filters.duplicate !== "any")
    + Number(filters.review !== "any");
}

export function scopeHealthLabel(state: string, t: Translator) {
  if (state === "permission_required") return t("libraryScopeHealthPermission");
  if (state === "reconciliation_required") return t("libraryScopeHealthReconciliation");
  if (state === "partial" || state === "degraded") return t("libraryScopeHealthPartial");
  if (state === "retry_exhausted") return t("libraryScopeHealthRetry");
  return t("libraryScopeHealthUnavailable");
}


export function replaceCopy(template: string, values: Record<string, string | number>) {
  return Object.entries(values).reduce((copy, [key, value]) => copy.replaceAll(`{${key}}`, String(value)), template);
}
