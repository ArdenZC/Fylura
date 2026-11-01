import { memo } from "react";
import { motion } from "motion/react";
import { Download, File } from "lucide-react";
import type { OperationPreview } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { percent } from "../../utils/format";
import { buttonSecondary, cn, inputSurface } from "../../utils/tw";
import { compactPath, formatDisplayPath, formatPreviewDisplayPath } from "../../utils/viewHelpers";
import { normalizeProposedFileNameExtension } from "../../utils/fileNaming";
import { ToneBadge, itemMotion } from "../shared/ui";
import { validateOrganizeFileName } from "../organize/organizeModel";
import { riskLabel } from "../fileLibrary/presentation/fileLibraryPresentation";
import { resolvePreviewEligibility, type PreviewExecutionIntent } from "../../store/useOperationQueueStore";

export const PreviewFileRow = memo(function PreviewFileRow({
  preview,
  isSelected,
  executionIntent,
  toggle,
  onRenamePreview,
  onMaterialize,
  busy,
  t
}: {
  preview: OperationPreview;
  isSelected: boolean;
  executionIntent?: PreviewExecutionIntent;
  toggle: (id: string) => void;
  onRenamePreview: (id: string, name: string) => void;
  onMaterialize?: (preview: OperationPreview) => Promise<void>;
  busy?: boolean;
  t: Translator;
}) {
  const trashOperation = preview.operation_type === "move_to_trash";
  const syntaxError = trashOperation ? null : validateOrganizeFileName(preview.new_name);
  const extensionNormalization = trashOperation ? null : normalizeProposedFileNameExtension(preview.old_name, preview.new_name);
  const nameError = syntaxError ?? extensionNormalization?.error ?? null;
  const eligibility = resolvePreviewEligibility(preview, executionIntent ?? null);
  const blocked = eligibility.reason === "blocked" || eligibility.reason === "unavailable" || eligibility.reason === "outsideWhitelist";
  const executionStatus = eligibility.executable ? "executable" : eligibility.reason === "invalidName" ? "invalid-name" : eligibility.reason === "unavailable" ? "unavailable" : eligibility.reason === "outsideWhitelist" ? "outside-whitelist" : "blocked";
  const executionStatusLabel = eligibility.executable ? t("operationExecutable") : eligibility.reason === "invalidName" ? t("operationInvalidName") : eligibility.reason === "unavailable" ? t("unavailable") : eligibility.reason === "outsideWhitelist" ? t("organizeOutsideWhitelist") : t("operationBlocked");

  return (
    <motion.div
      className={cn(
        "grid items-start gap-3 border-b border-[var(--zc-divider)] px-2 py-3 transition-[background,color] sm:grid-cols-[auto_auto_minmax(0,1fr)]",
        isSelected && "bg-[var(--zc-surface-selected)]",
        blocked && "opacity-65"
      )}
      layout={false}
      variants={itemMotion}
    >
      <input
        type="checkbox"
        disabled={Boolean(busy) || (!isSelected && !eligibility.executable)}
        checked={isSelected}
        onChange={() => toggle(preview.id)}
        aria-label={`${t("selectOperation")} · ${preview.old_name}`}
      />
      <File size={15} />
      <div className="min-w-0">
        <div className="grid gap-1 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start">
          <div className="min-w-0">
            <strong className="block truncate text-sm">{preview.old_name}</strong>
            <span className="block text-xs text-[var(--zc-text-secondary)]">
              {operationLabel(preview.operation_type, t)} / {percent(preview.confidence)}
            </span>
          </div>
          <span role="status" aria-label={executionStatusLabel} data-preview-execution-state={executionStatus}>
            <ToneBadge tone={blocked ? "danger" : nameError ? "warning" : "success"}>
              {executionStatusLabel}
            </ToneBadge>
          </span>
        </div>

        <div className="mt-2 flex flex-wrap gap-1 text-[11px]">
          <ToneBadge tone={riskTone(preview.risk_level)}>{riskLabel(preview.risk_level, t)}</ToneBadge>
          {preview.requires_confirmation && (
            <ToneBadge tone="warning">{t("operationNeedsConfirmation")}</ToneBadge>
          )}
          {preview.blocking_reason ? <ToneBadge tone="danger">{t("operationBlocked")}</ToneBadge> : null}
          {!trashOperation && preview.will_create_parent && (
            <ToneBadge tone="info">{t("operationCreatesParent")}</ToneBadge>
          )}
          {preview.strategy && <ToneBadge tone="info">{t("operationStrategy")}: {strategyLabel(preview.strategy, t)}</ToneBadge>}
          {preview.conflict_policy && <ToneBadge tone="slate">{t("operationConflictPolicy")}: {conflictPolicyLabel(preview.conflict_policy, t)}</ToneBadge>}
          {preview.will_copy && <ToneBadge tone="info">{t("operationWillCopy")}</ToneBadge>}
          {preview.will_move && <ToneBadge tone="info">{t("operationWillMove")}</ToneBadge>}
          {preview.will_download && <ToneBadge tone="warning">{t("operationWillDownload")}</ToneBadge>}
          {preview.materialization_requirement && preview.materialization_requirement !== "none" && (
            <ToneBadge tone={preview.materialization_requirement === "required" || preview.materialization_requirement === "explicit_download_required" || preview.materialization_requirement === "unknown" ? "warning" : "info"}>
              {t("operationMaterialization")}: {materializationLabel(preview.materialization_requirement, t)}
            </ToneBadge>
          )}
          {(preview.materialization_requirement === "explicit_download_required" || preview.materialization_requirement === "required") && preview.operationFingerprint && onMaterialize ? (
            <button
              type="button"
              className={cn(buttonSecondary, "min-h-7 px-2 py-1 text-[11px]")}
              disabled={busy}
              onClick={() => void onMaterialize(preview)}
              aria-label={`${t("operationMaterializationDownload")} · ${preview.old_name}`}
            >
              <Download size={13} />
              {t("operationMaterializationDownload")}
            </button>
          ) : null}
          {preview.will_replace && <ToneBadge tone="warning">{t("operationWillReplace")}</ToneBadge>}
          {preview.will_trash && <ToneBadge tone="warning">{t("operationWillTrash")}</ToneBadge>}
          {preview.metadataDegradationPossible && <ToneBadge tone="warning">{t("operationMetadataDegradation")}</ToneBadge>}
          {preview.sourceRetirementEligible === false && <ToneBadge tone="danger">{t("operationSourceRetirementBlocked")}</ToneBadge>}
        </div>
        {trashOperation && (
          <p className="mt-2 text-xs text-[var(--zc-text-secondary)]">{t("operationMoveToTrashRisk")}</p>
        )}
        {preview.reason ? <p className="mt-2 text-xs text-[var(--zc-text-secondary)]">{t("organizeReasonFromAnalysis")}</p> : null}

        <div className="mt-2 grid min-w-0 gap-2 xl:grid-cols-2">
          <PathBlock label={t("sourcePath")} path={preview.source_path} tone="source" t={t} />
          <PathBlock label={t("targetPath")} path={preview.target_path} tone="target" t={t} localizeLogicalPath />
        </div>

        {!trashOperation && (
          <input
            className={cn(inputSurface, "mt-2 w-full")}
            value={preview.new_name}
            disabled={Boolean(busy) || !preview.editable_new_name || eligibility.reason === "unavailable" || eligibility.reason === "outsideWhitelist"}
            onChange={(event) => onRenamePreview(preview.id, event.target.value)}
            aria-label={t("newFileName")}
            aria-invalid={Boolean(nameError)}
            aria-describedby={nameError ? `preview-name-error-${preview.id}` : undefined}
            onBlur={() => {
              if (!syntaxError && extensionNormalization && extensionNormalization.error === null && extensionNormalization.name !== preview.new_name) {
                onRenamePreview(preview.id, extensionNormalization.name);
              }
            }}
          />
        )}
        {nameError ? <p id={`preview-name-error-${preview.id}`} className="mt-1 text-xs text-[var(--zc-danger-text)]" role="alert">{t(nameError === "empty" ? "organizeNameErrorEmpty" : nameError === "reserved" ? "organizeNameErrorReserved" : nameError === "extension" ? "organizeNameErrorExtension" : "organizeNameErrorUnsafe")}</p> : null}
      </div>
    </motion.div>
  );
});

function PathBlock({ label, path, tone, t, localizeLogicalPath = false }: { label: string; path: string; tone: "source" | "target"; t: Translator; localizeLogicalPath?: boolean }) {
  const displayPath = localizeLogicalPath ? formatPreviewDisplayPath(path, t) : formatDisplayPath(path);
  return (
    <div
      className={cn(
        "min-w-0 rounded-lg border px-2 py-1.5",
        tone === "source"
          ? "border-[var(--zc-divider)] bg-[var(--zc-neutral-soft)]"
          : "border-[var(--zc-info-border)] bg-[var(--zc-info-soft)]"
      )}
      title={displayPath}
    >
      <span className="block text-[10px] font-semibold uppercase tracking-[0.12em] text-[var(--zc-text-tertiary)]">{label}</span>
      <code className="block min-w-0 truncate whitespace-nowrap text-[11px] leading-5 text-[var(--zc-text-secondary)]">
        {compactPath(displayPath, 78)}
      </code>
    </div>
  );
}

function operationLabel(operation: OperationPreview["operation_type"], t: Translator): string {
  if (operation === "move_to_trash") return t("operationMoveToTrash");
  if (operation === "rename") return t("operationRename");
  if (operation === "move_rename") return t("operationMoveRename");
  if (operation === "copy") return t("operationCopy");
  if (operation === "duplicate") return t("operationDuplicate");
  if (operation === "replace") return t("operationReplace");
  if (operation === "permanent_delete") return t("operationPermanentDelete");
  if (operation === "create_directory") return t("operationCreateDirectory");
  return t("operationMove");
}

function strategyLabel(strategy: string, t: Translator): string {
  if (strategy === "local_apfs") return t("operationStrategyLocalApfs");
  if (strategy === "local_portable") return t("operationStrategyLocalPortable");
  if (strategy === "cross_volume_copy_verify") return t("operationStrategyCrossVolume");
  if (strategy === "network_portable") return t("operationStrategyNetwork");
  if (strategy === "icloud_coordinated") return t("operationStrategyICloud");
  if (strategy === "file_provider_coordinated") return t("operationStrategyFileProvider");
  if (strategy === "backend_resolves_at_confirmation") return t("operationStrategyBackendResolved");
  return t("operationStrategyPlatformDefault");
}

function materializationLabel(requirement: string, t: Translator): string {
  if (requirement === "metadata_only") return t("operationMaterializationMetadataOnly");
  if (requirement === "required" || requirement === "explicit_download_required") return t("operationMaterializationRequired");
  if (requirement === "provider_managed") return t("operationMaterializationProviderManaged");
  if (requirement === "unknown") return t("operationMaterializationUnknown");
  return t("operationMaterializationNone");
}

function conflictPolicyLabel(policy: string, t: Translator): string {
  if (policy === "no_overwrite") return t("operationConflictNoOverwrite");
  if (policy === "replace_with_recovery_backup") return t("operationConflictReplaceBackup");
  if (policy === "safe_trash_recoverable") return t("operationConflictSafeTrash");
  if (policy === "permanent_delete_quarantine") return t("operationConflictPermanentDelete");
  return t("operationConflictExclusive");
}

function riskTone(risk: OperationPreview["risk_level"]): "info" | "success" | "warning" | "danger" | "slate" {
  if (risk === "Sensitive") return "danger";
  if (risk === "System") return "warning";
  if (risk === "Normal") return "success";
  return "slate";
}
