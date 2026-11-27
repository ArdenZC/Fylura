import type { CloseBehavior, FolderNamingLanguage } from "../../../types/domain";
import type { Language } from "../../../i18n";
import type { Density, ThemeMode } from "../../../types/ui";
import type { Translator } from "../../../types/ui";
import {
  SettingsSection,
  SettingsSelect
} from "../components/SettingsPrimitives";

export interface GeneralSettingsSectionProps {
  t: Translator;
  closeBehavior: CloseBehavior;
  onCloseBehavior: (value: CloseBehavior) => void;
  language: Language;
  onLanguage: (value: Language) => void;
  theme: ThemeMode;
  onTheme: (value: ThemeMode) => void;
  density: Density;
  onDensity: (value: Density) => void;
  folderNamingLanguage: FolderNamingLanguage;
  onFolderNamingLanguage: (value: FolderNamingLanguage) => void;
}

export function GeneralSettingsSection({
  t,
  closeBehavior,
  onCloseBehavior,
  language,
  onLanguage,
  theme,
  onTheme,
  density,
  onDensity,
  folderNamingLanguage,
  onFolderNamingLanguage
}: GeneralSettingsSectionProps) {
  return (
    <SettingsSection id="settings-general" title={t("settingsGeneral")} description={t("settingsGeneralDesc")}>
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
    </SettingsSection>
  );
}
