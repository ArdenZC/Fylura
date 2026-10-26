import { invokeCommand, listenTo, type EventHandler } from "./core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import type { GlobalHotkeyErrorPayload, GlobalHotkeyStatus, MainWindowReadyRequest, SearchWindowSnapshot } from "./types";
import type { SearchNavigatePayload, SearchSettingsTarget } from "../utils/searchNavigation";
import type { View } from "../types/ui";

export interface SearchNavigationAcknowledgement {
  generation: number;
  nonce: number;
  sessionId: number | null;
  revision: number | null;
  applied: boolean;
}

export const windowApi = {
  getGlobalHotkeyStatus(): Promise<GlobalHotkeyStatus | null> {
    return invokeCommand<GlobalHotkeyStatus | null>("get_global_hotkey_status");
  },
  registerGlobalSearchHotkey(accelerator: string): Promise<GlobalHotkeyStatus> {
    return invokeCommand<GlobalHotkeyStatus>("register_global_search_hotkey", { accelerator });
  },
  quitApp(): Promise<void> {
    return invokeCommand<void>("quit_app");
  },
  enterBackground(lastView: View): Promise<void> {
    return invokeCommand<void>("enter_background", { lastView });
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
    return invokeCommand<SearchWindowSnapshot>("search_window_ready", { request: { sessionId: snapshot.sessionId, expectedRevision: snapshot.revision } });
  },
  resizeSearchWindow(snapshot: SearchWindowSnapshot, expanded: boolean): Promise<SearchWindowSnapshot> {
    return invokeCommand<SearchWindowSnapshot>("resize_search_window", { request: { sessionId: snapshot.sessionId, expectedRevision: snapshot.revision, expanded } });
  },
  hideSearchWindow(snapshot: SearchWindowSnapshot): Promise<SearchWindowSnapshot> {
    return invokeCommand<SearchWindowSnapshot>("hide_search_window_command", { request: { sessionId: snapshot.sessionId, expectedRevision: snapshot.revision } });
  },
  markMainWindowReady(ready: boolean): Promise<void> {
    return invokeCommand<void>("mark_main_window_ready", { ready });
  },
  acknowledgeMainWindowReady(nonce: number): Promise<void> {
    return invokeCommand<void>("acknowledge_main_window_ready", { nonce });
  },
  acknowledgeSearchNavigation(request: SearchNavigationAcknowledgement): Promise<void> {
    return invokeCommand<void>("acknowledge_search_navigation", { request });
  },
  onSearchNavigate(handler: EventHandler<SearchNavigatePayload>): Promise<UnlistenFn> {
    return listenTo("search-navigate", handler);
  },
  onSearchWindowState(handler: EventHandler<SearchWindowSnapshot>): Promise<UnlistenFn> {
    return listenTo("search-window-state", handler);
  },
  onMainWindowReadyRequest(handler: EventHandler<MainWindowReadyRequest>): Promise<UnlistenFn> {
    return listenTo("search-main-ready-request", handler);
  },
  onGlobalHotkeyRegistrationFailed(handler: EventHandler<GlobalHotkeyErrorPayload>): Promise<UnlistenFn> {
    return listenTo("global-hotkey-registration-failed", handler);
  },
  onMainWindowCloseRequested(handler: EventHandler<void>): Promise<UnlistenFn> {
    return listenTo("main-window-close-requested", handler);
  }
};

export type WindowApi = typeof windowApi;
