import { useOperationQueueStore } from "../useOperationQueueStore";
import { tauriApi } from "../../api/tauriApi";
import type {
  ExecuteOrganizationPlanResult,
  OrganizationPlanDryRun,
  OrganizationPlanSelection
} from "../../types/domain";
import { readableError } from "../../utils/viewHelpers";
import type { OrganizationMutationResult, OrganizationPlanState } from "../useOrganizationPlanStore";
import { ownsPlanMutation, takeGroupProjectionOwnership } from "./ownership";
import { replacePlan } from "./planMutations";

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

export async function createDryRun(
  { get, set }: Context,
  selection: OrganizationPlanSelection = { allAccepted: true, itemIds: [] }
): Promise<OrganizationMutationResult<OrganizationPlanDryRun>> {
  const state = get();
  const plan = state.activePlan;
  if (!plan && state.groupLoadingOwner?.kind === "open_plan") return superseded();
  if (!plan) throw new Error("organization_plan_not_selected");
  if (!selection.allAccepted && selection.itemIds.length === 0) throw new Error("organization_selection_required");
  if (state.isMutating || state.groupLoadingOwner?.kind === "open_plan") return superseded();
  const persistedSelection: OrganizationPlanSelection = selection.allAccepted
    ? { allAccepted: true, itemIds: [] }
    : { allAccepted: false, itemIds: [...selection.itemIds] as [string, ...string[]] };
  const requestEpoch = state.requestEpoch;
  const mutationToken = state.mutationToken + 1;
  set((current) => takeGroupProjectionOwnership(current, { isMutating: true, mutationToken, error: null, dryRun: null, dryRunSelection: null }));
  try {
    const dryRun = await tauriApi.getOrganizationPlanDryRun({ planId: plan.id, expectedPlanRevision: plan.revision, itemIds: persistedSelection.itemIds, allAccepted: persistedSelection.allAccepted });
    if (!ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) return superseded(dryRun);
    set({ dryRun, dryRunSelection: persistedSelection, isMutating: false });
    return applied(dryRun);
  } catch (error) {
    if (ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) {
      set({ isMutating: false, error: readableError(error) });
      throw error;
    }
    return superseded();
  }
}

export async function executeDryRun({ get, set }: Context): Promise<OrganizationMutationResult<ExecuteOrganizationPlanResult>> {
  const { activePlan: plan, dryRun, dryRunSelection } = get();
  if (!plan || !dryRun) throw new Error("organization_dry_run_required");
  if (!dryRunSelection) throw new Error("organization_dry_run_selection_required");
  if (get().isExecutionInFlight) return superseded();
  if (useOperationQueueStore.getState().activeOperationKind) throw new Error("operation_already_running");
  useOperationQueueStore.setState({ activeOperationKind: "execute", isOperationCanceling: false, operationProgress: null });
  const requestEpoch = get().requestEpoch;
  const mutationToken = get().mutationToken + 1;
  set((state) => takeGroupProjectionOwnership(state, { isMutating: true, isExecutionInFlight: true, mutationToken, error: null }));
  try {
    const result = await tauriApi.executeOrganizationPlan({ planId: plan.id, expectedPlanRevision: dryRun.planRevision, dryRunFingerprint: dryRun.dryRunFingerprint, itemIds: dryRunSelection.itemIds, allAccepted: dryRunSelection.allAccepted, confirmed: true });
    if (!ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) return superseded(result);
    set((state) => ({ executionResult: result, activePlan: result.plan, plans: replacePlan(state.plans, result.plan), dryRun: null, dryRunSelection: null, isMutating: false, isExecutionInFlight: false }));
    const refreshEpoch = get().requestEpoch + 1;
    await get().openPlan(result.plan.id);
    if (get().requestEpoch === refreshEpoch && get().mutationToken === mutationToken && get().activePlan?.id === result.plan.id) set({ executionResult: result });
    return applied(result);
  } catch (error) {
    if (ownsPlanMutation(get, plan.id, plan.revision, requestEpoch, mutationToken)) {
      set({ isMutating: false, isExecutionInFlight: false, error: readableError(error) });
      throw error;
    }
    return superseded();
  } finally {
    useOperationQueueStore.setState({ activeOperationKind: null, isOperationCanceling: false, operationProgress: null });
  }
}
