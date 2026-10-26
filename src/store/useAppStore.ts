import { create } from "zustand";
import type { Language } from "../i18n";
import type { AutomationSurface, Density, ThemeMode, View } from "../types/ui";
import { preferredLanguage, preferredTheme } from "../utils/uiPreferences";
import { initialViewFromSearch, normalizeViewInput } from "../utils/viewRoutes";

export type ToastState = { message: string; type: "success" | "error" | "info" };

interface AppStore {
  language: Language;
  theme: ThemeMode;
  density: Density;
  sidebarHidden: boolean;
  view: View;
  automationSurface: AutomationSurface;
  searchQuery: string;
  globalHotkeyError: string;
  toast: ToastState | null;
  setLanguage: (language: Language) => void;
  setTheme: (theme: ThemeMode) => void;
  setDensity: (density: Density) => void;
  setSidebarHidden: (hidden: boolean) => void;
  setView: (view: View) => void;
  setAutomationSurface: (surface: AutomationSurface) => void;
  setSearchQuery: (searchQuery: string) => void;
  setGlobalHotkeyError: (message: string) => void;
  showToast: (toast: ToastState) => void;
  showSuccess: (message: string) => void;
  showError: (message: string) => void;
  clearToast: () => void;
}

export const useAppStore = create<AppStore>((set) => ({
  language: preferredLanguage(),
  theme: preferredTheme(),
  density: preferredDensity(),
  sidebarHidden: preferredSidebarHidden(),
  view: initialMainView(),
  automationSurface: "intents",
  searchQuery: "",
  globalHotkeyError: "",
  toast: null,
  setLanguage: (language) => {
    set({ language });
    try { window.localStorage.setItem("zc-language", language); } catch { /* optional preference */ }
  },
  setTheme: (theme) => {
    set({ theme });
    try { window.localStorage.setItem("zc-theme", theme); } catch { /* optional preference */ }
  },
  setDensity: (density) => {
    set({ density });
    try { window.localStorage.setItem("zc-density", density); } catch { /* optional preference */ }
  },
  setSidebarHidden: (sidebarHidden) => {
    set({ sidebarHidden });
    try { window.localStorage.setItem("zc-sidebar-hidden", String(sidebarHidden)); } catch { /* optional preference */ }
  },
  setView: (view) => set((state) => {
    const canonicalView = normalizeViewInput(view) ?? "scanner";
    return canonicalView === "automation"
      ? { view: canonicalView, automationSurface: "intents" }
      : { view: canonicalView, automationSurface: state.automationSurface };
  }),
  setAutomationSurface: (automationSurface) => set({ automationSurface }),
  setSearchQuery: (searchQuery) => set({ searchQuery }),
  setGlobalHotkeyError: (globalHotkeyError) => set({ globalHotkeyError }),
  showToast: (toast) => set({ toast }),
  showSuccess: (message) => set({ toast: { message, type: "success" } }),
  showError: (message) => set({ toast: { message, type: "error" } }),
  clearToast: () => set({ toast: null })
}));

function initialMainView(): View {
  return initialViewFromSearch(typeof window === "undefined" ? "" : window.location.search);
}

function preferredDensity(): Density {
  try {
    return window.localStorage.getItem("zc-density") === "compact" ? "compact" : "default";
  } catch {
    return "default";
  }
}

function preferredSidebarHidden(): boolean {
  try { return window.localStorage.getItem("zc-sidebar-hidden") === "true"; }
  catch { return false; }
}
