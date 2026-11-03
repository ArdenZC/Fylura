import type { OrganizationPlanSource } from "../api/organizationApi";
import { create } from "zustand";
import { tauriApi } from "../api/tauriApi";
import type {
  ExecuteOrganizationPlanResult,
  OrganizationPlan,
  OrganizationPlanDryRun,
  OrganizationPlanGroupSummary,
  OrganizationPlanItem,
  OrganizationPlanSelection
} from "../types/domain";
import { readableError } from "../utils/viewHelpers";
import { loadNextGroupPage, openPlan } from "./organizationPlan/groupPagination";
import {
  analyzeMissing,
  cancelPlan,
  createPlan,
  refreshPlan,
  updateDecision,
  updateGroupDecision
} from "./organizationPlan/planMutations";
import { createDryRun, executeDryRun } from "./organizationPlan/execution";
import { loadPlans } from "./organizationPlan/planListController";
import type { GroupLoadingOwner } from "./organizationPlan/ownership";

type DurableDecision = "accepted" | "kept" | "edited" | "undecided";

export type PlanListState = "idle" | "loading" | "loaded" | "failed";
export type ActivePlanState = "idle" | "opening" | "loaded" | "failed";

export type OrganizationMutationResult<T = void> =
  | { applied: true; value?: T }
  | { applied: false; value?: T; reason: "superseded" };

export interface OrganizationPlanState {
  plans: OrganizationPlan[];
  activePlan: OrganizationPlan | null;
  groups: OrganizationPlanGroupSummary[];
  groupProjectionFingerprint: string | null;
  groupNextCursor: string | null;
  groupHasMore: boolean;
  dryRun: OrganizationPlanDryRun | null;
  dryRunSelection: OrganizationPlanSelection | null;
  executionResult: ExecuteOrganizationPlanResult | null;
  planListState: PlanListState;
  planListError: string | null;
  planListRequestEpoch: number;
  activePlanState: ActivePlanState;
  openPlanError: string | null;
  openPlanErrorPlanId: string | null;
  createPlanError: string | null;
  isPlanListLoading: boolean;
  isLoading: boolean;
  isMutating: boolean;
  isExecutionInFlight: boolean;
  error: string | null;
  requestEpoch: number;
  mutationToken: number;
  groupRequestEpoch: number;
  groupLoadingOwner: GroupLoadingOwner | null;
  loadPlans: () => Promise<void>;
  createPlan: (source: OrganizationPlanSource, expectedCount: number, title?: string) => Promise<OrganizationMutationResult<OrganizationPlan>>;
  openPlan: (planId: string) => Promise<void>;
  loadNextGroupPage: () => Promise<OrganizationMutationResult<void>>;
  updateGroupDecision: (group: OrganizationPlanGroupSummary, decision: "accepted" | "kept" | "undecided", editedTargetTemplate?: string) => Promise<OrganizationMutationResult<void>>;
  updateDecision: (item: OrganizationPlanItem, decision: DurableDecision, editedFilename?: string) => Promise<OrganizationMutationResult<OrganizationPlan>>;
  refreshPlan: () => Promise<OrganizationMutationResult<OrganizationPlan>>;
  analyzeMissing: (itemIds?: string[], oneOffConsent?: boolean) => Promise<OrganizationMutationResult<number>>;
  createDryRun: (selection?: OrganizationPlanSelection) => Promise<OrganizationMutationResult<OrganizationPlanDryRun>>;
  executeDryRun: () => Promise<OrganizationMutationResult<ExecuteOrganizationPlanResult>>;
  cancelPlan: () => Promise<OrganizationMutationResult<OrganizationPlan>>;
  clearError: () => void;
}

function isTerminalOrganizationPlan(status: OrganizationPlan["status"]): boolean {
  return status === "completed" || status === "cancelled" || status === "failed";
}

export function isReviewableOrganizationPlan(status: OrganizationPlan["status"]): boolean {
  return status === "ready" || status === "partially_completed";
}

export function isHistoricalOrganizationPlan(status: OrganizationPlan["status"]): boolean {
  return isTerminalOrganizationPlan(status);
}

export function organizationPlanReviewCount(plan: OrganizationPlan): number {
  return plan.effectiveSummary?.pendingReview ?? plan.summary.pendingReview;
}

export function selectReviewableOrganizationPlan(
  plans: readonly OrganizationPlan[],
  activePlan: OrganizationPlan | null
): OrganizationPlan | null {
  const listedPlans = plans
    .filter((plan) => isReviewableOrganizationPlan(plan.status))
    .map((plan) => activePlan?.id === plan.id ? activePlan : plan);
  const activeReviewable = activePlan && isReviewableOrganizationPlan(activePlan.status)
    ? activePlan
    : null;
  if (activeReviewable && organizationPlanReviewCount(activeReviewable) > 0) return activeReviewable;
  return listedPlans.find((plan) => organizationPlanReviewCount(plan) > 0)
    ?? activeReviewable
    ?? listedPlans[0]
    ?? null;
}

export function organizationPlanPendingReview(
  plans: readonly OrganizationPlan[],
  activePlan: OrganizationPlan | null
): number {
  const selectedPlan = selectReviewableOrganizationPlan(plans, activePlan);
  return selectedPlan ? organizationPlanReviewCount(selectedPlan) : 0;
}

export const useOrganizationPlanStore = create<OrganizationPlanState>((set, get) => ({
  plans: [],
  activePlan: null,
  groups: [],
  groupProjectionFingerprint: null,
  groupNextCursor: null,
  groupHasMore: false,
  dryRun: null,
  dryRunSelection: null,
  executionResult: null,
  planListState: "idle",
  planListError: null,
  planListRequestEpoch: 0,
  activePlanState: "idle",
  openPlanError: null,
  openPlanErrorPlanId: null,
  createPlanError: null,
  isPlanListLoading: false,
  isLoading: false,
  isMutating: false,
  isExecutionInFlight: false,
  error: null,
  requestEpoch: 0,
  mutationToken: 0,
  groupRequestEpoch: 0,
  groupLoadingOwner: null,

  loadPlans: () => loadPlans({ get, set }),
  createPlan: (source, expectedCount, title) => createPlan({ get, set }, source, expectedCount, title),
  openPlan: (planId) => openPlan({ get, set }, planId),
  loadNextGroupPage: () => loadNextGroupPage({ get, set }),
  updateGroupDecision: (group, decision, editedTargetTemplate) => updateGroupDecision({ get, set }, group, decision, editedTargetTemplate),
  updateDecision: (item, decision, editedFilename) => updateDecision({ get, set }, item, decision, editedFilename),
  refreshPlan: () => refreshPlan({ get, set }),
  analyzeMissing: (itemIds = [], oneOffConsent = false) => analyzeMissing({ get, set }, itemIds, oneOffConsent),
  createDryRun: (selection = { allAccepted: true, itemIds: [] }) => createDryRun({ get, set }, selection),
  executeDryRun: () => executeDryRun({ get, set }),
  cancelPlan: () => cancelPlan({ get, set }),
  clearError: () => set({ error: null, planListError: null, openPlanError: null, createPlanError: null })
}));
