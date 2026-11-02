import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronRight, Cpu, FolderOpen, HardDrive, Layers, Search, ShieldCheck, Sparkles, Trash2, X } from "lucide-react";
import { tauriApi } from "../api/tauriApi";
import { useI18nContext, useNavigationContext } from "../contexts/AppContexts";
import type { AIProductFeatureReadiness, GlobalIndexSource } from "../types/domain";
import { cn, buttonGhost, buttonSecondary, overlaySurface, buttonPrimary, focusVisibleState } from "../utils/tw";
import { isFullVolumeAuthorizationTarget, isGlobalIndexSourceAuthorized } from "../utils/globalIndexSources";
import { BrandMark } from "./ui/BrandMark";
import { ModalPortal } from "./modal/ModalPortal";
import { openAISettingsForMode, openSettingsSection } from "../views/settings/settingsNavigation";
import { providerReadinessText } from "../views/shared/AIReadinessNotice";
import { isBrowserMockEnabled } from "../utils/runtimeMode";
import { enableGlobalSearchForSources } from "./onboardingGlobalSearch";
import { ONBOARDING_REOPEN_EVENT } from "./onboardingReopen";

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

export function resetOnboardingCompletion() {
  try {
    window.localStorage.removeItem(ONBOARDING_STORAGE_KEY);
  } catch {
    // A blocked storage write only means the intro reappears on next launch.
  }
}

type SearchSourcesState = "idle" | "loading" | "ready" | "failed";

/**
 * ADR-0016 / #527: first launch is a short, always-skippable product intro.
 * It never requires a folder, a Managed Library scope, an AI provider or a
 * Global Index authorization. Leaving the intro is durable and lands on the
 * ordinary Files workspace; no scan, index or managed scope is created here.
 */
export function OnboardingDialog() {
  const { t } = useI18nContext();
  const { setView, onError } = useNavigationContext();
  const [openDialog, setOpenDialog] = useState(false);
  const [dismissedForSession, setDismissedForSession] = useState(false);
  const [step, setStep] = useState(0);
  const [readiness, setReadiness] = useState<AIProductFeatureReadiness | null>(null);
  const [readinessError, setReadinessError] = useState(false);
  const [searchSources, setSearchSources] = useState<GlobalIndexSource[]>([]);
  const [searchSourcesState, setSearchSourcesState] = useState<SearchSourcesState>("idle");
  const [selectedSourceIds, setSelectedSourceIds] = useState<Set<string>>(() => new Set());
  const [isEnablingSearch, setIsEnablingSearch] = useState(false);
  const [searchError, setSearchError] = useState("");
  const [searchEnabled, setSearchEnabled] = useState(false);
  const [reopenedFromAbout, setReopenedFromAbout] = useState(false);
  const primaryRef = useRef<HTMLButtonElement | null>(null);
  const readinessEpoch = useRef(0);
  const sourcesEpoch = useRef(0);

  useEffect(() => {
    if (dismissedForSession) return undefined;
    if (hasCompletedOnboarding()) return undefined;
    const frame = window.requestAnimationFrame(() => {
      setReopenedFromAbout(false);
      setOpenDialog(true);
    });
    return () => window.cancelAnimationFrame(frame);
  }, [dismissedForSession]);

  useEffect(() => {
    const reopen = () => {
      setDismissedForSession(false);
      setReopenedFromAbout(true);
      setStep(0);
      setSearchError("");
      setOpenDialog(true);
    };
    window.addEventListener(ONBOARDING_REOPEN_EVENT, reopen);
    return () => window.removeEventListener(ONBOARDING_REOPEN_EVENT, reopen);
  }, []);

  const refreshReadiness = useCallback(async () => {
    const epoch = ++readinessEpoch.current;
    try {
      const next = await tauriApi.getAIFeatureReadiness();
      if (epoch !== readinessEpoch.current) return;
      setReadiness(next);
      setReadinessError(false);
    } catch {
      if (epoch !== readinessEpoch.current) return;
      setReadiness(null);
      setReadinessError(true);
    }
  }, []);

  const loadSearchSources = useCallback(async () => {
    const epoch = ++sourcesEpoch.current;
    setSearchSourcesState("loading");
    try {
      const sources = await tauriApi.listGlobalIndexSources();
      if (epoch !== sourcesEpoch.current) return;
      setSearchSources(sources);
      setSearchSourcesState("ready");
    } catch {
      if (epoch !== sourcesEpoch.current) return;
      setSearchSources([]);
      setSearchSourcesState("failed");
    }
  }, []);

  useEffect(() => {
    if (!openDialog) return undefined;
    void refreshReadiness();
    return () => { readinessEpoch.current += 1; };
  }, [openDialog, refreshReadiness]);

  useEffect(() => {
    if (!openDialog || step !== SEARCH_STEP) return undefined;
    if (searchSourcesState === "idle") void loadSearchSources();
    return () => { sourcesEpoch.current += 1; };
  }, [loadSearchSources, openDialog, searchSourcesState, step]);

  /**
   * Every exit is durable. The intro is never a gate: leaving it always lands
   * on the ordinary Files workspace with no folder, scope or index change.
   */
  function leaveIntro() {
    completeOnboarding();
    setOpenDialog(false);
    setDismissedForSession(true);
    setStep(0);
    setSearchError("");
    // A voluntary re-read from Help / About must not move the user out of
    // Settings; only the first-run flow lands on the Files workspace.
    if (!reopenedFromAbout) setView("library");
    setReopenedFromAbout(false);
  }

  function openProviderSettings(mode: "local" | "cloud") {
    completeOnboarding();
    setOpenDialog(false);
    setDismissedForSession(true);
    openAISettingsForMode(setView, mode);
  }

  function openGlobalSearchSettings() {
    completeOnboarding();
    setOpenDialog(false);
    setDismissedForSession(true);
    openSettingsSection(setView, "settings-global-index");
  }

  const selectedSources = useMemo(
    () => searchSources.filter((source) => selectedSourceIds.has(source.volume.id)),
    [searchSources, selectedSourceIds]
  );

  function toggleSource(source: GlobalIndexSource, checked: boolean) {
    setSearchError("");
    setSelectedSourceIds((current) => {
      const next = new Set(current);
      if (checked) next.add(source.volume.id);
      else next.delete(source.volume.id);
      return next;
    });
  }

  async function enableSearch() {
    if (isEnablingSearch || selectedSources.length === 0) return;
    setIsEnablingSearch(true);
    setSearchError("");
    try {
      const result = await enableGlobalSearchForSources(
        searchSources,
        new Set(selectedSources.map((source) => source.volume.id)),
        {
          authorizeGlobalIndexSource: (sourceId, confirmed) => tauriApi.authorizeGlobalIndexSource(sourceId, confirmed),
          setGlobalIndexSourceEnabled: (sourceId, enabled) => tauriApi.setGlobalIndexSourceEnabled(sourceId, enabled),
          startGlobalIndex: () => tauriApi.startGlobalIndex()
        }
      );
      if (result.enabled.length > 0) {
        setSearchEnabled(true);
        setSelectedSourceIds(new Set());
      }
      const failure = result.failed[0] ?? null;
      const startError = result.startError;
      if (failure || startError) {
        setSearchError(t("onboardingSearchFailed").replace("{error}", failure?.error ?? startError ?? ""));
        onError?.(failure?.error ?? startError ?? t("onboardingSearchFailed"));
      }
    } catch (caught) {
      const message = String(caught instanceof Error ? caught.message : caught);
      setSearchError(t("onboardingSearchFailed").replace("{error}", message));
      onError?.(message);
    } finally {
      setIsEnablingSearch(false);
    }
  }

  if (!openDialog) return null;

  const stepLabel = t("onboardingIntroStep").replace("{step}", String(step + 1)).replace("{total}", String(SCENE_COUNT));
  const providerMode = readiness?.provider.providerMode ?? "cloud";
  const aiNote = readiness
    ? providerReadinessText(readiness.provider.state, readiness.provider.reason, t)
    : readinessError
      ? t("aiReadinessUnavailableDesc")
      : t("onboardingConnectionNotConfigured");
  const titleId = "onboarding-title";
  const descriptionId = "onboarding-description";

  return (
    <ModalPortal modalId="onboarding-dialog" initialFocusRef={primaryRef} onEscape={leaveIntro}>
      <div className="fixed inset-0 z-50 grid place-items-center overflow-y-auto bg-[var(--zc-overlay)] p-3 sm:p-6">
        <section
          className={cn(overlaySurface, "grid max-h-[calc(100dvh-1.5rem)] w-full max-w-4xl grid-rows-[auto_minmax(0,1fr)_auto] gap-4 overflow-hidden p-4 sm:max-h-[calc(100dvh-3rem)] sm:gap-5 sm:p-7")}
          role="dialog"
          aria-modal="true"
          aria-labelledby={titleId}
          aria-describedby={descriptionId}
        >
          <header className="flex items-start justify-between gap-3">
            <div className="flex min-w-0 items-center gap-3">
              <BrandMark size="app" decorative />
              <div className="min-w-0">
                <p className="text-xs font-semibold uppercase tracking-[0.12em] text-[var(--zc-primary-text)]">Fylura</p>
                <h2 id={titleId} className="mt-1 truncate text-lg font-semibold text-[var(--zc-text-primary)] sm:text-xl">{t("onboardingIntroTitle")}</h2>
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <span className="text-xs font-medium text-[var(--zc-text-tertiary)]" aria-live="polite">{stepLabel}</span>
              <button
                type="button"
                className={cn("grid h-8 w-8 place-items-center rounded-md text-[var(--zc-text-tertiary)] hover:bg-[var(--zc-surface-hover)] hover:text-[var(--zc-text-primary)]", focusVisibleState)}
                aria-label={t("onboardingSkipIntro")}
                data-onboarding-close
                onClick={leaveIntro}
              >
                <X size={16} aria-hidden="true" />
              </button>
            </div>
          </header>

          <div id={descriptionId} className="min-h-0 overflow-y-auto overscroll-contain pr-1" data-onboarding-step={step + 1}>
            {isBrowserMockEnabled() ? <p className="mb-3 text-xs leading-5 text-[var(--zc-text-tertiary)]" data-onboarding-presentation-only>{t("aiReadinessBrowserMock")}</p> : null}
            {step === FILES_STEP ? <FilesScene t={t} /> : null}
            {step === ORGANIZE_STEP ? <OrganizeScene t={t} aiNote={aiNote} onConfigureAI={() => openProviderSettings(providerMode)} /> : null}
            {step === CLEANUP_STEP ? <CleanupScene t={t} aiNote={aiNote} onConfigureAI={() => openProviderSettings(providerMode)} /> : null}
            {step === SEARCH_STEP ? (
              <SearchScene
                t={t}
                sources={searchSources}
                state={searchSourcesState}
                selectedIds={selectedSourceIds}
                onToggle={toggleSource}
                onReload={() => void loadSearchSources()}
                onEnable={() => void enableSearch()}
                onOpenSettings={openGlobalSearchSettings}
                isEnabling={isEnablingSearch}
                enabled={searchEnabled}
                error={searchError}
              />
            ) : null}
          </div>

          <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--zc-divider)] pt-4">
            <button type="button" className={buttonGhost} data-onboarding-skip onClick={leaveIntro}>
              {t("onboardingSkipIntro")}
            </button>
            <div className="flex flex-wrap justify-end gap-2">
              {step > 0 ? (
                <button type="button" className={buttonSecondary} data-onboarding-back onClick={() => setStep((current) => Math.max(0, current - 1))}>
                  {t("onboardingBack")}
                </button>
              ) : null}
              {step === 0 ? (
                <>
                  <button type="button" className={buttonSecondary} data-onboarding-next onClick={() => setStep(1)}>
                    {t("onboardingNext")}
                  </button>
                  <button ref={primaryRef} type="button" className={buttonPrimary} data-onboarding-start onClick={leaveIntro}>
                    {t("onboardingStartUsing")}
                  </button>
                </>
              ) : step < SCENE_COUNT - 1 ? (
                <button ref={primaryRef} type="button" className={buttonPrimary} data-onboarding-next onClick={() => setStep((current) => current + 1)}>
                  {t("onboardingNext")}
                </button>
              ) : (
                <button ref={primaryRef} type="button" className={buttonPrimary} data-onboarding-finish onClick={leaveIntro}>
                  {t("onboardingFinish")}
                </button>
              )}
            </div>
          </footer>
        </section>
      </div>
    </ModalPortal>
  );
}

const FILES_STEP = 0;
const ORGANIZE_STEP = 1;
const CLEANUP_STEP = 2;
const SEARCH_STEP = 3;
const SCENE_COUNT = 4;

type Translator = ReturnType<typeof useI18nContext>["t"];

function SceneHeading({ icon, title, lead }: { icon: React.ReactNode; title: string; lead: string }) {
  return (
    <div className="grid gap-2">
      <div className="flex items-center gap-2 text-[var(--zc-primary-text)]">
        {icon}
        <h3 className="text-base font-semibold text-[var(--zc-text-primary)] sm:text-lg">{title}</h3>
      </div>
      <p className="text-sm leading-6 text-[var(--zc-text-secondary)]">{lead}</p>
    </div>
  );
}

function ScenePoints({ points }: { points: readonly string[] }) {
  return (
    <ul className="grid gap-2 sm:grid-cols-2">
      {points.map((point) => (
        <li key={point} className="flex items-start gap-2 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] px-3 py-2 text-sm leading-5 text-[var(--zc-text-secondary)]">
          <Check size={15} className="mt-0.5 shrink-0 text-[var(--zc-success-text)]" aria-hidden="true" />
          <span>{point}</span>
        </li>
      ))}
    </ul>
  );
}

/**
 * Representative Files composition. It is decorative (`aria-hidden`) and is
 * always paired with a text summary so the intro never depends on the mock.
 */
function FilesShowcase({ label }: { label: string }) {
  return (
    <div className="grid gap-1" data-onboarding-showcase="files">
      <div className="overflow-hidden rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface)] shadow-[var(--zc-shadow-raised)]" aria-hidden="true">
        <div className="flex items-center gap-1.5 border-b border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] px-2.5 py-1.5">
          <span className="flex items-center gap-1 rounded-t-md border border-[var(--zc-border)] border-b-transparent bg-[var(--zc-surface)] px-2 py-1 text-[11px] font-medium text-[var(--zc-text-primary)]"><FolderOpen size={11} aria-hidden="true" />{label}</span>
          <span className="rounded-md px-2 py-1 text-[11px] text-[var(--zc-text-tertiary)]">{label}</span>
          <span className="rounded-md px-1.5 py-1 text-[11px] text-[var(--zc-text-tertiary)]">+</span>
          <span className="ml-auto rounded-md border border-[var(--zc-border)] px-1.5 py-0.5 text-[10px] text-[var(--zc-text-tertiary)]">双栏</span>
        </div>
        <div className="grid grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)] gap-2 p-2.5">
          <ShowcasePane rows={3} accent />
          <ShowcasePane rows={3} />
        </div>
        <div className="flex items-center gap-2 border-t border-[var(--zc-divider)] bg-[var(--zc-surface-subtle)] px-2.5 py-1.5 text-[10px] text-[var(--zc-text-tertiary)]">
          <span className="rounded border border-[var(--zc-border)] px-1.5 py-0.5">文件篮 · 3</span>
          <span className="rounded border border-[var(--zc-border)] px-1.5 py-0.5">预览</span>
          <span className="rounded border border-[var(--zc-border)] px-1.5 py-0.5">历史</span>
          <span className="ml-auto">Ctrl+C · Ctrl+V</span>
        </div>
      </div>
      <p className="sr-only">{label}</p>
    </div>
  );
}

function ShowcasePane({ rows, accent = false }: { rows: number; accent?: boolean }) {
  return (
    <div className={cn("grid gap-1.5 rounded-[var(--zc-radius-control)] border p-2", accent ? "border-[var(--zc-control-border-selected)] bg-[var(--zc-selected-surface)]" : "border-[var(--zc-border)] bg-[var(--zc-surface)]")}>
      {Array.from({ length: rows }, (_, index) => (
        <div key={index} className="flex items-center gap-1.5">
          <span className="h-3.5 w-3.5 shrink-0 rounded-[3px] bg-[var(--zc-primary-soft)]" />
          <span className="h-1.5 flex-1 rounded-full bg-[var(--zc-divider)]" style={{ maxWidth: `${88 - index * 16}%` }} />
        </div>
      ))}
    </div>
  );
}

function FilesScene({ t }: { t: Translator }) {
  return (
    <div className="grid gap-4">
      <SceneHeading icon={<Layers size={19} aria-hidden="true" />} title={t("onboardingFilesTitle")} lead={t("onboardingFilesLead")} />
      <FilesShowcase label={t("onboardingFilesShowcaseLabel")} />
      <ScenePoints points={[
        t("onboardingFilesPointBrowse"),
        t("onboardingFilesPointPanes"),
        t("onboardingFilesPointBasket"),
        t("onboardingFilesPointPreview"),
        t("onboardingFilesPointHistory")
      ]} />
      <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]">{t("onboardingFilesNoConfig")}</p>
    </div>
  );
}

function FlowSteps({ steps }: { steps: readonly { title: string; description: string }[] }) {
  return (
    <ol className="grid gap-2 sm:grid-cols-2">
      {steps.map((step, index) => (
        <li key={step.title} className="grid gap-1 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3">
          <span className="text-[11px] font-semibold uppercase tracking-[0.1em] text-[var(--zc-primary-text)]">{String(index + 1).padStart(2, "0")}</span>
          <strong className="text-sm text-[var(--zc-text-primary)]">{step.title}</strong>
          <span className="text-sm leading-5 text-[var(--zc-text-secondary)]">{step.description}</span>
        </li>
      ))}
    </ol>
  );
}

function AINote({ note, onConfigureAI, t }: { note: string; onConfigureAI: () => void; t: Translator }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3">
      <span className="min-w-0 flex-1 text-xs leading-5 text-[var(--zc-text-secondary)]" data-onboarding-ai-readiness>{note}</span>
      <button type="button" className={buttonSecondary} data-onboarding-configure-ai onClick={onConfigureAI}>
        <Cpu size={14} aria-hidden="true" />
        {t("onboardingConfigureAI")}
      </button>
    </div>
  );
}

function OrganizeScene({ t, aiNote, onConfigureAI }: { t: Translator; aiNote: string; onConfigureAI: () => void }) {
  return (
    <div className="grid gap-4">
      <SceneHeading icon={<Sparkles size={19} aria-hidden="true" />} title={t("onboardingOrganizeTitle")} lead={t("onboardingOrganizeLead")} />
      <FlowSteps steps={[
        { title: t("onboardingOrganizeStepPick"), description: t("onboardingOrganizeStepPickDesc") },
        { title: t("onboardingOrganizeStepPlan"), description: t("onboardingOrganizeStepPlanDesc") },
        { title: t("onboardingOrganizeStepReview"), description: t("onboardingOrganizeStepReviewDesc") },
        { title: t("onboardingOrganizeStepRun"), description: t("onboardingOrganizeStepRunDesc") }
      ]} />
      <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]">{t("onboardingOrganizeNeedsAI")}</p>
      <AINote note={aiNote} onConfigureAI={onConfigureAI} t={t} />
    </div>
  );
}

function CleanupScene({ t, aiNote, onConfigureAI }: { t: Translator; aiNote: string; onConfigureAI: () => void }) {
  return (
    <div className="grid gap-4">
      <SceneHeading icon={<Trash2 size={19} aria-hidden="true" />} title={t("onboardingCleanupTitle")} lead={t("onboardingCleanupLead")} />
      <FlowSteps steps={[
        { title: t("onboardingCleanupStepScan"), description: t("onboardingCleanupStepScanDesc") },
        { title: t("onboardingCleanupStepPlan"), description: t("onboardingCleanupStepPlanDesc") },
        { title: t("onboardingCleanupStepReview"), description: t("onboardingCleanupStepReviewDesc") },
        { title: t("onboardingCleanupStepTrash"), description: t("onboardingCleanupStepTrashDesc") }
      ]} />
      <p className="flex items-start gap-2 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3 text-xs leading-5 text-[var(--zc-text-secondary)]">
        <ShieldCheck size={15} className="mt-0.5 shrink-0 text-[var(--zc-success-text)]" aria-hidden="true" />
        <span>{t("onboardingCleanupSafety")}</span>
      </p>
      <AINote note={aiNote} onConfigureAI={onConfigureAI} t={t} />
    </div>
  );
}

function SearchScene({
  t,
  sources,
  state,
  selectedIds,
  onToggle,
  onReload,
  onEnable,
  onOpenSettings,
  isEnabling,
  enabled,
  error
}: {
  t: Translator;
  sources: readonly GlobalIndexSource[];
  state: SearchSourcesState;
  selectedIds: ReadonlySet<string>;
  onToggle: (source: GlobalIndexSource, checked: boolean) => void;
  onReload: () => void;
  onEnable: () => void;
  onOpenSettings: () => void;
  isEnabling: boolean;
  enabled: boolean;
  error: string;
}) {
  return (
    <div className="grid gap-4">
      <SceneHeading icon={<Search size={19} aria-hidden="true" />} title={t("onboardingSearchTitle")} lead={t("onboardingSearchLead")} />
      <p className="text-sm leading-6 text-[var(--zc-text-secondary)]">{t("onboardingSearchExplain")}</p>

      <section className="grid gap-2" aria-label={t("onboardingSearchSourcesTitle")}>
        <h4 className="text-xs font-semibold uppercase tracking-[0.1em] text-[var(--zc-text-tertiary)]">{t("onboardingSearchSourcesTitle")}</h4>
        {state === "loading" ? (
          <p className="text-sm text-[var(--zc-text-secondary)]" data-onboarding-search-state="loading">{t("onboardingSearchLoading")}</p>
        ) : state === "failed" ? (
          <div className="grid gap-2 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] p-3" data-onboarding-search-state="failed">
            <p className="text-sm leading-5 text-[var(--zc-text-secondary)]">{t("onboardingSearchLoadFailed")}</p>
            <div>
              <button type="button" className={buttonSecondary} onClick={onReload}>{t("browseLocationsRetry")}</button>
            </div>
          </div>
        ) : sources.length === 0 ? (
          <p className="text-sm text-[var(--zc-text-secondary)]" data-onboarding-search-state="empty">{t("onboardingSearchNoSources")}</p>
        ) : (
          <ul className="grid gap-2" data-onboarding-search-state="ready">
            {sources.map((source) => {
              const authorized = isGlobalIndexSourceAuthorized(source);
              const fullVolume = isFullVolumeAuthorizationTarget(source);
              const checked = selectedIds.has(source.volume.id);
              return (
                <li key={source.volume.id} className="grid gap-1 rounded-[var(--zc-radius-field)] border border-[var(--zc-border)] bg-[var(--zc-surface-subtle)] px-3 py-2">
                  <label className="flex min-h-9 cursor-pointer items-center gap-3 text-sm">
                    <input
                      type="checkbox"
                      className="size-4 accent-[var(--zc-primary)]"
                      checked={checked}
                      data-onboarding-search-source={source.volume.id}
                      onChange={(event) => onToggle(source, event.currentTarget.checked)}
                    />
                    <span className="flex min-w-0 flex-1 items-center gap-2">
                      <HardDrive size={15} className="shrink-0 text-[var(--zc-text-tertiary)]" aria-hidden="true" />
                      <span className="truncate font-medium text-[var(--zc-text-primary)]">{source.volume.displayName}</span>
                      <span className="truncate text-xs text-[var(--zc-text-tertiary)]">{source.volume.mountPath}</span>
                    </span>
                  </label>
                  {checked && fullVolume && !authorized ? (
                    <p className="pl-7 text-xs leading-5 text-[var(--zc-warning-text)]">{t("onboardingSearchAuthorizeWarning")}</p>
                  ) : null}
                </li>
              );
            })}
          </ul>
        )}
        {sources.some((source) => source.volume.platform === "windows") ? (
          <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]">{t("onboardingSearchVolumeNote")}</p>
        ) : null}
      </section>

      {error ? <p className="text-sm text-[var(--zc-danger-text)]" role="alert">{error}</p> : null}
      {enabled ? <p className="text-sm text-[var(--zc-success-text)]" role="status" data-onboarding-search-enabled>{t("onboardingSearchEnabled")}</p> : null}

      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          className={buttonPrimary}
          data-onboarding-enable-search
          disabled={isEnabling || selectedIds.size === 0 || state !== "ready"}
          onClick={onEnable}
        >
          {isEnabling ? t("onboardingSaving") : t("onboardingSearchEnable")}
        </button>
        <button type="button" className={buttonSecondary} data-onboarding-search-settings onClick={onOpenSettings}>
          <ChevronRight size={14} aria-hidden="true" />
          {t("onboardingSearchOpenSettings")}
        </button>
      </div>
      <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]">{t("onboardingSearchSkipHint")}</p>
    </div>
  );
}
