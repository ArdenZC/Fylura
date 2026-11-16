import { AlertTriangle, ArrowRight, CheckCircle2, FolderSearch, LoaderCircle, RefreshCw, ScanSearch, Sparkles, X } from "lucide-react";
import type { Translator } from "../../types/ui";
import { buttonSecondary, cn, buttonPrimary, buttonGhost } from "../../utils/tw";
import { formatBytes } from "../../utils/format";
import type { OverviewPriorityTaskModel } from "./overviewModel";

export function OverviewPriorityTask({
  task,
  t,
  onPrimary,
  onChooseFolder,
  onCancel,
  metrics = []
}: {
  task: OverviewPriorityTaskModel;
  t: Translator;
  onPrimary: () => void;
  onChooseFolder: () => void;
  onCancel: () => void;
  metrics?: readonly OverviewMetric[];
}) {
  const content = priorityContent(task, t);
  const showChooseFolder = task.kind === "unindexed"
    || task.kind === "update"
    || task.kind === "scan-failed"
    || task.kind === "scan-permission"
    || task.kind === "scan-canceled";
  return (
    <section className="v340-status-band" aria-labelledby="overview-priority-title" data-overview-status-strip="true">
      <div className="v340-status-cell">
        <span className="v340-status-label">{task.kind === "orderly" ? t("overviewCurrentStatusLabel") : t("overviewPriorityLabel")}</span>
        <div className="v340-status-strong">
          <span className="v340-dot" data-tone={content.tone} aria-hidden="true" />
          <h2 id="overview-priority-title" title={content.title}>{content.title}</h2>
        </div>
        <span className="v340-status-meta v340-status-meta-clamp [overflow-wrap:anywhere]" title={content.description}>{content.description}</span>
      </div>
      {metrics.map((metric) => (
        <div key={metric.label} className="v340-status-cell">
          <span className="v340-status-label">{metric.label}</span>
          <strong className="v340-status-value" title={metric.value}>{metric.value}</strong>
          {metric.hint ? <span className="v340-status-meta" title={metric.hint}>{metric.hint}</span> : null}
        </div>
      ))}
      <div className="v340-status-actions">
        <button data-overview-primary="true" className={buttonPrimary} onClick={onPrimary}>
          <span>{content.primaryLabel}</span>
          <ArrowRight size={17} />
        </button>
        {showChooseFolder ? (
          <button className={buttonSecondary} onClick={onChooseFolder}>
            <FolderSearch size={17} />
            <span>{t("overviewChooseFolder")}</span>
          </button>
        ) : null}
        {task.kind === "scan-active" ? (
          <button className={buttonGhost} onClick={onCancel}>
            <X size={17} />
            <span>{t("cancelScan")}</span>
          </button>
        ) : null}
      </div>
    </section>
  );
}

export interface OverviewMetric {
  label: string;
  value: string;
  hint?: string;
}

function priorityContent(task: OverviewPriorityTaskModel, t: Translator) {
  if (task.kind === "search-permission") return {
    icon: AlertTriangle,
    iconClass: "bg-[var(--zc-danger-soft)] text-[var(--zc-danger-text)]",
    tone: "danger",
    title: task.reason === "no_source" ? t("overviewTaskSearchNoSource") : t("overviewTaskSearchPermission"),
    description: task.reason === "no_source"
      ? t("overviewTaskSearchNoSourceDesc")
      : task.error || t("overviewTaskSearchPermissionDesc"),
    primaryLabel: t("overviewReviewSearchSources")
  };
  if (task.kind === "operation") return {
    icon: task.reason === "failed" ? AlertTriangle : LoaderCircle,
    iconClass: task.reason === "failed" ? "bg-[var(--zc-danger-soft)] text-[var(--zc-danger-text)]" : "bg-[var(--zc-info-soft)] text-[var(--zc-info-text)]",
    title: task.reason === "failed" ? t("overviewTaskOperationAttention") : t("overviewTaskOperationRunning"),
    description: task.reason === "failed" ? t("overviewTaskOperationAttentionDesc") : t("overviewTaskOperationRunningDesc"),
    primaryLabel: t("overviewReviewHistory")
  };
  if (task.kind === "content-failure") return {
    icon: AlertTriangle,
    iconClass: "bg-[var(--zc-warning-soft)] text-[var(--zc-warning-text)]",
    tone: "warning",
    title: t("overviewTaskContentFailure"),
    description: task.error || t("overviewTaskContentFailureDesc"),
    primaryLabel: t("overviewReviewContent")
  };
  if (task.kind === "managed-root-stale") return {
    icon: RefreshCw,
    iconClass: "bg-[var(--zc-warning-soft)] text-[var(--zc-warning-text)]",
    tone: "warning",
    title: task.reason === "permission"
      ? t("overviewTaskManagedRootPermission")
      : task.reason === "retry_exhausted"
        ? t("overviewTaskManagedRootRetryExhausted")
        : task.reason === "reconciliation"
          ? t("overviewTaskManagedRootReconciliation")
          : task.reason === "partial"
            ? t("overviewTaskManagedRootPartial")
            : t("overviewTaskManagedRootStale"),
    description: (task.reason === "permission"
      ? t("overviewTaskManagedRootPermissionDesc")
      : task.reason === "retry_exhausted"
        ? t("overviewTaskManagedRootRetryExhaustedDesc")
        : task.reason === "reconciliation"
          ? t("overviewTaskManagedRootReconciliationDesc")
          : task.reason === "partial"
            ? t("overviewTaskManagedRootPartialDesc")
            : t("overviewTaskManagedRootStaleDesc")).replace("{count}", (task.count ?? 0).toLocaleString()),
    primaryLabel: task.reason === "permission"
      ? t("overviewTaskManagedRootPermissionAction")
      : task.reason === "retry_exhausted"
        ? t("overviewTaskManagedRootRetryExhaustedAction")
        : task.reason === "reconciliation"
          ? t("overviewTaskManagedRootReconciliationAction")
          : task.reason === "partial"
            ? t("overviewTaskManagedRootPartialAction")
            : t("overviewTaskManagedRootStaleAction")
  };
  if (task.kind === "scan-permission") return {
    icon: AlertTriangle,
    iconClass: "bg-[var(--zc-danger-soft)] text-[var(--zc-danger-text)]",
    tone: "danger",
    title: t("overviewTaskPermission"),
    description: task.error || t("overviewTaskPermissionDesc"),
    primaryLabel: t("overviewRetryScan")
  };
  if (task.kind === "scan-failed") return {
    icon: AlertTriangle,
    iconClass: "bg-[var(--zc-danger-soft)] text-[var(--zc-danger-text)]",
    tone: "danger",
    title: t("overviewTaskScanFailed"),
    description: task.error || t("overviewTaskScanFailedDesc"),
    primaryLabel: t("overviewRetryScan")
  };
  if (task.kind === "scan-active") return {
    icon: LoaderCircle,
    iconClass: "bg-[var(--zc-info-soft)] text-[var(--zc-info-text)]",
    tone: "info",
    title: t("overviewTaskScanning"),
    description: t("overviewTaskScanningDesc")
      .replace("{count}", (task.fileCount ?? 0).toLocaleString())
      .replace("{path}", task.path || t("overviewCurrentLocation")),
    primaryLabel: t("overviewViewProgress")
  };
  if (task.kind === "scan-canceling") return {
    icon: LoaderCircle,
    iconClass: "bg-[var(--zc-warning-soft)] text-[var(--zc-warning-text)]",
    tone: "warning",
    title: t("overviewTaskCanceling"),
    description: t("overviewTaskCancelingDesc").replace("{count}", (task.fileCount ?? 0).toLocaleString()),
    primaryLabel: t("overviewViewProgress")
  };
  if (task.kind === "scan-canceled") return {
    icon: X,
    iconClass: "bg-[var(--zc-neutral-soft)] text-[var(--zc-neutral-text)]",
    tone: "neutral",
    title: t("overviewTaskCanceled"),
    description: t("overviewTaskCanceledDesc").replace("{count}", (task.fileCount ?? 0).toLocaleString()),
    primaryLabel: t("overviewRetryScan")
  };
  if (task.kind === "scan-partial") return {
    icon: AlertTriangle,
    iconClass: "bg-[var(--zc-warning-soft)] text-[var(--zc-warning-text)]",
    tone: "warning",
    title: t("overviewTaskPartial"),
    description: t("overviewTaskPartialDesc")
      .replace("{files}", (task.fileCount ?? 0).toLocaleString())
      .replace("{errors}", (task.count ?? 0).toLocaleString()),
    primaryLabel: t("overviewViewFailures")
  };
  if (task.kind === "review") return {
    icon: Sparkles,
    iconClass: "bg-[var(--zc-warning-soft)] text-[var(--zc-warning-text)]",
    tone: "warning",
    title: t("overviewTaskReview").replace("{count}", (task.count ?? 0).toLocaleString()),
    description: t("overviewTaskReviewDesc"),
    primaryLabel: t("overviewViewSuggestions")
  };
  if (task.kind === "cleanup") return {
    icon: ScanSearch,
    iconClass: "bg-[var(--zc-success-soft)] text-[var(--zc-success-text)]",
    tone: "success",
    title: t("overviewTaskCleanup").replace("{count}", (task.count ?? 0).toLocaleString()),
    description: (task.bytesAreEstimated ? t("overviewTaskCleanupEstimatedDesc") : t("overviewTaskCleanupDesc")).replace("{size}", formatBytes(task.bytes ?? 0)),
    primaryLabel: t("overviewViewCleanup")
  };
  if (task.kind === "unindexed") return {
    icon: ScanSearch,
    iconClass: "bg-[var(--zc-info-soft)] text-[var(--zc-info-text)]",
    tone: "info",
    title: t("overviewTaskUnindexed"),
    description: t("overviewTaskUnindexedDesc"),
    primaryLabel: t("overviewStartScan")
  };
  if (task.kind === "update") return {
    icon: RefreshCw,
    iconClass: "bg-[var(--zc-info-soft)] text-[var(--zc-info-text)]",
    tone: "info",
    title: t("overviewTaskUpdate"),
    description: t("overviewTaskUpdateDesc"),
    primaryLabel: t("overviewCheckUpdates")
  };
  return {
    icon: CheckCircle2,
    iconClass: "bg-[var(--zc-success-soft)] text-[var(--zc-success-text)]",
    tone: "success",
    title: t("overviewTaskOrderly"),
    description: t("overviewTaskOrderlyDesc").replace("{count}", (task.fileCount ?? 0).toLocaleString()),
    primaryLabel: t("overviewScanNewLocation")
  };
}
