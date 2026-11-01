export const QA_FILENAME_CANDIDATE_FEATURE =
  "global-search-filename-relevance-candidate";

export function resolveTauriBuildFeatures(env = process.env) {
  const features = ["desktop-runtime"];
  if (env.QA_ONLY_PACKAGE_MODE !== "true") return features;
  if (env.GITHUB_EVENT_NAME !== "workflow_dispatch") {
    throw new Error("Candidate filename search may only be packaged in an explicit QA workflow dispatch.");
  }
  features.push(QA_FILENAME_CANDIDATE_FEATURE);
  return features;
}

