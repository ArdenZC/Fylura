import { invokeCommand } from "./core";
import type {
  CreateLibrarySavedViewRequest,
  CreateUserTagRequest,
  DashboardStats,
  DeleteLibrarySavedViewRequest,
  DeleteUserTagRequest,
  FileLibraryDetail,
  FileLibraryFilters,
  FileLibrarySelectionSummary,
  FileQueryRequestV2,
  FileQueryResponseV2,
  FileQueryResult,
  FileRecord,
  FileLibraryScopeV2,
  LibrarySavedView,
  LibraryScope,
  LibrarySelectionV1,
  MutateFileUserTagsRequest,
  MutateFileUserTagsResult,
  ResolveFileLibraryExactCountRequestV2,
  ResolveFileLibraryExactCountResponseV2,
  UpdateLibrarySavedViewRequest,
  UpdateUserTagRequest,
  UserTag
} from "../types/domain";

export const libraryApi = {
  personalMetadata(request: import("../types/domain").FilePersonalMetadataRequest): Promise<import("../types/domain").FilePersonalMetadataResponse> {
    return invokeCommand("file_library_personal_metadata", { request });
  },
  getPagedFiles(limit = 50, offset = 0, query?: string, scope?: LibraryScope, filters?: FileLibraryFilters): Promise<FileQueryResult> {
    const normalizedQuery = query?.trim();
    return invokeCommand<FileQueryResult>("get_paged_files", {
      limit,
      offset,
      query: normalizedQuery ? normalizedQuery : null,
      scope: scope ?? null,
      filter: filters ?? null
    });
  },
  queryFileLibraryV2(request: FileQueryRequestV2): Promise<FileQueryResponseV2> {
    return invokeCommand<FileQueryResponseV2>("query_file_library_v2", { request });
  },
  resolveFileLibraryExactCountV2(request: ResolveFileLibraryExactCountRequestV2): Promise<ResolveFileLibraryExactCountResponseV2> {
    return invokeCommand<ResolveFileLibraryExactCountResponseV2>("resolve_file_library_exact_count_v2", { request });
  },
  getFileLibraryDetail(fileId: string): Promise<FileLibraryDetail> {
    return invokeCommand<FileLibraryDetail>("get_file_library_detail", { fileId });
  },
  getFileLibrarySelectionSummary(selection: LibrarySelectionV1): Promise<FileLibrarySelectionSummary> {
    return invokeCommand<FileLibrarySelectionSummary>("get_file_library_selection_summary", { selection });
  },
  revealFileLibraryEntry(fileId: string): Promise<void> {
    return invokeCommand<void>("reveal_file_library_entry", { fileId });
  },
  requestMacosThumbnail(fileId: string, size = 512, requestId = createQuickLookRequestId()): Promise<string> {
    return invokeCommand<string>("request_macos_thumbnail", { fileId, size, requestId });
  },
  cancelMacosThumbnail(requestId: string): Promise<boolean> {
    return invokeCommand<boolean>("cancel_macos_thumbnail", { requestId });
  },
  listUserTags(): Promise<UserTag[]> {
    return invokeCommand<UserTag[]>("list_user_tags");
  },
  createUserTag(request: CreateUserTagRequest): Promise<UserTag> {
    return invokeCommand<UserTag>("create_user_tag", { request });
  },
  updateUserTag(request: UpdateUserTagRequest): Promise<UserTag> {
    return invokeCommand<UserTag>("update_user_tag", { request });
  },
  deleteUserTag(request: DeleteUserTagRequest): Promise<boolean> {
    return invokeCommand<boolean>("delete_user_tag", { request });
  },
  mutateFileUserTags(request: MutateFileUserTagsRequest): Promise<MutateFileUserTagsResult> {
    return invokeCommand<MutateFileUserTagsResult>("mutate_file_user_tags", { request });
  },
  listLibrarySavedViews(): Promise<LibrarySavedView[]> {
    return invokeCommand<LibrarySavedView[]>("list_library_saved_views");
  },
  createLibrarySavedView(request: CreateLibrarySavedViewRequest): Promise<LibrarySavedView> {
    return invokeCommand<LibrarySavedView>("create_library_saved_view", { request });
  },
  updateLibrarySavedView(request: UpdateLibrarySavedViewRequest): Promise<LibrarySavedView> {
    return invokeCommand<LibrarySavedView>("update_library_saved_view", { request });
  },
  deleteLibrarySavedView(request: DeleteLibrarySavedViewRequest): Promise<boolean> {
    return invokeCommand<boolean>("delete_library_saved_view", { request });
  },
  getStatsSummary(scope?: LibraryScope): Promise<DashboardStats> {
    return invokeCommand<DashboardStats>("get_stats_summary", { scope: scope ?? null });
  },
  searchFiles(query: string, limit = 12, scope?: LibraryScope): Promise<FileRecord[]> {
    return invokeCommand<FileRecord[]>("search_files", { query, limit, scope: scope ?? null });
  },
  initDatabase(): Promise<void> {
    return invokeCommand<void>("init_db");
  }
};

function createQuickLookRequestId(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") return crypto.randomUUID();
  return `quick-look-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export type LibraryApi = typeof libraryApi;
