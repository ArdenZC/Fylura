export type PerformanceProfile = "full" | "extended";
export type PerformanceSuite = "search" | "scan-schema" | "library-content" | "intelligence" | "workspace-foundation" | "preview-platform";
export type PerformanceTargetKey = "lib" | "fts" | "migrations" | "fileLibrary" | "fixtureBuilder";
export const PERFORMANCE_BUILD_FEATURES: "performance-test-tauri,global-search-windows-release-performance-contract,global-search-filename-relevance-candidate,global-search-filename-relevance-windows-default-on";
export const PREVIEW_PERFORMANCE_CONTRACT: Readonly<{
  metricDefinition: string;
  phaseBMetricDefinition: string;
  fixtureManifest: string;
  shellFirstVisibleTargetP95Ms: number;
  usefulRepresentationTargetP95Ms: number;
  nativeUsefulRepresentationTargetP95Ms: number;
  rapidSwitchEntries: number;
  warmupSamples: number;
  timingSamples: number;
}>;
export const PREVIEW_FIXTURES: readonly Readonly<{
  id: string;
  fileName: string;
  providerId: string;
  representationFamily: string;
  fixtureClass: string;
}>[];
export type PerformanceTarget = {
  id: string;
  cargoArgs: readonly string[];
  executableStem: string;
  shardTarget: boolean;
};
export type PerformancePrecompileTarget = {
  id: string;
  targetKey: PerformanceTargetKey;
  targetArgs: readonly string[];
};
export type PerformanceBenchmark = {
  id: string;
  label: string;
  targetKey: PerformanceTargetKey;
  targetArgs: readonly string[];
  testName: string;
  ignored: boolean;
  testThreads?: number;
  env: Readonly<Record<string, string>>;
};

export const PERFORMANCE_SUITES: Record<string, unknown>;
export const PERFORMANCE_SUITE_NAMES: readonly PerformanceSuite[];
export const PERFORMANCE_TARGETS: Record<PerformanceTargetKey, PerformanceTarget>;
export function resolvePerformanceSuite(argv?: readonly string[]): PerformanceSuite;
export function getPerformanceBenchmarks(suite: PerformanceSuite, profile: PerformanceProfile): readonly PerformanceBenchmark[];
export function getPrecompileTargets(suite: PerformanceSuite): readonly PerformancePrecompileTarget[];
export function getPrecompileTargetsForSuites(
  suites: readonly PerformanceSuite[]
): readonly PerformancePrecompileTarget[];
export function getRequiredBinaryKeys(suite: PerformanceSuite): readonly PerformanceTargetKey[];
export function getFixtureWorkingFiles(
  suite: PerformanceSuite,
  profile: PerformanceProfile
): readonly string[];
export function getFixtureWorkingFilesForSuites(
  suites: readonly PerformanceSuite[],
  profile: PerformanceProfile
): readonly string[];
export function getFixtureKeys(suite: PerformanceSuite, profile: PerformanceProfile): readonly string[];
