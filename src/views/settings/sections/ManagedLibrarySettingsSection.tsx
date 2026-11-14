import { FolderPlus, Trash2 } from "lucide-react";
import type { AiManagementStatus, ManagedScope } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { buttonIconDanger, buttonSecondary, cn } from "../../../utils/tw";
import { compactPath } from "../../../utils/viewHelpers";
import { compactInteractiveRow } from "../../shared/ui";
import { SettingsEmptyState, SettingsInlineMessage, SettingsSection, SettingsSwitchControl, SettingsTextField } from "../components/SettingsPrimitives";

export interface ManagedLibrarySettingsSectionProps {
  t: Translator;
  scopes: ManagedScope[];
  aiManagementStatus: AiManagementStatus | null;
  managedScopePath: string;
  onManagedScopePath: (value: string) => void;
  isUpdating: boolean;
  policyText: (policySummary: string | undefined) => string;
  onAdd: () => void;
  onUpdate: (scope: ManagedScope, patch: { enabled?: boolean; allowLocalAi?: boolean; allowCloudAi?: boolean }) => void;
  onRemove: (scope: ManagedScope) => void;
}

export function ManagedLibrarySettingsSection({
  t,
  scopes,
  aiManagementStatus,
  managedScopePath,
  onManagedScopePath,
  isUpdating,
  policyText,
  onAdd,
  onUpdate,
  onRemove
}: ManagedLibrarySettingsSectionProps) {
  return (
    <SettingsSection id="settings-managed-scopes" title={t("managedScopesTitle")} description={t("managedScopesDesc")}>
      <SettingsInlineMessage tone="info" role="status">
        <span>{policyText(aiManagementStatus?.policySummary)}</span>
      </SettingsInlineMessage>
      <div className="grid gap-3 min-[841px]:grid-cols-[minmax(0,1fr)_auto] min-[841px]:items-end">
        <SettingsTextField id="managed-scope-path" label={t("managedScopeAdd")} value={managedScopePath} placeholder={t("managedScopePathPlaceholder")} onChange={onManagedScopePath} />
        <button className={buttonSecondary} onClick={onAdd} disabled={!managedScopePath.trim() || isUpdating}>
          <FolderPlus size={15} />
          <span>{t("managedScopeAdd")}</span>
        </button>
      </div>
      {scopes.length ? (
        <div className="grid gap-2">
          {scopes.map((scope) => (
            <div key={scope.id} data-settings-item className={cn(compactInteractiveRow(), "px-3 py-2")}>
              <div className="grid min-w-0 gap-3 min-[841px]:grid-cols-[minmax(0,1fr)_auto] min-[841px]:items-center">
                <div className="min-w-0 text-left">
                  <strong className="block truncate text-sm font-medium text-[var(--zc-text-primary)]">{compactPath(scope.path, 72)}</strong>
                  <span className="block truncate text-xs leading-5 text-[var(--zc-text-tertiary)]">{scope.enabled ? t("managedScopeEnabled") : t("managedScopeDisabled")}</span>
                </div>
                <div className="flex flex-wrap items-center justify-start gap-2 min-[841px]:justify-end">
                  <SettingsSwitchControl id={`managed-scope-enabled-${scope.id}`} checked={scope.enabled} label={scope.enabled ? t("managedScopeEnabled") : t("managedScopeDisabled")} onChange={(enabled) => onUpdate(scope, { enabled })} />
                  <SettingsSwitchControl id={`managed-scope-local-${scope.id}`} checked={scope.allowLocalAi} label={t("managedScopeLocalAi")} onChange={(allowLocalAi) => onUpdate(scope, { allowLocalAi })} />
                  <SettingsSwitchControl id={`managed-scope-cloud-${scope.id}`} checked={scope.allowCloudAi} label={t("managedScopeCloudAi")} onChange={(allowCloudAi) => onUpdate(scope, { allowCloudAi })} />
                  <button className={buttonIconDanger} onClick={() => onRemove(scope)} title={t("managedScopeRemove")} aria-label={t("managedScopeRemove")} disabled={isUpdating}>
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      ) : <SettingsEmptyState title={t("managedScopeNone")} description={t("managedScopeNoneDesc")} />}
    </SettingsSection>
  );
}
