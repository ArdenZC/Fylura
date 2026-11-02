import packageInfo from "../../../../package.json";
import type { Translator } from "../../../types/ui";
import { buttonSecondary } from "../../../utils/tw";
import { requestOnboardingReopen } from "../../../components/onboardingReopen";
import { quietText } from "../../shared/ui";
import { SettingsControlGroup, SettingsDisclosure, SettingsRow, SettingsSection, SettingsSwitch } from "../components/SettingsPrimitives";

export interface AboutSettingsSectionProps {
  t: Translator;
  developerMode: boolean;
  onDeveloperMode: (value: boolean) => void;
}

export function AboutSettingsSection({ t, developerMode, onDeveloperMode }: AboutSettingsSectionProps) {
  return (
    <SettingsSection id="settings-about" title={t("settingsAbout")} description={t("settingsAboutDesc")}>
      <SettingsControlGroup title={t("aboutBuildInfo")} description={t("aboutBuildInfoDesc")}>
        <SettingsRow label={t("appName")} description={t("developerReleaseDesc")}>
          <span className="text-sm font-medium text-[var(--zc-text-primary)]">v{packageInfo.version}</span>
        </SettingsRow>
        <SettingsRow label={t("aboutProjectLink")} description={t("aboutProjectLinkDesc")}>
          <a className={buttonSecondary} href={packageInfo.homepage} target="_blank" rel="noreferrer">
            {t("aboutOpenProject")}
          </a>
        </SettingsRow>
      </SettingsControlGroup>
      <SettingsControlGroup title={t("aboutGettingStarted")} description={t("aboutGettingStartedDesc")}>
        <SettingsRow label={t("onboardingReopenIntro")} description={t("onboardingReopenIntroDesc")}>
          <button type="button" className={buttonSecondary} data-onboarding-restart onClick={requestOnboardingReopen}>
            {t("onboardingReopenIntro")}
          </button>
        </SettingsRow>
      </SettingsControlGroup>
      <SettingsDisclosure title={t("advancedSettings")} description={t("developerModeDesc")}>
        <SettingsSwitch id="settings-developer-mode" label={t("developerMode")} description={t("developerModeDesc")} checked={developerMode} onChange={onDeveloperMode} />
        {developerMode ? (
          <SettingsControlGroup title={t("searchSources")} description={t("searchSourcesDesc")}>
            <p className={quietText}>{t("localOnly")}</p>
            <div className="grid gap-1 text-sm">
              <strong className="text-[var(--zc-text-primary)]">{t("excludedDirs")}</strong>
              <span className="text-sm leading-6 text-[var(--zc-text-secondary)]">node_modules, .git, target, dist, build</span>
            </div>
          </SettingsControlGroup>
        ) : null}
      </SettingsDisclosure>
    </SettingsSection>
  );
}