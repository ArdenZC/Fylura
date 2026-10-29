import type { IndexCoverageRequest, IndexCoverageResponse, IndexCoverageSnapshot } from "../types/indexCoverage";
let snapshot: IndexCoverageSnapshot = { policy: { version: 1, revision: 1, mode: "smart_lean", rules: [] }, sources: [] };
export function mockIndexCoverage(raw: unknown): IndexCoverageResponse {
  const request = raw as IndexCoverageRequest;
  if (request.action !== "read" && request.expectedRevision !== snapshot.policy.revision) throw new Error("coverage_revision_conflict");
  if (request.action === "save") snapshot = { policy: { version: 1, revision: snapshot.policy.revision + 1, mode: request.mode!, rules: structuredClone(request.rules ?? []) }, sources: [] };
  return { snapshot: structuredClone(snapshot), preview: request.action === "preview" ? { sampled: 0, included: 0, excluded: 0, undetermined: 0, sampleLimit: 1000, isComplete: false } : null };
}
