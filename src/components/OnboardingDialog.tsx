import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Check, Cloud, Cpu, FolderOpen, LockKeyhole } from "lucide-react";
import { tauriApi } from "../api/tauriApi";
import { useI18nContext, useNavigationContext, useSettingsContext } from "../contexts/AppContexts";
import { upsertDefaultScanRoot } from "../hooks/useAppSettings";
import { maturityCopy } from "../i18n/maturityCopy";
import type { AIProductFeatureReadiness, AIProviderMode } from "../types/domain";
import { cn, buttonGhost, buttonSecondary, overlaySurface, buttonPrimary } from "../utils/tw";
import { BrandMark } from "./ui/BrandMark";
import { ModalPortal } from "./modal/ModalPortal";
import { openAISettingsForMode, openSettingsSection } from "../views/settings/settingsNavigation";
import { cleanupReadinessText, managedReadinessText, providerReadinessText } from "../views/shared/AIReadinessNotice";
import { isBrowserMockEnabled } from "../utils/runtimeMode";

export const ONBOARDING_STORAGE_KEY = "zc-onboarding-complete";

export function hasCompletedOnboarding() {
  try {
    return window.localStorage.getItem(ONBOARDING_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function completeOnboarding() {
  try {
    window.localStorage.setItem(ONBOARDING_STORAGE_KEY, "true");
  } catch {
    // Optional browser storage must never make the first-value flow unsafe to exit.
  }
}

export function OnboardingDialog() {
  const { t, language } = useI18nContext();
  const { view, setView, onError } = useNavigationContext();
  const { settings, isLoadingSettings, setDefaultScanFolders } = useSettingsContext();
  const copy = maturityCopy(language);
  const [openDialog, setOpenDialog] = useState(false);
  const [dismissedForSession, setDismissedForSession] = useState(false);
  const [step, setStep] = useState(0);
  const [selectedFolderPath, setSelectedFolderPath] = useState("");
  const [allowManagedLocal, setAllowManagedLocal] = useState(false);
  const [allowManagedCloud, setAllowManagedCloud] = useState(false);
  const [skipAIConfiguration, setSkipAIConfiguration] = useState(false);
  const [selectedProviderMode, setSelectedProviderMode] = useState<AIProviderMode | null>(null);
  const [readiness, setReadiness] = useState<AIProductFeatureReadiness | null>(null);
  const [isLoadingReadiness, setIsLoadingReadiness] = useState(false);
  const [readinessError, setReadinessError] = useState(false);
  const [isSavingScope, setIsSavingScope] = useState(false);
  const [folderAdded, setFolderAdded] = useState(false);
  const [error, setError] = useState("");
  const primaryRef = useRef<HTMLButtonElement | null>(null);
  const readinessRequestEpoch = useRef(0);

  const configuredScanCount = settings.defaultScanFolders.filter((root) => root.enabled && root.path.trim()).length;
  const scanCount = Math.max(configuredScanCount, folderAdded || selectedFolderPath ? 1 : 0);
  const hasUsefulConfiguredFolder = configuredScanCount > 0 || folderAdded;
  const canIndexInBackground = settings.backgroundIndexOnStartup !== false;

  const refreshReadiness = useCallback(async () => {
    const epoch = ++readinessRequestEpoch.current;
    setIsLoadingReadiness(true);
    setReadinessError(false);
    try {
      const next = await tauriApi.getAIFeatureReadiness();
      if (epoch !== readinessRequestEpoch.current) return;
      setReadiness(next);
      setSelectedProviderMode((current) => current ?? next.provider.providerMode);
    } catch {
      if (epoch !== readinessRequestEpoch.current) return;
      setReadiness(null);
      setReadinessError(true);
    } finally {
      if (epoch === readinessRequestEpoch.current) setIsLoadingReadiness(false);
    }
  }, []);

  useEffect(() => {
    if (isLoadingSettings || dismissedForSession || view !== "scanner") return undefined;
    const frame = window.requestAnimationFrame(() => {
      if (!hasCompletedOnboarding()) setOpenDialog(true);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [dismissedForSession, isLoadingSettings, view]);

  useEffect(() => {
    if (!openDialog) return undefined;
    void refreshReadiness().catch(() => undefined);
    return () => { readinessRequestEpoch.current += 1; };
  }, [openDialog, refreshReadiness]);

  function dismiss() {
    setOpenDialog(false);
    setDismissedForSession(true);
    setError("");
    setView("scanner");
  }

  function reopen() {
    setError("");
    setDismissedForSession(false);
    setOpenDialog(true);
  }

  async function chooseScanFolder() {
    try {
      const selected = await open({ directory: true, multiple: false, title: t("onboardingChooseFolder") });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path?.trim()) return;
      setSelectedFolderPath(path.trim());
      setError("");
    } catch (caught) {
      const message = String(caught instanceof Error ? caught.message : caught);
      setError(t("onboardingSaveFailed"));
      onError?.(message || t("onboardingSaveFailed"));
    }
  }

  async function saveFolderSetup() {
    if (isSavingScope) return;
    setIsSavingScope(true);
    setError("");
    try {
      let managedScopePaths = settings.defaultScanFolders
        .filter((root) => root.enabled && root.path.trim())
        .map((root) => root.path.trim());
      if (selectedFolderPath) {
        const nextRoots = upsertDefaultScanRoot(settings.defaultScanFolders, selectedFolderPath);
        const saved = await setDefaultScanFolders(nextRoots);
        if (!saved) {
          setError(t("onboardingSaveFailed"));
          return;
        }
        setFolderAdded(true);
        managedScopePaths = [selectedFolderPath];
      }
      if (!managedScopePaths.length) {
        setError(t("onboardingUsefulFolderRequired"));
        return;
      }
      if (!skipAIConfiguration && (allowManagedLocal || allowManagedCloud)) {
        for (const path of managedScopePaths) {
          await tauriApi.addManagedScope({
            path,
            enabled: true,
            allowLocalAi: allowManagedLocal,
            allowCloudAi: allowManagedCloud
          });
        }
      }
      setStep(skipAIConfiguration ? 4 : 3);
      await refreshReadiness();
    } catch (caught) {
      setError(t("onboardingSaveFailed"));
      onError?.(String(caught instanceof Error ? caught.message : caught));
    } finally {
      setIsSavingScope(false);
    }
  }

  function openProviderSettings(mode: AIProviderMode) {
    setSelectedProviderMode(mode);
    setOpenDialog(false);
    setDismissedForSession(false);
    openAISettingsForMode(setView, mode);
  }

  function openCleanupSettings() {
    setOpenDialog(false);
    setDismissedForSession(false);
    openSettingsSection(setView, "settings-ai-cleanup");
  }

  function nextStep() {
    setError("");
    if (step === 2) {
      void saveFolderSetup().catch(() => undefined);
      return;
    }
    if (step === 3 && skipAIConfiguration) {
      setStep(4);
      return;
    }
    if (step < 4) setStep((current) => current + 1);
    else finishOnboarding();
  }

  function skipAISetup() {
    setError("");
    setAllowManagedLocal(false);
    setAllowManagedCloud(false);
    setSkipAIConfiguration(true);
    if (step < 2) setStep(2);
    else if (step === 3) setStep(4);
  }

  function finishOnboarding() {
    if (!hasUsefulConfiguredFolder) {
      setStep(2);
      setError(t("onboardingUsefulFolderRequired"));
      return;
    }
    completeOnboarding();
    setOpenDialog(false);
    setDismissedForSession(true);
    setError("");
    setStep(0);
    setView(canIndexInBackground ? "library" : "scanner");
  }

  if (!openDialog) {
    return !isLoadingSettings && view === "scanner" ? (
      <button
        type="button"
        data-getting-started
        className={cn(buttonSecondary, "fixed bottom-5 right-5 z-40 min-h-9 px-3 text-xs shadow-[var(--zc-shadow-raised)]")}
        onClick={reopen}
      >
        <FolderOpen size={15} />
        {copy.onboardingRestart}
      </button>
    ) : null;
  }

  const stepLabel = copy.onboardingStep(step + 1, 5);
  const finishLabel = step === 4 ? copy.onboardingFinish : t("onboardingNext");
  const providerMode = selectedProviderMode ?? readiness?.provider.providerMode ?? null;
  const titleId = "onboarding-title";
  const descriptionId = "onboarding-description";

  return (
    <ModalPortal modalId="onboarding-dialog" initialFocusRef={primaryRef} onEscape={dismiss}>
      <div className="fixed inset-0 z-50 grid place-items-center overflow-y-auto bg-[var(--zc-overlay)] p-4 sm:p-6">
        <section className={cn(overlaySurface, "grid max-h-[calc(100dvh-2rem)] w-full max-w-3xl grid-rows-[auto_minmax(0,1fr)_auto_auto] gap-5 overflow-hidden p-5 sm:max-h-[calc(100dvh-3rem)] sm:p-7")} role="dialog" aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId}>
          <header className="flex items-start justify-between gap-4">
            <div className="flex items-center gap-3">
              <BrandMark size="app" decorative />
              <div>
                <p className="text-xs font-semibold uppercase tracking-[0.12em] text-[var(--zc-primary-text)]">Zen Canvas</p>
                <h2 id={titleId} className="mt-1 text-xl font-semibold text-[var(--zc-text-primary)]">{copy.onboardingTitle}</h2>
              </div>
            </div>
            <span className="shrink-0 text-xs font-medium text-[var(--zc-text-tertiary)]" aria-live="polite">{stepLabel}</span>
          </header>

          <div id={descriptionId} className="min-h-0 overflow-y-auto overscroll-contain pr-1" data-onboarding-step={step + 1}>
            {isBrowserMockEnabled() ? <p className="mb-3 text-xs leading-5 text-[var(--zc-text-tertiary)]" data-onboarding-presentation-only>{t("aiReadinessBrowserMock")}</p> : null}
            {step === 0 ? (
              <div className="grid gap-5">
                <div className="grid gap-2">
                  <div className="flex items-center gap-2 text-[var(--zc-success-text)]"><LockKeyhole size={19} aria-hidden="true" /><h3 className="text-lg font-semibold text-[var(--zc-text-primary)]">{t("onboardingPrivacyTitle")}</h3></div>
                  <p className="text-sm leading-6 text-[var(--zc-text-secondary)]">{t("onboardingPrivacyDesc")}</p>
                </div>
                <div className="grid gap-3 sm:grid-cols-3">
                  {[t("onboardingLocalIndex"), t("onboardingPreview"), t("onboardingRestorable")].map((label) => <div key={label} className="grid gap-2 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3 text-sm"><Check size={16} className="text-[var(--zc-success-text)]" aria-hidden="true" /><span>{label}</span></div>)}
                </div>
              </div>
            ) : null}

            {step === 1 ? (
              <div className="grid gap-5">
                <div className="grid gap-2"><h3 className="text-lg font-semibold text-[var(--zc-text-primary)]">{t("onboardingConnectionTitle")}</h3><p className="text-sm leading-6 text-[var(--zc-text-secondary)]">{t("onboardingConnectionDesc")}</p></div>
                <div className="grid gap-3 sm:grid-cols-2" role="group" aria-label={t("onboardingConnectionTitle")}>
                  <button type="button" data-onboarding-provider-mode="local" aria-pressed={providerMode === "local"} className={cn("grid gap-2 rounded-[var(--zc-radius-field)] border p-4 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--zc-focus-ring)]", providerMode === "local" ? "border-[var(--zc-control-border-selected)] bg-[var(--zc-selected-surface)]" : "border-[var(--zc-border)] bg-[var(--zc-surface-subtle)]")} onClick={() => setSelectedProviderMode("local")}>
                    <span className="flex items-center gap-2 font-semibold"><Cpu size={16} aria-hidden="true" />{t("onboardingAILocal")}</span>
                    <span className="text-sm leading-5 text-[var(--zc-text-secondary)]">{t("onboardingLocalProviderDesc")}</span>
                  </button>
                  <button type="button" data-onboarding-provider-mode="cloud" aria-pressed={providerMode === "cloud"} className={cn("grid gap-2 rounded-[var(--zc-radius-field)] border p-4 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--zc-focus-ring)]", providerMode === "cloud" ? "border-[var(--zc-control-border-selected)] bg-[var(--zc-selected-surface)]" : "border-[var(--zc-border)] bg-[var(--zc-surface-subtle)]")} onClick={() => setSelectedProviderMode("cloud")}>
                    <span className="flex items-center gap-2 font-semibold"><Cloud size={16} aria-hidden="true" />{t("onboardingAICloud")}</span>
                    <span className="text-sm leading-5 text-[var(--zc-text-secondary)]">{t("onboardingCloudProviderDesc")}</span>
                  </button>
                </div>
                <div className="flex flex-wrap items-center justify-between gap-3 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3">
                  <span className="text-sm text-[var(--zc-text-secondary)]" data-onboarding-provider-readiness>
                    {readiness ? providerReadinessText(readiness.provider.state, readiness.provider.reason, t) : readinessError ? t("aiReadinessUnavailableDesc") : isLoadingReadiness ? t("aiReadinessLoading") : t("onboardingConnectionNotConfigured")}
                  </span>
                  <button type="button" className={buttonSecondary} disabled={!providerMode} onClick={() => providerMode && openProviderSettings(providerMode)}>{t("onboardingConfigureAI")}</button>
                </div>
              </div>
            ) : null}

            {step === 2 ? (
              <div className="grid gap-5">
                <div className="grid gap-2"><h3 className="text-lg font-semibold text-[var(--zc-text-primary)]">{t("onboardingScopeTitle")}</h3><p className="text-sm leading-6 text-[var(--zc-text-secondary)]">{t("onboardingScopeDesc")}</p></div>
                <div className="flex flex-wrap items-center justify-between gap-3 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-4">
                  <div className="min-w-0"><strong className="block text-sm">{selectedFolderPath ? t("onboardingSelectedFolder").replace("{path}", selectedFolderPath) : scanCount ? t("onboardingCurrentScope").replace("{count}", String(scanCount)) : t("onboardingNoScope")}</strong><span className="mt-1 block break-all text-xs text-[var(--zc-text-tertiary)]">{selectedFolderPath || settings.defaultScanFolders.filter((root) => root.enabled).map((root) => root.label).join("、")}</span></div>
                  <button type="button" className={buttonSecondary} onClick={() => void chooseScanFolder()}><FolderOpen size={16} />{t("onboardingChooseFolder")}</button>
                </div>
                <fieldset className="grid gap-3 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] p-4">
                  <legend className="px-1 text-sm font-semibold text-[var(--zc-text-primary)]">{t("onboardingManagedConsentTitle")}</legend>
                  <p className="text-sm leading-5 text-[var(--zc-text-secondary)]">{t("onboardingManagedConsentDesc")}</p>
                  <label className="flex min-h-11 items-start gap-3 text-sm"><input type="checkbox" className="mt-0.5 size-4 accent-[var(--zc-primary)]" checked={allowManagedLocal} onChange={(event) => setAllowManagedLocal(event.target.checked)} /><span>{t("onboardingManagedLocalConsent")}</span></label>
                  <label className="flex min-h-11 items-start gap-3 text-sm"><input type="checkbox" className="mt-0.5 size-4 accent-[var(--zc-primary)]" checked={allowManagedCloud} onChange={(event) => setAllowManagedCloud(event.target.checked)} /><span>{t("onboardingManagedCloudConsent")}</span></label>
                  {readiness ? <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]" data-onboarding-managed-readiness>{managedReadinessText(readiness.managedScopes, t)}</p> : null}
                </fieldset>
              </div>
            ) : null}

            {step === 3 ? (
              <div className="grid gap-5">
                <div className="grid gap-2"><h3 className="text-lg font-semibold text-[var(--zc-text-primary)]">{t("onboardingCleanupTitle")}</h3><p className="text-sm leading-6 text-[var(--zc-text-secondary)]">{t("onboardingCleanupDesc")}</p></div>
                <div className="grid gap-2 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-4">
                  <strong className="text-sm text-[var(--zc-text-primary)]">{t("onboardingCleanupSeparateTitle")}</strong>
                  <p className="text-sm leading-5 text-[var(--zc-text-secondary)]">{t("onboardingCleanupSeparateDesc")}</p>
                  <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]" data-onboarding-cleanup-readiness>
                    {readiness ? cleanupReadinessText(readiness.cleanup.state, readiness.cleanup.reason, readiness.cleanup.provider.state, readiness.cleanup.provider.reason, t) : readinessError ? t("aiReadinessUnavailableDesc") : isLoadingReadiness ? t("aiReadinessLoading") : t("cleanupReadinessUnavailable")}
                  </p>
                  <button type="button" className={buttonSecondary} onClick={openCleanupSettings}>{t("onboardingCleanupSettings")}</button>
                </div>
              </div>
            ) : null}

            {step === 4 ? (
              <div className="grid gap-5">
                <div className="grid gap-2"><h3 className="text-lg font-semibold text-[var(--zc-text-primary)]">{t("onboardingCapabilitiesTitle")}</h3><p className="text-sm leading-6 text-[var(--zc-text-secondary)]">{t("onboardingCapabilitiesDesc")}</p></div>
                <ol className="grid gap-3 sm:grid-cols-2" aria-label={t("onboardingCapabilitiesTitle")}>
                  {[
                    t("onboardingCapabilityFiles"),
                    t("onboardingCapabilitySearch"),
                    t("onboardingCapabilityPreview"),
                    t("onboardingCapabilityOrganize"),
                    t("onboardingCapabilityCleanup"),
                    t("onboardingCapabilityRestore")
                  ].map((label, index) => <li key={label} className="grid gap-2 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-4"><span className="text-xs font-semibold uppercase tracking-[0.1em] text-[var(--zc-primary-text)]">{String(index + 1).padStart(2, "0")}</span><strong className="text-sm">{label}</strong></li>)}
                </ol>
                <p className="text-sm leading-6 text-[var(--zc-text-secondary)]">{t("onboardingSkipFeatureBoundary")}</p>
              </div>
            ) : null}
          </div>

          {error ? <p className="text-sm text-[var(--zc-danger-text)]" role="alert">{error}</p> : null}
          <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--zc-divider)] pt-4">
            <button type="button" data-onboarding-skip-ai className={buttonGhost} onClick={skipAISetup}>{t("onboardingSkipAI")}</button>
            <div className="flex flex-wrap justify-end gap-2">
              {step > 0 ? <button type="button" className={buttonSecondary} onClick={() => { setError(""); if (step === 2 && skipAIConfiguration) setSkipAIConfiguration(false); setStep((current) => current - 1); }}>{t("onboardingBack")}</button> : null}
              <button ref={primaryRef} type="button" data-onboarding-next className={buttonPrimary} onClick={nextStep} disabled={isSavingScope}>{isSavingScope ? t("onboardingSaving") : finishLabel}</button>
            </div>
          </footer>
        </section>
      </div>
    </ModalPortal>
  );
}
