import type { IndexCoverageRequest, IndexCoverageResponse } from "../types/indexCoverage";
import { invokeCommand } from "./core";
import type {
  AddManagedScopeRequest,
  AiManagementStatus,
  GlobalIndexSource,
  GlobalIndexStatus,
  GlobalSearchRequest,
  GlobalSearchResponse,
  ManagedScope,
  UpdateManagedScopePolicyRequest
} from "../types/domain";

export const globalSearchApi = {
  globalIndexCoverage(request: IndexCoverageRequest): Promise<IndexCoverageResponse> {
    return invokeCommand<IndexCoverageResponse>("global_index_coverage", { request });
  },
  searchGlobalEntries(request: GlobalSearchRequest): Promise<GlobalSearchResponse> {
    return invokeCommand<GlobalSearchResponse>("search_global_entries", { request });
  },
  getGlobalIndexStatus(): Promise<GlobalIndexStatus> {
    return invokeCommand<GlobalIndexStatus>("get_global_index_status");
  },
  listGlobalIndexSources(): Promise<GlobalIndexSource[]> {
    return invokeCommand<GlobalIndexSource[]>("list_global_index_sources");
  },
  startGlobalIndex(): Promise<void> {
    return invokeCommand<void>("start_global_index");
  },
  pauseGlobalIndex(): Promise<void> {
    return invokeCommand<void>("pause_global_index");
  },
  resumeGlobalIndex(): Promise<void> {
    return invokeCommand<void>("resume_global_index");
  },
  rebuildGlobalIndexSource(sourceId?: string): Promise<void> {
    return invokeCommand<void>("rebuild_global_index_source", { sourceId: sourceId ?? null });
  },
  setGlobalIndexSourceEnabled(sourceId: string, enabled: boolean): Promise<void> {
    return invokeCommand<void>("set_global_index_source_enabled", { sourceId, enabled });
  },
  authorizeGlobalIndexSource(sourceId: string, confirmed: boolean): Promise<void> {
    return invokeCommand<void>("authorize_global_index_source", { sourceId, confirmed });
  },
  openGlobalSearchResult(entryId: string): Promise<void> {
    return invokeCommand<void>("open_global_search_result", { entryId });
  },
  revealGlobalSearchResult(entryId: string): Promise<void> {
    return invokeCommand<void>("reveal_global_search_result", { entryId });
  },
  listManagedScopes(): Promise<ManagedScope[]> {
    return invokeCommand<ManagedScope[]>("list_managed_scopes");
  },
  addManagedScope(request: AddManagedScopeRequest): Promise<ManagedScope> {
    return invokeCommand<ManagedScope>("add_managed_scope", { request });
  },
  removeManagedScope(id: string): Promise<boolean> {
    return invokeCommand<boolean>("remove_managed_scope", { id });
  },
  updateManagedScopePolicy(request: UpdateManagedScopePolicyRequest): Promise<ManagedScope> {
    return invokeCommand<ManagedScope>("update_managed_scope_policy", { request });
  },
  getAiManagementStatus(): Promise<AiManagementStatus> {
    return invokeCommand<AiManagementStatus>("get_ai_management_status");
  }
};

export type GlobalSearchApi = typeof globalSearchApi;
