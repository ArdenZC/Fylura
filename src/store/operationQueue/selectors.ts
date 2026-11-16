import type { OperationLog, OperationPreview } from "../../types/domain";
import { validateOrganizeFileNameForOriginal } from "../../views/organize/organizeModel";

export type PreviewExecutionIntent =
  | { source: "organize"; scopeKey: string; allowedPreviewIds: Set<string>; initialAllowedCount: number; sessionId: string }
  | { source: "general" }
  | null;

export type PreviewExclusionReason = "invalidName" | "blocked" | "outsideWhitelist" | "unavailable";

export type PreviewEligibility =
  | { executable: true; reason: null }
  | { executable: false; reason: PreviewExclusionReason };

export interface ExecutablePreviewSelection {
  operations: OperationPreview[];
  selectedCount: number;
  excludedCount: number;
  outsideWhitelistCount: number;
  blockedCount: number;
  invalidNameCount: number;
  unavailableCount: number;
}

export type OperationConfirmationTone = "default" | "warning" | "danger";
export const MAX_OPERATION_LOGS = 1000;

export function previewsForExecutionIntent(previews: readonly OperationPreview[], intent: PreviewExecutionIntent) {
  return intent?.source === "organize"
    ? previews.filter((preview) => intent.allowedPreviewIds.has(preview.id))
    : [...previews];
}

export function isPreviewBackendApproved(preview: OperationPreview): boolean {
  return preview.status === "pending" && preview.is_executable !== false && !preview.blocking_reason;
}

export function requiresExplicitMaterialization(preview: OperationPreview): boolean {
  const requirement = preview.materialization_requirement ?? preview.materializationRequirement;
  return (requirement === "explicit_download_required" || requirement === "required")
    && (preview.operation_type === "copy" || preview.operation_type === "duplicate" || preview.operation_type === "replace");
}

export function resolvePreviewEligibility(
  preview: OperationPreview,
  intent: PreviewExecutionIntent
): PreviewEligibility {
  if (intent?.source === "organize" && !intent.allowedPreviewIds.has(preview.id)) {
    return { executable: false, reason: "outsideWhitelist" };
  }
  if (preview.status !== "pending") {
    return { executable: false, reason: "unavailable" };
  }
  if ((preview.is_executable === false || Boolean(preview.blocking_reason)) && !requiresExplicitMaterialization(preview)) {
    return { executable: false, reason: "blocked" };
  }
  if (preview.operation_type !== "move_to_trash" && validateOrganizeFileNameForOriginal(preview.old_name, preview.new_name) !== null) {
    return { executable: false, reason: "invalidName" };
  }
  return { executable: true, reason: null };
}

export function isPreviewExecutable(preview: OperationPreview): boolean {
  return resolvePreviewEligibility(preview, null).executable;
}

export function selectionForPreviewGroup(current: Set<string>, previews: readonly OperationPreview[], select: boolean, intent: PreviewExecutionIntent) {
  const next = new Set(current);
  for (const preview of previewsForExecutionIntent(previews, intent)) {
    if (select) {
      if (resolvePreviewEligibility(preview, intent).executable) next.add(preview.id);
    } else {
      next.delete(preview.id);
    }
  }
  return next;
}

export function resolveExecutableSelectedPreviews(
  previews: readonly OperationPreview[],
  selectedIds: ReadonlySet<string>,
  intent: PreviewExecutionIntent
): ExecutablePreviewSelection {
  const selectedPreviews: OperationPreview[] = [];
  const presentIds = new Set<string>();
  for (const preview of previews) {
    if (!selectedIds.has(preview.id) || presentIds.has(preview.id)) continue;
    selectedPreviews.push(preview);
    presentIds.add(preview.id);
  }
  const operations: OperationPreview[] = [];
  let outsideWhitelistCount = 0;
  let blockedCount = 0;
  let invalidNameCount = 0;
  let unavailableCount = 0;

  for (const preview of selectedPreviews) {
    const eligibility = resolvePreviewEligibility(preview, intent);
    if (eligibility.executable) {
      operations.push(preview);
    } else if (eligibility.reason === "outsideWhitelist") {
      outsideWhitelistCount += 1;
    } else if (eligibility.reason === "unavailable") {
      unavailableCount += 1;
    } else if (eligibility.reason === "blocked") {
      blockedCount += 1;
    } else {
      invalidNameCount += 1;
    }
  }
  unavailableCount += [...selectedIds].filter((id) => !presentIds.has(id)).length;
  const excludedCount = outsideWhitelistCount + blockedCount + invalidNameCount + unavailableCount;
  return {
    operations,
    selectedCount: selectedIds.size,
    excludedCount,
    outsideWhitelistCount,
    blockedCount,
    invalidNameCount,
    unavailableCount
  };
}

export function operationNeedsCleanupConfirmation(preview: OperationPreview): boolean {
  return preview.operation_type === "move_to_trash"
    || preview.is_duplicate === true
    || preview.suggested_action === "DeleteCandidate"
    || preview.suggested_action === "Review"
    || preview.requires_confirmation
    || preview.risk_level === "Sensitive"
    || preview.risk_level === "System"
    || preview.confidence < 0.7
    || preview.will_create_parent === true;
}

export function operationConfirmationTone(previews: readonly OperationPreview[]): OperationConfirmationTone {
  if (previews.some((preview) => preview.operation_type === "move_to_trash")) return "danger";
  if (previews.some(operationNeedsCleanupConfirmation)) return "warning";
  return "default";
}

export function mergeOperationLogs(persisted: OperationLog[], current: OperationLog[]): OperationLog[] {
  const seen = new Set<string>();
  const merged: OperationLog[] = [];
  for (const log of [...current, ...persisted]) {
    if (seen.has(log.id)) continue;
    seen.add(log.id);
    merged.push(log);
  }
  return merged.slice(0, MAX_OPERATION_LOGS);
}
