import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { makeTranslator } from "../src/i18n";

const read = (path: string) => readFileSync(resolve(path), "utf8");

const RULE_PROPOSAL_SCHEMA_MINIMUM_VERSION = 33;

const readRuleProposalSchemaVersion = (schema: string): number => {
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
  if (version < RULE_PROPOSAL_SCHEMA_MINIMUM_VERSION) {
    throw new Error(
      `Rule Proposal requires schema version ${RULE_PROPOSAL_SCHEMA_MINIMUM_VERSION} or later`
    );
  }

  return version;
};

const proposalCommands = [
  "create_rule_proposal",
  "regenerate_rule_proposal",
  "get_rule_proposal",
  "list_rule_proposals",
  "cancel_rule_proposal",
  "delete_rule_proposal",
  "replace_rule_proposal_candidate",
  "preview_rule_proposal",
  "resolve_rule_proposal_exact_impact",
  "apply_rule_proposal"
];

const repositoryCommands = [
  "get_rule_catalog_state",
  "list_user_rules_v2",
  "create_user_rule_v2",
  "update_user_rule_v2",
  "set_user_rule_enabled_v2",
  "delete_user_rule_v2",
  "execute_authoritative_rules_for_paths",
  "execute_rules_for_scope_v2"
];

describe("Task 07 natural-language Rule Proposal contracts", () => {
  it("keeps schema 33 additive and leaves files and operation journals untouched", () => {
    const schema = read("src-tauri/src/db/schema.rs");
    const section = schema.slice(
      schema.indexOf("fn ensure_rule_proposal_schema"),
      schema.indexOf("fn require_exact_table_columns")
    );
    expect(readRuleProposalSchemaVersion(schema)).toBeGreaterThanOrEqual(
      RULE_PROPOSAL_SCHEMA_MINIMUM_VERSION
    );
    expect(section).toContain("CREATE TABLE IF NOT EXISTS rule_proposals");
    expect(section).toContain("CREATE TABLE IF NOT EXISTS rule_catalog_state");
    expect(section).toContain("ALTER TABLE rules ADD COLUMN ast_version");
    expect(section).toContain("ALTER TABLE rules ADD COLUMN revision");
    expect(section).toContain("ALTER TABLE rules ADD COLUMN origin_proposal_id");
    expect(section).not.toContain("ALTER TABLE files");
    expect(section).not.toContain("ALTER TABLE operation_logs");
    expect(section).not.toContain("ALTER TABLE cleanup_");
  });

  it("keeps the Rule Proposal schema floor forward-compatible and rejects invalid versions", () => {
    const schemaWithVersion = (version: string) =>
      `pub(crate) const CURRENT_SCHEMA_VERSION: i32 = ${version};`;

    expect(readRuleProposalSchemaVersion(schemaWithVersion("39"))).toBe(39);
    expect(() => readRuleProposalSchemaVersion(schemaWithVersion("32"))).toThrow(
      /33 or later/
    );
    expect(() => readRuleProposalSchemaVersion(schemaWithVersion("not-a-number"))).toThrow(
      /integer literal/
    );
    expect(() => readRuleProposalSchemaVersion(schemaWithVersion("2147483648"))).toThrow(
      /fit in an i32/
    );
    expect(() => readRuleProposalSchemaVersion(schemaWithVersion("-2147483649"))).toThrow(
      /fit in an i32/
    );
    expect(() =>
      readRuleProposalSchemaVersion("const CURRENT_SCHEMA_VERSION: i32 = 38;")
    ).toThrow(/exactly one/);
    expect(() =>
      readRuleProposalSchemaVersion(
        `${schemaWithVersion("38")}\n${schemaWithVersion("39")}`
      )
    ).toThrow(/exactly one/);
  });

  it("exposes V2 and proposal commands only to the main window", () => {
    const main = JSON.parse(read("src-tauri/capabilities/default.json")) as { permissions: string[] };
    const search = JSON.parse(read("src-tauri/capabilities/search.json")) as { permissions: string[] };
    for (const command of [...proposalCommands, ...repositoryCommands]) {
      const permission = `allow-${command.replaceAll("_", "-")}`;
      expect(main.permissions).toContain(permission);
      expect(search.permissions).not.toContain(permission);
    }
    expect(main.permissions).not.toContain("allow-save-user-rule");
    expect(main.permissions).not.toContain("allow-delete-user-rule");
  });

  it("keeps renderer execution ID-scoped and rejects a renderer-owned Rule vector", () => {
    const api = read("src/api/rulesApi.ts");
    const engine = read("src-tauri/src/db/classification/engine.rs");
    const watcher = read("src/hooks/useFsWatcher.ts");
    const request = engine.match(
      /pub struct ExecuteRulesForScopeV2Request\s*\{(?<body>[\s\S]*?)\n\}/
    )?.groups?.body ?? "";
    expect(api).toContain('invokeCommand<RuleExecutionResultV2>("execute_rules_for_scope_v2"');
    expect(api).toContain('invokeCommand<RuleExecutionSummary>("execute_authoritative_rules_for_paths"');
    expect(engine).toContain("pub struct ExecuteRulesForScopeV2Request");
    expect(engine).toContain("load_enabled_persisted_rules");
    expect(request).not.toMatch(/\brules\s*:/);
    expect(watcher).not.toContain("rules: useRulesStore");
    expect(watcher).not.toContain("executeRulesForPaths");
    expect(watcher).toContain("executeAuthoritativeRulesForPaths");
  });

  it("keeps proposal generation bounded to the existing provider adapter and prompt text", () => {
    const adapter = read("src-tauri/src/rule_proposals.rs");
    const proposalRepo = read("src-tauri/src/db/queries/rule_proposals/mod.rs");
    const proposalProduction = proposalRepo.split("#[cfg(test)]")[0];
    expect(adapter).toContain("RULE_PROPOSAL_GENERATION_LIMIT: usize = 2");
    expect(adapter).toContain("provider_for_settings");
    expect(adapter).toContain("chat_json");
    expect(adapter).not.toContain("CREATE TABLE");
    expect(adapter).not.toContain("std::process::Command");
    expect(adapter).not.toContain("std::fs::read");
    expect(adapter).not.toContain("read_to_string");
    expect(proposalProduction).not.toContain("operation_logs");
    expect(proposalProduction).not.toContain("cleanup_operation_logs");
  });

  it("hydrates proposal and catalog truth from SQLite rather than localStorage", () => {
    const proposalStore = read("src/store/useRuleProposalStore.ts");
    const rulesStore = read("src/store/useRulesStore.ts");
    const persistence = read("src/hooks/useRulePersistence.ts");
    expect(proposalStore).toContain("listRuleProposals");
    expect(proposalStore).toContain("getRuleProposal");
    expect(rulesStore).not.toMatch(/(?:window\.|globalThis\.)?localStorage\.(?:getItem|setItem|removeItem)/);
    expect(persistence).toContain("listUserRulesV2");
    expect(persistence).toContain("getRuleCatalogState");
    expect(persistence).not.toMatch(/(?:window\.|globalThis\.)?localStorage\.(?:getItem|setItem|removeItem)/);
  });

  it("labels browser proposal behavior as mock without claiming native execution", () => {
    const mock = read("src/api/browserMockApi.ts");
    const workspace = read("src/views/rules/RuleProposalWorkspace.tsx");
    const i18n = read("src/i18n/dictionary.ts");
    expect(mock).toContain("browser-mock");
    expect(mock).toContain("MOCK deterministic proposal");
    expect(workspace).toContain('t("ruleProposalBrowserMock")');
    expect(workspace).not.toContain("const copy");
    expect(i18n).toContain("Browser preview uses a deterministic mock");
    expect(i18n).toContain("不代表真实 AI 或原生持久化");
  });

  it("keeps generation, validation, impact, and Apply review accessible", () => {
    const workspace = read("src/views/rules/RuleProposalWorkspace.tsx");
    const en = makeTranslator("en");
    expect(workspace).toContain("<textarea");
    expect(workspace).toContain('aria-live="polite"');
    expect(workspace).toContain("ConfirmDialog");
    expect(workspace).toContain('t("ruleProposalApply")');
    expect(en("ruleProposalApply")).toBe("Apply as disabled rule");
    expect(en("ruleProposalApplied")).toContain("currently disabled");
    expect(en("ruleProposalApplySafety")).toContain("remain separate");
  });
});
