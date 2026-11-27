import { Check, FileSearch, FolderOpen, MoreHorizontal, RefreshCw, ShieldCheck, Square } from "lucide-react";
import { useState } from "react";
import type { AnalysisFinding, AnalysisFindingEvidence } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { formatBytes } from "../../utils/format";
import { compactPath } from "../../utils/viewHelpers";
import { isBrowserMockEnabled } from "../../utils/runtimeMode";
import { cn } from "../../utils/tw";
import { Button, quietText } from "../shared/ui";
import { isFindingSelectable, canRequestCleanupReview, savedCleanupAIAdvice, type CleanupTier } from "./cleanupModel";

export function FindingRow({
  finding,
  selected,
  evidence,
  evidenceExpanded,
  t,
  index,
  virtualized,
  measureElement,
  style,
  onToggle,
  onReveal,
  onToggleEvidence,
  onRevalidate,
  onKeep,
  interactionLocked,
  tierLabel
}: {
  finding: AnalysisFinding;
  selected: boolean;
  evidence?: AnalysisFindingEvidence[];
  evidenceExpanded: boolean;
  t: Translator;
  index: number;
  virtualized: boolean;
  measureElement?: (element: HTMLElement | null) => void;
  style?: { transform: string };
  onToggle: (finding: AnalysisFinding) => void;
  onReveal: (finding: AnalysisFinding) => void;
  onToggleEvidence: (finding: AnalysisFinding) => void;
  onKeep?: (finding: AnalysisFinding) => void;
  onRevalidate: (finding: AnalysisFinding) => void;
  interactionLocked: boolean;
  tierLabel: (tier: string, t: Translator) => string;
}) {
  const isCaution = finding.tier === "caution";
  const selectable = isFindingSelectable(finding);
  const reviewable = canRequestCleanupReview(finding);
  const confidence = finding.confidence === "exact" ? t("storageCleanupConfidenceExact") : finding.confidence === "estimated" ? t("storageCleanupConfidenceEstimated") : t("storageCleanupConfidenceUnknown");
  const category = findingCategoryLabel(finding.category, t);
  const reason = findingReasonLabel(finding.reason, t);
  const aiAdvice = savedCleanupAIAdvice(finding);
  const riskNote = finding.riskNote ? findingRiskLabel(finding.riskNote, t) : null;
  const selectionLabel = selected
    ? t("storageCleanupSelected")
    : reviewable && finding.decision !== "acknowledged"
      ? t("storageCleanupFindingAcknowledge")
      : t("storageCleanupSelectForTrash");
  const [detailsOpen, setDetailsOpen] = useState(false);
  return (
    <article
      className={cn(virtualized ? "absolute left-0 top-0" : "relative", "grid w-full grid-cols-[28px_minmax(0,1fr)_auto_28px] items-center gap-x-2 border-b border-[var(--zc-divider)] px-3 py-2 max-[640px]:grid-cols-[28px_minmax(0,1fr)_auto_28px]", selected && "bg-[var(--zc-surface-selected)]")}
      ref={virtualized ? measureElement : undefined}
      data-index={virtualized ? index : undefined}
      style={virtualized ? style : undefined}
      data-analysis-finding-id={finding.id}
      data-tier={finding.tier}
    >
      <div className="flex items-center justify-center">
        {isCaution ? <span className="text-[var(--zc-text-tertiary)]" aria-label={t("storageCleanupCautionHint")} title={t("storageCleanupCautionHint")}><ShieldCheck size={16} aria-hidden="true" /></span> : (
          <Button
            variant={selected ? "secondary" : "ghost"}
            size="compact"
            className="h-7 min-h-7 w-7 p-1"
            disabled={interactionLocked || (!selectable && !(reviewable && finding.decision !== "acknowledged"))}
            aria-label={selectionLabel}
            title={selectionLabel}
            aria-pressed={selected}
            onClick={() => onToggle(finding)}
          >
            {selected ? <Check size={14} aria-hidden="true" /> : <Square size={14} aria-hidden="true" />}
          </Button>
        )}
      </div>
      <div className="min-w-0">
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
          <strong className="max-w-full truncate text-sm font-medium text-[var(--zc-text-primary)]">{finding.title || category}</strong>
          <span className="sr-only">{tierLabel(finding.tier, t)}{selected ? ` · ${t("storageCleanupSelected")}` : ""}</span>
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-x-3 text-xs leading-4 text-[var(--zc-text-secondary)]">
          <span className="max-w-[42%] shrink-0 truncate" title={finding.pathSnapshot ?? undefined}>{finding.pathSnapshot ? compactPath(finding.pathSnapshot, 110) : t("storageCleanupPathUnavailable")}</span>
          <span className="min-w-0 flex-1 truncate" title={reason}><strong className="font-medium text-[var(--zc-text-primary)]">{t("storageCleanupFindingWhy")}:</strong> {reason}</span>
        </div>
        {riskNote ? <p className="mt-0.5 truncate text-[11px] leading-4 text-[var(--zc-warning-text)]" title={riskNote}>{t("storageCleanupFindingRisk")}: {riskNote}</p> : null}
      </div>
      <span className="shrink-0 text-right text-sm font-semibold tabular-nums text-[var(--zc-text-primary)]">{formatBytes(finding.sizeBytes)}</span>
      <details
        className="relative flex h-7 w-7 items-center justify-center"
        open={detailsOpen || evidenceExpanded}
        onToggle={(event) => setDetailsOpen(event.currentTarget.open)}
        data-cleanup-row-details
      >
        <summary className="flex h-7 w-7 cursor-pointer list-none items-center justify-center rounded-md text-[var(--zc-text-secondary)] hover:bg-[var(--zc-surface-hover)] focus-visible:outline-2 focus-visible:outline-[var(--zc-focus)]" aria-label={t("details")} title={t("details")}>
          <MoreHorizontal size={16} aria-hidden="true" />
        </summary>
        <div className="absolute right-0 top-full z-10 mt-1 grid min-w-64 max-w-[min(420px,80vw)] gap-2 rounded-lg border border-[var(--zc-border)] bg-[var(--zc-surface)] p-3 text-left shadow-lg" data-cleanup-row-details-panel>
          {riskNote || aiAdvice ? (
            <p className="m-0 text-xs leading-5 [overflow-wrap:anywhere]" title={[riskNote, aiAdvice?.reason].filter(Boolean).join(" · ")}>
              {riskNote ? <span className="text-[var(--zc-warning-text)]">{t("storageCleanupFindingRisk")}: {riskNote}</span> : null}
              {riskNote && aiAdvice ? <span className="text-[var(--zc-text-tertiary)]"> · </span> : null}
              {aiAdvice ? <span className="text-[var(--zc-text-secondary)]" data-cleanup-ai-advice>{t("cleanupSavedAIAdvice")}: {aiAdvice.reason}</span> : null}
            </p>
          ) : null}
          <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px] leading-4 text-[var(--zc-text-tertiary)]">
            <span>{t("storageCleanupFindingConfidence")}: {confidence}</span>
            <span>{reviewable ? t("cleanupPendingPreview") : finding.executable ? t("storageCleanupFindingExecutable") : t("storageCleanupFindingBlocked")}</span>
            <span>{category}</span>
          </div>
          <div className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5" data-cleanup-row-actions>
            {finding.pathSnapshot ? <Button variant="ghost" size="compact" onClick={() => onReveal(finding)}><FolderOpen size={14} aria-hidden="true" />{t("storageCleanupReveal")}</Button> : null}
            <Button variant="ghost" size="compact" onClick={() => onToggleEvidence(finding)}><FileSearch size={14} aria-hidden="true" />{evidenceExpanded ? t("storageCleanupFindingHideEvidence") : t("storageCleanupFindingEvidence")}</Button>
            {onKeep && finding.tier !== "caution" ? <Button variant="ghost" size="compact" disabled={interactionLocked} onClick={() => onKeep(finding)}>{t(finding.decision === "dismissed" ? "cleanupIncludeAgain" : "cleanupKeep")}</Button> : null}
            {finding.status === "stale" ? <Button variant="secondary" size="compact" disabled={interactionLocked} onClick={() => onRevalidate(finding)}><RefreshCw size={14} aria-hidden="true" />{t("storageCleanupFindingRecheck")}</Button> : null}
          </div>
          {evidenceExpanded ? <div className="grid gap-2 border-t border-[var(--zc-divider)] pt-2" data-finding-evidence><strong className="text-xs font-semibold text-[var(--zc-text-tertiary)]">{t("storageCleanupFindingEvidence")}</strong>{evidence?.length ? evidence.map((item) => <div key={item.id} className="text-xs leading-5 text-[var(--zc-text-secondary)]" data-ai-assessment-current={item.evidenceKind === "ai_assessment" ? (item.isCurrentAssessment ? "true" : "false") : undefined} data-ai-assessment-source={item.evidenceKind === "ai_assessment" ? (isBrowserMockEnabled() ? "presentation-fixture" : "backend-authority") : undefined}>{item.evidenceKind === "ai_assessment" ? item.isCurrentAssessment ? t(isBrowserMockEnabled() ? "storageCleanupCurrentAIAssessmentFixture" : "storageCleanupCurrentAIAssessment") : t("storageCleanupHistoricalAIAssessment") : item.evidenceKind}{item.pathSnapshot ? ` · ${compactPath(item.pathSnapshot, 100)}` : ""}</div>) : <span className={quietText}>{t("storageCleanupFindingEvidenceEmpty")}</span>}</div> : null}
      </div>
      </details>
    </article>
  );
}

export function tierLabel(tier: string, t: Translator): string {
  if (tier === "safe") return t("storageCleanupSafeTier");
  if (tier === "review") return t("storageCleanupReviewTier");
  return t("storageCleanupCautionTier");
}

export function findingCategoryLabel(category: string, t: Translator): string {
  if (category === "macos_package") return t("storageCleanupMacosPackageCategory");
  if (category === "cloud_item") return t("storageCleanupCloudItemCategory");
  return category;
}

function findingReasonLabel(reason: string, t: Translator): string {
  if (reason === "macos_package_logical_entity") return t("storageCleanupMacosPackageReason");
  if (reason === "cloud_item_not_local_reclaim_deferred") return t("storageCleanupCloudItemReason");
  return reason;
}

function findingRiskLabel(risk: string, t: Translator): string {
  if (risk === "macos_package_cleanup_disabled") return t("storageCleanupMacosPackageRisk");
  if (risk === "cloud_item_not_local_no_reclaim_estimate") return t("storageCleanupCloudItemRisk");
  return risk;
}
