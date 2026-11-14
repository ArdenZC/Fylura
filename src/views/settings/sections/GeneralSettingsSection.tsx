import type { AISettings, CloseBehavior, FolderNamingLanguage } from "../../../types/domain";
import type { Language } from "../../../i18n";
import type { Density, ThemeMode } from "../../../types/ui";
import type { Translator } from "../../../types/ui";
import { ShieldCheck } from "lucide-react";
import { quietText } from "../../shared/ui";
import {
  SettingsControlGroup,
  SettingsInlineMessage,
  SettingsSelect,
  SettingsRow,
  SettingsSection,
  SettingsSwitch
} from "../components/SettingsPrimitives";

export interface GeneralSettingsSectionProps {
  t: Translator;
  closeBehavior: CloseBehavior;
  onCloseBehavior: (value: CloseBehavior) => void;
  backgroundIndexOnStartup: boolean;
  onBackgroundIndexOnStartup: (value: boolean) => void;
  launchAtLogin: boolean;
  onLaunchAtLogin: (value: boolean) => void;
  language: Language;
  onLanguage: (value: Language) => void;
  theme: ThemeMode;
  onTheme: (value: ThemeMode) => void;
  density: Density;
  onDensity: (value: Density) => void;
  aiSettings: Pick<AISettings, "enabled" | "provider"> | null;
  onOpenAISettings: () => void;
  folderNamingLanguage: FolderNamingLanguage;
  onFolderNamingLanguage: (value: FolderNamingLanguage) => void;
}

export function GeneralSettingsSection({
  t,
  closeBehavior,
  onCloseBehavior,
  launchAtLogin,
  onLaunchAtLogin,
  language,
  onLanguage,
  theme,
  onTheme,
  density,
  onDensity,
  aiSettings,
  onOpenAISettings,
  folderNamingLanguage,
  onFolderNamingLanguage
}: GeneralSettingsSectionProps) {
  return (
    <SettingsSection id="settings-general" title={t("settingsGeneral")} description={t("settingsGeneralDesc")}>
      <SettingsControlGroup className="zc-settings-interface-group" title={t("settingsInterface")} description={t("settingsInterfaceDesc")}>
        <SettingsSelect
          id="settings-theme"
          label={t("appearance")}
          description={t("appearanceDesc")}
          value={theme}
          options={[
            { value: "system" as const, label: t("systemTheme") },
            { value: "light" as const, label: t("lightTheme") },
            { value: "dark" as const, label: t("darkTheme") }
          ]}
          onChange={onTheme}
        />
        <SettingsSelect
          id="settings-density"
          label={t("density")}
          description={t("densityDesc")}
          value={density}
          options={[
            { value: "default" as const, label: t("densityDefault") },
            { value: "compact" as const, label: t("densityCompact") }
          ]}
          onChange={onDensity}
        />
      </SettingsControlGroup>

      <SettingsControlGroup className="zc-settings-window-group" title={t("settingsWindowBehavior")} description={t("settingsWindowBehaviorDesc")}>
        <SettingsSelect
          id="settings-close-behavior"
          label={t("closeBehavior")}
          description={t("closeBehaviorDesc")}
          value={closeBehavior}
          options={[
            { value: "ask" as const, label: t("askEveryTime") },
            { value: "minimize" as const, label: t("minimizeToTray") },
            { value: "quit" as const, label: t("quitApp") }
          ]}
          onChange={onCloseBehavior}
        />
      </SettingsControlGroup>

      <SettingsControlGroup className="zc-settings-workspace-group" title={t("settingsFilesWorkspace")} description={t("settingsScanRootsDesc")}>
        <span className="sr-only">{t("settingsStartup")}</span>
        <SettingsRow label={t("quickPreviewSetting")} description={t("quickPreviewSettingDesc")}>
          <span data-settings-capability-status className={quietText}>{t("quickPreviewSettingStatus")}</span>
        </SettingsRow>
        <SettingsSwitch
          id="settings-launch-at-login"
          label={t("launchAtLogin")}
          description={t("launchAtLoginDesc")}
          checked={launchAtLogin}
          onChange={onLaunchAtLogin}
        />
      </SettingsControlGroup>

      <SettingsControlGroup className="zc-settings-ai-group" title={t("settingsAIPrivacy")}>
        <SettingsInlineMessage tone={aiSettings?.enabled && aiSettings.provider !== "ollama" ? "info" : "warning"}>
          <div className="zc-settings-ai-notice">
            <ShieldCheck size={15} aria-hidden="true" />
            <div>
              <strong>{t(aiSettings?.enabled && aiSettings.provider !== "ollama" ? "settingsAIPrivacyCloudOn" : "settingsAIPrivacyCloudOff")}</strong>
              <p>{t(aiSettings?.enabled && aiSettings.provider !== "ollama" ? "settingsAIPrivacyCloudOnDesc" : "settingsAIPrivacyCloudOffDesc")}</p>
            </div>
          </div>
        </SettingsInlineMessage>
        <SettingsRow label={t("settingsAIPrivacyProvider")} description={t("settingsAIPrivacyProviderDesc")}>
          <button type="button" className="zc-settings-ai-configure" onClick={onOpenAISettings}>{t("settingsConfigure")}</button>
        </SettingsRow>
      </SettingsControlGroup>

      <SettingsControlGroup className="zc-settings-language-group" title={t("settingsAppearanceLanguage")} description={t("settingsAppearanceLanguageDesc")}>
        <SettingsSelect
          id="settings-language"
          label={t("language")}
          description={t("languageDesc")}
          value={language}
          options={[
            { value: "zh" as const, label: t("languageChinese") },
            { value: "en" as const, label: t("languageEnglish") }
          ]}
          onChange={onLanguage}
        />
        <SettingsSelect
          id="settings-folder-naming"
          label={t("folderNaming")}
          description={t("folderNamingDesc")}
          value={folderNamingLanguage}
          options={[
            { value: "en" as const, label: t("englishFolderNames") },
            { value: "zh" as const, label: t("chineseFolderNames") }
          ]}
          onChange={onFolderNamingLanguage}
        />
      </SettingsControlGroup>
    </SettingsSection>
  );
}
