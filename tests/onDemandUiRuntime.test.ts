import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function resolveLocalModule(importer: string, specifier: string): string | null {
  if (!specifier.startsWith(".")) return null;
  const base = resolve(dirname(importer), specifier);
  const candidates = [
    base,
    `${base}.tsx`,
    `${base}.ts`,
    `${base}.jsx`,
    `${base}.js`,
    resolve(base, "index.tsx"),
    resolve(base, "index.ts"),
    resolve(base, "index.jsx"),
    resolve(base, "index.js")
  ];
  return candidates.find((candidate) => existsSync(candidate) && statSync(candidate).isFile()) ?? null;
}

function runtimeImports(file: string): string[] {
  const source = readFileSync(file, "utf8");
  const parsed = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true);
  const specifiers: string[] = [];
  for (const statement of parsed.statements) {
    if (ts.isImportDeclaration(statement)) {
      if (!statement.importClause?.isTypeOnly && ts.isStringLiteral(statement.moduleSpecifier)) {
        specifiers.push(statement.moduleSpecifier.text);
      }
    } else if (ts.isExportDeclaration(statement)) {
      if (!statement.isTypeOnly && statement.moduleSpecifier && ts.isStringLiteral(statement.moduleSpecifier)) {
        specifiers.push(statement.moduleSpecifier.text);
      }
    }
  }
  const visit = (node: ts.Node) => {
    if (
      ts.isCallExpression(node)
      && node.expression.kind === ts.SyntaxKind.ImportKeyword
      && node.arguments.length === 1
      && ts.isStringLiteral(node.arguments[0])
    ) {
      specifiers.push(node.arguments[0].text);
    }
    ts.forEachChild(node, visit);
  };
  visit(parsed);
  return specifiers;
}

function runtimeGraph(entry: string): Set<string> {
  const visited = new Set<string>();
  const pending = [entry];
  while (pending.length > 0) {
    const file = pending.pop()!;
    if (visited.has(file)) continue;
    visited.add(file);
    for (const specifier of runtimeImports(file)) {
      const resolved = resolveLocalModule(file, specifier);
      if (resolved && !resolved.endsWith(".css")) pending.push(resolved);
    }
  }
  return visited;
}

function relativeFiles(files: Set<string>): Set<string> {
  return new Set([...files].map((file) => file.slice(repositoryRoot.length + 1).replaceAll("\\", "/")));
}

describe("on-demand UI runtime boundaries", () => {
  it("keeps Search on a mini runtime dependency graph", () => {
    const graph = relativeFiles(runtimeGraph(resolve(repositoryRoot, "src/SearchApp.tsx")));
    const forbidden = [
      "src/components/AppRuntimeProviders.tsx",
      "src/components/DatabaseBootstrapper.tsx",
      "src/components/AppShell.tsx",
      "src/store/useBackgroundIndexerStore.ts",
      "src/hooks/useFsWatcher.ts",
      "src/store/useOperationQueueStore.ts",
      "src/store/useFileLibraryStore.ts",
      "src/store/useOrganizationPlanStore.ts",
      "src/store/useRulesStore.ts",
      "src/store/useScanManagerStore.ts",
      "src/api/tauriApi.ts"
    ];

    expect(graph).toContain("src/SearchApp.tsx");
    expect(graph).toContain("src/components/CommandModal.tsx");
    expect(graph).toContain("src/api/searchRuntimeApi.ts");
    expect(graph).not.toContain("src/api/globalSearchApi.ts");
    expect(graph).not.toContain("src/api/windowApi.ts");
    expect(graph).not.toContain("src/views/shared/ui.ts");
    for (const file of forbidden) expect(graph, `${file} must stay outside Search runtime`).not.toContain(file);
  });

  it("keeps Main-only IPC out of the Search runtime API", () => {
    const searchApi = readFileSync(resolve(repositoryRoot, "src/api/searchRuntimeApi.ts"), "utf8");
    expect(searchApi).not.toMatch(/enter_background|quit_app|mark_main_window_ready|acknowledge_main_window_ready/);
    expect(searchApi).not.toMatch(/get_global_index_status|start_global_index|pause_global_index|update_managed_scope/);
  });

  it("loads Main and Search through separate dynamic entry points and keeps windows non-static", () => {
    const mainEntry = readFileSync(resolve(repositoryRoot, "src/main.tsx"), "utf8");
    const config = JSON.parse(readFileSync(resolve(repositoryRoot, "src-tauri/tauri.conf.json"), "utf8")) as {
      app?: { windows?: unknown[] };
    };
    const mainCapability = JSON.parse(readFileSync(resolve(repositoryRoot, "src-tauri/capabilities/default.json"), "utf8")) as {
      windows: string[];
      permissions: string[];
    };
    const searchCapability = JSON.parse(readFileSync(resolve(repositoryRoot, "src-tauri/capabilities/search.json"), "utf8")) as {
      windows: string[];
      permissions: string[];
    };

    expect(mainEntry).toContain('import("./SearchApp")');
    expect(mainEntry).toContain('import("./App")');
    expect(mainEntry).not.toMatch(/^\s*import\s+.*\b(?:App|SearchApp)\b/m);
    expect(config.app?.windows ?? []).toEqual([]);
    expect(mainCapability.windows).toContain("main");
    expect(searchCapability.windows).toContain("search");
    expect(mainCapability.permissions).toContain("allow-enter-background");
    expect(searchCapability.permissions).not.toContain("allow-enter-background");
  });

  it("creates Search only from its show path and destroys it on dismissal", () => {
    const appControl = readFileSync(resolve(repositoryRoot, "src-tauri/src/app_control.rs"), "utf8");
    const main = readFileSync(resolve(repositoryRoot, "src-tauri/src/main.rs"), "utf8");

    expect(appControl).toContain("fn create_search_window");
    expect(appControl).toContain("None => {");
    expect(appControl).toContain("create_search_window(app)?");
    const dismiss = appControl.slice(
      appControl.indexOf("fn hide_search_window_with_state"),
      appControl.indexOf('#[cfg(not(feature = "desktop-runtime"))]', appControl.indexOf("fn hide_search_window_with_state"))
    );
    expect(dismiss).toContain("lifecycle.hide_with_native(");
    expect(dismiss).toContain("if let Err(error) = window.destroy()");
    expect(dismiss).toContain("return Err(error.to_string())");
    expect((dismiss.match(/drop\(teardown\)/g) ?? []).length).toBe(1);
    expect((dismiss.match(/teardown\.complete\(\)/g) ?? []).length).toBe(1);
    expect(dismiss).not.toContain("teardown.complete(app.webview_windows().len())");
    expect(dismiss.indexOf("begin_internal_window_teardown")).toBeLessThan(dismiss.indexOf("window.destroy()"));
    expect(dismiss.indexOf("drop(teardown)")).toBeLessThan(dismiss.indexOf("return Err(error.to_string())"));
    expect(dismiss.indexOf("return Err(error.to_string())")).toBeLessThan(dismiss.indexOf("teardown.complete()"));
    expect(dismiss.indexOf("window.destroy()")).toBeLessThan(dismiss.indexOf("ui_runtime search_window_destroyed"));
    expect(main).not.toContain("setup_search_window");
  });

  it("keeps Main teardown ordered and launches it only for interactive startup", () => {
    const appControl = readFileSync(resolve(repositoryRoot, "src-tauri/src/app_control.rs"), "utf8");
    const main = readFileSync(resolve(repositoryRoot, "src-tauri/src/main.rs"), "utf8");
    const ensureMain = appControl.slice(
      appControl.indexOf("fn ensure_main_window_locked"),
      appControl.indexOf("#[tauri::command]\npub fn enter_background")
    );
    const enterBackground = appControl.slice(
      appControl.indexOf("pub fn enter_background"),
      appControl.indexOf("pub fn register_global_search_hotkey")
    );

    expect(ensureMain).toContain("if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL)");
    expect(ensureMain).toContain("let generation = lifecycle.next_generation()?");
    expect(ensureMain).toContain("main_window_restore_url(generation, last_view)");
    expect(ensureMain).toContain("record_window_created()");
    expect(enterBackground.indexOf("session.set_last_view")).toBeLessThan(enterBackground.indexOf("readiness.set_ready(generation, false)"));
    expect(enterBackground.indexOf("readiness.set_ready(generation, false)")).toBeLessThan(enterBackground.indexOf("workspace.dispose_generation(generation)"));
    expect(enterBackground.indexOf("workspace.dispose_generation(generation)")).toBeLessThan(enterBackground.indexOf("window.destroy()"));
    expect(main).toContain("if !background_launch {");
    expect(main).toContain('.app_name("Fylura")');
    expect(main).toContain('.args(["--background"])');
    expect(main).toContain("tauri_plugin_single_instance::init");
  });

  it("keeps the real Windows resident lifecycle regression in the hosted gate", () => {
    const probe = readFileSync(resolve(repositoryRoot, "src-tauri/src/native_resident_lifecycle_qa.rs"), "utf8");
    const workflow = readFileSync(resolve(repositoryRoot, ".github/workflows/ci.yml"), "utf8");
    const intent = readFileSync(resolve(repositoryRoot, "src-tauri/src/exit_intent.rs"), "utf8");

    expect(probe).toContain("enter_background(");
    expect(probe).toContain("qa.assert_resident(app, &owners)?");
    expect(probe).toContain("show_main_window(app)?");
    expect(probe).toContain("background_delivery_verified");
    expect(probe).toContain("filesystem_manifest_verified");
    expect(probe).toContain("quit_app(");
    expect(probe).toContain("exit_app(app)");
    expect(probe).not.toMatch(/mock_app|exit_requested_action/);
    expect(workflow).toContain("./scripts/runWindowsResidentLifecycleQa.ps1");
    expect(intent).not.toContain("std::mem::take(&mut ledger.stay_resident_pending)");
  });
});
