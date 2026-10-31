import { mkdir, rm, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { assertCheckoutEvidence } from "./ciEvidence.mjs";
import { chromium } from "playwright";
import { createServer } from "vite";
import {
  collectW201BrowserMeasurement,
  evaluateW201CompactGate,
  evaluateW201ProjectionGate,
  evaluateW201ResponsiveGate,
  evaluateW201VirtualizationInteraction,
  W201_VIEWPORTS
} from "./w2-01-browser-gate.mjs";

const SOURCE_HEAD = process.env.W201_SOURCE_HEAD ?? process.env.GITHUB_SHA ?? "local";
const EXPECTED_CHECKOUT_SHA = process.env.W201_EXPECTED_CHECKOUT_SHA ?? null;
const CHROMIUM_EXECUTABLE = process.env.W201_CHROMIUM_EXECUTABLE?.trim() || undefined;
const ACTUAL_CHECKOUT_SHA = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const ACTUAL_CHECKOUT_TREE = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
if (EXPECTED_CHECKOUT_SHA) {
  assertCheckoutEvidence(EXPECTED_CHECKOUT_SHA, ACTUAL_CHECKOUT_SHA);
}
const CHECKOUT_EVIDENCE = {
  lane: process.env.W201_LANE ?? "local",
  claimedSourceHead: SOURCE_HEAD,
  expectedCheckoutSha: EXPECTED_CHECKOUT_SHA,
  actualCheckoutSha: ACTUAL_CHECKOUT_SHA,
  actualCheckoutTree: ACTUAL_CHECKOUT_TREE
};
const ARTIFACT_DIR = path.resolve(process.env.W201_BROWSER_ARTIFACT_DIR ?? ".tmp-tests/w2-01-browser-gate");
const TASK_TEMP_DIR = path.resolve(".tmp-tests/w2-01-browser-runtime");

process.env.TEMP = TASK_TEMP_DIR;
process.env.TMP = TASK_TEMP_DIR;
process.env.TMPDIR = TASK_TEMP_DIR;

class GateFailure extends Error {
  constructor(message, diagnostic) {
    super(message);
    this.name = "GateFailure";
    this.diagnostic = diagnostic;
  }
}

async function startFrontendServer() {
  const server = await createServer({
    configFile: path.resolve("vite.config.ts"),
    server: {
      host: "127.0.0.1",
      port: 0,
      strictPort: false
    }
  });
  await server.listen();
  const baseUrl = server.resolvedUrls?.local?.[0];
  if (!baseUrl) {
    await server.close();
    throw new Error("Vite did not expose a local browser URL.");
  }
  return { server, baseUrl: baseUrl.replace(/\/$/, "") };
}

async function waitForApp(page) {
  if (await page.title() !== "Fylura") {
    throw new Error(`Unexpected page title: ${await page.title()}`);
  }
  await page.waitForSelector("#root", { state: "attached" });
  await page.getByRole("button", { name: "Files", exact: true }).waitFor({ state: "visible" });
}

async function openLibrary(page) {
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await page.waitForSelector(".file-library-workspace", { state: "visible" });
  await page.waitForFunction(() => {
    const listbox = document.querySelector('[role="listbox"][data-file-library-scroll-owner="tanstack-virtualizer"]');
    const allIndexedFiles = [...document.querySelectorAll("button")].some((button) => {
      const rect = button.getBoundingClientRect();
      return button.textContent?.trim() === "View all indexed files"
        && rect.width > 0
        && rect.height > 0
        && getComputedStyle(button).visibility !== "hidden";
    });
    return Boolean(listbox?.clientHeight > 0 || allIndexedFiles);
  }, undefined, { timeout: 20_000 });
  const allIndexedFiles = page.getByRole("button", { name: "View all indexed files", exact: true });
  if (await allIndexedFiles.count() > 0 && await allIndexedFiles.first().isVisible()) {
    await allIndexedFiles.first().click();
  }
  await page.waitForFunction(() => {
    const listbox = document.querySelector('[role="listbox"][data-file-library-scroll-owner="tanstack-virtualizer"]');
    return Boolean(listbox && listbox.clientHeight > 0 && listbox.scrollHeight > listbox.clientHeight);
  }, undefined, { timeout: 20_000 });
  await page.waitForTimeout(100);
}

async function openDetachedBrowse(page) {
  await openLibrary(page);
  await page.getByRole("tab", { name: "Browse Folder", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="browse"][data-detached-browse="true"]', { state: "visible" });
  await page.getByText("No folder is open. Nothing is being read, indexed, or added to your File Library.", { exact: true }).waitFor({ state: "visible" });
}

async function openOverview(page) {
  await page.getByRole("button", { name: "Overview", exact: true }).waitFor({ state: "visible" });
  await page.getByRole("button", { name: "Overview", exact: true }).click();
  await page.waitForFunction(() => document.querySelector(".file-library-workspace") === null, undefined, { timeout: 10_000 });
}

async function collectMeasurement(page, viewport) {
  return page.evaluate(collectW201BrowserMeasurement, [SOURCE_HEAD, viewport]);
}

function failedAssertions(result) {
  return result?.assertions?.filter((item) => !item.passed).map((item) => ({
    name: item.name,
    detail: item.detail
  })) ?? [];
}

function measurementDiagnostic(scene, viewport, measurement, result, extra = {}) {
  return {
    scene,
    sourceHead: SOURCE_HEAD,
    checkoutEvidence: CHECKOUT_EVIDENCE,
    viewport,
    failedAssertions: failedAssertions(result),
    hardAssertionSummary: result?.hardAssertionSummary ?? null,
    bounds: measurement?.bounds ?? null,
    listbox: {
      bounds: measurement?.bounds?.fileLibraryList ?? null,
      scroll: measurement?.scrollOwnership?.fileLibraryList ?? null,
      selector: measurement?.scrollOwnership?.fileLibraryListSelector ?? null,
      virtualization: measurement?.virtualization ?? null
    },
    scrollOwnership: measurement?.scrollOwnership ?? null,
    page: measurement?.page ?? null,
    ...extra
  };
}

function assertGate(scene, viewport, measurement, result, extra = {}) {
  if (result?.passed) return;
  const diagnostic = measurementDiagnostic(scene, viewport, measurement, result, extra);
  throw new GateFailure(`${scene} failed: ${diagnostic.failedAssertions.map((item) => item.name).join(", ") || "gate assertion"}`, diagnostic);
}

async function runCompact(page, viewport, state) {
  await openLibrary(page);
  const before = await state.measure(page, viewport);
  const layout = evaluateW201CompactGate(before, viewport);
  state.results.layout = layout;
  assertGate("compact-library", viewport, before, layout);

  const listbox = page.locator('[role="listbox"][data-file-library-scroll-owner="tanstack-virtualizer"]');
  await listbox.focus();
  const beforeScrollTop = before.scrollOwnership.fileLibraryList.scrollTop;
  const beforeLogicalCount = before.virtualization.logicalCount;
  let quickAfter = null;
  for (let attempt = 0; attempt < 24; attempt += 1) {
    await listbox.press("PageDown");
    await page.waitForTimeout(120);
    quickAfter = await listbox.evaluate((element) => ({
      scrollTop: element.scrollTop,
      logicalCount: Number(element.getAttribute("data-file-library-logical-count")),
      hasMore: element.getAttribute("data-file-library-has-more") === "true"
    }));
    if (
      quickAfter.scrollTop > beforeScrollTop
      && (quickAfter.logicalCount > beforeLogicalCount || quickAfter.hasMore === false)
    ) break;
  }
  if (
    !quickAfter
    || quickAfter.scrollTop <= beforeScrollTop
    || (quickAfter.logicalCount <= beforeLogicalCount && quickAfter.hasMore)
  ) {
    await listbox.press("End");
    await page.waitForTimeout(250);
  }

  const after = await state.measure(page, viewport);
  const interaction = evaluateW201VirtualizationInteraction(before, after);
  state.results.interaction = interaction;
  assertGate("compact-library-scroll", viewport, after, interaction, {
    before: measurementDiagnostic("compact-library", viewport, before, layout),
    after: measurementDiagnostic("compact-library-scroll", viewport, after, interaction)
  });
  const adapterBefore = before.scrollOwnership.legacyLibraryAdapter?.scrollTop ?? 0;
  const adapterAfter = after.scrollOwnership.legacyLibraryAdapter?.scrollTop ?? 0;
  if (adapterBefore !== adapterAfter) {
    throw new GateFailure("compact-library-scroll failed: adapter scroll owner changed", measurementDiagnostic(
      "compact-library-scroll",
      viewport,
      after,
      interaction,
      { adapterScrollTopBefore: adapterBefore, adapterScrollTopAfter: adapterAfter }
    ));
  }
  return { before, after, layout, interaction };
}

async function runResponsive(page, viewport, scene, state) {
  await openLibrary(page);
  const measurement = await state.measure(page, viewport);
  const result = evaluateW201ResponsiveGate(measurement, viewport);
  state.results.layout = result;
  assertGate(scene, viewport, measurement, result);
  return { measurement, result };
}

async function runDetached(page, viewport, state) {
  await openDetachedBrowse(page);
  const measurement = await state.measure(page, viewport);
  const result = evaluateW201ProjectionGate(measurement, viewport, "detached-browse");
  state.results.projection = result;
  assertGate("detached-browse", viewport, measurement, result, {
    currentStateCopy: "No folder is open. Nothing is being read, indexed, or added to your File Library."
  });
  if (await page.locator('[data-vault-presentation="embedded"]').count() !== 0) {
    throw new GateFailure("detached-browse failed: embedded Vault content was mounted", measurementDiagnostic(
      "detached-browse",
      viewport,
      measurement,
      result
    ));
  }
  return { measurement, result };
}

async function runOverview(page, viewport, state) {
  await openOverview(page);
  const measurement = await state.measure(page, viewport);
  const result = evaluateW201ProjectionGate(measurement, viewport, "overview");
  state.results.projection = result;
  assertGate("overview", viewport, measurement, result);
  return { measurement, result };
}

async function runScene(browser, baseUrl, scene, viewport, action) {
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  await context.addInitScript(() => {
    window.localStorage.setItem("zc-onboarding-complete", "true");
    window.localStorage.setItem("zc-language", "en");
  });
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  page.setDefaultNavigationTimeout(60_000);
  const consoleErrors = [];
  const pageErrors = [];
  const state = { measurements: [], results: {} };
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", (error) => pageErrors.push(String(error)));
  state.measure = async (targetPage, targetViewport) => {
    const measurement = await collectMeasurement(targetPage, targetViewport);
    state.measurements.push(measurement);
    return measurement;
  };

  try {
    await page.goto(`${baseUrl}?w2-01-browser-fixture=virtualized`, { waitUntil: "commit", timeout: 60_000 });
    await waitForApp(page);
    const result = await action(page, viewport, state);
    if (pageErrors.length || consoleErrors.length) {
      throw new GateFailure(`${scene} emitted browser errors`, {
        scene,
        sourceHead: SOURCE_HEAD,
        checkoutEvidence: CHECKOUT_EVIDENCE,
        viewport,
        pageErrors,
        consoleErrors,
        measurement: state.measurements.at(-1) ?? null
      });
    }
    const latest = state.measurements.at(-1) ?? null;
    const summary = {
      scene,
      sourceHead: SOURCE_HEAD,
      checkoutEvidence: CHECKOUT_EVIDENCE,
      requestedViewport: viewport,
      viewportContract: latest?.viewportContract ?? null,
      bounds: {
        root: latest?.bounds?.root ?? null,
        titlebar: latest?.bounds?.titlebar ?? null,
        viewStage: latest?.bounds?.viewStage ?? null,
        workspace: latest?.bounds?.workspace ?? null,
        workspaceBody: latest?.bounds?.workspaceBody ?? null,
        contentSlot: latest?.bounds?.contentSlot ?? null,
        legacyLibraryAdapter: latest?.bounds?.legacyLibraryAdapter ?? null,
        fileLibraryList: latest?.bounds?.fileLibraryList ?? null
      },
      scrollOwnership: {
        viewStage: latest?.scrollOwnership?.viewStage ?? null,
        workspace: latest?.scrollOwnership?.workspace ?? null,
        workspaceBody: latest?.scrollOwnership?.workspaceBody ?? null,
        contentSlot: latest?.scrollOwnership?.contentSlot ?? null,
        legacyLibraryAdapter: latest?.scrollOwnership?.legacyLibraryAdapter ?? null,
        vaultRoot: latest?.scrollOwnership?.vaultRoot ?? null,
        fileLibraryList: latest?.scrollOwnership?.fileLibraryList ?? null,
        fileLibraryListSelector: latest?.scrollOwnership?.fileLibraryListSelector ?? null,
        document: latest?.scrollOwnership?.document ?? null,
        body: latest?.scrollOwnership?.body ?? null
      },
      page: latest?.page ?? null,
      virtualization: latest?.virtualization ?? null
    };
    if (scene === "compact-library" && result?.before && result?.after) {
      summary.scrollVerification = {
        before: {
          listScrollTop: result.before.scrollOwnership?.fileLibraryList?.scrollTop ?? null,
          adapterScrollTop: result.before.scrollOwnership?.legacyLibraryAdapter?.scrollTop ?? null,
          virtualRange: [result.before.virtualization?.firstMountedRowIndex, result.before.virtualization?.lastMountedRowIndex],
          logicalCount: result.before.virtualization?.logicalCount ?? null
        },
        after: {
          listScrollTop: result.after.scrollOwnership?.fileLibraryList?.scrollTop ?? null,
          adapterScrollTop: result.after.scrollOwnership?.legacyLibraryAdapter?.scrollTop ?? null,
          virtualRange: [result.after.virtualization?.firstMountedRowIndex, result.after.virtualization?.lastMountedRowIndex],
          logicalCount: result.after.virtualization?.logicalCount ?? null
        },
        actualOwner: result.after.scrollOwnership?.fileLibraryListSelector ?? null
      };
    }
    console.log(`[w2-01-real] PASS ${scene} ${viewport.width}x${viewport.height}`);
    console.log(JSON.stringify(summary, null, 2));
    return { scene, viewport, result, measurements: state.measurements };
  } catch (error) {
    const measurement = state.measurements.at(-1) ?? null;
    const diagnostic = error instanceof GateFailure
      ? error.diagnostic
      : measurementDiagnostic(scene, viewport, measurement, null, { error: String(error), pageErrors, consoleErrors });
    await writeFailureArtifacts(page, scene, diagnostic);
    console.error(`[w2-01-real] FAIL ${scene} ${viewport.width}x${viewport.height}`);
    console.error(JSON.stringify(diagnostic, null, 2));
    return { scene, viewport, ok: false, diagnostic };
  } finally {
    await context.close();
  }
}

async function writeFailureArtifacts(page, scene, diagnostic) {
  await mkdir(ARTIFACT_DIR, { recursive: true });
  const safeScene = scene.replace(/[^a-z0-9-]+/gi, "-").toLowerCase();
  await writeFile(path.join(ARTIFACT_DIR, `${safeScene}.json`), JSON.stringify(diagnostic, null, 2), "utf8");
  await page.screenshot({ path: path.join(ARTIFACT_DIR, `${safeScene}.png`), fullPage: false });
}

async function main() {
  let frontend = null;
  let browser = null;
  const results = [];
  let runnerFailed = false;
  try {
    await mkdir(TASK_TEMP_DIR, { recursive: true });
    frontend = await startFrontendServer();
    browser = await chromium.launch({
      headless: true,
      ...(CHROMIUM_EXECUTABLE ? { executablePath: CHROMIUM_EXECUTABLE } : {})
    });
    results.push(await runScene(browser, frontend.baseUrl, "wide-library", W201_VIEWPORTS.wide, (page, viewport, state) => runResponsive(page, viewport, "wide-library", state)));
    results.push(await runScene(browser, frontend.baseUrl, "medium-library", W201_VIEWPORTS.medium, (page, viewport, state) => runResponsive(page, viewport, "medium-library", state)));
    results.push(await runScene(browser, frontend.baseUrl, "compact-library", W201_VIEWPORTS.compact, runCompact));
    results.push(await runScene(browser, frontend.baseUrl, "detached-browse", W201_VIEWPORTS.compact, runDetached));
    results.push(await runScene(browser, frontend.baseUrl, "overview", W201_VIEWPORTS.compact, runOverview));
  } catch (error) {
    const diagnostic = {
      sourceHead: SOURCE_HEAD,
      checkoutEvidence: CHECKOUT_EVIDENCE,
      error: String(error),
      installHint: "Install Playwright browsers from the main worktree into a main-worktree cache, or set W201_CHROMIUM_EXECUTABLE to an approved local Chromium binary; do not install dependencies in a child worktree."
    };
    await mkdir(ARTIFACT_DIR, { recursive: true });
    await writeFile(path.join(ARTIFACT_DIR, "runner.json"), JSON.stringify(diagnostic, null, 2), "utf8");
    console.error("[w2-01-real] RUNNER FAILURE");
    console.error(JSON.stringify(diagnostic, null, 2));
    process.exitCode = 1;
    runnerFailed = true;
  } finally {
    await browser?.close();
    await frontend?.server.close();
    await rm(TASK_TEMP_DIR, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
  }

  if (runnerFailed) return;

  const failures = results.filter((result) => result.ok === false);
  if (failures.length > 0) {
    await mkdir(ARTIFACT_DIR, { recursive: true });
    await writeFile(path.join(ARTIFACT_DIR, "summary.json"), JSON.stringify({ sourceHead: SOURCE_HEAD, checkoutEvidence: CHECKOUT_EVIDENCE, failures }, null, 2), "utf8");
    process.exitCode = 1;
    return;
  }

  await rm(ARTIFACT_DIR, { recursive: true, force: true });
  console.log(`[w2-01-real] PASS all scenes; sourceHead=${SOURCE_HEAD}`);
}

await main();
