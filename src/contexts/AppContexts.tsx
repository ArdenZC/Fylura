import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { Language } from "../i18n";
import type { useAppChrome } from "../hooks/useAppChrome";
import type { useAppSettings } from "../hooks/useAppSettings";
import type { useWindowBehavior } from "../hooks/useWindowBehavior";
import type {
  FolderNamingLanguage,
  OrganizeRootMode,
  RestoreRetentionDays,
  ScanRootSetting,
  SearchRootSetting,
  SearchScopeMode,
  Rule,
  RuntimeCapabilities
} from "../types/domain";
import type { ThemeMode, Translator, View } from "../types/ui";

type ProviderProps<T> = {
  value: T;
  children: ReactNode;
};

export type AppSettingsContextState = Pick<
  ReturnType<typeof useAppSettings>,
  "settings" | "isLoadingSettings" | "updateSettings"
>;

export interface SettingsContextValue extends AppSettingsContextState {
  setFolderNamingLanguage: (next: FolderNamingLanguage) => Promise<boolean>;
  setDefaultScanFolders: (next: ScanRootSetting[]) => Promise<boolean>;
  setRestoreRetentionDays: (next: RestoreRetentionDays) => Promise<boolean>;
  setLaunchAtLogin: (next: boolean) => Promise<boolean>;
  setBackgroundIndexOnStartup: (next: boolean) => Promise<boolean>;
  setSearchHotkey: (next: string) => Promise<boolean>;
  setSearchScopeMode: (next: SearchScopeMode) => Promise<boolean>;
  setCustomSearchRoots: (next: SearchRootSetting[]) => Promise<boolean>;
  setOrganizeRootMode: (next: OrganizeRootMode) => Promise<boolean>;
  setOrganizeRootPath: (next?: string) => Promise<boolean>;
}

export interface RulesContextValue {
  rules: Rule[];
  saveRule: (rule: Rule) => Promise<void>;
  toggleRuleEnabled: (rule: Rule, enabled: boolean) => Promise<void>;
  deleteRule: (rule: Rule) => Promise<boolean>;
}

export interface ChromeContextValue extends ReturnType<typeof useAppChrome>, ReturnType<typeof useWindowBehavior> {
  language: Language;
  setLanguage: (language: Language) => void;
  theme: ThemeMode;
  setTheme: (theme: ThemeMode) => void;
  view: View;
  setView: (view: View) => void;
  onError: (message: string) => void;
  t: Translator;
}

export interface I18nContextValue {
  language: Language;
  setLanguage: (language: Language) => void;
  t: Translator;
}

export interface NavigationContextValue {
  view: View;
  setView: (view: View) => void;
  onError: (message: string) => void;
}

export interface CommandContextValue extends Pick<ReturnType<typeof useAppChrome>, "commandInputRef" | "isCommandOpen" | "setIsCommandOpen" | "platform" | "hotkeyLabel" | "isSearchMode"> {}

export interface WindowContextValue extends Pick<ReturnType<typeof useWindowBehavior>, "closeBehavior" | "setCloseBehavior" | "isCloseChoiceOpen" | "onCancelCloseChoice" | "handleWindowAction" | "requestClose" | "resolveCloseChoice"> {
  isWindows: boolean;
}

export interface ThemeContextValue {
  theme: ThemeMode;
  setTheme: (theme: ThemeMode) => void;
  effectiveTheme: Exclude<ThemeMode, "system">;
}

export interface RuntimeCapabilitiesContextValue {
  capabilities: RuntimeCapabilities | null;
  isLoadingCapabilities: boolean;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);
const RulesContext = createContext<RulesContextValue | null>(null);
const ChromeContext = createContext<ChromeContextValue | null>(null);
const I18nContext = createContext<I18nContextValue | null>(null);
const NavigationContext = createContext<NavigationContextValue | null>(null);
const CommandContext = createContext<CommandContextValue | null>(null);
const WindowContext = createContext<WindowContextValue | null>(null);
const ThemeContext = createContext<ThemeContextValue | null>(null);
const RuntimeCapabilitiesContext = createContext<RuntimeCapabilitiesContextValue>({
  capabilities: null,
  isLoadingCapabilities: false
});

function useRequiredContext<T>(value: T | null, hookName: string, providerName: string): T {
  if (!value) throw new Error(`${hookName} must be used within ${providerName}.`);
  return value;
}

export function SettingsProvider({ value, children }: ProviderProps<SettingsContextValue>) {
  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettingsContext() {
  return useRequiredContext(useContext(SettingsContext), "useSettingsContext", "SettingsProvider");
}

export function RulesProvider({ value, children }: ProviderProps<RulesContextValue>) {
  return <RulesContext.Provider value={value}>{children}</RulesContext.Provider>;
}

export function useRulesContext() {
  return useRequiredContext(useContext(RulesContext), "useRulesContext", "RulesProvider");
}

export function ChromeProvider({ value, children }: ProviderProps<ChromeContextValue>) {
  const i18nValue = useMemo<I18nContextValue>(() => ({
    language: value.language,
    setLanguage: value.setLanguage,
    t: value.t
  }), [value.language, value.setLanguage, value.t]);
  const navigationValue = useMemo<NavigationContextValue>(() => ({
    view: value.view,
    setView: value.setView,
    onError: value.onError
  }), [value.onError, value.setView, value.view]);
  const commandValue = useMemo<CommandContextValue>(() => ({
    commandInputRef: value.commandInputRef,
    isCommandOpen: value.isCommandOpen,
    setIsCommandOpen: value.setIsCommandOpen,
    platform: value.platform,
    hotkeyLabel: value.hotkeyLabel,
    isSearchMode: value.isSearchMode
  }), [value.commandInputRef, value.hotkeyLabel, value.isCommandOpen, value.isSearchMode, value.platform, value.setIsCommandOpen]);
  const windowValue = useMemo<WindowContextValue>(() => ({
    isWindows: value.isWindows,
    closeBehavior: value.closeBehavior,
    setCloseBehavior: value.setCloseBehavior,
    isCloseChoiceOpen: value.isCloseChoiceOpen,
    onCancelCloseChoice: value.onCancelCloseChoice,
    handleWindowAction: value.handleWindowAction,
    requestClose: value.requestClose,
    resolveCloseChoice: value.resolveCloseChoice
  }), [value.closeBehavior, value.handleWindowAction, value.isCloseChoiceOpen, value.isWindows, value.onCancelCloseChoice, value.requestClose, value.resolveCloseChoice, value.setCloseBehavior]);
  const themeValue = useMemo<ThemeContextValue>(() => ({
    theme: value.theme,
    setTheme: value.setTheme,
    effectiveTheme: value.effectiveTheme
  }), [value.effectiveTheme, value.setTheme, value.theme]);

  return (
    <I18nContext.Provider value={i18nValue}>
      <NavigationContext.Provider value={navigationValue}>
        <CommandContext.Provider value={commandValue}>
          <WindowContext.Provider value={windowValue}>
            <ThemeContext.Provider value={themeValue}>
              <ChromeContext.Provider value={value}>{children}</ChromeContext.Provider>
            </ThemeContext.Provider>
          </WindowContext.Provider>
        </CommandContext.Provider>
      </NavigationContext.Provider>
    </I18nContext.Provider>
  );
}

export function useChromeContext() {
  return useRequiredContext(useContext(ChromeContext), "useChromeContext", "ChromeProvider");
}

export function useI18nContext() {
  return useRequiredContext(useContext(I18nContext), "useI18nContext", "ChromeProvider");
}

export function useNavigationContext() {
  return useRequiredContext(useContext(NavigationContext), "useNavigationContext", "ChromeProvider");
}

export function useCommandContext() {
  return useRequiredContext(useContext(CommandContext), "useCommandContext", "ChromeProvider");
}

export function useWindowContext() {
  return useRequiredContext(useContext(WindowContext), "useWindowContext", "ChromeProvider");
}

export function useThemeContext() {
  return useRequiredContext(useContext(ThemeContext), "useThemeContext", "ChromeProvider");
}

export function RuntimeCapabilitiesProvider({ value, children }: ProviderProps<RuntimeCapabilitiesContextValue>) {
  return <RuntimeCapabilitiesContext.Provider value={value}>{children}</RuntimeCapabilitiesContext.Provider>;
}

export function useRuntimeCapabilitiesContext() {
  return useContext(RuntimeCapabilitiesContext);
}
