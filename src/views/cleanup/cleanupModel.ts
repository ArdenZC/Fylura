import type {
  AnalysisFinding,
  AnalysisRun,
  CleanupFindingSelection,
  OperationPreview,
  OperationPreviewResult
} from "../../types/domain";
import { normalizePathLike } from "../../utils/viewHelpers";

export type CleanupTier = "safe" | "review" | "caution";
export const FINDING_PAGE_SIZE = 100;
export const AI_RECHECK_BATCH_SIZE = 50;
export const FINDING_ROW_HEIGHT = 238;

export function isCleanupPreviewExecutable(preview: OperationPreview): boolean {
  return preview.status === "pending" && preview.is_executable === true && !preview.blocking_reason;
}

export function isCleanupPreviewScopeExecutable(preview: OperationPreviewResult, expectedFindingIds: readonly string[]): boolean {
  if (preview.truncated || preview.hasMore || preview.total !== preview.previews.length || preview.previews.length !== expectedFindingIds.length) return false;
  const expectedIds = new Set(expectedFindingIds);
  if (expectedIds.size !== expectedFindingIds.length) return false;
  const previewIds = new Set(preview.previews.map((item) => item.fileId || item.file_id || ""));
  return previewIds.size === expectedIds.size
    && preview.previews.every((item) => expectedIds.has(item.fileId || item.file_id || "") && isCleanupPreviewExecutable(item));
}

export function isCleanupRun(run: AnalysisRun): boolean {
  const kind = typeof run.scope?.kind === "string" ? run.scope.kind : "";
  return kind === "approvedCleanupPaths" || kind === "approved_cleanup_paths";
}

export function normalizeScopePaths(paths: readonly string[]): string[] {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const path of paths) {
    const trimmed = path.trim();
    if (!trimmed) continue;
    const comparisonKey = normalizeScopePathForComparison(trimmed);
    if (!comparisonKey || seen.has(comparisonKey)) continue;
    seen.add(comparisonKey);
    normalized.push(trimmed);
  }
  return normalized;
}

export function scopeKey(paths: readonly string[]): string {
  return [...new Set(
    paths
      .map((path) => normalizeScopePathForComparison(path))
      .filter(Boolean)
  )]
    .sort()
    .join("\u0000");
}

export function normalizeScopePathForComparison(path: string): string {
  let normalized = path.trim().replaceAll("\\", "/");
  const lower = normalized.toLocaleLowerCase();
  if (lower.startsWith("//?/unc/")) {
    normalized = `//${normalized.slice(8)}`;
  } else if (lower.startsWith("//?/")) {
    normalized = normalized.slice(4);
  }
  if (normalized === "/") return "/";
  if (/^[a-z]:\/?$/i.test(normalized)) return `${normalized[0].toLowerCase()}:/`;
  return normalizePathLike(normalized);
}

export function scopePaths(run: AnalysisRun): string[] {
  const paths = run.scope?.paths;
  return Array.isArray(paths) ? paths.filter((value): value is string => typeof value === "string" && Boolean(value.trim())) : [];
}

export function isRunInProgress(run: AnalysisRun | null): boolean {
  if (!run) return false;
  return ["queued", "running", "cancelling", "cancel_requested"].includes(run.status) || ["preparing", "running_detectors", "finalizing"].includes(run.phase);
}

export function isPartialRun(run: AnalysisRun): boolean {
  return ["partial", "completed_with_warnings", "completed_partial"].includes(run.status)
    || run.warningCount > 0
    || run.errorCount > 0
    || run.detectorsFailed > 0;
}

export function durableRunState(run: AnalysisRun): "running" | "partial" | "completed" | "failed" | "canceled" {
  if (isRunInProgress(run)) return "running";
  if (["cancelled", "canceled"].includes(run.status)) return "canceled";
  if (["failed", "error"].includes(run.status)) return "failed";
  if (isPartialRun(run)) return "partial";
  return "completed";
}

export function isBackendDefaultSafeFinding(finding: AnalysisFinding): boolean {
  return isFindingSelectable(finding) && finding.tier === "safe" && finding.executable && !finding.requiresConfirmation && isTrashAction(finding.actionKind);
}

export function reconcileAuthoritativeFindingUpdates(
  selectedIds: ReadonlySet<string>,
  updatedFindings: readonly AnalysisFinding[]
): Set<string> {
  const updates = new Map(updatedFindings.map((finding) => [finding.id, finding]));
  const next = new Set<string>();
  for (const id of selectedIds) {
    const updated = updates.get(id);
    if (!updated || isFindingSelectable(updated)) next.add(id);
  }
  return next;
}

export function retainSelectedCleanupFindingsAfterExecution(
  selectedIds: readonly string[],
  successfulIds: ReadonlySet<string>,
  refreshedFindings: readonly AnalysisFinding[]
): Set<string> {
  const refreshed = new Map(refreshedFindings.map((finding) => [finding.id, finding]));
  return new Set(selectedIds.filter((id) => {
    if (successfulIds.has(id)) return false;
    const finding = refreshed.get(id);
    return Boolean(finding && isFindingSelectable(finding)
      && (finding.tier !== "review" || finding.decisionRevision != null));
  }));
}

export function cleanupSelectionFingerprint(runId: string, selections: readonly CleanupFindingSelection[]): string {
  return [runId, ...selections
    .map((selection) => [
      selection.findingId,
      selection.expectedRevision,
      selection.reviewConfirmation?.decisionRevision ?? ""
    ].join(":"))
    .sort()]
    .join("\u0000");
}

export function isAnalysisFinding(value: unknown): value is AnalysisFinding {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<AnalysisFinding>;
  return typeof candidate.id === "string"
    && typeof candidate.findingKey === "string"
    && typeof candidate.revision === "number"
    && typeof candidate.status === "string";
}

/** Offers a separate Review decision, never filesystem execution authority. */
export function canRequestCleanupReview(finding: AnalysisFinding): boolean {
  return finding.tier === "review" && finding.status === "active"
    && isTrashAction(finding.actionKind) && finding.primarySubjectKind !== "duplicate_group"
    && finding.decision !== "dismissed" && finding.decision !== "snoozed";
}

export function isFindingSelectable(finding: AnalysisFinding): boolean {
  return finding.status === "active" && finding.decision !== "dismissed" && finding.decision !== "snoozed"
    && isTrashAction(finding.actionKind)
    && ((finding.tier === "safe" && finding.executable)
      || (canRequestCleanupReview(finding) && finding.decision === "acknowledged"));
}

export function isTrashAction(actionKind: string): boolean {
  return /trash|move/i.test(actionKind);
}

/** A complete preview may refuse some selections. Only its verified subset can execute. */
export function executableCleanupSubset(preview: OperationPreviewResult, selections: readonly CleanupFindingSelection[]): CleanupFindingSelection[] {
  if (preview.truncated || preview.hasMore || preview.total !== preview.previews.length) return [];
  const wanted = new Set(selections.map((s) => s.findingId));
  const ids = preview.previews.map((item) => item.fileId || item.file_id || "");
  if (wanted.size !== selections.length || new Set(ids).size !== ids.length || ids.some((id) => !wanted.has(id)) || ids.length !== wanted.size) return [];
  const allowed = new Set(preview.previews.filter(isCleanupPreviewExecutable).map((item) => item.fileId || item.file_id || ""));
  return selections.filter((selection) => allowed.has(selection.findingId));
}

/** Display persisted advice only. Raw evidence never proves currentness or eligibility. */
export function savedCleanupAIAdvice(finding: AnalysisFinding): { reason: string; riskNote: string | null } | null {
  const envelope = finding.evidenceSummary?.aiAssessment;
  if (!envelope || typeof envelope !== "object" || Array.isArray(envelope)) return null;
  const record = envelope as Record<string, unknown>;
  if (record.schema !== "cleanup_ai_assessment_v1" || !record.assessment || typeof record.assessment !== "object") return null;
  const assessment = record.assessment as Record<string, unknown>;
  if (assessment.candidateId !== finding.id || typeof assessment.reason !== "string" || !assessment.reason.trim()) return null;
  return { reason: assessment.reason, riskNote: typeof assessment.riskNote === "string" ? assessment.riskNote : null };
}
