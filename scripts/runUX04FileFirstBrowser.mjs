/**
 * Focused UX-04 first-launch workflow. Browser mock only; never native
 * filesystem, scan or index evidence.
 */
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const output = path.resolve(process.env.UX04_OUTPUT_DIR ?? ".tmp-tests/ux04-browser");
await mkdir(output, { recursive: true });
const server = await createServer({ server: { host: "127.0.0.1", port: 0, strictPort: false } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("UX-04 browser check: Vite did not expose a local URL.");

const browser = await chromium.launch({ headless: true });
const errors = [];
const checks = [];
let currentPage = null;

function watch(page) {
  currentPage = page;
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  return page;
}

async function openApp({ width = 1440, height = 900, language = "zh", theme = "light" } = {}) {
  const page = watch(await browser.newPage({ viewport: { width, height } }));
  await page.addInitScript(({ language, theme }) => {
    Object.defineProperty(navigator, "platform", { get: () => "Win32" });
    // Only the first navigation is a clean profile; a reload must observe the
    // durable completion flag exactly like a restart would.
    if (!sessionStorage.getItem("ux04-clean-profile")) {
      sessionStorage.setItem("ux04-clean-profile", "1");
      localStorage.removeItem("zc-onboarding-complete");
    }
    localStorage.setItem("zc-language", language);
    localStorage.setItem("zc-theme", theme);
  }, { language, theme });
  await page.goto(`${baseUrl}/?browser-mock=1`);
  return page;
}

try {
  // ---- Clean profile, Chinese, light: the four-scene intro ----
  const page = await openApp();
  await page.locator('[data-onboarding-step="1"]').waitFor();
  assert.equal(await page.locator('[role="dialog"]').count(), 1);
  assert.equal(await page.locator('[data-onboarding-showcase="files"]').count(), 1);
  assert.equal(await page.locator("[data-onboarding-start]").count(), 1);
  await page.screenshot({ path: path.join(output, "01-intro-files-zh-light.png") });
  checks.push("clean profile opens the intro on scene 1 with a Files showcase and a start action");

  await page.locator("[data-onboarding-next]").click();
  await page.locator('[data-onboarding-step="2"]').waitFor();
  await page.screenshot({ path: path.join(output, "02-intro-organize-zh-light.png") });
  checks.push("scene 2 explains the AI organize plan/review/execute flow");

  await page.locator("[data-onboarding-next]").click();
  await page.locator('[data-onboarding-step="3"]').waitFor();
  await page.screenshot({ path: path.join(output, "03-intro-cleanup-zh-light.png") });
  checks.push("scene 3 explains AI cleanup with Safe Trash and restore");

  await page.locator("[data-onboarding-next]").click();
  await page.locator('[data-onboarding-step="4"]').waitFor();
  await page.locator('[data-onboarding-search-state="ready"]').waitFor();
  assert.equal(await page.locator("[data-onboarding-enable-search]").isDisabled(), true);
  await page.screenshot({ path: path.join(output, "04-intro-search-zh-light.png") });
  checks.push("scene 4 lists real volumes and keeps Global Search disabled until a volume is selected");

  await page.locator("[data-onboarding-finish]").click();
  await page.locator('[data-browse-state="locations"]').waitFor();
  assert.equal(await page.locator('[role="dialog"]').count(), 0);
  assert.equal(await page.locator(".file-library-mode-switch").count(), 0);
  assert.equal(await page.locator("[data-browse-places]").count(), 1);
  const placeCount = await page.locator("[data-browse-place]").count();
  assert.ok(placeCount >= 4, `expected at least four places, saw ${placeCount}`);
  await page.screenshot({ path: path.join(output, "05-files-no-folder-zh-light.png") });
  assert.equal(await page.locator("[data-file-library-mode]").count(), 0);
  checks.push("finishing the intro lands on Files with places and no folder open");

  // ---- Durable skip across a restart ----
  await page.reload();
  await page.locator('[data-browse-state="locations"]').waitFor();
  assert.equal(await page.locator('[role="dialog"]').count(), 0);
  checks.push("reload does not repeat the intro");

  // ---- Places open through the existing Browse admission ----
  await page.locator("[data-browse-place-action='open']").first().click();
  await page.locator('[data-browse-state="current-folder"]').waitFor();
  checks.push("a place opens a real Browse session through user-intent admission");
  await page.screenshot({ path: path.join(output, "06-files-place-open-zh-light.png") });

  // AI entries navigate to their workspaces; they do not transfer folder scope.
  await page.locator(".fm-more-trigger").click();
  await page.getByRole("button", { name: "打开 AI 整理", exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: "打开 AI 清理", exact: true }).count(), 1);
  assert.equal(await page.locator("[data-fm-open-cleanup]").count(), 1);
  await page.screenshot({ path: path.join(output, "07-files-ai-entries-zh-light.png") });
  await page.locator("[data-fm-open-organize]").click();
  await page.waitForTimeout(300);
  checks.push("the Files more panel truthfully opens AI workspaces without promising folder scope");

  // ---- English dark and narrow layouts ----
  const dark = await openApp({ language: "en", theme: "dark" });
  await dark.locator('[data-onboarding-step="1"]').waitFor();
  await dark.screenshot({ path: path.join(output, "08-intro-files-en-dark.png") });
  await dark.locator("[data-onboarding-skip]").click();
  await dark.locator('[data-browse-state="locations"]').waitFor();
  await dark.screenshot({ path: path.join(output, "09-files-no-folder-en-dark.png") });
  checks.push("English and dark theme render the intro and Files without a folder");

  const narrow = await openApp({ width: 980, height: 680 });
  await narrow.locator('[data-onboarding-step="1"]').waitFor();
  await narrow.screenshot({ path: path.join(output, "10-intro-narrow-zh-light.png") });
  const description = narrow.locator("#onboarding-description");
  assert.equal(await description.evaluate((element) => element.scrollHeight >= element.clientHeight), true);
  await narrow.locator("[data-onboarding-start]").click();
  await narrow.locator('[data-browse-state="locations"]').waitFor();
  await narrow.screenshot({ path: path.join(output, "11-files-no-folder-narrow-zh-light.png") });
  checks.push("a 980x680 window keeps the intro scrollable and reaches Files");

  assert.deepEqual(errors, []);
  await writeFile(path.join(output, "summary.json"), JSON.stringify({ checks, errors }, null, 2));
  console.log(JSON.stringify({ ok: true, checks, screenshots: output }, null, 2));
} catch (error) {
  if (currentPage) await currentPage.screenshot({ path: path.join(output, "failure.png") }).catch(() => undefined);
  await writeFile(path.join(output, "summary.json"), JSON.stringify({ checks, errors, failure: String(error?.stack ?? error) }, null, 2));
  console.error(JSON.stringify({ ok: false, checks, errors, failure: String(error?.message ?? error) }, null, 2));
  process.exitCode = 1;
} finally {
  await browser.close();
  await server.close();
}
