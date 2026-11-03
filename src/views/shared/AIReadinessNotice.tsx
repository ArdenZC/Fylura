import type { AIProductFeatureReadiness, AIReadinessState } from "../../types/domain";
import type { Translator, View } from "../../types/ui";
import { Button, NoticeBanner } from "./ui";
import { openSettingsSection } from "../settings/settingsNavigation";
import { isBrowserMockEnabled } from "../../utils/runtimeMode";

type Feature = "organize" | "cleanup";
type PlanReadiness = "no_plan" | "needs_analysis" | "ready" | "review" | "blocked" | "empty";

export function organizationPlanReadiness(plan: {
  summary: { needsAnalysis: number };
  effectiveSummary?: { ready: number; reviewed: number; pendingReview: number; blocked: number } | null;
} | null): PlanReadiness {
  if (!plan) return "no_plan";
  if (plan.summary.needsAnalysis > 0) return "needs_analysis";
  const readiness = plan.effectiveSummary;
  if (readiness?.blocked) return "blocked";
  if (readiness?.pendingReview) return "review";
  if (readiness?.ready || readiness?.reviewed) return "ready";
  return "empty";
}

export function AIReadinessNotice({
  feature,
  readiness,
  loading,
  error,
  planReadiness,
  t,
  setView,
  onRetry,
  sourceMode = "managed",
  compact = false
}: {
  sourceMode?: "managed" | "one_off";
  compact?: boolean;
  feature: Feature;
  readiness: AIProductFeatureReadiness | null;
  loading: boolean;
  error: boolean;
  planReadiness?: PlanReadiness;
  t: Translator;
  setView: (view: View) => void;
  onRetry: () => void;
}) {
  if (loading && !readiness) {
    return <div data-ai-readiness-feature={feature} data-ai-readiness-source={isBrowserMockEnabled() ? "presentation-fixture" : "backend-authority"}><NoticeBanner tone="info" title={t("aiReadinessLoading")} /><BrowserMockNote t={t} /></div>;
  }
  if (!readiness) {
    return <div data-ai-readiness-feature={feature} data-ai-readiness-source={isBrowserMockEnabled() ? "presentation-fixture" : "backend-authority"}>
      <NoticeBanner
        tone="error"
        title={t("aiReadinessUnavailableTitle")}
        action={<Button variant="secondary" size="compact" onClick={onRetry}>{t("aiReadinessRetry")}</Button>}
      >{error ? t("aiReadinessUnavailableDesc") : t("aiReadinessLoading")}</NoticeBanner>
      <BrowserMockNote t={t} />
    </div>;
  }

  if (feature === "organize" && sourceMode === "one_off") {
    const ready = readiness.provider.state === "ready";
    return <div data-ai-readiness-feature="organize" data-ai-organize-source="one_off" data-ai-managed-ready={readiness.managedScopes.some((scope) => scope.state === "ready") ? "true" : "false"} data-ai-readiness-source={isBrowserMockEnabled() ? "presentation-fixture" : "backend-authority"}>
      {ready ? <p className="text-xs text-[var(--zc-text-secondary)]">{t("organizeOneOffDisclosure")}</p> : <NoticeBanner tone="warning" title={providerReadinessText(readiness.provider.state, readiness.provider.reason, t)} action={<Button variant="secondary" size="compact" onClick={() => openSettingsSection(setView, "settings-ai-provider")}>{t("aiReadinessOpenSettings")}</Button>}>{t("organizeOneOffDisclosure")}</NoticeBanner>}
      <BrowserMockNote t={t} />
    </div>;
  }

  const providerText = providerReadinessText(readiness.provider.state, readiness.provider.reason, t);
  const readyScopeCount = readiness.managedScopes.filter((scope) => scope.state === "ready").length;
  if (feature === "organize") {
    const managedText = managedReadinessText(readiness.managedScopes, t);
    const providerReady = readiness.provider.state === "ready";
    const managedReady = sourceMode === "one_off" || readyScopeCount > 0;
    const sectionId = !providerReady ? "settings-ai-provider" : !managedReady ? "settings-managed-scopes" : null;
    const planText = planReadiness ? organizationPlanReadinessText(planReadiness, t) : "";
    const tone = !providerReady || !managedReady || planReadiness === "blocked" || planReadiness === "needs_analysis" ? "warning" : "info";
    return <div
      data-ai-readiness-feature="organize"
      data-ai-readiness-source={isBrowserMockEnabled() ? "presentation-fixture" : "backend-authority"}
      data-ai-provider-state={readiness.provider.state}
      data-ai-managed-ready={readyScopeCount > 0 ? "true" : "false"}
      data-ai-organize-source={sourceMode}
      data-ai-plan-readiness={planReadiness}
    >
      <NoticeBanner
        tone={tone}
        title={t("organizeReadinessTitle")}
        action={sectionId ? <Button variant="secondary" size="compact" onClick={() => openSettingsSection(setView, sectionId)}>{t("aiReadinessOpenSettings")}</Button> : undefined}
      >
        <p>{t("organizeReadinessProvider").replace("{status}", providerText)}</p>
        <p className="mt-1">{sourceMode === "one_off" ? t("organizeOneOffBoundary") : t("organizeReadinessManaged").replace("{status}", managedText)}</p>
        {planReadiness ? <p className="mt-1">{t("organizeReadinessPlan").replace("{status}", planText)}</p> : null}
        <p className="mt-1">{sourceMode === "one_off" ? t("organizeOneOffDisclosure") : t("organizeReadinessBoundary")}</p>
      </NoticeBanner>
      <BrowserMockNote t={t} />
    </div>;
  }

  const cleanup = readiness.cleanup;
  const cleanupState = cleanup.state;
  const cleanupText = cleanupReadinessText(cleanupState, cleanup.reason, cleanup.provider.state, cleanup.provider.reason, t);
  const cleanupTarget = cleanup.provider.state !== "ready" ? "settings-ai-provider" : "settings-ai-cleanup";
  const tone = cleanupState === "ready" ? "info" : cleanupState === "error" || cleanupState === "temporarily_unavailable" ? "error" : "warning";
  if (compact) return <div
    data-ai-readiness-feature="cleanup"
    data-ai-readiness-source={isBrowserMockEnabled() ? "presentation-fixture" : "backend-authority"}
    data-ai-provider-state={cleanup.provider.state}
    data-ai-cleanup-state={cleanupState}
  >
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-sm">
      <span className="text-[var(--zc-text-secondary)]">{cleanupText}</span>
      {cleanupState !== "ready" ? <Button variant="secondary" size="compact" onClick={() => openSettingsSection(setView, cleanupTarget)}>{t("aiReadinessOpenSettings")}</Button> : null}
    </div>
    <BrowserMockNote t={t} />
  </div>;
  return <div
    data-ai-readiness-feature="cleanup"
    data-ai-readiness-source={isBrowserMockEnabled() ? "presentation-fixture" : "backend-authority"}
    data-ai-provider-state={cleanup.provider.state}
    data-ai-cleanup-state={cleanupState}
  >
    <NoticeBanner
      tone={tone}
      title={t("cleanupReadinessTitle")}
      action={cleanupState === "ready" ? undefined : <Button variant="secondary" size="compact" onClick={() => openSettingsSection(setView, cleanupTarget)}>{t("aiReadinessOpenSettings")}</Button>}
    >
      <p>{cleanupText}</p>
      <p className="mt-1">{t("cleanupReadinessBoundary")}</p>
    </NoticeBanner>
    <BrowserMockNote t={t} />
  </div>;
}

function BrowserMockNote({ t }: { t: Translator }) {
  return isBrowserMockEnabled()
    ? <p className="text-xs leading-5 text-[var(--zc-text-tertiary)]" data-ai-readiness-presentation-only>{t("aiReadinessBrowserMock")}</p>
    : null;
}

function organizationPlanReadinessText(readiness: PlanReadiness, t: Translator) {
  if (readiness === "needs_analysis") return t("organizeReadinessPlanNeedsAnalysis");
  if (readiness === "ready") return t("organizeReadinessPlanReady");
  if (readiness === "review") return t("organizeReadinessPlanReview");
  if (readiness === "blocked") return t("organizeReadinessPlanBlocked");
  if (readiness === "empty") return t("organizeReadinessPlanEmpty");
  return t("organizeReadinessPlanNone");
}

export function providerReadinessText(state: AIReadinessState, reason: string, t: Translator) {
  if (state === "ready") return t("aiReadinessProviderReady");
  if (state === "disabled") return t("aiReadinessProviderDisabled");
  if (state === "needs_credential") return t("aiReadinessProviderCredential");
  if (state === "needs_provider") return reason === "provider_configuration_invalid"
    ? t("aiReadinessProviderInvalid")
    : reason === "provider_model_missing"
      ? t("aiReadinessProviderModelMissing")
      : t("aiReadinessProviderSetup");
  if (state === "temporarily_unavailable" || state === "error") return t("aiReadinessProviderUnavailable");
  return t("aiReadinessProviderSetup");
}

export function managedReadinessText(scopes: AIProductFeatureReadiness["managedScopes"], t: Translator) {
  if (!scopes.length || scopes.every((scope) => scope.state === "scope_missing")) return t("aiReadinessManagedMissing");
  if (scopes.some((scope) => scope.state === "ready")) return t("aiReadinessManagedReady");
  if (scopes.some((scope) => scope.state === "needs_consent" || scope.state === "policy_blocked")) return t("aiReadinessManagedConsent");
  if (scopes.some((scope) => scope.state === "scope_disabled")) return t("aiReadinessManagedDisabled");
  return t("aiReadinessManagedUnavailable");
}

export function cleanupReadinessText(
  state: AIReadinessState,
  reason: string,
  providerState: AIReadinessState,
  providerReason: string,
  t: Translator
) {
  if (providerState !== "ready") return providerReadinessText(providerState, providerReason, t);
  if (state === "ready") return t("cleanupReadinessReady");
  if (state === "disabled" || reason === "cleanup_ai_disabled") return t("cleanupReadinessDisabled");
  if (state === "needs_consent") return reason === "cleanup_local_ai_consent_required"
    ? t("cleanupReadinessLocalConsent")
    : t("cleanupReadinessCloudConsent");
  if (state === "temporarily_unavailable" || state === "error") return t("cleanupReadinessUnavailable");
  return t("cleanupReadinessDisabled");
}
