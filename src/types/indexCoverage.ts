export type IndexCoverageMode = "smart_lean" | "broad_coverage" | "custom";
export interface IndexCoverageRule { sourceId: string; path: string; include: boolean }
export interface IndexCoveragePolicy { version: number; revision: number; mode: IndexCoverageMode; rules: IndexCoverageRule[] }
export interface IndexCoverageSnapshot {
  policy: IndexCoveragePolicy;
  sources: { sourceId: string; enabled: boolean; status: string; hasUndetermined: boolean; pending: boolean; included: number; excluded: number; undetermined: number }[];
}
export interface IndexCoveragePreview { sampled: number; included: number; excluded: number; undetermined: number; sampleLimit: number; isComplete: boolean }
export interface IndexCoverageRequest { action: "read" | "preview" | "save"; expectedRevision?: number; mode?: IndexCoverageMode; rules?: IndexCoverageRule[] }
export interface IndexCoverageResponse { snapshot: IndexCoverageSnapshot; preview: IndexCoveragePreview | null }
