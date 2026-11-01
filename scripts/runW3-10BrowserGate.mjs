import { mkdir, rm, writeFile } from "node:fs/promises";
import { execFileSync, spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";
import { createServer } from "vite";
import {
  assert,
  assertArchivePreview,
  assertNoHorizontalOverflow,
  chooseLibraryFile,
  openFloating,
  pinPreview,
  resolveDeferredPreview,
  trackPageSecurity,
  unpinPreview,
  waitForApp,
} from "./w3PreviewBrowserHarness.mjs";

const actualCheckoutSha = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const actualCheckoutTree = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
const sourceHead = process.env.W310_SOURCE_HEAD ?? process.env.GITHUB_SHA ?? actualCheckoutSha;
const expectedCheckoutSha = process.env.W310_EXPECTED_CHECKOUT_SHA ?? actualCheckoutSha;
const VIEWPORTS = Object.freeze([
  { width: 1600, height: 900 },
  { width: 980, height: 680 },
]);
const FIXTURE_QUERY = "w3-09-browser-fixture=integration&w3-02-browser-fixture=preview&w3-03-browser-fixture=pinned&w3-04-browser-fixture=providers&w3-05-browser-fixture=providers&w3-06-browser-fixture=images&w3-07-browser-fixture=folders&w3-08-browser-fixture=archives";
const TASK_TEMP_DIR = path.resolve(`.tmp-tests/w3-10-browser-runtime-${process.pid}`);
const ARTIFACT_DIR = path.resolve(`.tmp-tests/w3-10-browser-gate-${process.pid}`);

async function chooseArchiveAfterBurst(page, name) {
  const list = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="library"]').first();
  await list.waitFor({ state: "visible" });
  const item = list.locator('[role="option"]').filter({ hasText: name }).first();
  for (let attempt = 0; attempt < 4 && !(await item.isVisible()); attempt += 1) {
    const loadMore = page.locator(".shared-file-list-load-more").first();
    if (await loadMore.count() > 0 && await loadMore.isVisible()) {
      await loadMore.click();
    } else {
      await list.evaluate((element) => {
        element.scrollTop = element.scrollHeight;
        element.dispatchEvent(new Event("scroll", { bubbles: true }));
      });
    }
    await page.waitForFunction((expected) => [...document.querySelectorAll('[data-shared-file-list-source="library"] [role="option"]')]
      .some((row) => row.textContent?.includes(expected)), name);
  }
  await item.waitFor({ state: "visible" });
  const itemId = await item.getAttribute("id");
  assert(itemId !== null, `W3-10 rapid-switch: could not identify ${name}`);
  await item.focus();
  await item.evaluate((element) => element instanceof HTMLElement && element.click());
  await page.waitForFunction((id) => {
    const row = id === null ? null : document.getElementById(id);
    return row?.getAttribute("aria-selected") === "true"
      && row.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") === id;
  }, itemId);
  return { list, item, itemId };
}

async function clearLibrarySelectionAfterUnpin(page, list) {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const cleared = await page.evaluate(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-selection-kind") === "none"
      && document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") === null);
    if (cleared) return;
    await list.focus();
    await list.press("Escape");
    await page.waitForFunction(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-selection-kind") === "none"
      && document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") === null,
      undefined, { timeout: 5_000 }).catch(() => undefined);
  }
  const state = await page.evaluate(() => ({
    owner: document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-selection-kind") ?? null,
    activeDescendant: document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") ?? null,
  }));
  throw new Error(`W3-10 rapid-switch: library selection did not clear after unpin ${JSON.stringify(state)}`);
}

process.env.TEMP = TASK_TEMP_DIR;
process.env.TMP = TASK_TEMP_DIR;
process.env.TMPDIR = TASK_TEMP_DIR;
await mkdir(TASK_TEMP_DIR, { recursive: true });
await mkdir(ARTIFACT_DIR, { recursive: true });

function runGate(script, env) {
  const result = spawnSync(
    process.execPath,
    [path.resolve("scripts", script)],
    {
      cwd: process.cwd(),
      env: { ...process.env, ...env },
      stdio: "inherit",
    },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

runGate("runW3-10PhaseABrowserHarness.mjs", {
  W310_SOURCE_HEAD: sourceHead,
  W310_EXPECTED_CHECKOUT_SHA: expectedCheckoutSha,
});

const server = await createServer({
  configFile: path.resolve("vite.config.ts"),
  server: { host: "127.0.0.1", port: 0, strictPort: false },
});
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("Vite did not expose a local browser URL.");
const appOrigin = new URL(baseUrl).origin;
const browser = await chromium.launch({
  headless: true,
  ...(process.env.W310_CHROMIUM_EXECUTABLE ? { executablePath: process.env.W310_CHROMIUM_EXECUTABLE } : {}),
});

async function installBrowserEvidence(context) {
  await context.addInitScript(() => {
    window.localStorage.setItem("zc-onboarding-complete", "true");
    window.localStorage.setItem("zc-language", "en");
    const nativeCreate = URL.createObjectURL.bind(URL);
    const nativeRevoke = URL.revokeObjectURL.bind(URL);
    const created = [];
    const revoked = [];
    const live = new Set();
    URL.createObjectURL = (blob) => {
      const url = nativeCreate(blob);
      created.push(url);
      live.add(url);
      return url;
    };
    URL.revokeObjectURL = (url) => {
      revoked.push(url);
      live.delete(url);
      nativeRevoke(url);
    };
    window.__zcW310Browser = { created, revoked, get live() { return [...live]; } };
  });
}

async function runRapidSwitchViewport(viewport) {
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  await installBrowserEvidence(context);
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  page.setDefaultNavigationTimeout(60_000);
  const security = trackPageSecurity(page, appOrigin);
  try {
    await page.goto(`${baseUrl}?${FIXTURE_QUERY}`, { waitUntil: "commit" });
    await waitForApp(page, `W3-10 rapid-switch ${viewport.width}x${viewport.height}`);

    const imageSelection = await chooseLibraryFile(page, "image-sample.png", { unfiltered: true });
    await openFloating(page, imageSelection.list, "W3-10 rapid-switch image seed");
    const image = page.locator('[data-preview-representation="image"]');
    await image.waitFor({ state: "visible" }).catch(async (error) => {
      const diagnostics = await page.evaluate(() => ({
        state: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
        source: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-source") ?? null,
        representations: [...document.querySelectorAll('[data-preview-representation]')].map((element) => ({
          family: element.getAttribute("data-preview-representation"),
          visible: element.getBoundingClientRect().width > 0 && element.getBoundingClientRect().height > 0,
        })),
        selected: document.querySelector('[data-shared-file-list-source="library"] [aria-selected="true"]')?.textContent?.trim() ?? null,
        activeDescendant: document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") ?? null,
        w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
        w306: window.__zcW306 ? JSON.parse(JSON.stringify(window.__zcW306)) : null,
      }));
      throw new Error(`W3-10 rapid-switch ${viewport.width}x${viewport.height}: image seed did not mount ${JSON.stringify(diagnostics)} (${String(error)})`);
    });
    await page.waitForFunction(() => (window.__zcW306?.pendingAssetCount ?? 0) > 0);
    await page.evaluate(() => window.__zcW306?.resolveAllAssets());
    await page.waitForFunction(() => document.querySelector('[data-preview-representation="image"]')?.getAttribute("data-preview-image-status") === "ready").catch(async (error) => {
      const diagnostics = await page.evaluate(() => ({
        state: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
        source: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-source") ?? null,
        imageStatus: document.querySelector('[data-preview-representation="image"]')?.getAttribute("data-preview-image-status") ?? null,
        representations: [...document.querySelectorAll('[data-preview-representation]')].map((element) => ({
          family: element.getAttribute("data-preview-representation"),
          visible: element.getBoundingClientRect().width > 0 && element.getBoundingClientRect().height > 0,
        })),
        w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
        w306: window.__zcW306 ? JSON.parse(JSON.stringify(window.__zcW306)) : null,
        w310: window.__zcW310Browser ? JSON.parse(JSON.stringify(window.__zcW310Browser)) : null,
      }));
      throw new Error(`W3-10 rapid-switch ${viewport.width}x${viewport.height}: image did not become ready ${JSON.stringify(diagnostics)} (${String(error)})`);
    });
    const imageLifecycle = await page.evaluate(() => JSON.parse(JSON.stringify(window.__zcW310Browser ?? {})));
    assert(imageLifecycle.created?.length > 0, `W3-10 rapid-switch ${viewport.width}x${viewport.height}: image object URL was not created`);

    const structuredSelection = await chooseLibraryFile(page, "structured-sample.json", { unfiltered: true });
    const finalSelection = await chooseArchiveAfterBurst(page, "archive-sample.zip");
    await page.waitForFunction(() => (window.__zcW302?.pendingStartCount ?? 0) > 0);
    await resolveDeferredPreview(page, "W3-10 rapid-switch latest-wins burst");

    await assertArchivePreview(page, "W3-10 rapid-switch final archive");
    const finalState = await page.evaluate(() => ({
      shellCount: document.querySelectorAll('[data-preview-shell="true"]').length,
      floatingCount: document.querySelectorAll('[data-preview-host="zen-floating"]').length,
      pinnedCount: document.querySelectorAll('[data-preview-host="zen-pinned"]').length,
      identity: document.querySelector('[data-preview-host="zen-floating"] [data-preview-card="true"]')?.getAttribute("data-preview-identity") ?? null,
      source: document.querySelector('[data-preview-host="zen-floating"] [data-preview-card="true"]')?.getAttribute("data-preview-source") ?? null,
      backendSources: Object.entries(window.__zcW302?.activeBackendSources ?? {}).filter(([previewId]) =>
        window.__zcW302?.activeBackendHostKinds[previewId] === "zen_floating"),
      representations: [...document.querySelectorAll('[data-preview-host="zen-floating"] [data-preview-representation]')]
        .map((element) => element.getAttribute("data-preview-representation")),
      selectedId: document.querySelector('[data-shared-file-list-source="library"] [aria-selected="true"]')?.getAttribute("id") ?? null,
      activeDescendant: document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") ?? null,
      focusInsideFloating: document.activeElement?.closest('[data-preview-host="zen-floating"]') !== null,
      lateStarts: window.__zcW302?.lateStarts ?? null,
      switchCalls: window.__zcW302?.switchCalls ?? null,
      pendingStartCount: window.__zcW302?.pendingStartCount ?? null,
    }));
    assert(finalState.shellCount === 1 && finalState.floatingCount === 1 && finalState.pinnedCount === 0,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: duplicate Preview host ${JSON.stringify(finalState)}`);
    assert(finalState.identity?.includes("w3-08-sample") === true,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: final source identity did not win ${JSON.stringify(finalState)}`);
    assert(finalState.source === "library", `W3-10 rapid-switch ${viewport.width}x${viewport.height}: final source kind was not library ${JSON.stringify(finalState)}`);
    assert(finalState.backendSources.length === 1
      && finalState.backendSources[0]?.[1]?.kind === "managed"
      && finalState.backendSources[0]?.[1]?.fileId === finalState.identity,
    `W3-10 rapid-switch ${viewport.width}x${viewport.height}: Preview Controller and backend source identity diverged ${JSON.stringify(finalState)}`);
    assert(finalState.representations.length === 1 && finalState.representations[0] === "archive_tree",
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: stale representation remained ${JSON.stringify(finalState)}`);
    assert(finalState.selectedId === finalSelection.itemId && finalState.activeDescendant === finalSelection.itemId,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: final selection did not match final Preview source ${JSON.stringify({ finalState, structuredId: structuredSelection.itemId, finalId: finalSelection.itemId })}`);
    assert(finalState.focusInsideFloating === true,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: focus escaped Floating Preview ownership ${JSON.stringify(finalState)}`);
    assert(finalState.switchCalls >= 2 && finalState.pendingStartCount === 0,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: deterministic switch burst did not settle ${JSON.stringify(finalState)}`);
    assert(finalState.lateStarts >= 1,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: stale deferred completion was not exercised ${JSON.stringify(finalState)}`);

    const afterSwitchLifecycle = await page.evaluate(() => JSON.parse(JSON.stringify(window.__zcW310Browser ?? {})));
    assert((afterSwitchLifecycle.live ?? []).length === 0,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: image object URL remained live after source switch ${JSON.stringify(afterSwitchLifecycle)}`);
    assert((afterSwitchLifecycle.created ?? []).every((url) => (afterSwitchLifecycle.revoked ?? []).includes(url)),
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: image object URL was not revoked ${JSON.stringify(afterSwitchLifecycle)}`);
    await assertNoHorizontalOverflow(page, `W3-10 rapid-switch final Floating ${viewport.width}x${viewport.height}`);

    await pinPreview(page, viewport, `W3-10 rapid-switch Pinned ${viewport.width}x${viewport.height}`);
    await assertArchivePreview(page, `W3-10 rapid-switch Pinned ${viewport.width}x${viewport.height}`, "zen-pinned");
    const pinnedState = await page.evaluate(() => ({
      shellCount: document.querySelectorAll('[data-preview-shell="true"]').length,
      floatingCount: document.querySelectorAll('[data-preview-host="zen-floating"]').length,
      pinnedCount: document.querySelectorAll('[data-preview-host="zen-pinned"]').length,
      identity: document.querySelector('[data-preview-host="zen-pinned"] [data-preview-card="true"]')?.getAttribute("data-preview-identity") ?? null,
    }));
    assert(pinnedState.shellCount === 1 && pinnedState.floatingCount === 0 && pinnedState.pinnedCount === 1,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: Floating/Pinned ownership was not exclusive ${JSON.stringify(pinnedState)}`);
    assert(pinnedState.identity?.includes("w3-08-sample") === true,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: Pinned source identity changed ${JSON.stringify(pinnedState)}`);
    await assertNoHorizontalOverflow(page, `W3-10 rapid-switch Pinned ${viewport.width}x${viewport.height}`);
    await unpinPreview(page, `W3-10 rapid-switch Unpin ${viewport.width}x${viewport.height}`);

    const reopenedList = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="library"]').first();
    await reopenedList.waitFor({ state: "visible" });
    await clearLibrarySelectionAfterUnpin(page, reopenedList);
    const reopened = await chooseArchiveAfterBurst(page, "archive-sample.zip");
    await openFloating(page, reopened.list, `W3-10 rapid-switch close focus ${viewport.width}x${viewport.height}`);
    await page.keyboard.press("Escape");
    await page.waitForSelector('[data-preview-shell="true"]', { state: "detached" });
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => resolve(null))));
    const restoredFocus = await page.evaluate(() => ({
      shellCount: document.querySelectorAll('[data-preview-shell="true"]').length,
      listSource: document.activeElement?.closest('[data-shared-file-list="true"]')?.getAttribute("data-shared-file-list-source") ?? null,
      activeDescendant: document.activeElement?.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") ?? null,
    }));
    assert(restoredFocus.shellCount === 0 && restoredFocus.listSource === "library" && restoredFocus.activeDescendant === finalSelection.itemId,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: keyboard focus was not restored to the originating list ${JSON.stringify(restoredFocus)}`);
    await assertNoHorizontalOverflow(page, `W3-10 rapid-switch closed ${viewport.width}x${viewport.height}`);

    assert(security.networkViolations.length === 0,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: unexpected navigation/resource request ${JSON.stringify(security.networkViolations)}`);
    assert(security.errors.length === 0,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: console/page errors ${JSON.stringify(security.errors)}`);
    const lifecycle = await page.evaluate(() => JSON.parse(JSON.stringify(window.__zcW310Browser ?? {})));
    assert((lifecycle.live ?? []).length === 0,
      `W3-10 rapid-switch ${viewport.width}x${viewport.height}: object URLs remained live after close ${JSON.stringify(lifecycle)}`);
    return {
      viewport,
      sourceHead,
      actualCheckoutSha,
      actualCheckoutTree,
      security: {
        errors: security.errors,
        networkViolations: security.networkViolations,
        blobRequests: security.blobRequests,
      },
      imageLifecycle,
      finalState,
      afterSwitchLifecycle,
      pinnedState,
      restoredFocus,
      lifecycle,
    };
  } finally {
    await page.close();
    await context.close();
  }
}

const evidence = [];
let completed = false;
try {
  for (const viewport of VIEWPORTS) evidence.push(await runRapidSwitchViewport(viewport));
  await writeFile(path.join(ARTIFACT_DIR, "rapid-switch.json"), JSON.stringify({
    sourceHead,
    actualCheckoutSha,
    actualCheckoutTree,
    fixtureQuery: FIXTURE_QUERY,
    viewports: VIEWPORTS,
    evidence,
  }, null, 2));
  console.log(`[w3-10-rapid-switch] PASS sourceHead=${sourceHead} actualSha=${actualCheckoutSha} tree=${actualCheckoutTree}`);
  console.log(JSON.stringify({ fixtureQuery: FIXTURE_QUERY, evidence }, null, 2));
  completed = true;
} finally {
  await browser.close();
  await server.close();
  if (completed) await rm(ARTIFACT_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  await rm(TASK_TEMP_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
}

runGate("runW3-09BrowserGate.mjs", {
  W309_SOURCE_HEAD: sourceHead,
  W309_EXPECTED_CHECKOUT_SHA: expectedCheckoutSha,
});

console.log(`[w3-10-real] PASS sourceHead=${sourceHead} actualSha=${actualCheckoutSha} tree=${actualCheckoutTree}`);
