import type { AnalysisFinding, AnalysisRun } from "../../types/domain";
import type { CleanupApi } from "./cleanupControllerTypes";
import { FINDING_PAGE_SIZE, type CleanupTier } from "./cleanupModel";

/** Read a complete existing Finding group; page samples never become a group selection. */
export async function collectCleanupGroup(api: CleanupApi, run: AnalysisRun, tier: CleanupTier, signal: AbortSignal, onProgress: (count: number) => void): Promise<AnalysisFinding[]> {
  if (!api.listAnalysisFindings || !api.getAnalysisRun) throw new Error("cleanup_group_unavailable");
  const expected = tier === "safe" ? run.safeCount : tier === "review" ? run.reviewCount : run.cautionCount;
  const findings: AnalysisFinding[] = [];
  const ids = new Set<string>();
  const cursors = new Set<string>();
  let cursor: string | null = null;
  do {
    signal.throwIfAborted();
    const page = await api.listAnalysisFindings({ runId: run.id, tier, status: "active", cursor, limit: FINDING_PAGE_SIZE });
    signal.throwIfAborted();
    if (page.runRevision != null && page.runRevision !== run.revision) throw new Error("cleanup_group_changed");
    for (const finding of page.findings) {
      if (finding.runId !== run.id || finding.tier !== tier || finding.status !== "active" || ids.has(finding.id)) throw new Error("cleanup_group_changed");
      ids.add(finding.id); findings.push(finding);
    }
    if (findings.length > expected) throw new Error("cleanup_group_changed");
    onProgress(findings.length);
    cursor = page.nextCursor;
    if (cursor) {
      if (cursors.has(cursor) || !page.findings.length) throw new Error("cleanup_group_incomplete");
      cursors.add(cursor);
    }
  } while (cursor);
  const current = await api.getAnalysisRun(run.id);
  signal.throwIfAborted();
  if (current.revision !== run.revision || findings.length !== expected) throw new Error("cleanup_group_changed");
  return findings;
}
