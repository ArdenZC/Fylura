import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

// Presentation evidence only. Browser API calls use the repository's browser mock.
const artifactDir = path.resolve(process.env.PM03_EVIDENCE_DIR ?? "docs/project/tasks/evidence/PM-03");
await mkdir(artifactDir, { recursive: true });

const commandAuditPlugin = {
  name: "pm03-browser-command-audit",
  enforce: "pre",
  transform(code, id) {
    if (!id.replaceAll("\\", "/").endsWith("/src/api/browserMockApi.ts")) return null;
    const signature = "export async function mockInvokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {";
    if (!code.includes(signature)) throw new Error("Could not instrument the browser mock command boundary");
    return code.replace(signature, `${signature}\n  (globalThis as typeof globalThis & { __pm03CommandLog?: Array<{ command: string }> }).__pm03CommandLog?.push({ command });`);
  }
};

const server = await createServer({
  plugins: [commandAuditPlugin],
  server: { host: "127.0.0.1", port: 0, strictPort: false }
});
await server.listen();
const baseUrl = server.resolvedUrls.local[0];
const browser = await chromium.launch({ headless: true });
const results = [];
const browserErrors = [];
const mutationCommands = new Set([
  "execute_moves", "restore_moves", "resolve_operation_recovery", "materialize_provider_preview",
  "execute_organization_plan", "run_automation_intent_manual", "create_automation_intent",
  "update_automation_intent", "set_automation_intent_enabled", "archive_automation_intent",
  "execute_authoritative_rules_for_paths", "execute_rules_for_scope_v2", "create_user_rule_v2",
  "update_user_rule_v2", "set_user_rule_enabled_v2", "delete_user_rule_v2",
  "create_rule_proposal", "regenerate_rule_proposal", "cancel_rule_proposal",
  "delete_rule_proposal", "replace_rule_proposal_candidate", "apply_rule_proposal",
  "move_cleanup_candidates_to_safe_trash", "restore_cleanup_trash_items"
]);

function labels(language) {
  return language === "en"
    ? { automation: "Automation", intents: "Intents", advanced: "Advanced Policies", settings: "Settings", settingsGroup: "Smart organize", search: /Search Fylura/ }
    : { automation: "自动化", intents: "自动化意图", advanced: "高级策略", settings: "设置", settingsGroup: "智能整理", search: /搜索 Fylura/ };
}

async function newPage(name, language, viewport, query = "") {
  const context = await browser.newContext({ viewport });
  await context.addInitScript((value) => {
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", value);
    window.__pm03CommandLog = [];
  }, language);
  const page = await context.newPage();
  page.on("pageerror", (error) => browserErrors.push({ scenario: name, type: "pageerror", message: error.message }));
  page.on("console", (message) => {
    if (message.type() === "error") browserErrors.push({ scenario: name, type: "console", message: message.text() });
  });
  await page.goto(`${baseUrl}${query}`);
  return { context, page, text: labels(language) };
}

async function assertIntentsDefault(page, text) {
  await page.getByRole("heading", { name: text.automation, exact: true }).last().waitFor();
  const intents = page.getByRole("button", { name: text.intents, exact: true }).last();
  await intents.waitFor();
  if (await intents.getAttribute("aria-pressed") !== "true") throw new Error("Automation did not open on Intents");
  const panel = page.locator("#automation-workspace-panel");
  if (await panel.getAttribute("aria-label") !== text.intents) throw new Error("Intents panel is not the active Automation surface");
  if (await page.getByRole("heading", { name: /Rule Library|规则库/ }).count()) throw new Error("Legacy route opened the advanced Rules surface");
}

async function openFromSpotlight(page, text) {
  await page.getByRole("button", { name: text.search }).click();
  const input = page.getByRole("combobox");
  await input.fill(text.automation);
  const option = page.getByRole("option", { name: new RegExp(text.automation) });
  await option.waitFor();
  await input.press("Enter");
  await assertIntentsDefault(page, text);
}

async function enterAdvancedAndReturnByKeyboard(page, text, screenshotStem) {
  const advanced = page.getByRole("button", { name: text.advanced, exact: true }).last();
  await advanced.focus();
  await page.keyboard.press("Enter");
  if (await advanced.getAttribute("aria-pressed") !== "true") throw new Error("Advanced Policies did not open from its explicit keyboard action");
  await page.getByRole("heading", { name: text.advanced, exact: true }).last().waitFor();
  if (!await page.getByText(/Existing Rules workspace|现有规则工作区/).count()) {
    // The actual Rules view has localized repository headings rather than the mounted test marker.
    await page.getByRole("heading", { name: /Rule Library|规则库/ }).last().waitFor();
  }
  await page.screenshot({ path: path.join(artifactDir, `${screenshotStem}-advanced-policies.png`), fullPage: true });

  const intents = page.getByRole("button", { name: text.intents, exact: true }).last();
  await intents.focus();
  await page.keyboard.press("Enter");
  if (await intents.getAttribute("aria-pressed") !== "true") throw new Error("Intents did not return from its explicit keyboard action");
  if (!await intents.evaluate((element) => element === document.activeElement)) throw new Error("Keyboard focus was not retained on Intents after the transition");
  await assertIntentsDefault(page, text);
  await page.screenshot({ path: path.join(artifactDir, `${screenshotStem}-intents-return.png`), fullPage: true });
}

async function assertNoMutationCommands(page, name) {
  const calls = await page.evaluate(() => window.__pm03CommandLog ?? []);
  const mutations = calls.filter((entry) => mutationCommands.has(entry.command));
  if (mutations.length) throw new Error(`${name} invoked mutation commands: ${JSON.stringify(mutations)}`);
  return { commandCallCount: calls.length, mutationCommandCount: mutations.length, mutationCommands: mutations };
}

async function runLocaleFlow(language, viewport, name) {
  const text = labels(language);
  const { context, page } = await newPage(name, language, viewport);
  try {
    await openFromSpotlight(page, text);
    await page.screenshot({ path: path.join(artifactDir, `${name}-spotlight-intents.png`), fullPage: true });
    await enterAdvancedAndReturnByKeyboard(page, text, name);

    await page.getByRole("button", { name: text.settings, exact: true }).first().click();
    await page.getByRole("heading", { name: text.settings, exact: true }).last().waitFor();
    await page.getByRole("button", { name: text.settingsGroup, exact: true }).click();
    const settingsSection = page.locator("#settings-automation");
    await settingsSection.scrollIntoViewIfNeeded();
    await settingsSection.getByRole("button", { name: text.automation, exact: true }).click();
    await assertIntentsDefault(page, text);
    await page.screenshot({ path: path.join(artifactDir, `${name}-settings-intents.png`), fullPage: true });

    const dimensions = await page.evaluate(() => ({
      viewportWidth: window.innerWidth,
      documentWidth: Math.max(document.documentElement.scrollWidth, document.body.scrollWidth),
      workspaceWidth: document.querySelector(".zc-app-workspace")?.clientWidth ?? null,
      workspaceScrollWidth: document.querySelector(".zc-app-workspace")?.scrollWidth ?? null
    }));
    const horizontalOverflow = dimensions.documentWidth > dimensions.viewportWidth + 1;
    if (horizontalOverflow) throw new Error(`${name} has horizontal viewport overflow: ${JSON.stringify(dimensions)}`);
    if (viewport.width <= 900 && dimensions.workspaceScrollWidth > dimensions.workspaceWidth + 2) {
      throw new Error(`${name} Automation workspace overflows its narrow container: ${JSON.stringify(dimensions)}`);
    }
    await page.screenshot({ path: path.join(artifactDir, `${name}-narrow-or-settings.png`), fullPage: true });
    const commandAudit = await assertNoMutationCommands(page, name);
    results.push({
      scenario: name,
      language,
      viewport,
      spotlightToIntents: true,
      explicitAdvancedPolicies: true,
      keyboardReturnAndFocus: true,
      settingsToIntents: true,
      horizontalOverflow,
      dimensions,
      fileAndRuleMutationCommandCount: commandAudit.mutationCommandCount,
      observedBrowserMockCommandCount: commandAudit.commandCallCount,
      nativeWindowsEvidence: false
    });
  } finally {
    await context.close();
  }
}

async function runDirectRoute(language, view, name) {
  const text = labels(language);
  const { context, page } = await newPage(name, language, { width: 1120, height: 850 }, `?view=${view}`);
  try {
    await assertIntentsDefault(page, text);
    const commandAudit = await assertNoMutationCommands(page, name);
    await page.screenshot({ path: path.join(artifactDir, `${name}.png`), fullPage: true });
    results.push({
      scenario: name,
      language,
      directRoute: view,
      resolvedProductRoute: "automation",
      defaultSurface: "intents",
      fileAndRuleMutationCommandCount: commandAudit.mutationCommandCount,
      observedBrowserMockCommandCount: commandAudit.commandCallCount,
      nativeWindowsEvidence: false
    });
  } finally {
    await context.close();
  }
}

try {
  await runLocaleFlow("en", { width: 1440, height: 960 }, "en-desktop");
  await runLocaleFlow("zh", { width: 1440, height: 960 }, "zh-desktop");
  await runLocaleFlow("en", { width: 760, height: 900 }, "en-narrow");
  await runLocaleFlow("zh", { width: 760, height: 900 }, "zh-narrow");
  await runDirectRoute("en", "automation", "en-canonical-url");
  await runDirectRoute("en", "rules", "en-legacy-url-rules");
  await runDirectRoute("zh", "rules", "zh-legacy-url-rules");
  if (browserErrors.length) throw new Error(`Browser errors detected: ${JSON.stringify(browserErrors)}`);
  const report = {
    generatedAt: new Date().toISOString(),
    evidenceBoundary: "Browser mock integration and presentation evidence only; it is not native Windows, filesystem or provider acceptance.",
    fileMutationBoundary: "The browser mock command boundary was instrumented. Zero file, Rule or Intent mutation commands were observed during navigation scenarios.",
    results,
    browserErrors
  };
  await writeFile(path.join(artifactDir, "measurements.json"), `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify({ artifactDir, results, browserErrors }, null, 2));
} finally {
  await browser.close();
  await server.close();
}
