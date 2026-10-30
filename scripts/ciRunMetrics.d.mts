export interface CiJob {
  name?: string;
  conclusion?: string;
  startedAt?: string;
  completedAt?: string;
}
export interface CiRun {
  databaseId?: number;
  status?: string;
  conclusion?: string;
  jobs?: CiJob[];
}
export interface CiSummary {
  total: number;
  outcomes: { success: number; failure: number; cancelled: number; other: number };
  slowJobs: { name: string; minutes: number; runId?: number }[];
  failures: [string, number][];
}
export function parseCacheLog(log: string): Record<string, string>;
export function summarizeCiRuns(runs: CiRun[]): CiSummary;
export function formatCiMetrics(summary: CiSummary, cache?: Record<string, string> | null): string;
export function runCiMetrics(args?: string[]): number;
