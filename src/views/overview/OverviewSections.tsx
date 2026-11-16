import { AlertTriangle, BrainCircuit, ChevronRight, Clock3, FileCog, FolderSync, History, type LucideIcon } from "lucide-react";
import type { Language } from "../../i18n";
import type { Translator } from "../../types/ui";
import { formatDate } from "../../utils/format";
import { buttonGhost, cn, focusVisibleState } from "../../utils/tw";
import { compactPath, formatDisplayPath } from "../../utils/viewHelpers";
import type { OverviewActivity, OverviewBackgroundTask } from "./overviewModel";

export interface OverviewSystemCoverageModel {
  search: "ready" | "partial" | "attention" | "unknown";
  managedCount: number;
  managedTotal: number;
  managedAttention: number;
  contentKnown: boolean;
  contentEnabled: number;
  contentTotal: number;
}

export function OverviewSystemCoverage({ coverage, t }: { coverage: OverviewSystemCoverageModel; t: Translator }) {
  const searchLabel = coverage.search === "ready"
    ? t("overviewSystemSearchReady")
    : coverage.search === "partial"
      ? t("overviewSystemSearchPartial")
      : coverage.search === "attention"
        ? t("overviewSystemSearchAttention")
        : t("overviewSystemUnknown");
  return (
    <section className="v340-section" aria-labelledby="overview-system-coverage-title">
      <div className="v340-section-head">
        <h2 id="overview-system-coverage-title" className="v340-section-title">{t("overviewSystemCoverage")}</h2>
      </div>
      <div className="v340-panel v340-fact-list">
        <div className="v340-fact-row">
          <span className="v340-fact-label">{t("overviewSystemSearch")}</span>
          <span className="v340-fact-value">{searchLabel}</span>
        </div>
        <div className="v340-fact-row">
          <span className="v340-fact-label">{t("overviewSystemManaged")}</span>
          <span className="v340-fact-value">
            {coverage.managedCount} / {coverage.managedTotal}
            <span className="v340-fact-hint">{t("overviewSystemManagedHint").replace("{count}", coverage.managedCount.toLocaleString())}</span>
          </span>
        </div>
        <div className="v340-fact-row">
          <span className="v340-fact-label">{t("overviewSystemContent")}</span>
          <span className="v340-fact-value">
            {coverage.contentKnown ? `${coverage.contentEnabled} / ${coverage.contentTotal}` : t("overviewSystemUnknown")}
            {coverage.contentKnown ? <span className="v340-fact-hint">{t("overviewSystemContentHint").replace("{enabled}", coverage.contentEnabled.toLocaleString()).replace("{total}", coverage.contentTotal.toLocaleString())}</span> : null}
          </span>
        </div>
      </div>
      {coverage.managedAttention > 0 ? <p className="text-xs leading-5 text-[var(--zc-warning-text)]">{t("overviewTaskManagedRootDesc").replace("{count}", coverage.managedAttention.toLocaleString())}</p> : null}
    </section>
  );
}

export function OverviewSpaceSummary({ summary, t }: { summary: string; t: Translator }) {
  return (
    <section className="v340-section" aria-labelledby="overview-summary-title">
      <div className="v340-section-head">
        <h2 id="overview-summary-title" className="v340-section-title">{t("overviewSpaceSummary")}</h2>
      </div>
      <p className="v340-panel v340-panel-pad min-w-0 max-w-full text-sm leading-6 text-[var(--zc-text-secondary)] [overflow-wrap:anywhere]">{summary}</p>
    </section>
  );
}

export interface OverviewQuickAction {
  id: string;
  label: string;
  description: string;
  icon: LucideIcon;
  onClick: () => void;
  disabled?: boolean;
}

export function OverviewQuickActions({ actions, t }: { actions: readonly OverviewQuickAction[]; t: Translator }) {
  return (
    <section className="v340-section" aria-labelledby="overview-next-steps-title" data-overview-next-steps="true">
      <div className="v340-section-head">
        <h2 id="overview-next-steps-title" className="v340-section-title">{t("overviewNextSteps")}</h2>
        <span className="v340-section-hint">{t("overviewNextStepsHint")}</span>
      </div>
      <div className="v340-panel v340-task-list">
        {actions.map((action) => {
          const Icon = action.icon;
          return (
            <button
              key={action.id}
              type="button"
              className="v340-task-row"
              onClick={action.onClick}
              disabled={action.disabled}
              data-overview-quick-action={action.id}
            >
              <span className="v340-task-icon" aria-hidden="true"><Icon size={16} /></span>
              <span className="v340-task-text">
                <strong>{action.label}</strong>
                <span title={action.description}>{action.description}</span>
              </span>
              <ChevronRight size={15} className="v340-task-chevron" aria-hidden="true" />
            </button>
          );
        })}
      </div>
    </section>
  );
}

export function OverviewRecentActivityList({ activities, t, language, onOpenHistory }: { activities: OverviewActivity[]; t: Translator; language?: Language; onOpenHistory?: () => void }) {
  if (activities.length === 0 && !onOpenHistory) return null;
  return (
    <section className="v340-section" aria-labelledby="overview-activity-title">
      <div className="v340-section-head">
        <h2 id="overview-activity-title" className="v340-section-title">{t("overviewRecentActivity")}</h2>
        {onOpenHistory ? <button type="button" className={buttonGhost} onClick={onOpenHistory}>{t("overviewReviewHistory")}<ChevronRight size={15} aria-hidden="true" /></button> : null}
      </div>
      {activities.length > 0 ? (
        <div className="v340-panel divide-y divide-[var(--zc-divider)]">
          {activities.map((activity) => (
            <div key={activity.id} className="flex min-w-0 items-center gap-3 py-3">
              <span className={cn(
                "grid h-8 w-8 shrink-0 place-items-center rounded-[var(--zc-radius-control)]",
                activity.status === "failed" || activity.status === "manual_review" ? "bg-[var(--zc-danger-soft)] text-[var(--zc-danger-text)]" : "bg-[var(--zc-neutral-soft)] text-[var(--zc-neutral-text)]"
              )} aria-hidden="true">
                {activity.status === "failed" || activity.status === "manual_review" ? <AlertTriangle size={16} /> : <History size={16} />}
              </span>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium text-[var(--zc-text-primary)]">{activity.title}</p>
                {activity.description ? <p className="truncate text-xs text-[var(--zc-text-secondary)]">{activity.description}</p> : null}
              </div>
              <time className="shrink-0 text-xs text-[var(--zc-text-tertiary)]" dateTime={activity.createdAt}>{formatDate(activity.createdAt, language)}</time>
            </div>
          ))}
        </div>
      ) : (
        <div className="v340-panel v340-panel-pad flex flex-wrap items-center justify-between gap-3">
          <p className="text-sm text-[var(--zc-text-secondary)]">{t("overviewActivityEmpty")}</p>
          {onOpenHistory ? <button type="button" className={buttonGhost} onClick={onOpenHistory}>{t("overviewReviewHistory")}<ChevronRight size={15} aria-hidden="true" /></button> : null}
        </div>
      )}
    </section>
  );
}

export function OverviewBackgroundTaskList({
  tasks,
  t,
  onRetryIndex
}: {
  tasks: OverviewBackgroundTask[];
  t: Translator;
  onRetryIndex: (path: string) => void;
}) {
  if (tasks.length === 0) return null;
  return (
    <section className="v340-section" aria-labelledby="overview-background-title">
      <div className="v340-section-head">
        <h2 id="overview-background-title" className="v340-section-title">{t("overviewBackgroundTasks")}</h2>
      </div>
      <div className="v340-panel divide-y divide-[var(--zc-divider)]">
        {tasks.map((task, index) => {
          const presentation = backgroundPresentation(task, t);
          const Icon = presentation.icon;
          return (
            <div className="flex min-w-0 items-center gap-3 py-3" key={`${task.kind}-${task.currentPath ?? index}`} role="status">
              <Icon size={17} className={presentation.iconClass} aria-hidden="true" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium text-[var(--zc-text-primary)]">{presentation.title}</p>
                {presentation.description ? <p className="truncate text-xs text-[var(--zc-text-secondary)]">{presentation.description}</p> : null}
              </div>
              {task.kind === "background-failure" && task.currentPath ? (
                <button className={buttonGhost} onClick={() => onRetryIndex(task.currentPath!)}>
                  <span>{t("overviewRetryBackground")}</span>
                  <ChevronRight size={15} />
                </button>
              ) : null}
            </div>
          );
        })}
      </div>
    </section>
  );
}

function backgroundPresentation(task: OverviewBackgroundTask, t: Translator) {
  const path = task.currentPath ? compactPath(formatDisplayPath(task.currentPath), 64) : "";
  if (task.kind === "background-index") return {
    icon: FolderSync,
    iconClass: "animate-pulse text-[var(--zc-info-text)] motion-reduce:animate-none",
    title: t("overviewBackgroundIndexing"),
    description: [path, task.pending ? t("overviewBackgroundIndexQueue").replace("{count}", task.pending.toLocaleString()) : ""].filter(Boolean).join(" · ")
  };
  if (task.kind === "operation") return {
    icon: FileCog,
    iconClass: "text-[var(--zc-info-text)]",
    title: t("overviewBackgroundOperation"),
    description: [path, typeof task.processed === "number" && typeof task.total === "number" ? `${task.processed.toLocaleString()} / ${task.total.toLocaleString()}` : ""].filter(Boolean).join(" · ")
  };
  if (task.kind === "ai") return {
    icon: BrainCircuit,
    iconClass: "text-[var(--zc-info-text)]",
    title: t("overviewBackgroundAI"),
    description: [path, typeof task.processed === "number" && typeof task.total === "number" ? `${task.processed.toLocaleString()} / ${task.total.toLocaleString()}` : ""].filter(Boolean).join(" · ")
  };
  return {
    icon: AlertTriangle,
    iconClass: "text-[var(--zc-danger-text)]",
    title: t("overviewBackgroundFailure"),
    description: [path, task.message].filter(Boolean).join(" · ")
  };
}
