export interface FailureStep {
  name?: string;
  conclusion?: string;
}
export interface FailureJob {
  name?: string;
  conclusion?: string;
  steps?: FailureStep[];
}
export interface FailureRun {
  workflowName?: string;
  name?: string;
  conclusion?: string;
  status?: string;
  url?: string;
  headSha?: string;
  jobs?: FailureJob[];
}
export function formatFailureDigest(run: FailureRun): string;
export function runFailureDigest(args?: string[]): number;
