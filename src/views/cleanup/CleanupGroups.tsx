import type { AnalysisFindingGroup, AnalysisRun } from "../../types/domain";
import type { Translator } from "../../types/ui";
import { formatBytes } from "../../utils/format";
import { resolveReclaimableBytes } from "../../utils/reclaimableBytes";
import { cn } from "../../utils/tw";
import { findingCategoryLabel, tierLabel } from "./FindingRow";
import type { CleanupTier } from "./cleanupModel";

const tiers: CleanupTier[] = ["safe", "review", "caution"];
export function CleanupGroups({ run, groups, active, disabled, onChange, t, inline = false }: {
  run: AnalysisRun; groups: AnalysisFindingGroup[]; active: CleanupTier; disabled: boolean; onChange: (tier: CleanupTier) => void; t: Translator;
  inline?: boolean;
}) {
  return <div role="group" aria-label={t("storageCleanupFindingTabsLabel")} className={cn("v340-cleanup-tier-filter", inline && "v340-cleanup-tier-filter-inline")} data-cleanup-groups>
    {tiers.map((tier, index) => {
      const group = groups.find((item) => item.tier === tier);
      const count = group ? group.count - group.excludedCount : (tier === "safe" ? run.safeCount : tier === "review" ? run.reviewCount : run.cautionCount);
      const reclaim = group ? resolveReclaimableBytes({ exact: group.exactReclaimableBytes, potential: group.potentialReclaimableBytes }) : null;
      const categories = group?.categories.slice(0, 2).map((item) => `${findingCategoryLabel(item.category, t)} ${item.count}`).join(" · ");
      const ariaLabel = t("storageCleanupTierTab").replace("{label}", tierLabel(tier, t)).replace("{count}", String(count));
      return <button key={tier} type="button" disabled={disabled} aria-pressed={active === tier} data-cleanup-group-tier={tier}
        title={categories || undefined}
        aria-label={categories ? `${ariaLabel} · ${categories}` : ariaLabel}
        className={cn("v340-cleanup-tier-option min-w-0 text-left disabled:opacity-50", active === tier && "is-active")}
        onClick={() => onChange(tier)} onKeyDown={(event) => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          const next = tiers[(index + (event.key === "ArrowRight" ? 1 : 2)) % tiers.length];
          onChange(next);
          event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`[data-cleanup-group-tier="${next}"]`)?.focus();
        }}>
        <span className="truncate text-sm font-semibold">{t(tier === "safe" ? "cleanupGroupSafe" : tier === "review" ? "cleanupGroupReview" : "cleanupGroupProtected")} · {count}</span>
        <span className="shrink-0 truncate text-xs text-[var(--zc-text-secondary)]">{tier === "caution" ? t("cleanupNeverExecute") : reclaim ? `${reclaim.estimated ? "≈ " : ""}${formatBytes(reclaim.bytes)}` : t("cleanupGroupBytesUnknown")}{group?.excludedCount ? ` · ${t("cleanupExcludedCount").replace("{count}", String(group.excludedCount))}` : ""}</span>
      </button>;
    })}
  </div>;
}
