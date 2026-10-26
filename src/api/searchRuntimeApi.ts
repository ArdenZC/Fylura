import type { UnlistenFn } from "@tauri-apps/api/event";
import type { GlobalSearchRequest, GlobalSearchResponse } from "../types/domain";
import type { View } from "../types/ui";
import type { SearchSettingsTarget } from "../utils/searchNavigation";
import { invokeCommand, listenTo, type EventHandler } from "./core";
import type { SearchWindowSnapshot } from "./types";

/** The standalone Search WebView's deliberately narrow IPC surface. */
export const searchRuntimeApi = {
  searchGlobalEntries(request: GlobalSearchRequest): Promise<GlobalSearchResponse> {
    return invokeCommand<GlobalSearchResponse>("search_global_entries", { request });
  },
  openGlobalSearchResult(entryId: string): Promise<void> {
    return invokeCommand<void>("open_global_search_result", { entryId });
  },
  revealGlobalSearchResult(entryId: string): Promise<void> {
    return invokeCommand<void>("reveal_global_search_result", { entryId });
  },
  activateSearchResult(
    view: View,
    fileId: string | null,
    snapshot?: Pick<SearchWindowSnapshot, "sessionId" | "revision">,
    settingsTarget?: SearchSettingsTarget | null,
    globalEntryId?: string
  ): Promise<void> {
    return invokeCommand<void>("activate_search_result", {
      request: {
        sessionId: snapshot?.sessionId ?? null,
        expectedRevision: snapshot?.revision ?? null,
        view,
        fileId,
        settingsTarget: settingsTarget ?? null,
        ...(globalEntryId === undefined ? {} : { globalEntryId })
      }
    });
  },
  getSearchWindowState(): Promise<SearchWindowSnapshot> {
    return invokeCommand<SearchWindowSnapshot>("get_search_window_state");
  },
  searchWindowReady(snapshot: SearchWindowSnapshot): Promise<SearchWindowSnapshot> {
    return invokeCommand<SearchWindowSnapshot>("search_window_ready", {
      request: { sessionId: snapshot.sessionId, expectedRevision: snapshot.revision }
    });
  },
  resizeSearchWindow(snapshot: SearchWindowSnapshot, expanded: boolean): Promise<SearchWindowSnapshot> {
    return invokeCommand<SearchWindowSnapshot>("resize_search_window", {
      request: { sessionId: snapshot.sessionId, expectedRevision: snapshot.revision, expanded }
    });
  },
  hideSearchWindow(snapshot: SearchWindowSnapshot): Promise<SearchWindowSnapshot> {
    return invokeCommand<SearchWindowSnapshot>("hide_search_window_command", {
      request: { sessionId: snapshot.sessionId, expectedRevision: snapshot.revision }
    });
  },
  onSearchWindowState(handler: EventHandler<SearchWindowSnapshot>): Promise<UnlistenFn> {
    return listenTo("search-window-state", handler);
  }
};

export type SearchRuntimeApi = typeof searchRuntimeApi;
