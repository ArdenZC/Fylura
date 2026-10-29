export interface PreflightOptions {
  planOnly: boolean;
  full: boolean;
  rustTest: string;
  base: string;
}
export interface PreflightCommand {
  label: string;
  command: string;
  args: string[];
}
export interface PreflightPlan {
  scope: {
    docs_only: boolean;
    frontend_changed: boolean;
    rust_changed: boolean;
    high_risk: boolean;
    [key: string]: unknown;
  };
  commands: PreflightCommand[];
  warnings: string[];
}
export function parseOptions(args: string[]): PreflightOptions;
export function changedPathsForPreflight(baseRef?: string): string[];
export function buildPreflightPlan(paths: string[], options?: Partial<PreflightOptions>): PreflightPlan;
export function runPreflight(args?: string[]): number;
