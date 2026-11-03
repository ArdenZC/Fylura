import type { OrganizationPlanSource } from "../../api/organizationApi";
import { tauriApi } from "../../api/tauriApi";
import type {
  ExecuteOrganizationPlanResult,
  OrganizationPlan,
  OrganizationPlanGroupSummary,
  OrganizationPlanItem
} from "../../types/domain";
import { readableError } from "../../utils/viewHelpers";
import type { OrganizationMutationResult, OrganizationPlanState } from "../useOrganizationPlanStore";
import {
  matchesGroupPage,
  ownsMutation,
  ownsPlanMutation,
  takeGroupProjectionOwnership
} from "./ownership";

type Context = {
  get: () => OrganizationPlanState;
  set: (
    update:
      | Partial<OrganizationPlanState>
      | ((state: OrganizationPlanState) => Partial<OrganizationPlanState>)
  ) => void;
};

function applied<T>(value?: T): OrganizationMutationResult<T> {
  return value === undefined ? { applied: true } : { applied: true, value };
}

function superseded<T>(value?: T): OrganizationMutationResult<T> {
  return value === undefined ? { applied: false, reason: "superseded" } : { applied: false, value, reason: "superseded" };
}

function replacePlan(plans: OrganizationPlan[], plan: OrganizationPlan) {
  return [plan, ...plans.filter((item) => item.id !== plan.id)]
    .sort((left, right) => right.updatedAt - left.updatedAt || left.id.localeCompare(right.id));
}

function isTerminalOrganizationPlan(status: OrganizationPlan["status"]): boolean {
  return status === "completed" || status === "cancelled" || status === "failed";
}

function isCancelableOrganizationPlan(status: OrganizationPlan["status"]): boolean {
  return ["draft", "building", "ready", "stale"].includes(status);
}

export async function createPlan(
  { get, set }: Context,
  source: OrganizationPlanSource,
  expectedCount: number,
  title?: string
): Promise<OrganizationMutationResult<OrganizationPlan>> {
  const state = get();
  if (state.planListState !== "loaded" || state.plans.some((plan) => !isTerminalOrganizationPlan(plan.status)) || state.isMutating || state.isPlanListLoading || state.isLoading) return superseded();
  const requestEpoch = state.requestEpoch;
  const mutationToken = state.mutationToken + 1;
  set((current) => takeGroupProjectionOwnership(current, { isMutating: true, mutationToken, createPlanError: null, error: null }));
  try {
    const common = { requestId: `organization-plan-${Date.now().toString(36)}`, title: title?.trim() || null, expectedCount };
    const plan = await tauriApi.createOrganizationPlan(source.kind === "one_off"
      ? { ...common, version: 2, source }
      : { ...common, version: 1, source });
    if (!ownsMutation(get, requestEpoch, mutationToken)) return superseded(plan);
    set((current) => ({
      plans: replacePlan(current.plans, plan),
      activePlan: plan,
      groups: [],
      groupProjectionFingerprint: null,
      groupNextCursor: null,
      groupHasMore: false,
      dryRun: null,
      dryRunSelection: null,
      executionResult: null,
      isMutating: false,
      createPlanError: null
    }));
    await get().openPlan(plan.id);
    if (get().activePlan?.id !== plan.id) return superseded(plan);
    return applied(plan);
  } catch (error) {
    if (ownsMutation(get, requestEpoch, mutationToken)) {
      set({ isMutating: false, createPlanError: readableError(error) });
      throw error;
    }
    return superseded();
  }
}

export async function updateGroupDecision(
  { get, set }: Context,
  group: OrganizationPlanGroupSummary,
  decision: "accepted" | "kept" | "undecided",
  editedTargetTemplate?: string
): Promise<OrganizationMutationResult<void>> {
  const plan = get().activePlan;
  if (!plan || get().isExecutionInFlight) return superseded();
  const requestEpoch = get().requestEpoch;
  const mutationToken = get().mutationToken + 1;
  set((state) => takeGroupProjectionOwnership(state, { isMutating: true, mutationToken, error: null, dryRun: null, dryRunSelection: null }));
  try {
    await tauriApi.updateOrganizationPlanGroupDecision({
      planId: plan.id,
      groupId: group.groupId,
      expectedPlanRevision: plan.revision,
      expectedProjectionFingerprint: group.projectionFingerprint,
      expectedItemCount: group.itemCount,
      decision,
      ...(editedTargetTemplate ? { editedTargetTemplate } : {})
    });
    if (!ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) return superseded();
    await get().openPlan(plan.id);
    return applied();
  } catch (error) {
    if (ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) {
      set({ isMutating: false, error: readableError(error) });
      throw error;
    }
    return superseded();
  }
}

export async function updateDecision(
  { get, set }: Context,
  item: OrganizationPlanItem,
  decision: "accepted" | "kept" | "edited" | "undecided",
  editedFilename?: string
): Promise<OrganizationMutationResult<OrganizationPlan>> {
  const plan = get().activePlan;
  if (!plan || get().isExecutionInFlight) return superseded();
  const requestEpoch = get().requestEpoch;
  const mutationToken = get().mutationToken + 1;
  set((state) => takeGroupProjectionOwnership(state, { isMutating: true, mutationToken, error: null, dryRun: null, dryRunSelection: null }));
  try {
    const updatedPlan = await tauriApi.updateOrganizationPlanDecisions({
      planId: plan.id,
      expectedPlanRevision: plan.revision,
      mutations: [{ itemId: item.id, expectedItemRevision: item.revision, decision, editedFilename: editedFilename ?? null }]
    });
    if (!ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) return superseded(updatedPlan);
    const groupPage = await tauriApi.queryOrganizationPlanGroups({ planId: updatedPlan.id, pageSize: 100, cursor: null });
    if (!ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) return superseded(updatedPlan);
    if (!matchesGroupPage(groupPage, updatedPlan.id, updatedPlan.revision)) throw new Error("organization_group_page_stale");
    const projectedPlan = { ...updatedPlan, effectiveSummary: groupPage.effectiveSummary };
    set((state) => ({
      activePlan: projectedPlan,
      plans: replacePlan(state.plans, projectedPlan),
      groups: groupPage.groups,
      groupProjectionFingerprint: groupPage.projectionFingerprint,
      groupNextCursor: groupPage.nextCursor,
      groupHasMore: groupPage.hasMore,
      groupRequestEpoch: state.groupRequestEpoch + 1,
      isMutating: false
    }));
    return applied(updatedPlan);
  } catch (error) {
    if (ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) {
      set({ isMutating: false, error: readableError(error) });
      throw error;
    }
    return superseded();
  }
}

export async function refreshPlan({ get, set }: Context): Promise<OrganizationMutationResult<OrganizationPlan>> {
  const plan = get().activePlan;
  if (!plan || get().isExecutionInFlight) return superseded();
  const requestEpoch = get().requestEpoch;
  const mutationToken = get().mutationToken + 1;
  set((state) => takeGroupProjectionOwnership(state, { isMutating: true, mutationToken, error: null, dryRun: null, dryRunSelection: null, groupRequestEpoch: state.groupRequestEpoch + 1 }));
  try {
    const updated = await tauriApi.refreshOrganizationPlan({ planId: plan.id, expectedPlanRevision: plan.revision });
    if (!ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) return superseded(updated);
    set((state) => ({ activePlan: updated, plans: replacePlan(state.plans, updated), isMutating: false }));
    await get().openPlan(updated.id);
    return applied(updated);
  } catch (error) {
    if (ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) {
      set({ isMutating: false, error: readableError(error) });
      throw error;
    }
    return superseded();
  }
}

export async function analyzeMissing({ get, set }: Context, itemIds: string[] = [], oneOffConsent = false): Promise<OrganizationMutationResult<number>> {
  const state = get();
  const plan = state.activePlan;
  if (!plan) return superseded(0);
  if (state.isMutating || state.groupLoadingOwner?.kind === "open_plan") return superseded(0);
  const requestEpoch = state.requestEpoch;
  const mutationToken = state.mutationToken + 1;
  set((current) => takeGroupProjectionOwnership(current, { isMutating: true, mutationToken, error: null }));
  try {
    const result = await tauriApi.analyzeOrganizationPlanItems({ planId: plan.id, expectedPlanRevision: plan.revision, itemIds, ...(oneOffConsent ? { oneOffConsent: true } : {}) });
    if (!ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) return superseded(result.queuedCount);
    set({ isMutating: false });
    return applied(result.queuedCount);
  } catch (error) {
    if (ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) {
      set({ isMutating: false, error: readableError(error) });
      throw error;
    }
    return superseded(0);
  }
}

export async function cancelPlan({ get, set }: Context): Promise<OrganizationMutationResult<OrganizationPlan>> {
  const plan = get().activePlan;
  if (!plan || get().isExecutionInFlight || !isCancelableOrganizationPlan(plan.status)) return superseded();
  const requestEpoch = get().requestEpoch;
  const mutationToken = get().mutationToken + 1;
  set((state) => takeGroupProjectionOwnership(state, { isMutating: true, mutationToken, error: null }));
  try {
    const updated = await tauriApi.cancelOrganizationPlan({ planId: plan.id, expectedPlanRevision: plan.revision });
    if (!ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) return superseded(updated);
    set((state) => ({ activePlan: updated, plans: replacePlan(state.plans, updated) }));
    await get().openPlan(updated.id);
    return applied(updated);
  } catch (error) {
    if (ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) {
      set({ isMutating: false, error: readableError(error) });
      throw error;
    }
    return superseded();
  }
}

export { replacePlan };
