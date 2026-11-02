import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(path), "utf8");

const ORGANIZATION_PLAN_SCHEMA_MINIMUM_VERSION = 32;

const readSchemaVersionAtLeast = (
  schema: string,
  minimum: number,
  feature: string
): number => {
  const declarations = [
    ...schema.matchAll(
      /^[ \t]*pub\(crate\)[ \t]+const[ \t]+CURRENT_SCHEMA_VERSION:[ \t]*i32[ \t]*=[ \t]*([^;\r\n]*)[ \t]*;[ \t]*$/gm
    )
  ];
  if (declarations.length !== 1) {
    throw new Error("Expected exactly one CURRENT_SCHEMA_VERSION i32 declaration");
  }

  const literal = declarations[0]?.[1];
  if (literal === undefined || !/^[+-]?\d(?:_?\d)*$/.test(literal)) {
    throw new Error("CURRENT_SCHEMA_VERSION must be an integer literal");
  }

  const version = Number(literal.replaceAll("_", ""));
  if (
    !Number.isSafeInteger(version) ||
    version < -2_147_483_648 ||
    version > 2_147_483_647
  ) {
    throw new Error("CURRENT_SCHEMA_VERSION must fit in an i32");
  }
  if (version < minimum) {
    throw new Error(`${feature} requires schema version ${minimum} or later`);
  }

  return version;
};

describe("Task 06 durable Organization Plan contracts", () => {
  it("retains the schema 32 plan ledger and small-table revisions after later migrations", () => {
    const schema = read("src-tauri/src/db/schema.rs");
    const section = schema.slice(
      schema.indexOf("fn ensure_organization_plan_schema"),
      schema.indexOf("fn ensure_journal_state_triggers")
    );
    expect(
      readSchemaVersionAtLeast(
        schema,
        ORGANIZATION_PLAN_SCHEMA_MINIMUM_VERSION,
        "Organization Plan"
      )
    ).toBeGreaterThanOrEqual(ORGANIZATION_PLAN_SCHEMA_MINIMUM_VERSION);
    expect(section).toContain("CREATE TABLE IF NOT EXISTS organization_plans");
    expect(section).toContain("CREATE TABLE IF NOT EXISTS organization_plan_items");
    expect(section).toContain("ALTER TABLE user_tags ADD COLUMN revision");
    expect(section).toContain("ALTER TABLE library_saved_views ADD COLUMN revision");
    expect(section).not.toContain("ALTER TABLE files");
    expect(section).not.toContain("ALTER TABLE operation_logs");
    expect(section).not.toContain("CREATE TABLE operation_logs");
    expect(section).not.toContain("CREATE TABLE managed_ai");
  });

  it("keeps the schema-version check valid for later migrations and rejects invalid declarations", () => {
    const schemaWithVersion = (version: string) =>
      `pub(crate) const CURRENT_SCHEMA_VERSION: i32 = ${version};`;
    const feature = "Organization Plan";

    expect(
      readSchemaVersionAtLeast(schemaWithVersion("39"), ORGANIZATION_PLAN_SCHEMA_MINIMUM_VERSION, feature)
    ).toBe(39);
    expect(() =>
      readSchemaVersionAtLeast(schemaWithVersion("31"), ORGANIZATION_PLAN_SCHEMA_MINIMUM_VERSION, feature)
    ).toThrow(/schema version 32 or later/);
    expect(() => readSchemaVersionAtLeast("", ORGANIZATION_PLAN_SCHEMA_MINIMUM_VERSION, feature)).toThrow(
      /exactly one/
    );
    expect(() =>
      readSchemaVersionAtLeast(
        `${schemaWithVersion("38")}\n${schemaWithVersion("39")}`,
        ORGANIZATION_PLAN_SCHEMA_MINIMUM_VERSION,
        feature
      )
    ).toThrow(/exactly one/);
    for (const invalid of ["not-a-number", "2147483648", "-2147483649"]) {
      expect(() =>
        readSchemaVersionAtLeast(schemaWithVersion(invalid), ORGANIZATION_PLAN_SCHEMA_MINIMUM_VERSION, feature)
      ).toThrow();
    }
  });

  it("keeps plan execution on the existing authoritative preview and operation journal", () => {
    const organization =
      read("src-tauri/src/db/queries/organization/mod.rs") +
      read("src-tauri/src/db/queries/organization/queries.rs");
    const fileOps = read("src-tauri/src/file_ops.rs");
    expect(organization).toContain("operation_preview_from_indexed");
    expect(organization).toContain("authoritative_preview_id");
    expect(organization).toContain("active_operation_batch_id");
    expect(organization).toContain("operation_logs");
    expect(fileOps).toContain("execute_canonical_operations");
    expect(organization).not.toContain("move_to_trash");
    expect(organization).not.toContain("std::fs::read");
    expect(organization).not.toContain("read_to_string");
    expect(organization).not.toContain("std::process::Command");
  });

  it("reuses the managed AI queue and exposes only ID-based main-window commands", () => {
    const organization = read("src-tauri/src/db/queries/organization/mod.rs");
    const repository = read("src-tauri/src/global_index/repository.rs");
    const commands = read("src-tauri/src/db/commands.rs");
    const search = JSON.parse(read("src-tauri/capabilities/search.json")) as { permissions: string[] };
    expect(organization).toContain("enqueue_managed_ai_for_library_files");
    expect(repository).toContain("enqueue_ai_jobs_for_entry");
    expect(organization).not.toContain("CREATE TABLE");
    expect(commands).toContain("require_main_window");
    expect(commands).not.toMatch(/ExecuteOrganizationPlanRequest[\s\S]{0,300}(source_path|target_path)/);
    expect(search.permissions.some((permission) => permission.includes("organization-plan"))).toBe(false);
    expect(search.permissions).not.toContain("allow-resolve-file-library-exact-count-v2");
  });

  it("hydrates review state from the backend and keeps browser execution honest", () => {
    const view = read("src/views/organize/OrganizeSuggestionsView.tsx");
    const store = read("src/store/useOrganizationPlanStore.ts");
    const planList = read("src/store/organizationPlan/planListController.ts");
    const pagination = read("src/store/organizationPlan/groupPagination.ts");
    const mock = read("src/api/browserMockApi.ts");
    expect(view).toContain("useOrganizationPlanStore");
    expect(view).toContain("useVirtualizer");
    expect(view).toContain("ConfirmDialog");
    expect(store).not.toContain("queryOrganizationPlanItems");
    expect(store).not.toContain("loadNextPage:");
    expect(store).not.toContain("updateBatch:");
    expect(planList).toContain("queryOrganizationPlanGroups");
    expect(pagination).toContain("queryOrganizationPlanGroups");
    expect(store).toContain("requestEpoch");
    expect(store).not.toContain("localStorage");
    expect(view).not.toContain("useOrganizeDecisionStore");
    expect(view).not.toContain("useOperationQueueStore");
    expect(mock).toContain("browser_mock_native_execution_unavailable");
  });

  it("projects complete Plan groups without adding a second ledger or Schema 35", () => {
    const organization =
      read("src-tauri/src/db/queries/organization/mod.rs") +
      read("src-tauri/src/db/queries/organization/queries.rs");
    const domain = read("src/types/domain.ts");
    const api = read("src/api/organizationApi.ts");
    const store = read("src/store/useOrganizationPlanStore.ts");
    const controllers = [
      read("src/store/organizationPlan/groupPagination.ts"),
      read("src/store/organizationPlan/planMutations.ts"),
      read("src/store/organizationPlan/execution.ts")
    ].join("\n");
    expect(organization).toContain("pub struct OrganizationPlanGroupSummaryDto");
    expect(organization).toContain("pub fn query_organization_plan_groups");
    expect(organization).toContain("pub fn query_organization_plan_group_items");
    expect(organization).toContain("pub fn update_organization_plan_group_decision");
    expect(organization).toContain("organization_group_action_not_available");
    expect(organization).toContain("organization_group_changed");
    expect(organization).toContain("expected_projection_fingerprint");
    expect(organization).toContain("expected_item_count");
    expect(organization).toContain("projection_fingerprint");
    expect(organization).toContain("can_accept_all");
    expect(organization).not.toContain("load_organization_plan_group_summaries(");
    expect(organization).toContain("ORGANIZATION_GROUP_SAMPLE_MAX");
    expect(organization).not.toContain("CREATE TABLE");
    expect(organization).not.toContain("schema 35");
    expect(domain).toContain("export interface OrganizationPlanGroupSummary");
    expect(domain).toContain("projectionFingerprint");
    expect(domain).toContain("canAcceptAll");
    expect(domain).not.toContain("groupNextCursor");
    expect(api).toContain('"query_organization_plan_groups"');
    expect(api).toContain('"query_organization_plan_group_items"');
    expect(api).toContain('"update_organization_plan_group_decision"');
    expect(controllers).toContain("queryOrganizationPlanGroups");
    expect(controllers).toContain("updateGroupDecision");
    expect(controllers).toContain("expectedProjectionFingerprint");
    expect(store).not.toContain("items.reduce");
  });
});
