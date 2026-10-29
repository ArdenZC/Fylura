import { useEffect, useRef, useState } from "react";
import { tauriApi } from "../../../api/tauriApi";
import type { AiManagementStatus, GlobalIndexSource, GlobalIndexStatus, ManagedScope } from "../../../types/domain";
import type { Translator } from "../../../types/ui";
import { committedLifecycleRecoveryFailure, localizedStableError } from "../../../utils/viewHelpers";

type StatusTone = "success" | "warning";
type SettingsStatus = (message: string, tone?: StatusTone) => void;
export type GlobalIndexLoadState = "loading" | "loaded" | "error";

export function useSettingsGlobalIndexController({ t, showStatus }: { t: Translator; showStatus: SettingsStatus }) {
  const [globalIndexStatus, setGlobalIndexStatus] = useState<GlobalIndexStatus | null>(null);
  const [globalIndexSources, setGlobalIndexSources] = useState<GlobalIndexSource[]>([]);
  const [managedScopes, setManagedScopes] = useState<ManagedScope[]>([]);
  const [aiManagementStatus, setAiManagementStatus] = useState<AiManagementStatus | null>(null);
  const [managedScopePath, setManagedScopePath] = useState("");
  const [isLoadingGlobalIndex, setIsLoadingGlobalIndex] = useState(true);
  const [globalIndexLoadState, setGlobalIndexLoadState] = useState<GlobalIndexLoadState>("loading");
  const [globalIndexLoadError, setGlobalIndexLoadError] = useState<string | null>(null);
  const [isUpdatingGlobalIndex, setIsUpdatingGlobalIndex] = useState(false);
  const translatorRef = useRef(t);
  const showStatusRef = useRef(showStatus);
  translatorRef.current = t;
  showStatusRef.current = showStatus;

  async function refreshGlobalIndexData() {
    try {
      const [status, sources, scopes, aiStatus] = await Promise.all([
        tauriApi.getGlobalIndexStatus(),
        tauriApi.listGlobalIndexSources(),
        tauriApi.listManagedScopes(),
        tauriApi.getAiManagementStatus()
      ]);
      setGlobalIndexStatus(status);
      setGlobalIndexSources(sources);
      setManagedScopes(scopes);
      setAiManagementStatus(aiStatus);
      setGlobalIndexLoadError(null);
      setGlobalIndexLoadState("loaded");
    } catch (error) {
      setGlobalIndexLoadError(localizedStableError(error, translatorRef.current));
      setGlobalIndexLoadState("error");
      throw error;
    }
  }

  useEffect(() => {
    let disposed = false;
    setIsLoadingGlobalIndex(true);
    setGlobalIndexLoadState("loading");
    void refreshGlobalIndexData().catch((error) => {
      if (!disposed) {
        showStatusRef.current(`${translatorRef.current("globalIndexLoadFailed")}：${localizedStableError(error, translatorRef.current)}`, "warning");
      }
    }).finally(() => {
      if (!disposed) setIsLoadingGlobalIndex(false);
    });
    return () => {
      disposed = true;
    };
  }, []);

  async function runGlobalIndexAction(action: () => Promise<void>, successMessage: string) {
    if (isUpdatingGlobalIndex) return;
    setIsUpdatingGlobalIndex(true);
    try {
      await action();
      await refreshGlobalIndexData();
      showStatusRef.current(successMessage);
    } catch (error) {
      const recoveryFailure = committedLifecycleRecoveryFailure(error);
      if (recoveryFailure) {
        await refreshGlobalIndexData().catch(() => undefined);
        showStatusRef.current(
          `${translatorRef.current("globalIndexChangeCommittedRecoveryPending")} ${recoveryFailure}`,
          "warning"
        );
        return;
      }
      showStatusRef.current(`${translatorRef.current("globalIndexActionFailed")}：${localizedStableError(error, translatorRef.current)}`, "warning");
    } finally {
      setIsUpdatingGlobalIndex(false);
    }
  }

  async function addManagedScopeFromSettings() {
    const path = managedScopePath.trim();
    if (!path || isUpdatingGlobalIndex) return;
    setIsUpdatingGlobalIndex(true);
    try {
      await tauriApi.addManagedScope({ path, enabled: true, allowLocalAi: true, allowCloudAi: false });
      setManagedScopePath("");
      await refreshGlobalIndexData();
      showStatusRef.current(translatorRef.current("managedScopeAdded"));
    } catch (error) {
      showStatusRef.current(`${translatorRef.current("managedScopeActionFailed")}：${localizedStableError(error, translatorRef.current)}`, "warning");
    } finally {
      setIsUpdatingGlobalIndex(false);
    }
  }

  async function updateManagedScope(scope: ManagedScope, patch: { enabled?: boolean; allowLocalAi?: boolean; allowCloudAi?: boolean }) {
    if (isUpdatingGlobalIndex) return;
    setIsUpdatingGlobalIndex(true);
    try {
      await tauriApi.updateManagedScopePolicy({ id: scope.id, ...patch });
      await refreshGlobalIndexData();
      showStatusRef.current(translatorRef.current("settingsSavedInline"));
    } catch (error) {
      showStatusRef.current(`${translatorRef.current("managedScopeActionFailed")}：${localizedStableError(error, translatorRef.current)}`, "warning");
    } finally {
      setIsUpdatingGlobalIndex(false);
    }
  }

  async function removeManagedScope(scope: ManagedScope) {
    if (isUpdatingGlobalIndex) return;
    setIsUpdatingGlobalIndex(true);
    try {
      await tauriApi.removeManagedScope(scope.id);
      await refreshGlobalIndexData();
      showStatusRef.current(translatorRef.current("settingsSavedInline"));
    } catch (error) {
      showStatusRef.current(`${translatorRef.current("managedScopeActionFailed")}：${localizedStableError(error, translatorRef.current)}`, "warning");
    } finally {
      setIsUpdatingGlobalIndex(false);
    }
  }

  return {
    globalIndexStatus,
    globalIndexSources,
    globalIndexLoadState,
    globalIndexLoadError,
    managedScopes,
    aiManagementStatus,
    managedScopePath,
    setManagedScopePath,
    isLoadingGlobalIndex,
    isUpdatingGlobalIndex,
    refreshGlobalIndexData,
    runGlobalIndexAction,
    addManagedScopeFromSettings,
    updateManagedScope,
    removeManagedScope
  };
}
