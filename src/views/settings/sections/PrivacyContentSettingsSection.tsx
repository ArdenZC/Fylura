import type { RestoreRetentionDays } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { quietText } from "../../shared/ui";
import { SettingsSection, SettingsSelect } from "../components/SettingsPrimitives";

export interface PrivacyContentSettingsSectionProps {
  t: Translator;
  restoreRetentionDays: RestoreRetentionDays;
  onRestoreRetentionDays: (value: RestoreRetentionDays) => void;
}

export function PrivacyContentSettingsSection({ t, restoreRetentionDays, onRestoreRetentionDays }: PrivacyContentSettingsSectionProps) {
  return (
    <SettingsSection id="settings-privacy" title={t("settingsPrivacy")} description={t("settingsPrivacyDesc")}>
      <p className={quietText}>{t("privacyLine")}</p>
      <SettingsSelect
        id="settings-log-retention"
        label={t("logRetention")}
        description={t("logRetentionDesc")}
        value={String(restoreRetentionDays)}
        options={([15, 30, 60, 90] as RestoreRetentionDays[]).map((days) => ({ value: String(days), label: `${days} ${t("days")}` }))}
        onChange={(next: string) => onRestoreRetentionDays(Number(next) as RestoreRetentionDays)}
      />
      <p className={quietText}>{t("settingsSafetyRestoreDesc")}</p>
    </SettingsSection>
  );
}
