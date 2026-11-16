import { useEffect, useRef, useState } from "react";
import type { Language } from "../i18n";
import type { ThemeMode } from "../types/ui";
import { formatHotkeyLabel, matchesAcceleratorEvent } from "../utils/hotkeys";
import {
  detectBrowserPlatform,
  preferredLanguage,
  preferredTheme,
  prefersDarkScheme
} from "../utils/viewHelpers";

const IS_SEARCH_MODE = new URLSearchParams(window.location.search).get("mode") === "search";

interface UseAppChromeOptions {
  theme: ThemeMode;
  setTheme: (theme: ThemeMode) => void;
  setLanguage: (language: Language) => void;
  searchHotkey: string;
}

export function useAppChrome({ theme, setTheme, setLanguage, searchHotkey }: UseAppChromeOptions) {
  const [systemDark, setSystemDark] = useState(() => prefersDarkScheme());
  const [isCommandOpen, setIsCommandOpen] = useState(false);
  const commandInputRef = useRef<HTMLInputElement | null>(null);
  const platform = detectBrowserPlatform();
  const isWindows = platform === "win32";
  const effectiveTheme: Exclude<ThemeMode, "system"> = theme === "system" ? (systemDark ? "dark" : "light") : theme;
  const hotkeyLabel = formatHotkeyLabel(searchHotkey, platform);

  useEffect(() => {
    document.documentElement.classList.toggle("search-window-root", IS_SEARCH_MODE);
    document.body.classList.toggle("search-window-root", IS_SEARCH_MODE);
    return () => {
      document.documentElement.classList.remove("search-window-root");
      document.body.classList.remove("search-window-root");
    };
  }, []);

  useEffect(() => {
    const mediaQuery = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!mediaQuery) return;
    const handleChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    setSystemDark(mediaQuery.matches);
    mediaQuery.addEventListener("change", handleChange);
    return () => mediaQuery.removeEventListener("change", handleChange);
  }, []);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", effectiveTheme === "dark");
    try { window.localStorage.setItem("zc-theme", theme); } catch { /* optional preference */ }
  }, [effectiveTheme, theme]);

  useEffect(() => {
    const handleStorage = (event: StorageEvent) => {
      if (!event.key || event.key === "zc-theme") setTheme(preferredTheme());
      if (!event.key || event.key === "zc-language" || event.key === "fma-language") {
        setLanguage(preferredLanguage());
      }
    };
    window.addEventListener("storage", handleStorage);
    return () => window.removeEventListener("storage", handleStorage);
  }, [setLanguage, setTheme]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (matchesAcceleratorEvent(event, searchHotkey, platform)) {
        event.preventDefault();
        setIsCommandOpen(true);
      }
      // The command modal owns Escape while it is mounted so its 130ms exit
      // and ModalPortal focus restoration can complete.
      if (event.key === "Escape" && !document.querySelector("[data-command-modal]")) setIsCommandOpen(false);
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [platform, searchHotkey]);

  useEffect(() => {
    if (isCommandOpen) window.setTimeout(() => commandInputRef.current?.focus(), 40);
  }, [isCommandOpen]);

  useEffect(() => {
    if (!IS_SEARCH_MODE) return;
    const focusSearchInput = () => {
      setIsCommandOpen(true);
      window.setTimeout(() => commandInputRef.current?.focus(), 40);
    };
    window.addEventListener("focus", focusSearchInput);
    return () => window.removeEventListener("focus", focusSearchInput);
  }, []);

  useEffect(() => {
    if (IS_SEARCH_MODE) {
      setTheme(preferredTheme());
      setLanguage(preferredLanguage());
      setIsCommandOpen(true);
    }
  }, [setLanguage, setTheme]);

  return {
    commandInputRef,
    isCommandOpen,
    setIsCommandOpen,
    platform,
    isWindows,
    effectiveTheme,
    hotkeyLabel,
    isSearchMode: IS_SEARCH_MODE
  };
}
