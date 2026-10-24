import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

type CommandClass =
  | "READ_ONLY"
  | "MAIN_WINDOW_MUTATION"
  | "MAIN_WINDOW_DIAGNOSTIC_MUTATION"
  | "SEARCH_WINDOW_LIFECYCLE"
  | "EXPLICITLY_SHARED_READ";

type GuardExpectation =
  | "none"
  | "require_main_window"
  | "require_search_window"
  | "search_or_main_window";

type CommandContract = {
  command: string;
  source: string;
  class: CommandClass;
  guard: GuardExpectation;
};

const appControlSource = readFileSync(resolve("src-tauri/src/app_control.rs"), "utf8");
const mainSource = readFileSync(resolve("src-tauri/src/main.rs"), "utf8");
const buildSource = readFileSync(resolve("src-tauri/build.rs"), "utf8");
const matrixSource = readFileSync(
  resolve("docs/security/TAURI_COMMAND_PERMISSION_MATRIX.md"),
  "utf8",
);
const databaseBootstrapperSource = readFileSync(
  resolve("src/components/DatabaseBootstrapper.tsx"),
  "utf8",
);
const dbCommandsSource = readFileSync(resolve("src-tauri/src/db/commands.rs"), "utf8");
const fileWorkspaceCommandsSource = readFileSync(
  resolve("src-tauri/src/file_workspace/integration/commands.rs"),
  "utf8",
);
const mainCapability = JSON.parse(
  readFileSync(resolve("src-tauri/capabilities/default.json"), "utf8"),
) as { permissions: string[] };
const searchCapability = JSON.parse(
  readFileSync(resolve("src-tauri/capabilities/search.json"), "utf8"),
) as { permissions: string[] };

const source = (relativePath: string) => resolve(relativePath);

function groupedContracts(
  className: CommandClass,
  guard: GuardExpectation,
  relativePath: string,
  commands: readonly string[],
): CommandContract[] {
  return commands.map((command) => ({
    command,
    source: relativePath,
    class: className,
    guard,
  }));
}

// This is the authorization metadata, not a name-based heuristic. Every command
// is listed with its real command source, category, and expected window boundary.
// A command omitted here is a contract failure; it is never treated as read-only
// by default.
const explicitContracts: CommandContract[] = [
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/db/automation/commands.rs", ["create_automation_intent", "update_automation_intent", "set_automation_intent_enabled", "archive_automation_intent", "run_automation_intent_manual"]),
  ...groupedContracts("READ_ONLY", "require_main_window", "src-tauri/src/db/automation/commands.rs", ["list_automation_intents", "get_automation_intent", "list_automation_runs"]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/db/commands.rs", [
    "init_db",
    "insert_file",
    "remove_files_by_paths",
    "upsert_files_by_paths",
    "create_user_tag",
    "update_user_tag",
    "delete_user_tag",
    "mutate_file_user_tags",
    "create_library_saved_view",
    "update_library_saved_view",
    "delete_library_saved_view",
    "create_organization_plan",
    "update_organization_plan_decisions",
    "update_organization_plan_group_decision",
    "refresh_organization_plan",
    "cancel_organization_plan",
    "delete_organization_plan",
    "analyze_organization_plan_items",
    "execute_organization_plan",
    "create_user_rule_v2",
    "update_user_rule_v2",
    "set_user_rule_enabled_v2",
    "delete_user_rule_v2",
    "execute_rules_for_scope_v2",
    "execute_authoritative_rules_for_paths",
  ]),
  ...groupedContracts(
    "MAIN_WINDOW_MUTATION",
    "require_main_window",
    "src-tauri/src/db/learning.rs",
    ["confirm_classification", "correct_classification"],
  ),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/global_index/commands.rs", [
    "start_global_index",
    "pause_global_index",
    "resume_global_index",
    "rebuild_global_index_source",
    "set_global_index_source_enabled",
    "add_managed_scope",
    "remove_managed_scope",
    "update_managed_scope_policy",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/content/commands.rs", [
    "set_content_scope_policy",
    "start_content_run",
    "cancel_content_run",
    "rebuild_content_artifact",
    "delete_content_artifact",
    "purge_content_scope",
    "understand_content_artifacts",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/rule_proposals.rs", [
    "create_rule_proposal",
    "regenerate_rule_proposal",
    "cancel_rule_proposal",
    "delete_rule_proposal",
    "replace_rule_proposal_candidate",
    "apply_rule_proposal",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/settings.rs", [
    "save_settings",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/ai/settings.rs", [
    "save_ai_settings",
  ]),
  ...groupedContracts(
    "MAIN_WINDOW_DIAGNOSTIC_MUTATION",
    "require_main_window",
    "src-tauri/src/ai/trace.rs",
    ["clear_ai_request_traces"],
  ),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/ai/classification.rs", [
    "classify_files_with_ai",
    "classify_selected_files_with_ai",
    "cancel_ai_classification",
  ]),
  ...groupedContracts(
    "MAIN_WINDOW_DIAGNOSTIC_MUTATION",
    "require_main_window",
    "src-tauri/src/ai/debug.rs",
    ["debug_ai_classification_once"],
  ),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/ai/cleanup.rs", [
    "analyze_cleanup_candidates_with_ai",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/app_control.rs", [
    "quit_app",
    "enter_background",
    "mark_main_window_ready",
    "acknowledge_main_window_ready",
    "acknowledge_search_navigation",
    "register_global_search_hotkey",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/scanner.rs", [
    "start_managed_scan",
    "cancel_scan_run",
    "retry_interrupted_scan",
    "scan_directory",
    "cancel_scan",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/dedupe.rs", [
    "cancel_dedupe",
    "start_dedupe_run",
    "retry_dedupe_run",
    "cancel_dedupe_run",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/analysis.rs", [
    "start_analysis_run",
    "cancel_analysis_run",
    "retry_analysis_run",
    "set_analysis_finding_decision",
    "revalidate_analysis_finding",
  ]),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/file_ops.rs", [
    "execute_moves",
    "materialize_provider_preview",
    "restore_moves",
    "resolve_operation_recovery",
    "cancel_operations",
  ]),
  ...groupedContracts(
    "MAIN_WINDOW_MUTATION",
    "require_main_window",
    "src-tauri/src/file_workspace/integration/commands.rs",
    [
      "file_workspace_browse_open",
      "file_workspace_browse_restore",
      "file_workspace_location_browse",
      "file_workspace_browse_start_enumeration",
      "file_workspace_browse_next_page",
      "file_workspace_browse_cancel_enumeration",
      "file_workspace_browse_release_page",
      "file_workspace_browse_release_path",
      "file_workspace_browse_retain_path",
      "file_workspace_browse_dispose",
      "file_workspace_change_start",
      "file_workspace_change_refresh",
      "file_workspace_change_dispose",
      "file_workspace_thumbnail_request",
      "file_workspace_thumbnail_cancel",
      "file_workspace_preview_create",
      "file_workspace_preview_start",
      "file_workspace_preview_cancel",
      "file_workspace_preview_dispose",
      "file_workspace_preview_switch_source",
      "file_workspace_preview_asset_request",
    ],
  ),
  ...groupedContracts("MAIN_WINDOW_MUTATION", "require_main_window", "src-tauri/src/storage_analyzer.rs", [
    "start_storage_cleanup_scan",
    "cancel_storage_cleanup_scan",
    "move_cleanup_candidates_to_safe_trash",
    "restore_cleanup_trash_items",
    "cancel_cleanup_restore",
  ]),
  ...groupedContracts("SEARCH_WINDOW_LIFECYCLE", "search_or_main_window", "src-tauri/src/app_control.rs", [
    "activate_search_result",
  ]),
  ...groupedContracts("SEARCH_WINDOW_LIFECYCLE", "require_search_window", "src-tauri/src/app_control.rs", [
    "search_window_ready",
    "resize_search_window",
    "hide_search_window_command",
  ]),
  ...groupedContracts("SEARCH_WINDOW_LIFECYCLE", "none", "src-tauri/src/app_control.rs", [
    "get_search_window_state",
  ]),
  ...groupedContracts("EXPLICITLY_SHARED_READ", "none", "src-tauri/src/global_index/commands.rs", [
    "search_global_entries",
    "open_global_search_result",
    "reveal_global_search_result",
  ]),
  ...groupedContracts("EXPLICITLY_SHARED_READ", "none", "src-tauri/src/settings.rs", [
    "get_settings",
  ]),
  ...groupedContracts("EXPLICITLY_SHARED_READ", "none", "src-tauri/src/runtime_capabilities.rs", [
    "get_runtime_capabilities",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/db/commands.rs", [
    "search_files",
    "get_paged_files",
    "get_operation_previews_for_scope",
    "get_operation_previews_by_file_ids",
    "get_manual_rename_operation_preview",
    "get_permanent_delete_operation_preview",
    "get_operation_previews_for_selection",
    "get_stats_summary",
    "get_operation_logs",
    "get_rule_catalog_state",
    "list_user_rules_v2",
  ]),
  ...groupedContracts("READ_ONLY", "require_main_window", "src-tauri/src/db/commands.rs", [
    "query_file_library_v2",
    "resolve_file_library_exact_count_v2",
    "get_file_library_detail",
    "get_file_library_selection_summary",
    "reveal_file_library_entry",
    "list_user_tags",
    "list_library_saved_views",
    "list_organization_plans",
    "get_organization_plan",
    "query_organization_plan_items",
    "query_organization_plan_groups",
    "query_organization_plan_group_items",
    "get_organization_plan_dry_run",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/global_index/commands.rs", [
    "get_global_index_status",
    "list_global_index_sources",
    "list_managed_scopes",
    "get_ai_management_status",
  ]),
  ...groupedContracts("READ_ONLY", "require_main_window", "src-tauri/src/content/commands.rs", [
    "get_content_scope_policy",
    "get_content_catalog_revision",
    "preview_content",
    "get_content_run",
    "list_content_runs",
    "get_active_content_run_for_file",
    "query_content_run_items",
    "get_content_artifact",
    "query_content_artifacts",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/ai/settings.rs", [
    "get_ai_settings",
    "list_ai_provider_presets",
    "test_ai_provider_connection",
    "list_ai_models",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/ai/readiness.rs", [
    "get_ai_feature_readiness",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/ai/trace.rs", [
    "list_ai_request_traces",
    "export_ai_request_traces",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/app_control.rs", [
    "get_global_hotkey_status",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/scanner.rs", [
    "get_managed_scan_snapshot",
    "get_scan_run",
    "list_scan_runs",
    "list_scan_roots",
    "get_scan_root_health",
    "create_scan_job_id",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/dedupe.rs", [
    "get_dedupe_run",
    "list_dedupe_runs",
    "get_active_dedupe_run",
    "list_duplicate_groups",
    "get_duplicate_group",
    "list_duplicate_group_members",
    "get_file_duplicate_membership",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/analysis.rs", [
    "list_analysis_detectors",
    "get_analysis_run",
    "get_active_analysis_run",
    "list_analysis_runs",
    "list_analysis_run_detectors",
    "list_analysis_findings",
    "get_analysis_finding",
    "list_analysis_finding_evidence",
    "get_dedupe_authority",
  ]),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/file_ops.rs", [
    "reveal_in_folder",
  ]),
  ...groupedContracts("READ_ONLY", "require_main_window", "src-tauri/src/file_ops.rs", [
    "request_macos_thumbnail",
    "cancel_macos_thumbnail",
  ]),
  ...groupedContracts(
    "READ_ONLY",
    "require_main_window",
    "src-tauri/src/file_workspace/integration/commands.rs",
    [
      "file_workspace_location_list",
      "file_workspace_change_pending",
      "file_workspace_read_eligibility",
      "file_workspace_preview_snapshot",
    ],
  ),
  ...groupedContracts("READ_ONLY", "none", "src-tauri/src/storage_analyzer.rs", [
    "get_storage_cleanup_scan_status",
    "get_storage_cleanup_candidate_page",
    "reveal_storage_candidate",
    "preview_cleanup_candidates",
    "preview_cleanup_operations",
    "list_cleanup_trash_batches",
    "preview_restore_cleanup_trash",
  ]),
  ...groupedContracts("READ_ONLY", "require_main_window", "src-tauri/src/rule_proposals.rs", [
    "get_rule_proposal",
    "list_rule_proposals",
    "preview_rule_proposal",
    "resolve_rule_proposal_exact_impact",
  ]),
];

function handlerCommands(): string[] {
  const handlerBlock = mainSource.match(/generate_handler!\[([\s\S]*?)\]\)/)?.[1] ?? "";
  return [...handlerBlock.matchAll(/zen_canvas_tauri::(?:[a-z0-9_]+::)+([a-z0-9_]+)/g)].map(
    (match) => match[1],
  );
}

function manifestCommands(): string[] {
  const commandsBlock = buildSource.match(/const COMMANDS:[\s\S]*?= &\[([\s\S]*?)\];/)?.[1] ?? "";
  return [...commandsBlock.matchAll(/"([a-z0-9_]+)"/g)].map((match) => match[1]);
}

function permissionFor(command: string): string {
  return `allow-${command.replaceAll("_", "-")}`;
}

function matrixCommands(): string[] {
  return [...matrixSource.matchAll(/^\| `([a-z0-9_]+)` \|/gm)].map((match) => match[1]);
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function matchingBrace(sourceText: string, openIndex: number): number {
  let depth = 0;
  let quote: "'" | '"' | null = null;
  let lineComment = false;
  let blockComment = false;
  let rawDelimiter: string | null = null;

  for (let index = openIndex; index < sourceText.length; index += 1) {
    const character = sourceText[index];
    const next = sourceText[index + 1];

    if (lineComment) {
      if (character === "\n") lineComment = false;
      continue;
    }
    if (blockComment) {
      if (character === "*" && next === "/") {
        blockComment = false;
        index += 1;
      }
      continue;
    }
    if (rawDelimiter) {
      if (sourceText.startsWith(rawDelimiter, index)) {
        index += rawDelimiter.length - 1;
        rawDelimiter = null;
      }
      continue;
    }
    if (quote) {
      if (character === "\\") {
        index += 1;
      } else if (character === quote) {
        quote = null;
      }
      continue;
    }
    if (character === "/" && next === "/") {
      lineComment = true;
      index += 1;
      continue;
    }
    if (character === "/" && next === "*") {
      blockComment = true;
      index += 1;
      continue;
    }
    if (character === "r" && (next === '"' || next === "#")) {
      const rawStart = sourceText.slice(index).match(/^r(#+)?"/);
      if (rawStart) {
        const hashes = rawStart[1] ?? "";
        rawDelimiter = `"${hashes}`;
        index += rawStart[0].length - 1;
        continue;
      }
    }
    if (character === '"' || character === "'") {
      quote = character;
      continue;
    }
    if (character === "{") depth += 1;
    if (character === "}" && --depth === 0) return index;
  }
  return -1;
}

function functionBody(sourceText: string, command: string): { signature: string; body: string } | null {
  const commandFunction = new RegExp(
    `#\\[(?:tauri::)?command\\](?:\\s*#\\[[^\\]]+\\])*\\s*(?:pub\\s+)?(?:async\\s+)?fn\\s+${escapeRegExp(command)}\\b`,
    "g",
  );
  const functionMatch = commandFunction.exec(sourceText);
  if (!functionMatch || functionMatch.index === undefined) return null;
  const functionStart = functionMatch.index + functionMatch[0].lastIndexOf("fn ");
  const openIndex = sourceText.indexOf("{", functionMatch.index + functionMatch[0].length);
  if (openIndex < 0) return null;
  const closeIndex = matchingBrace(sourceText, openIndex);
  if (closeIndex < 0) return null;
  return {
    signature: sourceText.slice(functionStart, openIndex),
    body: sourceText.slice(openIndex + 1, closeIndex),
  };
}

function readSource(relativePath: string): string {
  return readFileSync(source(relativePath), "utf8");
}

function contractsForManifest(manifestCommands: readonly string[]): CommandContract[] {
  if (new Set(explicitContracts.map((contract) => contract.command)).size !== explicitContracts.length) {
    throw new Error("Duplicate Tauri command contract");
  }
  const explicitByCommand = new Map(explicitContracts.map((contract) => [contract.command, contract]));
  const missing = manifestCommands.filter((command) => !explicitByCommand.has(command));
  if (missing.length > 0) {
    throw new Error(`Unclassified Tauri commands: ${missing.join(", ")}`);
  }
  const contracts = manifestCommands.map((command) => explicitByCommand.get(command)!);
  if (new Set(contracts.map((contract) => contract.command)).size !== contracts.length) {
    throw new Error("Duplicate Tauri command contract");
  }
  return contracts;
}

function assertMainWindowMutationContract(
  contract: CommandContract,
  sourceText: string,
): void {
  const parsed = functionBody(sourceText, contract.command);
  if (!parsed) throw new Error(`${contract.command} function body`);
  if (!/\b(?:WebviewWindow|Window)\s*</.test(parsed.signature)) {
    throw new Error(`${contract.command} window parameter`);
  }
  const guardIndex = parsed.body.indexOf("require_main_window");
  if (guardIndex < 0) throw new Error(`${contract.command} guard`);
  const prefix = parsed.body.slice(0, guardIndex).trim();
  if (prefix !== "" && !/^if\s*$/.test(prefix)) {
    throw new Error(`${contract.command} guard ordering`);
  }
}

const manifest = manifestCommands();
const handler = handlerCommands();
const contracts = contractsForManifest(manifest);
const contractByCommand = new Map(contracts.map((contract) => [contract.command, contract]));

describe("Tauri command permission contract", () => {
  it("keeps AppManifest and invoke_handler command sets identical", () => {
    expect(manifest).toEqual(handler);
    expect(new Set(manifest).size).toBe(manifest.length);
  });

  it("has explicit metadata for every registered command", () => {
    expect(new Set(contracts.map((contract) => contract.command)).size).toBe(contracts.length);
    expect([...contractByCommand.keys()].sort()).toEqual([...manifest].sort());
    expect(matrixCommands().sort()).toEqual([...manifest].sort());
    expect(explicitContracts.filter((contract) => contract.class === "MAIN_WINDOW_MUTATION").length).toBeGreaterThan(0);
    expect(explicitContracts.some((contract) => contract.class === "MAIN_WINDOW_DIAGNOSTIC_MUTATION")).toBe(true);
    expect(explicitContracts.some((contract) => contract.class === "SEARCH_WINDOW_LIFECYCLE")).toBe(true);
    expect(explicitContracts.some((contract) => contract.class === "EXPLICITLY_SHARED_READ")).toBe(true);
  });

  it("fails closed when a newly registered command has no classification", () => {
    expect(() => contractsForManifest([...manifest, "new_unclassified_command"])).toThrow(
      "new_unclassified_command",
    );
  });

  it("matches raw Rust string delimiters exactly while finding a function body", () => {
    const sourceText = `
      #[tauri::command]
      fn raw_plain() { let _ = r"quoted } text"; }
      #[tauri::command]
      fn raw_one_hash() { let _ = r#"quoted } text"#; }
      #[tauri::command]
      fn raw_two_hashes() { let _ = r##"quoted } text"##; }
    `;
    expect(functionBody(sourceText, "raw_plain")?.body).toContain('r"quoted } text"');
    expect(functionBody(sourceText, "raw_one_hash")?.body).toContain('r#"quoted } text"#');
    expect(functionBody(sourceText, "raw_two_hashes")?.body).toContain('r##"quoted } text"##');
  });

  it("fails a mutation contract when that command loses its own main-window guard", () => {
    const sourceText = `
      #[tauri::command]
      fn target_command() { Ok(()) }
      #[tauri::command]
      fn sibling_command() { require_main_window(&window)?; Ok(()) }
    `;
    const parsed = functionBody(sourceText, "target_command");
    expect(parsed?.body).not.toContain("require_main_window");
  });

  it("does not let a sibling function guard satisfy the current command", () => {
    const sourceText = `
      #[tauri::command]
      fn target_command() { Ok(()) }
      #[tauri::command]
      fn sibling_command() { require_main_window(&window)?; Ok(()) }
    `;
    expect(functionBody(sourceText, "target_command")?.body).not.toContain("require_main_window");
    expect(functionBody(sourceText, "sibling_command")?.body).toContain("require_main_window");
  });

  it("rejects a mutation contract when that command loses its own guard", () => {
    const sourceText = `
      #[tauri::command]
      fn target_command<R: Runtime>(window: WebviewWindow<R>) { Ok(()) }
    `;
    expect(() =>
      assertMainWindowMutationContract(
        {
          command: "target_command",
          source: "synthetic.rs",
          class: "MAIN_WINDOW_MUTATION",
          guard: "require_main_window",
        },
        sourceText,
      ),
    ).toThrow("target_command guard");
  });

  it("checks the current function body instead of a sibling function body", () => {
    const sourceText = `
      #[tauri::command]
      fn target_command<R: Runtime>(window: WebviewWindow<R>) { Ok(()) }
      #[tauri::command]
      fn sibling_command<R: Runtime>(window: WebviewWindow<R>) {
        require_main_window(&window)?;
        Ok(())
      }
    `;
    const targetContract: CommandContract = {
      command: "target_command",
      source: "synthetic.rs",
      class: "MAIN_WINDOW_MUTATION",
      guard: "require_main_window",
    };
    expect(() => assertMainWindowMutationContract(targetContract, sourceText)).toThrow(
      "target_command guard",
    );
    expect(functionBody(sourceText, "sibling_command")?.body).toContain("require_main_window");
  });

  it("grants exactly one main capability permission to every registered command", () => {
    const defaultCommands = mainCapability.permissions
      .filter((permission) => permission.startsWith("allow-"))
      .map((permission) => permission.slice("allow-".length).replaceAll("-", "_"));
    const searchCommands = searchCapability.permissions
      .filter((permission) => permission.startsWith("allow-"))
      .map((permission) => permission.slice("allow-".length).replaceAll("-", "_"));

    expect(new Set(defaultCommands).size).toBe(defaultCommands.length);
    expect(new Set(searchCommands).size).toBe(searchCommands.length);
    expect(defaultCommands.filter((command) => !manifest.includes(command))).toEqual([]);
    expect(searchCommands.filter((command) => !manifest.includes(command))).toEqual([]);
    expect([...new Set([...defaultCommands, ...searchCommands])].sort()).toEqual([...manifest].sort());
  });

  it("documents every command exactly once", () => {
    for (const command of manifest) {
      const row = `| \`${command}\` |`;
      expect(matrixSource.split(row).length - 1).toBe(1);
    }
  });

  it("checks every main-window mutation function body independently", () => {
    const mutationContracts = explicitContracts.filter(
      (contract) =>
        contract.class === "MAIN_WINDOW_MUTATION" ||
        contract.class === "MAIN_WINDOW_DIAGNOSTIC_MUTATION",
    );
    expect(mutationContracts.length).toBeGreaterThan(0);

    for (const contract of mutationContracts) {
      expect(() => assertMainWindowMutationContract(contract, readSource(contract.source))).not.toThrow();
    }
  });

  it("keeps all File Workspace commands async and isolates blocking work", () => {
    const commands = [
      "file_workspace_browse_open",
      "file_workspace_browse_restore",
      "file_workspace_location_browse",
      "file_workspace_browse_start_enumeration",
      "file_workspace_browse_next_page",
      "file_workspace_browse_cancel_enumeration",
      "file_workspace_browse_release_page",
      "file_workspace_browse_release_path",
      "file_workspace_browse_retain_path",
      "file_workspace_browse_dispose",
      "file_workspace_location_list",
      "file_workspace_change_start",
      "file_workspace_change_pending",
      "file_workspace_change_refresh",
      "file_workspace_change_dispose",
      "file_workspace_read_eligibility",
      "file_workspace_thumbnail_request",
      "file_workspace_thumbnail_cancel",
      "file_workspace_preview_create",
      "file_workspace_preview_snapshot",
      "file_workspace_preview_start",
      "file_workspace_preview_cancel",
      "file_workspace_preview_dispose",
      "file_workspace_preview_switch_source",
      "file_workspace_preview_asset_request",
    ] as const;
    const directCancellationCommands = new Set([
      "file_workspace_browse_cancel_enumeration",
      "file_workspace_change_pending",
      "file_workspace_thumbnail_cancel",
      "file_workspace_preview_snapshot",
      "file_workspace_preview_cancel",
      "file_workspace_preview_dispose",
    ]);

    expect(commands).toHaveLength(25);
    for (const command of commands) {
      const parsed = functionBody(fileWorkspaceCommandsSource, command);
      expect(parsed, `${command} function body`).not.toBeNull();
      expect(fileWorkspaceCommandsSource).toMatch(new RegExp(`pub async fn ${command}\\b`));
      if (directCancellationCommands.has(command)) {
        expect(parsed?.body).not.toContain("spawn_runtime(");
      } else {
        expect(parsed?.body).toContain("spawn_runtime(");
      }
    }
  });

  it("keeps search-window lifecycle functions on their declared boundary", () => {
    for (const contract of explicitContracts.filter(
      ({ class: className }) => className === "SEARCH_WINDOW_LIFECYCLE",
    )) {
      const parsed = functionBody(readSource(contract.source), contract.command);
      expect(parsed, `${contract.command} function body`).not.toBeNull();
      const body = parsed?.body ?? "";
      if (contract.guard === "require_search_window") {
        expect(body).toContain("require_search_window");
      } else if (contract.guard === "search_or_main_window") {
        expect(body).toContain("SEARCH_WINDOW_LABEL");
        expect(body).toContain("validate_active_search");
        const validator = appControlSource.slice(appControlSource.indexOf("fn validate_active_search"), appControlSource.indexOf("impl MainWindowReadinessState", appControlSource.indexOf("fn validate_active_search")));
        expect(validator).toContain("validate_search_window_cas");
        expect(validator).toContain("SearchWindowPhase::VisibleCollapsed");
        expect(validator).toContain("SearchWindowPhase::VisibleExpanded");
        expect(body).toContain("require_main_window");
      } else {
        expect(body).not.toContain("require_main_window");
        expect(body).not.toContain("require_search_window");
      }
    }
  });

  it("keeps the search window on the bounded global-search and lifecycle allowlist", () => {
    expect(searchCapability.permissions).toEqual([
      "core:default",
      "allow-get-settings",
      "allow-search-global-entries",
      "allow-open-global-search-result",
      "allow-reveal-global-search-result",
      "allow-get-runtime-capabilities",
      "allow-activate-search-result",
      "allow-get-search-window-state",
      "allow-search-window-ready",
      "allow-resize-search-window",
      "allow-hide-search-window-command",
    ]);
    expect(searchCapability.permissions).not.toContain("core:window:allow-hide");
    expect(searchCapability.permissions).not.toContain("allow-search-files");
    expect(searchCapability.permissions).not.toContain("allow-get-paged-files");
    expect(searchCapability.permissions.some((permission) =>
      explicitContracts.some(
        (contract) =>
          contract.class === "MAIN_WINDOW_MUTATION" && permission === permissionFor(contract.command),
      ),
    )).toBe(false);
  });

  it("keeps database initialization out of the search window", () => {
    expect(searchCapability.permissions).not.toContain("allow-init-db");
    expect(databaseBootstrapperSource).toContain("isSearchWindowMode");
    expect(databaseBootstrapperSource).toContain("if (isSearchWindowMode)");
  });

  it("does not expose legacy whole-object Rule mutations", () => {
    expect(dbCommandsSource).not.toContain("pub fn save_user_rule<");
    expect(dbCommandsSource).not.toContain("pub fn delete_user_rule<");
    expect(dbCommandsSource).not.toContain("pub fn get_user_rules(");
    expect(manifest).not.toEqual(
      expect.arrayContaining(["save_user_rule", "delete_user_rule", "get_user_rules"]),
    );
    expect(mainCapability.permissions).not.toContain("allow-save-user-rule");
    expect(mainCapability.permissions).not.toContain("allow-delete-user-rule");
  });
});
