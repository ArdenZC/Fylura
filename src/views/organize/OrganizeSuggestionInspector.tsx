import { ArrowLeft, FileSearch, ShieldAlert } from "lucide-react";
import type { RefObject } from "react";
import type { Translator } from "../../types/ui";
import { compactPath, formatDisplayPath, formatPreviewDisplayPath } from "../../utils/viewHelpers";
import { lifecycleLabel, purposeLabel, riskLabel, typeLabel } from "../fileLibrary/presentation/fileLibraryPresentation";
import { buttonGhost, buttonSecondary, cn } from "../../utils/tw";
import { DecisionBadge } from "./OrganizeSuggestionList";
import { effectiveTargetPath, type OrganizeSuggestion } from "./organizeModel";

export function OrganizeSuggestionInspector({
  suggestion,
  t,
  inspectorRef,
  isNarrowLayout,
  narrowVisible,
  onAccept,
  onKeep,
  onEdit,
  onClear,
  onReturnToList
}: {
  suggestion: OrganizeSuggestion | null;
  t: Translator;
  inspectorRef: RefObject<HTMLElement | null>;
  isNarrowLayout: boolean;
  narrowVisible: boolean;
  onAccept: () => void;
  onKeep: () => void;
  onEdit: () => void;
  onClear: () => void;
  onReturnToList: () => void;
}) {
  if (!suggestion) {
    return (
      <aside id="organize-inspector" ref={inspectorRef} tabIndex={-1} className={cn("grid min-h-64 place-items-center bg-[var(--zc-surface-subtle)] p-5 text-center", isNarrowLayout ? !narrowVisible && "hidden" : "border-l border-[var(--zc-divider)]")} aria-label={t("organizeFileDetails")}>
        <div className="grid max-w-xs gap-2">
          <FileSearch size={24} className="mx-auto text-[var(--zc-info-text)]" aria-hidden="true" />
          <strong>{t("organizeInspectorEmptyTitle")}</strong>
          <span className="text-sm text-[var(--zc-text-secondary)]">{t("organizeInspectorEmptyDesc")}</span>
        </div>
      </aside>
    );
  }

  const { file, preview } = suggestion;
  const targetPath = effectiveTargetPath(suggestion);
  const blockingText = organizeBlockingText(suggestion, t);
  return (
    <aside id="organize-inspector" ref={inspectorRef} tabIndex={-1} className={cn("min-h-0 overflow-y-auto overflow-x-hidden overscroll-contain bg-[var(--zc-surface-subtle)] p-4", isNarrowLayout ? !narrowVisible && "hidden" : "border-l border-[var(--zc-divider)]")} aria-labelledby="organize-inspector-title" aria-label={t("organizeDetailsForFile").replace("{name}", file.name)} aria-keyshortcuts={isNarrowLayout ? "Escape" : undefined} onKeyDown={(event) => { if (isNarrowLayout && event.key === "Escape") { event.preventDefault(); onReturnToList(); } }}>
      <div className="grid gap-4">
        {isNarrowLayout && narrowVisible ? <button type="button" className={cn(buttonGhost, "min-h-8 w-fit px-2 py-1.5 text-xs")} aria-controls="organize-suggestion-pane" onClick={onReturnToList}><ArrowLeft size={14} aria-hidden="true" />{t("organizeBackToFileList")}</button> : null}
        <div className="grid min-h-32 place-items-center gap-2 border-y border-[var(--zc-divider)] bg-[var(--zc-surface)] px-4 py-5 text-center">
          <FileSearch size={28} className="text-[var(--zc-info-text)]" aria-hidden="true" />
          <strong className="text-sm text-[var(--zc-text-primary)]">{typeLabel(file, t)}</strong>
          <span className="text-xs text-[var(--zc-text-secondary)]">{t("libraryPreviewUnavailable")}</span>
        </div>

        <div className="min-w-0 border-b border-[var(--zc-divider)] pb-3">
          <div className="flex items-start justify-between gap-3">
            <h2 id="organize-inspector-title" className="min-w-0 break-words text-lg font-semibold text-[var(--zc-text-primary)]">{file.name}</h2>
            <DecisionBadge decision={suggestion.decision} t={t} />
          </div>
          <p className="mt-1 text-sm text-[var(--zc-text-secondary)]">{typeLabel(file, t)} · {purposeLabel(file, t)}</p>
        </div>

        <dl className="grid gap-3 text-sm">
          <InspectorField label={t("organizeCurrentPath")} value={formatDisplayPath(file.path)} />
          <InspectorField label={t("organizeSuggestedTarget")} value={targetPath ? formatPreviewDisplayPath(targetPath, t) : t("organizeTargetUnavailable")} tone={targetPath ? "normal" : "warning"} />
          <InspectorField label={t("organizeSuggestedAction")} value={preview ? operationLabel(preview.operation_type, t) : t("organizeNoExecutableAction")} />
          <InspectorField label={t("organizeWhySuggested")} value={userFacingReason(suggestion, t)} />
          <InspectorField label={t("lifecycle")} value={lifecycleLabel(file, t)} />
          <InspectorField label={t("risk")} value={riskLabel(file.risk_level, t)} tone={file.risk_level === "Normal" ? "normal" : "warning"} />
        </dl>

        {blockingText ? (
          <div className="flex gap-2 border-y border-[var(--zc-warning-border)] bg-[var(--zc-warning-soft)] px-3 py-2 text-sm text-[var(--zc-warning-text)]" role={suggestion.decision === "blocked" ? "alert" : "status"}>
            <ShieldAlert size={17} className="mt-0.5 shrink-0" aria-hidden="true" />
            <span>{blockingText}</span>
          </div>
        ) : null}

        <div className="grid gap-2">
          <span className="text-xs font-semibold text-[var(--zc-text-tertiary)]">{t("organizeDecisionActions")}</span>
          <div className="flex flex-wrap gap-2">
            <button className={buttonSecondary} disabled={!suggestion.canAccept} onClick={onAccept}>{t("organizeAcceptSuggestion")}</button>
            <button className={buttonSecondary} disabled={suggestion.decision === "blocked"} onClick={onKeep}>{t("organizeKeepOriginal")}</button>
            <button className={buttonSecondary} disabled={!suggestion.canEdit} onClick={onEdit}>{t("organizeEditTargetName")}</button>
            {suggestion.decision === "accepted" || suggestion.decision === "kept" || suggestion.decision === "edited" ? <button className={cn(buttonGhost, "px-2")} onClick={onClear}>{t("organizeClearDecision")}</button> : null}
          </div>
        </div>

        <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]">{t("organizeKeyboardHints")}</p>

        <details className="border-t border-[var(--zc-divider)] pt-3 text-sm">
          <summary className="cursor-pointer font-medium text-[var(--zc-text-secondary)]">{t("organizeAdvancedAnalysis")}</summary>
          <dl className="mt-3 grid gap-3">
            <InspectorField label={t("confidence")} value={`${Math.round(file.confidence * 100)}%`} />
            <InspectorField label={t("organizeMatchedRules")} value={userFacingMatchedRules(suggestion, t)} />
            <InspectorField label={t("organizeContextSignal")} value={userFacingContext(suggestion, t)} />
            <InspectorField label={t("organizeTechnicalBasis")} value={userFacingTechnicalBasis(suggestion, t)} />
            <InspectorField label={t("organizeExecutionRecheck")} value={t("organizeExecutionRecheckDesc")} />
          </dl>
        </details>
      </div>
    </aside>
  );
}

function userFacingReason(suggestion: OrganizeSuggestion, t: Translator) {
  if (suggestion.file.is_duplicate) return t("organizeDuplicateReviewDesc");
  if (suggestion.file.risk_level === "Sensitive") return t("organizeSensitiveReviewDesc");
  if (suggestion.file.confidence < 0.7) return t("organizeLowConfidenceDesc");
  if (suggestion.preview) return t("organizeReasonFromAnalysis");
  return t("organizeReasonUnavailable");
}

function userFacingTechnicalBasis(suggestion: OrganizeSuggestion, t: Translator) {
  const basis = suggestion.file.classification_reason || suggestion.preview?.reason || "";
  if (!basis) return t("unknown");
  return /\b(?:browser|qa)\s+mock\b/i.test(basis) ? t("organizeTechnicalBasisAvailable") : basis;
}

function userFacingMatchedRules(suggestion: OrganizeSuggestion, t: Translator) {
  const rules = suggestion.file.matched_rules.filter((rule) => !/\bmock\b/i.test(rule));
  if (rules.length) return rules.join(", ");
  return suggestion.file.matched_rules.length ? t("organizeAnalysisRulesAvailable") : t("unknown");
}

function userFacingContext(suggestion: OrganizeSuggestion, t: Translator) {
  const context = suggestion.file.context;
  if (!context) return t("unknown");
  return /\bmock\b/i.test(context) ? t("organizeContextAvailable") : context;
}

function InspectorField({ label, value, tone = "normal" }: { label: string; value: string; tone?: "normal" | "warning" }) {
  return <div className="min-w-0"><dt className="text-xs font-semibold text-[var(--zc-text-tertiary)]">{label}</dt><dd className={cn("mt-0.5 break-words text-sm", tone === "warning" ? "text-[var(--zc-warning-text)]" : "text-[var(--zc-text-primary)]")} title={value}>{compactPath(value, 92)}</dd></div>;
}

function operationLabel(type: NonNullable<OrganizeSuggestion["preview"]>["operation_type"], t: Translator) {
  if (type === "rename") return t("operationRename");
  if (type === "move_rename") return t("operationMoveRename");
  if (type === "move_to_trash") return t("operationMoveToTrash");
  if (type === "copy") return t("operationCopy");
  if (type === "duplicate") return t("operationDuplicate");
  if (type === "replace") return t("operationReplace");
  if (type === "permanent_delete") return t("operationPermanentDelete");
  if (type === "create_directory") return t("operationCreateDirectory");
  return t("operationMove");
}

function organizeBlockingText(suggestion: OrganizeSuggestion, t: Translator) {
  const { file, preview } = suggestion;
  if (file.is_deleted || file.is_stale) return t("organizeSourceUnavailable");
  if (preview?.is_executable === false) return t("organizeOperationBlockedDesc");
  if (file.risk_level === "Sensitive" || file.lifecycle === "Sensitive") return t("organizeSensitiveReviewDesc");
  if (file.is_duplicate) return t("organizeDuplicateReviewDesc");
  if (file.confidence < 0.7) return t("organizeLowConfidenceDesc");
  if (file.requires_confirmation || preview?.requires_confirmation) return t("organizeNeedsReviewDesc");
  if (!preview) return t("organizeNoExecutablePreviewDesc");
  if (preview.will_create_parent) return t("organizeWillCreateParentDesc");
  return "";
}
