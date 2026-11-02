import type { BrowseEntryRef, BrowsePathRef } from "../types/fileWorkspace";
import { invokeCommand } from "./core";
import type {
  ExecuteOrganizationPlanResult,
  OrganizationPlan,
  OrganizationPlanDryRun,
  OrganizationPlanGroupItemPage,
  OrganizationPlanGroupPage,
  OrganizationPlanItemPage,
  UpdateOrganizationPlanGroupDecisionResult,
  LibrarySelectionV1
} from "../types/domain";

export interface OneOffOrganizationSourceV1 {
  kind: "one_off";
  version: 1;
  sessionId: string;
  pathRef: BrowsePathRef;
  entries: BrowseEntryRef[];
}
export type OrganizationPlanSource = LibrarySelectionV1 | OneOffOrganizationSourceV1;
export type CreateOrganizationPlanRequest =
  | { version: 1; requestId: string; title?: string | null; source: LibrarySelectionV1; expectedCount?: number | null }
  | { version: 2; requestId: string; title?: string | null; source: OneOffOrganizationSourceV1; expectedCount: number };

export const organizationApi = {
  createOrganizationPlan(request: CreateOrganizationPlanRequest): Promise<OrganizationPlan> {
    return invokeCommand<OrganizationPlan>("create_organization_plan", { request });
  },
  listOrganizationPlans(): Promise<OrganizationPlan[]> {
    return invokeCommand<OrganizationPlan[]>("list_organization_plans");
  },
  getOrganizationPlan(planId: string): Promise<OrganizationPlan> {
    return invokeCommand<OrganizationPlan>("get_organization_plan", { planId });
  },
  queryOrganizationPlanItems(request: { planId: string; cursor?: string | null; pageSize: number }): Promise<OrganizationPlanItemPage> {
    return invokeCommand<OrganizationPlanItemPage>("query_organization_plan_items", { request });
  },
  queryOrganizationPlanGroups(request: { planId: string; cursor?: string | null; pageSize: number }): Promise<OrganizationPlanGroupPage> {
    return invokeCommand<OrganizationPlanGroupPage>("query_organization_plan_groups", { request });
  },
  queryOrganizationPlanGroupItems(request: { planId: string; groupId: string; cursor?: string | null; expectedProjectionFingerprint: string; pageSize: number }): Promise<OrganizationPlanGroupItemPage> {
    return invokeCommand<OrganizationPlanGroupItemPage>("query_organization_plan_group_items", { request });
  },
  updateOrganizationPlanDecisions(request: { planId: string; expectedPlanRevision: number; safeBatch?: boolean; mutations: Array<{ itemId: string; expectedItemRevision: number; decision: "accepted" | "kept" | "edited" | "undecided"; editedFilename?: string | null }> }): Promise<OrganizationPlan> {
    return invokeCommand<OrganizationPlan>("update_organization_plan_decisions", { request });
  },
  updateOrganizationPlanGroupDecision(request: { planId: string; groupId: string; expectedPlanRevision: number; expectedProjectionFingerprint: string; expectedItemCount: number; decision: "accepted" | "kept" | "undecided"; editedTargetTemplate?: string }): Promise<UpdateOrganizationPlanGroupDecisionResult> {
    return invokeCommand<UpdateOrganizationPlanGroupDecisionResult>("update_organization_plan_group_decision", { request });
  },
  refreshOrganizationPlan(request: { planId: string; expectedPlanRevision: number }): Promise<OrganizationPlan> {
    return invokeCommand<OrganizationPlan>("refresh_organization_plan", { request });
  },
  cancelOrganizationPlan(request: { planId: string; expectedPlanRevision: number }): Promise<OrganizationPlan> {
    return invokeCommand<OrganizationPlan>("cancel_organization_plan", { request });
  },
  deleteOrganizationPlan(request: { planId: string; expectedPlanRevision: number; confirmed: boolean }): Promise<boolean> {
    return invokeCommand<boolean>("delete_organization_plan", { request });
  },
  analyzeOrganizationPlanItems(request: { planId: string; expectedPlanRevision: number; itemIds?: string[]; oneOffConsent?: boolean }): Promise<{ planId: string; queuedCount: number; requiresRefresh: boolean }> {
    return invokeCommand("analyze_organization_plan_items", { request });
  },
  getOrganizationPlanDryRun(request: { planId: string; expectedPlanRevision: number; itemIds?: string[]; allAccepted: boolean }): Promise<OrganizationPlanDryRun> {
    return invokeCommand<OrganizationPlanDryRun>("get_organization_plan_dry_run", { request });
  },
  executeOrganizationPlan(request: { planId: string; expectedPlanRevision: number; dryRunFingerprint: string; itemIds?: string[]; allAccepted: boolean; confirmed: boolean }): Promise<ExecuteOrganizationPlanResult> {
    return invokeCommand<ExecuteOrganizationPlanResult>("execute_organization_plan", { request });
  }
};

export type OrganizationApi = typeof organizationApi;
