import { mkdir, rm, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";
import { createServer } from "vite";
import { assertCheckoutEvidence } from "./ciEvidence.mjs";
import { PREVIEW_PERFORMANCE_CONTRACT } from "./performanceManifest.mjs";

const SOURCE_HEAD = process.env.W310_SOURCE_HEAD ?? process.env.GITHUB_SHA ?? "local";
const EXPECTED_CHECKOUT_SHA = process.env.W310_EXPECTED_CHECKOUT_SHA ?? null;
const ACTUAL_CHECKOUT_SHA = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const ACTUAL_CHECKOUT_TREE = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
if (EXPECTED_CHECKOUT_SHA) assertCheckoutEvidence(EXPECTED_CHECKOUT_SHA, ACTUAL_CHECKOUT_SHA);

const TASK_TEMP_DIR = path.resolve(`.tmp-tests/w3-10-phase-a-browser-runtime-${process.pid}`);
const ARTIFACT_DIR = path.resolve(`.tmp-tests/w3-10-phase-a-browser-${process.pid}`);
const FIXTURE_QUERIES = Object.freeze({
  text: "w3-04-browser-fixture=providers",
  structured: "w3-05-browser-fixture=providers&w3-02-browser-fixture=preview",
  image: "w3-06-browser-fixture=images&w3-02-browser-fixture=preview",
  folder: "w3-07-browser-fixture=folders",
  archive: "w3-08-browser-fixture=archives",
});
const VIEWPORTS = Object.freeze([
  { width: 1600, height: 900 },
  { width: 980, height: 680 },
]);

process.env.TEMP = TASK_TEMP_DIR;
process.env.TMP = TASK_TEMP_DIR;
process.env.TMPDIR = TASK_TEMP_DIR;
await mkdir(TASK_TEMP_DIR, { recursive: true });
await mkdir(ARTIFACT_DIR, { recursive: true });

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

const server = await createServer({
  configFile: path.resolve("vite.config.ts"),
  server: { host: "127.0.0.1", port: 0, strictPort: false },
});
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("Vite did not expose a local browser URL.");
const browser = await chromium.launch({
  headless: true,
  ...(process.env.W310_CHROMIUM_EXECUTABLE ? { executablePath: process.env.W310_CHROMIUM_EXECUTABLE } : {}),
});

async function installMeasurementObserver(context) {
  await context.addInitScript(() => {
    window.localStorage.setItem("zc-onboarding-complete", "true");
    window.localStorage.setItem("zc-language", "en");
  });
  await context.addInitScript(({ metricDefinition, shellTargetMs, usefulTargetMs }) => {
    const state = {
      metricDefinition,
      shellTargetMs,
      usefulTargetMs,
      shellSamples: [],
      usefulSamples: [],
      triggerLabel: null,
      triggerAt: null,
      usefulAt: null,
    };
    const testWindow = window;
    testWindow.__zcW310 = state;
    testWindow.__zcW310MarkPreviewTrigger = (label) => {
      state.triggerLabel = label;
      state.triggerAt = performance.now();
      state.usefulAt = state.triggerAt;
    };
    const isActuallyVisible = (element) => {
      const style = getComputedStyle(element);
      const rect = element.getBoundingClientRect();
      return style.display !== "none"
        && style.visibility !== "hidden"
        && Number(style.opacity) > 0
        && rect.width > 0
        && rect.height > 0;
    };
    const record = () => {
      const shell = document.querySelector('[data-preview-shell="true"]');
      if (!shell || !isActuallyVisible(shell) || state.triggerAt === null) return;
      const elapsed = performance.now() - state.triggerAt;
      if (!state.shellSamples.some((sample) => sample.label === state.triggerLabel)) {
        state.shellSamples.push({
          label: state.triggerLabel,
          elapsedMs: elapsed,
          state: shell.getAttribute("data-preview-state"),
          actualDomVisibilityMeasured: true,
        });
      }
      const representation = [...document.querySelectorAll("[data-preview-representation]")]
        .find((candidate) => isActuallyVisible(candidate));
      const usefulImage = representation?.getAttribute("data-preview-representation") !== "image"
        || representation?.getAttribute("data-preview-image-status") !== "loading";
      if (representation && usefulImage && state.usefulAt !== null
        && !state.usefulSamples.some((sample) => sample.label === state.triggerLabel)) {
        state.usefulSamples.push({
          label: state.triggerLabel,
          elapsedMs: performance.now() - state.usefulAt,
          family: representation.getAttribute("data-preview-representation"),
          actualDomVisibilityMeasured: true,
        });
      }
    };
    const startObserver = () => {
      const root = document.documentElement;
      if (!root) return;
      const observer = new MutationObserver(record);
      observer.observe(root, { childList: true, subtree: true, attributes: true });
      window.requestAnimationFrame(record);
      setInterval(record, 16);
    };
    if (document.documentElement) startObserver();
    else document.addEventListener("DOMContentLoaded", startObserver, { once: true });
  }, {
    metricDefinition: PREVIEW_PERFORMANCE_CONTRACT.metricDefinition,
    shellTargetMs: PREVIEW_PERFORMANCE_CONTRACT.shellFirstVisibleTargetP95Ms,
    usefulTargetMs: PREVIEW_PERFORMANCE_CONTRACT.usefulRepresentationTargetP95Ms,
  });
}

async function waitForLibrary(page) {
  const navigation = page.getByRole("button", { name: "Files", exact: true });
  await navigation.waitFor({ state: "visible" });
  for (let attempt = 0; attempt < 3; attempt += 1) {
    if (await page.locator(".file-library-workspace").count() > 0) break;
    await navigation.click();
    try {
      await page.waitForSelector(".file-library-workspace", { timeout: 5_000 });
      break;
    } catch (error) {
      if (attempt === 2) {
        const diagnostics = await page.evaluate(() => ({
          modes: [...document.querySelectorAll(".file-library-workspace")].map((element) => element.getAttribute("data-mode")),
          navigationCurrent: document.querySelector('[aria-current="page"]')?.textContent?.trim() ?? null,
        }));
        throw new Error(`File Library did not mount ${JSON.stringify(diagnostics)} (${error.message})`);
      }
    }
  }
  const libraryTab = page.locator('[role="tab"][data-file-library-mode="library"]');
  await libraryTab.waitFor({ state: "visible" });
  if (await page.locator('.file-library-workspace[data-mode="library"]').count() === 0) {
    await libraryTab.click();
    await page.waitForSelector('.file-library-workspace[data-mode="library"]', { state: "visible" });
  }
  const allIndexedFiles = page.getByRole("button", { name: "View all indexed files", exact: true });
  if (await allIndexedFiles.count() > 0 && await allIndexedFiles.first().isVisible()) await allIndexedFiles.first().click();
  const list = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="library"]');
  await list.waitFor({ state: "visible" });
  await list.locator('[role="option"]').first().waitFor({ state: "visible" });
  return list;
}

async function chooseLibraryFile(page, name) {
  const list = await waitForLibrary(page);
  const search = page.locator('[data-file-library-local-search="true"]');
  await search.fill(name);
  await page.waitForFunction((expected) => {
    const input = document.querySelector('[data-file-library-local-search="true"]');
    return input?.value === expected
      && document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-provenance") === "query-v2-snapshot";
  }, name);
  const item = list.locator('[role="option"]').filter({ hasText: name }).first();
  await item.waitFor({ state: "visible" });
  await page.waitForFunction(() => {
    const count = Number(document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("data-file-library-logical-count") ?? 0);
    return count === 1;
  });
  const itemId = await item.getAttribute("id");
  if (itemId === null) throw new Error(`Could not identify selected ${name} row`);
  const alreadyActive = await item.getAttribute("aria-selected") === "true"
    && await list.getAttribute("aria-activedescendant") === itemId;
  if (alreadyActive) {
    await list.focus();
    await list.press("Escape");
    await page.waitForFunction(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-selection-kind") === "none"
      && document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") === null);
  }
  await item.focus();
  await item.evaluate((element) => element instanceof HTMLElement && element.click());
  await page.waitForFunction((id) => {
    const row = id === null ? null : document.getElementById(id);
    return row?.getAttribute("aria-selected") === "true"
      && row.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") === id;
  }, itemId);
  return list;
}

async function chooseBrowseFile(page, name) {
  const browseTab = page.getByRole("tab", { name: "Browse Folder", exact: true });
  if (await browseTab.count() === 0) await waitForLibrary(page);
  const browseWorkspace = page.locator('.file-library-workspace[data-mode="browse"]');
  await browseTab.click();
  await browseWorkspace.waitFor({ state: "visible" });
  if (await page.locator('[data-browse-state="current-folder"]').count() === 0) {
    const openable = page.locator('[data-browse-location-openable="true"] [data-browse-location-action="open"]');
    await openable.first().waitFor({ state: "visible" });
    await openable.first().click();
  }
  await page.locator('[data-browse-state="current-folder"]').waitFor({ state: "visible" });
  const list = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="browse"]');
  await list.locator('[role="option"]').first().waitFor({ state: "visible" });
  const search = page.locator('[data-file-library-local-search="true"][data-file-library-local-search-state]');
  await search.fill(name);
  await page.waitForFunction((query) => {
    const browse = document.querySelector('[data-browse-source-owner="browse"]');
    const searchState = browse?.getAttribute("data-browse-search-state");
    return browse?.getAttribute("data-browse-query") === query
      && browse?.getAttribute("data-browse-provenance") === "browse-enumeration"
      && (searchState === "partial" || searchState === "complete");
  }, name);
  const item = list.locator('[role="option"]').filter({ hasText: name }).first();
  await item.waitFor({ state: "visible" });
  const itemId = await item.getAttribute("id");
  if (itemId === null) throw new Error(`Could not identify Browse row for ${name}`);
  await item.evaluate((element) => element instanceof HTMLElement && element.click());
  await page.waitForFunction((id) => {
    const row = document.getElementById(id);
    return row?.getAttribute("aria-selected") === "true"
      && row.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") === id;
  }, itemId);
  return list;
}

async function resolveDeferredPreview(page, label) {
  await page.waitForFunction(() => (window.__zcW302?.pendingStartCount ?? 0) > 0);
  for (let attempt = 0; attempt < 40; attempt += 1) {
    await page.evaluate(() => window.__zcW302?.resolveAll());
    await page.evaluate(() => Promise.resolve());
    try {
      await page.waitForFunction(() => {
        const pending = window.__zcW302?.pendingStartCount ?? 0;
        const state = document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null;
        return pending === 0 && ["content", "metadata_fallback", "unsupported_representation"].includes(state ?? "");
      }, undefined, { polling: "raf", timeout: 250 });
      return;
    } catch {
      // The deferred mock is released above; continue yielding to the browser
      // event loop until the controller publishes the settled snapshot.
    }
  }
  const diagnostics = await page.evaluate(() => ({
    w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
    state: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
    source: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-source") ?? null,
    selectionOwner: document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-selection-kind") ?? null,
    activeDescendant: document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") ?? null,
    selected: document.querySelector('[data-shared-file-list-source="library"] [aria-selected="true"]')?.getAttribute("id") ?? null,
    representation: document.querySelector('[data-preview-representation]')?.getAttribute("data-preview-representation") ?? null,
  }));
  throw new Error(`${label}: deferred Preview did not settle ${JSON.stringify(diagnostics)}`);
}

async function openPreview(page, list, label) {
  await page.evaluate((value) => window.__zcW310MarkPreviewTrigger?.(value), label);
  await list.focus();
  if (await list.getAttribute("aria-activedescendant") === null) await page.keyboard.press("ArrowDown");
  const activeId = await list.getAttribute("aria-activedescendant");
  if (activeId !== null) {
    const activeRow = page.locator(`#${activeId}`);
    if (await activeRow.getAttribute("aria-selected") !== "true") {
      await activeRow.focus();
      await activeRow.evaluate((element) => element instanceof HTMLElement && element.click());
      await page.waitForFunction((id) => {
        const row = id === null ? null : document.getElementById(id);
        return row?.getAttribute("aria-selected") === "true"
          && row.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") === id;
      }, activeId);
    }
  }
  await list.press("Space");
  await page.waitForSelector('[data-preview-host="zen-floating"]', { timeout: 5_000 }).catch(async (error) => {
    const diagnostics = await page.evaluate(() => ({
      activeDescendant: document.querySelector('[data-shared-file-list-source="library"], [data-shared-file-list-source="browse"]')?.getAttribute("aria-activedescendant") ?? null,
      selected: document.querySelector('[data-shared-file-list-source="library"] [aria-selected="true"], [data-shared-file-list-source="browse"] [aria-selected="true"]')?.getAttribute("id") ?? null,
      state: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
      w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
    }));
    throw new Error(`${label}: Floating Preview did not open ${JSON.stringify(diagnostics)} (${String(error)})`);
  });
  await page.waitForSelector('[data-preview-shell="true"]');
  if (await page.evaluate(() => new URLSearchParams(window.location.search).get("w3-02-browser-fixture") === "preview")) {
    await resolveDeferredPreview(page, label);
  }
}

async function closePreview(page, label) {
  await page.locator('[data-preview-host="zen-floating"] [aria-label="Close preview"]').click();
  await page.waitForSelector('[data-preview-shell="true"]', { state: "detached" });
  assert(await page.locator('[data-preview-shell="true"]').count() === 0, `${label}: Preview remained mounted`);
  await page.waitForFunction(() => {
    const state = window.__zcW302;
    return state === undefined
      || ((state.pendingStartCount ?? 0) === 0
        && Object.keys(state.activeBackendHostKinds ?? {}).length === 0);
  }, undefined, { timeout: 5_000 });
}

function summarizeBrowserTiming(samples, targetP95Ms) {
  assert(samples.length === PREVIEW_PERFORMANCE_CONTRACT.timingSamples, `Expected ${PREVIEW_PERFORMANCE_CONTRACT.timingSamples} browser samples, got ${samples.length}`);
  const values = samples.map((sample) => sample.elapsedMs).sort((left, right) => left - right);
  const percentile = (p) => values[Math.ceil((values.length - 1) * p)];
  return {
    metricDefinition: PREVIEW_PERFORMANCE_CONTRACT.metricDefinition,
    metricKind: "timing_percentile",
    unit: "ms",
    percentileMethod: "nearest_observed_rank_ceil_n_minus_1_times_p",
    measurementBoundary: "browser_accepted_command_to_visible_dom",
    warmupCount: PREVIEW_PERFORMANCE_CONTRACT.warmupSamples,
    sampleCount: values.length,
    minMs: values[0],
    p50Ms: percentile(0.5),
    p95Ms: percentile(0.95),
    maxMs: values.at(-1),
    targetP95Ms,
    actualDomVisibilityMeasured: samples.every((sample) => sample.actualDomVisibilityMeasured === true),
  };
}

async function collectScenario(page, name, fileName, representation, chooseFile = chooseLibraryFile) {
  const shellSamples = [];
  const usefulSamples = [];
  const totalSamples = PREVIEW_PERFORMANCE_CONTRACT.warmupSamples + PREVIEW_PERFORMANCE_CONTRACT.timingSamples;
  for (let index = 0; index < totalSamples; index += 1) {
    const sampleLabel = `${name}-${index.toString().padStart(2, "0")}`;
    const list = await chooseFile(page, fileName);
    await openPreview(page, list, sampleLabel);
    try {
      await page.locator(`[data-preview-representation="${representation}"]`).waitFor({ state: "visible" });
    } catch (error) {
      const diagnostics = await page.evaluate(() => ({
        state: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
        representations: [...document.querySelectorAll("[data-preview-representation]")].map((element) => ({
          family: element.getAttribute("data-preview-representation"),
          visible: element.getBoundingClientRect().width > 0 && element.getBoundingClientRect().height > 0,
        })),
        w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
      }));
      throw new Error(`${sampleLabel}: expected ${representation} did not mount ${JSON.stringify(diagnostics)} (${error.message})`);
    }
    if (representation === "image") {
      await page.waitForFunction(() => (window.__zcW306?.pendingAssetCount ?? 0) > 0);
      await page.evaluate(() => window.__zcW306?.resolveAllAssets());
      await page.waitForFunction(() => document.querySelector('[data-preview-representation="image"]')?.getAttribute("data-preview-image-status") !== "loading");
    }
    const evidence = await page.evaluate((label) => ({
      shell: window.__zcW310?.shellSamples?.find((sample) => sample.label === label) ?? null,
      useful: window.__zcW310?.usefulSamples?.find((sample) => sample.label === label) ?? null,
    }), sampleLabel);
    assert(evidence.shell?.actualDomVisibilityMeasured === true, `${sampleLabel}: shell visibility was not measured from the DOM ${JSON.stringify(evidence)}`);
    assert(evidence.useful?.actualDomVisibilityMeasured === true, `${sampleLabel}: useful representation was not measured from the DOM ${JSON.stringify(evidence)}`);
    if (index >= PREVIEW_PERFORMANCE_CONTRACT.warmupSamples) {
      shellSamples.push(evidence.shell);
      usefulSamples.push(evidence.useful);
    }
    await closePreview(page, sampleLabel);
  }
  return {
    label: name,
    shell: summarizeBrowserTiming(shellSamples, PREVIEW_PERFORMANCE_CONTRACT.shellFirstVisibleTargetP95Ms),
    useful: summarizeBrowserTiming(usefulSamples, PREVIEW_PERFORMANCE_CONTRACT.usefulRepresentationTargetP95Ms),
    ...(process.env.ZC_QUALIFICATION_OUTPUT ? { shellSamples, usefulSamples } : {}),
  };
}

async function navigateToFixture(page, query) {
  await page.goto(`${baseUrl}?${query}`, { waitUntil: "commit" });
  await page.waitForSelector("#root", { state: "attached" });
  await page.getByRole("button", { name: "Files", exact: true }).waitFor({ state: "visible" });
}

const SCENARIOS = Object.freeze([
  ["library-structured", "structured-sample.json", "structured_tree", FIXTURE_QUERIES.structured, chooseLibraryFile],
  ["browse-structured", "structured-sample.json", "structured_tree", FIXTURE_QUERIES.structured, chooseBrowseFile],
  ["library-image", "image-sample.png", "image", FIXTURE_QUERIES.image, chooseLibraryFile],
  ["library-text", "bounded-prefix.txt", "text", FIXTURE_QUERIES.text, chooseLibraryFile],
  ["library-source-code", "preview-fixture.rs", "text", FIXTURE_QUERIES.text, chooseLibraryFile],
  ["library-markdown", "W3-04-hostile.md", "safe_html", FIXTURE_QUERIES.text, chooseLibraryFile],
  ["library-table", "structured-records.csv", "table", FIXTURE_QUERIES.structured, chooseLibraryFile],
  ["browse-folder", "w3-07-mixed-folder", "folder_summary", FIXTURE_QUERIES.folder, chooseBrowseFile],
  ["library-archive", "archive-sample.zip", "archive_tree", FIXTURE_QUERIES.archive, chooseLibraryFile],
]);

async function runViewport(viewport) {
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  await installMeasurementObserver(context);
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  page.setDefaultNavigationTimeout(60_000);
  const errors = [];
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  page.on("pageerror", (error) => errors.push(String(error)));
  const evidence = [];

  try {
    for (const [name, fileName, representation, fixtureQuery, chooseFile] of SCENARIOS) {
      await navigateToFixture(page, fixtureQuery);
      const scenarioEvidence = await collectScenario(page, name, fileName, representation, chooseFile);
      evidence.push({ ...scenarioEvidence, viewport });
    }
    assert(errors.length === 0, `W3-10 browser timing matrix ${viewport.width}x${viewport.height} reported console/page errors: ${JSON.stringify(errors)}`);
    return { viewport, evidence, errors };
  } finally {
    await page.close();
    await context.close();
  }
}

try {
  const evidence = [];
  for (const viewport of VIEWPORTS) evidence.push(await runViewport(viewport));
  await writeFile(path.join(ARTIFACT_DIR, "phase-a-browser-metrics.json"), JSON.stringify({
    sourceHead: SOURCE_HEAD,
    actualCheckoutSha: ACTUAL_CHECKOUT_SHA,
    actualCheckoutTree: ACTUAL_CHECKOUT_TREE,
    metricDefinition: PREVIEW_PERFORMANCE_CONTRACT.metricDefinition,
    phaseBMetricDefinition: PREVIEW_PERFORMANCE_CONTRACT.phaseBMetricDefinition ?? null,
    shellTargetP95Ms: PREVIEW_PERFORMANCE_CONTRACT.shellFirstVisibleTargetP95Ms,
    usefulRepresentationTargetP95Ms: PREVIEW_PERFORMANCE_CONTRACT.usefulRepresentationTargetP95Ms,
    warmupSamples: PREVIEW_PERFORMANCE_CONTRACT.warmupSamples,
    measuredSamples: PREVIEW_PERFORMANCE_CONTRACT.timingSamples,
    viewports: VIEWPORTS,
    evidence,
  }, null, 2));
  console.log(`[w3-10-phase-a-browser] OBSERVED shell/useful DOM timing matrix sourceHead=${SOURCE_HEAD} actualSha=${ACTUAL_CHECKOUT_SHA} tree=${ACTUAL_CHECKOUT_TREE}`);
  console.log(JSON.stringify({ metricDefinition: PREVIEW_PERFORMANCE_CONTRACT.metricDefinition, viewports: VIEWPORTS, evidence }, null, 2));
  if (process.env.ZC_QUALIFICATION_OUTPUT) {
    const output = path.resolve(process.env.ZC_QUALIFICATION_OUTPUT);
    const boundary = path.resolve('.performance-artifacts/qualification') + path.sep;
    assert(output.startsWith(boundary), 'Qualification output must stay inside its evidence boundary');
    await mkdir(path.dirname(output), { recursive: true });
    await writeFile(output, JSON.stringify({ sourceHead: SOURCE_HEAD, actualCheckoutSha: ACTUAL_CHECKOUT_SHA, actualCheckoutTree: ACTUAL_CHECKOUT_TREE, evidence }, null, 2));
  }
} finally {
  await browser.close();
  await server.close();
  await rm(ARTIFACT_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  await rm(TASK_TEMP_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
}
