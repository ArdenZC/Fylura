import type { AnalysisFindingGroup, AnalysisRun } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { formatBytes } from "../../utils/format";
import { resolveReclaimableBytes } from "../../utils/reclaimableBytes";
import { cn } from "../../utils/tw";
import { findingCategoryLabel, tierLabel } from "./FindingRow";
import type { CleanupTier } from "./cleanupModel";

const tiers: CleanupTier[] = ["safe", "review", "caution"];
export function CleanupGroups({ run, groups, active, disabled, onChange, t }: {
  run: AnalysisRun; groups: AnalysisFindingGroup[]; active: CleanupTier; disabled: boolean; onChange: (tier: CleanupTier) => void; t: Translator;
}) {
  return <div role="group" aria-label={t("storageCleanupFindingTabsLabel")} className="grid grid-cols-3 gap-2" data-cleanup-groups>
    {tiers.map((tier, index) => {
      const group = groups.find((item) => item.tier === tier);
      const count = group ? group.count - group.excludedCount : (tier === "safe" ? run.safeCount : tier === "review" ? run.reviewCount : run.cautionCount);
      const reclaim = group ? resolveReclaimableBytes({ exact: group.exactReclaimableBytes, potential: group.potentialReclaimableBytes }) : null;
      const categories = group?.categories.slice(0, 2).map((item) => `${findingCategoryLabel(item.category, t)} ${item.count}`).join(" · ");
      return <button key={tier} type="button" disabled={disabled} aria-pressed={active === tier} aria-label={t("storageCleanupTierTab").replace("{label}", tierLabel(tier, t)).replace("{count}", String(count))} data-cleanup-group-tier={tier}
        className={cn("min-w-0 rounded-[var(--zc-radius-row)] border px-3 py-2 text-left disabled:opacity-50", active === tier ? "border-[var(--zc-primary)] bg-[var(--zc-surface-selected)]" : "border-[var(--zc-border)] bg-[var(--zc-surface)]")}
        onClick={() => onChange(tier)} onKeyDown={(event) => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          const next = tiers[(index + (event.key === "ArrowRight" ? 1 : 2)) % tiers.length];
          onChange(next);
          event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`[data-cleanup-group-tier="${next}"]`)?.focus();
        }}>
        <span className="block truncate text-sm font-semibold">{t(tier === "safe" ? "cleanupGroupSafe" : tier === "review" ? "cleanupGroupReview" : "cleanupGroupProtected")} · {count}</span>
        <span className="block truncate text-xs text-[var(--zc-text-secondary)]">{tier === "caution" ? t("cleanupNeverExecute") : reclaim ? `${reclaim.estimated ? "≈ " : ""}${formatBytes(reclaim.bytes)}` : t("cleanupGroupBytesUnknown")}{group?.excludedCount ? ` · ${t("cleanupExcludedCount").replace("{count}", String(group.excludedCount))}` : ""}</span>
        {categories ? <span className="block truncate text-xs text-[var(--zc-text-tertiary)]" title={categories}>{categories}</span> : null}
      </button>;
    })}
  </div>;
}
