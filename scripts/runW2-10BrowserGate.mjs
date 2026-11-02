import { mkdir, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { assertCheckoutEvidence } from "./ciEvidence.mjs";
import { chromium } from "playwright";
import { createServer } from "vite";

const SOURCE_HEAD = process.env.W210_SOURCE_HEAD ?? process.env.GITHUB_SHA ?? "local";
const EXPECTED_CHECKOUT_SHA = process.env.W210_EXPECTED_CHECKOUT_SHA ?? null;
const ACTUAL_CHECKOUT_SHA = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const ACTUAL_CHECKOUT_TREE = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
if (EXPECTED_CHECKOUT_SHA) {
  assertCheckoutEvidence(EXPECTED_CHECKOUT_SHA, ACTUAL_CHECKOUT_SHA);
}
const TASK_TEMP_DIR = path.resolve(".tmp-tests/w2-10-browser-runtime");
const ARTIFACT_DIR = path.resolve(".tmp-tests/w2-10-browser-gate");
const FIXTURE_QUERY = "managed-library-test=1&w2-04-browser-fixture=source-owner&w2-05-browser-fixture=interaction&w2-09-browser-fixture=platform";
const SEARCH_MODIFIER = process.platform === "darwin" ? "Meta" : "Control";

process.env.TEMP = TASK_TEMP_DIR;
process.env.TMP = TASK_TEMP_DIR;
process.env.TMPDIR = TASK_TEMP_DIR;
await mkdir(TASK_TEMP_DIR, { recursive: true });
await mkdir(ARTIFACT_DIR, { recursive: true });

const server = await createServer({
  configFile: path.resolve("vite.config.ts"),
  server: { host: "127.0.0.1", port: 0, strictPort: false }
});
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("Vite did not expose a local browser URL.");

const browser = await chromium.launch({
  headless: true,
  ...(process.env.W210_CHROMIUM_EXECUTABLE ? { executablePath: process.env.W210_CHROMIUM_EXECUTABLE } : {})
});

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function waitForLibrary(page) {
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
  const allIndexedFiles = page.getByRole("button", { name: "View all indexed files", exact: true });
  if (await allIndexedFiles.count() > 0 && await allIndexedFiles.first().isVisible()) await allIndexedFiles.first().click();
  const commandSearch = page.locator('[data-file-library-command-search="true"] [data-file-library-local-search="true"]');
  await commandSearch.waitFor({ state: "visible" });
  assert(await commandSearch.count() === 1, "File Library did not expose exactly one command-bar search input");
  return commandSearch;
}

async function assertNoHorizontalOverflow(page, label) {
  const overflow = await page.evaluate(() => ({
    document: document.documentElement.scrollWidth > window.innerWidth + 1,
    body: document.body.scrollWidth > window.innerWidth + 1,
    workspace: [...document.querySelectorAll(".file-library-workspace")].some((element) => element.scrollWidth > element.clientWidth + 1)
  }));
  assert(!overflow.document && !overflow.body && !overflow.workspace, `${label}: unexpected horizontal overflow ${JSON.stringify(overflow)}`);
}

async function assertSearchShortcut(page, commandSearch) {
  const libraryList = page.locator('[data-shared-file-list-source="library"]');
  await libraryList.waitFor({ state: "visible" });
  await libraryList.focus();
  await page.keyboard.press(`${SEARCH_MODIFIER}+f`);
  assert(await page.evaluate(() => document.activeElement?.matches('[data-file-library-command-search="true"] [data-file-library-local-search="true"]') === true), "Ctrl/Cmd+F did not focus the File Library command-bar search");

  const filterButton = page.locator('[data-file-library-source-actions="true"] button[aria-haspopup="dialog"]');
  if (await filterButton.count() > 0) {
    await filterButton.click();
    const filterDialog = page.locator('[data-file-library-source-actions="true"] [role="dialog"]');
    await filterDialog.waitFor({ state: "visible" });
    const filterSelect = filterDialog.locator("select").first();
    if (await filterSelect.count() > 0) {
      await filterSelect.focus();
      await page.keyboard.press(`${SEARCH_MODIFIER}+f`);
      assert(await page.evaluate(() => document.activeElement?.tagName === "SELECT"), "Ctrl/Cmd+F stole focus from a filter select");
    }
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => document.querySelector('[data-file-library-source-actions="true"] button[aria-haspopup="dialog"]')?.getAttribute("aria-expanded") === "false");
  }
}

async function assertNoKeyboardContextMenu(page, surfaceSelector, expectedLabel) {
  const surface = page.locator(surfaceSelector);
  await surface.waitFor({ state: "visible" });
  await surface.focus();
  await page.keyboard.press("Shift+F10");
  await page.waitForTimeout(50);
  assert(await page.locator(`[role="menu"][aria-label="${expectedLabel}"]`).count() === 0, `${expectedLabel} opened without a logical target`);
}

async function openAndCloseKeyboardMenu(page, surfaceSelector, expectedLabel) {
  const surface = page.locator(surfaceSelector);
  await surface.waitFor({ state: "visible" });
  await surface.focus();
  await page.keyboard.press("ArrowDown");
  await page.waitForFunction((selector) => {
    const element = document.querySelector(selector);
    return element?.getAttribute("aria-activedescendant") !== null;
  }, surfaceSelector);
  const targetTitle = await page.evaluate((selector) => {
    const surfaceElement = document.querySelector(selector);
    const activeId = surfaceElement?.getAttribute("aria-activedescendant");
    const activeElement = activeId ? document.getElementById(activeId) : null;
    return activeElement?.querySelector(".shared-file-list-name strong, .shared-file-grid-name")?.textContent?.trim()
      ?? activeElement?.getAttribute("aria-label")?.split(",")[0]?.trim()
      ?? "";
  }, surfaceSelector);
  assert(Boolean(targetTitle), `${expectedLabel} did not establish a real logical focus target`);
  await page.keyboard.press("Shift+F10");
  const menu = page.locator(`[role="menu"][aria-label="${expectedLabel}"]`);
  await menu.waitFor({ state: "visible" });
  assert(await menu.getByRole("menuitem").count() > 0, `${expectedLabel} did not expose a menuitem`);
  const menuTitle = (await menu.locator("p").first().textContent())?.trim();
  assert(menuTitle === targetTitle, `${expectedLabel} targeted ${menuTitle ?? "<empty>"}, expected ${targetTitle}`);
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "detached" });
  await page.waitForTimeout(50);
  const restored = await page.evaluate((selector) => document.activeElement?.matches(selector) === true, surfaceSelector);
  assert(restored, `${expectedLabel} did not restore focus to its list/grid surface`);
}

async function openAndClosePointerMenu(page, surfaceSelector, expectedLabel) {
  const surface = page.locator(surfaceSelector);
  await surface.waitFor({ state: "visible" });
  const entry = surface.locator('[role="option"], [role="gridcell"]').first();
  await entry.click({ button: "right" });
  const menu = page.locator(`[role="menu"][aria-label="${expectedLabel}"]`);
  await menu.waitFor({ state: "visible" });
  assert(await menu.getByRole("menuitem").count() > 0, `${expectedLabel} did not expose a menuitem`);
  await page.keyboard.press("Escape");
  await menu.waitFor({ state: "detached" });
  await page.waitForTimeout(50);
  const restored = await page.evaluate((selector) => document.activeElement?.matches(selector) === true, surfaceSelector);
  assert(restored, `${expectedLabel} did not restore focus to its list/grid surface`);
}

async function openAndDismissMenuToSearch(page, surfaceSelector, search, expectedLabel) {
  const surface = page.locator(surfaceSelector);
  await surface.waitFor({ state: "visible" });
  await surface.locator('[role="option"], [role="gridcell"]').first().click({ button: "right" });
  const menu = page.locator(`[role="menu"][aria-label="${expectedLabel}"]`);
  await menu.waitFor({ state: "visible" });
  await search.click();
  await menu.waitFor({ state: "detached" });
  await page.waitForTimeout(50);
  const restored = await page.evaluate(() => document.activeElement?.matches('[data-file-library-command-search="true"] [data-file-library-local-search="true"]') === true);
  assert(restored, `${expectedLabel} outside-pointer dismissal stole focus from Search`);
}

async function exerciseLibrary(page, viewport) {
  const commandSearch = await waitForLibrary(page);
  for (const selector of [
    '[data-file-library-nav-toggle="true"]',
    '[data-file-library-view="list"]',
    '[data-file-library-view="grid"]',
    '[data-file-library-context-toggle="true"]'
  ]) {
    await page.locator(selector).waitFor({ state: "visible" });
  }
  assert(await page.getByRole("button", { name: "Back", exact: true }).count() === 1, "Back control missing");
  assert(await page.getByRole("button", { name: "Forward", exact: true }).count() === 1, "Forward control missing");
  await page.locator('[data-file-library-view="list"]').click();
  await page.locator('[data-shared-file-list-source="library"]').waitFor({ state: "visible" });
  await assertSearchShortcut(page, commandSearch);

  const list = page.locator('[data-shared-file-list-source="library"]');
  await assertNoKeyboardContextMenu(page, '[data-shared-file-list-source="library"]', "File actions menu");
  await openAndCloseKeyboardMenu(page, '[data-shared-file-list-source="library"]', "File actions menu");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  await page.locator('[data-file-library-view="grid"]').click();
  const grid = page.locator('[data-shared-file-grid-source="library"]');
  await grid.waitFor({ state: "visible" });
  await assertNoKeyboardContextMenu(page, '[data-shared-file-grid-source="library"]', "File actions menu");
  await openAndCloseKeyboardMenu(page, '[data-shared-file-grid-source="library"]', "File actions menu");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  await openAndClosePointerMenu(page, '[data-shared-file-grid-source="library"]', "File actions menu");
  await openAndDismissMenuToSearch(page, '[data-shared-file-grid-source="library"]', commandSearch, "File actions menu");

  const contextToggle = page.locator('[data-file-library-context-toggle="true"]');
  const navigationToggle = page.locator('[data-file-library-nav-toggle="true"]');
  await contextToggle.click();
  await page.waitForFunction(() => document.querySelector('.file-library-workspace')?.getAttribute("data-context-open") === "true");
  if (viewport.width < 1120) {
    await page.locator('[data-side-sheet="true"]').waitFor({ state: "visible" });
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => document.querySelector('.file-library-workspace')?.getAttribute("data-context-open") === "false");
    await page.waitForFunction(() => document.activeElement?.matches('[data-file-library-context-toggle="true"]') === true);
    await navigationToggle.click();
    await page.waitForFunction(() => document.querySelector('[data-file-library-nav-toggle="true"]')?.getAttribute("aria-expanded") === "true");
    assert(await page.locator('[data-side-sheet="true"]').count() === 1, "Compact Navigation did not own the single modal overlay");
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => document.querySelector('[data-file-library-nav-toggle="true"]')?.getAttribute("aria-expanded") === "false");
    await page.waitForFunction(() => document.activeElement?.matches('[data-file-library-nav-toggle="true"]') === true);
    await contextToggle.click();
    await page.waitForFunction(() => document.querySelector('.file-library-workspace')?.getAttribute("data-context-open") === "true");
    assert(await page.locator('[data-side-sheet="true"]').count() === 1, "Compact Context did not replace Navigation as the single modal overlay");
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => document.querySelector('.file-library-workspace')?.getAttribute("data-context-open") === "false");
    await page.waitForFunction(() => document.activeElement?.matches('[data-file-library-context-toggle="true"]') === true);
  } else {
    await contextToggle.click();
  }

  await page.locator('[data-file-library-mode="browse"]').click();
  await page.waitForSelector('.file-library-workspace[data-mode="browse"]');
}

async function openBrowseLocation(page) {
  if (await page.locator('[data-browse-state="current-folder"]').count() === 0) {
    const openable = page.locator('[data-browse-location-openable="true"] [data-browse-location-action="open"]');
    await openable.first().waitFor({ state: "visible" });
    await openable.first().click();
  }
  await page.locator('[data-browse-state="current-folder"]').waitFor({ state: "visible" });
}

async function exerciseBrowse(page) {
  await openBrowseLocation(page);
  await page.locator('[data-file-library-view="list"]').click();
  const browseList = page.locator('[data-shared-file-list-source="browse"]');
  await browseList.waitFor({ state: "visible" });
  const browseWorkspace = page.locator('[data-browse-source-owner="browse"]');
  const initialCompletion = await browseWorkspace.getAttribute("data-browse-enumeration-completion");
  const initialHasMore = await browseList.getAttribute("data-file-library-has-more");
  if (initialHasMore === "true") {
    assert(initialCompletion === "partial", `Browse reported hasMore without partial completion: ${initialCompletion}`);
    assert(await page.locator('[data-browse-enumeration-status="true"]').count() === 1, "Browse partial enumeration did not expose one live status");
  }

  await assertNoKeyboardContextMenu(page, '[data-shared-file-list-source="browse"]', "Browse item menu");
  await openAndCloseKeyboardMenu(page, '[data-shared-file-list-source="browse"]', "Browse item menu");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  await page.locator('[data-file-library-view="grid"]').click();
  const browseGrid = page.locator('[data-shared-file-grid-source="browse"]');
  await browseGrid.waitFor({ state: "visible" });
  if (initialHasMore === "true") assert(await browseGrid.getAttribute("aria-rowcount") === null, "Partial Browse grid exposed an exact aria-rowcount");
  await assertNoKeyboardContextMenu(page, '[data-shared-file-grid-source="browse"]', "Browse item menu");
  await openAndCloseKeyboardMenu(page, '[data-shared-file-grid-source="browse"]', "Browse item menu");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(50);
  await openAndClosePointerMenu(page, '[data-shared-file-grid-source="browse"]', "Browse item menu");

  await page.locator('[data-file-library-view="list"]').click();
  await browseList.waitFor({ state: "visible" });
  const browseSearch = page.locator('[data-file-library-command-search="true"] [data-file-library-local-search="true"]');
  await browseSearch.fill("notes");
  await page.locator('select[data-browse-query-kind]').selectOption("file");
  await page.waitForFunction(() => document.querySelector('[data-browse-query="notes"][data-browse-query-kind="file"]') !== null);
  const browseSort = page.locator('select[data-browse-sort]');
  await browseSort.waitFor({ state: "visible" });
  assert(await browseSort.isEnabled(), "Browse backend sort control is unavailable");
  await browseSort.selectOption("name");
  await page.locator('[data-browse-sort-capability="backend-snapshot"]').waitFor({ state: "visible" });
  await page.locator('[data-browse-entry]').filter({ hasText: "notes.md" }).waitFor({ state: "visible" });
  assert(await browseSort.inputValue() === "name", "Browse did not retain the selected backend sort");
  await browseSort.selectOption("native");

  await browseSearch.fill("");
  await page.locator('select[data-browse-query-kind]').selectOption("all");
  await page.waitForFunction(() => document.querySelector('[data-browse-query]') === null && document.querySelector('[data-browse-query-kind="all"]') !== null);
  await page.locator('[data-browse-entry-kind="directory"] button').first().waitFor({ state: "visible" });
  await page.locator('[data-browse-entry-kind="directory"] button').first().click();
  const breadcrumbs = page.locator('[data-browse-breadcrumbs]');
  await breadcrumbs.waitFor({ state: "visible" });
  await page.waitForFunction(() => document.querySelectorAll('[data-browse-breadcrumbs] .browse-breadcrumb').length > 0);
  const parent = breadcrumbs.locator('button:not([disabled])').first();
  if (await parent.count() > 0) await parent.click();
  await page.waitForTimeout(100);
  await browseSearch.fill("notes");
  await page.locator('select[data-browse-query-kind]').selectOption("file");
  await page.waitForFunction(() => document.querySelector('[data-browse-query="notes"][data-browse-query-kind="file"]') !== null);
}

async function exerciseBackForward(page) {
  const libraryTab = page.locator('[data-file-library-mode="library"]');
  await libraryTab.click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
  const back = page.getByRole("button", { name: "Back", exact: true });
  assert(await back.isEnabled(), "Back did not retain the previous Browse target");
  await back.click();
  await page.waitForSelector('.file-library-workspace[data-mode="browse"]');
  await page.waitForFunction(() => document.querySelector('[data-browse-query="notes"][data-browse-query-kind="file"]') !== null);
  const forward = page.getByRole("button", { name: "Forward", exact: true });
  assert(await forward.isEnabled(), "Forward did not retain the Library target");
  await forward.click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
}

try {
  for (const viewport of [{ width: 1600, height: 900 }, { width: 980, height: 680 }]) {
    for (const deviceScaleFactor of [1, 1.25, 2]) {
      const context = await browser.newContext({ viewport, deviceScaleFactor });
      await context.addInitScript(() => {
        window.localStorage.setItem("zc-onboarding-complete", "true");
        window.localStorage.setItem("zc-language", "en");
      });
      const page = await context.newPage();
      page.setDefaultTimeout(60_000);
      page.setDefaultNavigationTimeout(60_000);
      const consoleErrors = [];
      const pageErrors = [];
      page.on("console", (message) => { if (message.type() === "error") consoleErrors.push(message.text()); });
      page.on("pageerror", (error) => pageErrors.push(String(error)));
      try {
        await page.goto(`${baseUrl}?${FIXTURE_QUERY}`, { waitUntil: "commit" });
        await exerciseLibrary(page, viewport);
        await exerciseBrowse(page);
        await exerciseBackForward(page);
        await assertNoHorizontalOverflow(page, `${viewport.width}x${viewport.height}@${deviceScaleFactor}`);
        assert(consoleErrors.length === 0 && pageErrors.length === 0, `${viewport.width}x${viewport.height}@${deviceScaleFactor}: browser errors ${JSON.stringify({ consoleErrors, pageErrors })}`);
        const dpr = await page.evaluate(() => window.devicePixelRatio);
        console.log(`[w2-10-real] PASS ${viewport.width}x${viewport.height} dpr=${dpr} lane=${process.env.W210_LANE ?? "local"} sourceHead=${SOURCE_HEAD} actualSha=${ACTUAL_CHECKOUT_SHA} tree=${ACTUAL_CHECKOUT_TREE}`);
      } finally {
        await context.close();
      }
    }
  }
} finally {
  await browser.close();
  await server.close();
  await rm(ARTIFACT_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  await rm(TASK_TEMP_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
}
