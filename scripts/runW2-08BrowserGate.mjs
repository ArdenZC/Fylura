import { mkdir, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";
import { createServer } from "vite";

const SOURCE_HEAD = process.env.W208_SOURCE_HEAD ?? process.env.GITHUB_SHA ?? "local";
const ACTUAL_CHECKOUT_SHA = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const ACTUAL_CHECKOUT_TREE = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
const TASK_TEMP_DIR = path.resolve(".tmp-tests/w2-08-browser-runtime");
const ARTIFACT_DIR = path.resolve(".tmp-tests/w2-08-browser-gate");

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
  ...(process.env.W208_CHROMIUM_EXECUTABLE ? { executablePath: process.env.W208_CHROMIUM_EXECUTABLE } : {})
});

try {
  for (const viewport of [{ width: 1600, height: 900 }, { width: 980, height: 680 }]) {
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
    page.on("console", (message) => { if (message.type() === "error") consoleErrors.push(message.text()); });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    try {
      await page.goto(`${baseUrl}?w2-04-browser-fixture=source-owner&w2-05-browser-fixture=interaction&w2-05-browser-stale=true`, { waitUntil: "commit" });
      await page.getByRole("button", { name: "Files", exact: true }).click();
      await page.waitForSelector('.file-library-workspace[data-mode="library"]');

      const allIndexedFiles = page.getByRole("button", { name: "View all indexed files", exact: true });
      if (await allIndexedFiles.count() > 0 && await allIndexedFiles.first().isVisible()) await allIndexedFiles.first().click();

      const commandSearch = page.locator('[data-file-library-command-search="true"] [data-file-library-local-search="true"]');
      await commandSearch.waitFor({ state: "visible" });
      if (await commandSearch.count() !== 1) throw new Error("Library did not expose exactly one command-bar local search input");
      if (await page.locator('[data-library-source-owner] [data-file-library-local-search="true"]').count() !== 0) throw new Error("Library search was duplicated inside the source body");

      await commandSearch.fill("report");
      if (await commandSearch.inputValue() !== "report") throw new Error("Library Query V2 search input did not retain the committed text");
      await page.keyboard.press("Control+f");
      if (await page.evaluate(() => document.activeElement?.matches('[data-file-library-command-search="true"] [data-file-library-local-search="true"]') !== true)) throw new Error("Ctrl/Cmd+F did not focus the local search");

      const sourceActions = page.locator('[data-file-library-source-actions="true"]');
      const filterButton = sourceActions.getByRole("button", { name: /^Filter/iu });
      await filterButton.click();
      await page.locator('[id^="library-filter-popover"] [role="dialog"]').waitFor({ state: "visible" });
      await page.getByRole("button", { name: "Done", exact: true }).click();
      await page.waitForFunction(() => document.querySelector('[data-file-library-source-actions="true"] button[aria-haspopup="dialog"]')?.getAttribute("aria-expanded") === "false");
      if (await filterButton.getAttribute("aria-expanded") !== "false") throw new Error("Library Filter did not close through its source-owner control");

      const sortButton = sourceActions.locator('button[aria-haspopup="menu"]');
      await sortButton.click();
      await page.getByRole("menu", { name: "File Library sort" }).waitFor({ state: "visible" });
      await page.getByRole("menuitemradio", { name: "Name", exact: true }).click();
      await page.waitForFunction(() => document.querySelector('[data-file-library-source-actions="true"] button[aria-haspopup="menu"]')?.getAttribute("aria-expanded") === "false");
      if (await sortButton.getAttribute("aria-expanded") !== "false") throw new Error("Library Sort did not close through its source-owner action");
      await page.locator('[data-shared-file-list="true"][data-shared-file-list-source="library"]').waitFor({ state: "visible" });

      await page.getByRole("button", { name: "Grid", exact: true }).click();
      await page.locator('[data-shared-file-grid="true"][data-shared-file-grid-source="library"]').waitFor({ state: "visible" });
      await page.getByRole("button", { name: /context/iu }).first().click();
      await page.getByRole("button", { name: /context/iu }).first().click();
      await page.waitForFunction(() => document.querySelector('.file-library-workspace')?.getAttribute("data-context-open") === "false");

      await page.getByRole("tab", { name: "Browse Folder", exact: true }).click();
      await page.waitForSelector('.file-library-workspace[data-mode="browse"]');
      if (await page.locator('[data-browse-state="current-folder"]').count() === 0) {
        const openableLocation = page.locator('[data-browse-location="true"][data-browse-location-openable="true"]');
        await openableLocation.first().waitFor({ state: "visible" });
        await openableLocation.first().locator('[data-browse-location-action="open"]').click();
      }
      await page.locator('[data-browse-state="current-folder"]').waitFor({ state: "visible" });
      const browseSearch = page.locator('[data-file-library-command-search="true"] [data-file-library-local-search="true"]');
      await browseSearch.waitFor({ state: "visible" });
      if (!(await browseSearch.isEnabled())) throw new Error("Browse current-folder search remained disabled");
      await browseSearch.fill("notes");
      if (await browseSearch.inputValue() !== "notes") throw new Error("Browse current-folder search did not retain the committed text");
      await page.locator('select[data-browse-query-kind]').selectOption("file");
      const browseSort = page.locator('select[data-browse-sort]');
      await browseSort.waitFor({ state: "visible" });
      if (!(await browseSort.isEnabled())) throw new Error("Browse backend sorting remained disabled");
      await browseSort.selectOption("name");
      await page.locator('[data-browse-entry]').filter({ hasText: "notes.md" }).waitFor({ state: "visible" });
      if (await browseSort.inputValue() !== "name") throw new Error("Browse did not retain backend sorting");
      await page.waitForFunction(() => document.querySelector('[data-browse-query="notes"][data-browse-query-kind="file"]') !== null);

      const overflow = await page.evaluate(() => ({
        documentOverflow: document.documentElement.scrollWidth > window.innerWidth + 1,
        bodyOverflow: document.body.scrollWidth > window.innerWidth + 1
      }));
      if (overflow.documentOverflow || overflow.bodyOverflow) throw new Error(`unexpected horizontal overflow: ${JSON.stringify(overflow)}`);
      if (consoleErrors.length || pageErrors.length) throw new Error(JSON.stringify({ consoleErrors, pageErrors }));
      console.log(`[w2-08-real] PASS ${viewport.width}x${viewport.height} sourceHead=${SOURCE_HEAD} actualSha=${ACTUAL_CHECKOUT_SHA} tree=${ACTUAL_CHECKOUT_TREE}`);
    } finally {
      await context.close();
    }
  }
} finally {
  await browser.close();
  await server.close();
  await rm(ARTIFACT_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  await rm(TASK_TEMP_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
}
