export type CiScope = {
  docs_only: boolean;
  windows_native_preview_handler_changed: boolean;
  windows_native_filesystem_smoke_changed: boolean;
  windows_global_index_service_qualification_changed: boolean;
  frontend_changed: boolean;
  rust_changed: boolean;
  macos_sensitive: boolean;
  performance_sensitive: boolean;
  high_risk: boolean;
  package_sensitive: boolean;
  dependency_sensitive: boolean;
  release_sensitive: boolean;
  all_domains_100k: boolean;
  workflow_changed: boolean;
  base_missing: boolean;
  full_validation: boolean;
  perf_search: boolean;
  perf_scan_schema: boolean;
  perf_library_content: boolean;
  perf_intelligence: boolean;
  perf_workspace_foundation: boolean;
  perf_preview_platform: boolean;
  performance_any: boolean;
  diff_head: string;
  diff_base: string;
};

export function classifyCiScope(args?: {
  event?: string;
  changedPaths?: readonly string[];
  baseMissing?: boolean;
  dispatchFull?: boolean;
  prLabels?: readonly string[];
}): CiScope;
