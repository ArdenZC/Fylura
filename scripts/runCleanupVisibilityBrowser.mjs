import { mkdir, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const TASK_TEMP_DIR = path.resolve(ROOT, ".tmp-tests/cleanup-visibility-browser-runtime");
const ARTIFACT_DIR = path.resolve(
  process.env.CLEANUP_VISUAL_ARTIFACT_DIR
    ?? path.join(os.tmpdir(), "fylura-cleanup-visibility", new Date().toISOString().replaceAll(":", "-"))
);
const ACTUAL_CHECKOUT_SHA = execFileSync("git", ["rev-parse", "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
const ACTUAL_CHECKOUT_TREE = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { cwd: ROOT, encoding: "utf8" }).trim();
const WORKTREE_STATUS = execFileSync("git", ["status", "--short"], { cwd: ROOT, encoding: "utf8" }).trim();

process.env.TEMP = TASK_TEMP_DIR;
process.env.TMP = TASK_TEMP_DIR;
process.env.TMPDIR = TASK_TEMP_DIR;
await mkdir(TASK_TEMP_DIR, { recursive: true });
await mkdir(ARTIFACT_DIR, { recursive: true });

const server = await createServer({
  configFile: path.join(ROOT, "vite.config.ts"),
  server: { host: "127.0.0.1", port: 0, strictPort: false }
});
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("Vite did not expose a local browser URL.");

const browser = await chromium.launch();

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function assertVisibleAndReachable(button, label) {
  await button.scrollIntoViewIfNeeded();
  assert(await button.isVisible(), `${label}: selection control is not visible`);
  const report = await button.evaluate((element) => {
    const bounds = element.getBoundingClientRect();
    const visible = {
      left: 0,
      top: 0,
      right: window.innerWidth,
      bottom: window.innerHeight
    };
    for (let ancestor = element.parentElement; ancestor; ancestor = ancestor.parentElement) {
      const style = window.getComputedStyle(ancestor);
      const rect = ancestor.getBoundingClientRect();
      if (style.overflowX !== "visible") {
        visible.left = Math.max(visible.left, rect.left + ancestor.clientLeft);
        visible.right = Math.min(visible.right, rect.left + ancestor.clientLeft + ancestor.clientWidth);
      }
      if (style.overflowY !== "visible") {
        visible.top = Math.max(visible.top, rect.top + ancestor.clientTop);
        visible.bottom = Math.min(visible.bottom, rect.top + ancestor.clientTop + ancestor.clientHeight);
      }
    }
    const centerX = bounds.left + bounds.width / 2;
    const centerY = bounds.top + bounds.height / 2;
    const hit = document.elementFromPoint(centerX, centerY);
    return {
      button: { left: bounds.left, top: bounds.top, right: bounds.right, bottom: bounds.bottom, width: bounds.width, height: bounds.height },
      visible,
      centerInsideVisibleArea: centerX >= visible.left && centerX <= visible.right && centerY >= visible.top && centerY <= visible.bottom,
      hitTarget: Boolean(hit && (hit === element || element.contains(hit)))
    };
  });
  assert(
    report.centerInsideVisibleArea && report.hitTarget,
    `${label}: selection control is clipped or covered: ${JSON.stringify(report)}`
  );
  return report;
}

async function waitForFindingSelection(page, findingId, expected) {
  await page.waitForFunction(({ id, pressed }) => {
    const row = document.querySelector(`[data-analysis-finding-id="${CSS.escape(id)}"]`);
    return row?.querySelector("button[aria-pressed]")?.getAttribute("aria-pressed") === String(pressed);
  }, { id: findingId, pressed: expected });
}

async function runCase(viewport, density, scenario = "visibility", readiness = "disconnected") {
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  await context.addInitScript((initialDensity) => {
    window.localStorage.setItem("zc-onboarding-complete", "true");
    window.localStorage.setItem("zc-language", "en");
    window.localStorage.setItem("zc-density", initialDensity);
  }, density);
  const page = await context.newPage();
  page.setDefaultTimeout(20_000);
  page.setDefaultNavigationTimeout(20_000);
  const consoleErrors = [];
  const pageErrors = [];
  page.on("console", (message) => { if (message.type() === "error") consoleErrors.push(message.text()); });
  page.on("pageerror", (error) => pageErrors.push(String(error)));
  const label = `${viewport.width}x${viewport.height}-${density}-${scenario}-${readiness}`;
  const screenshotPrefix = path.join(ARTIFACT_DIR, label);

  try {
    await page.goto(`${baseUrl}/?pm01-cleanup=${scenario}&pm01-readiness=${readiness}`, { waitUntil: "commit" });
    assert((await page.title()).toLowerCase().includes("fylura"), `${label}: unexpected page title ${await page.title()}`);
    await page.getByRole("button", { name: "Storage Cleanup", exact: true }).click();
    await page.locator("[data-analysis-run-id]").waitFor({ state: "visible" });
    const findings = page.locator("[data-cleanup-findings] [data-analysis-finding-id]");
    await page.locator("[data-cleanup-findings][data-cleanup-findings-virtualized]").waitFor({ state: "visible" });
    const expectedFindingCount = scenario === "visibility-many" ? 48 : 1;
    await page.waitForFunction((expectedCount) => {
      const candidateCount = document.querySelector("[data-cleanup-summary] strong")?.textContent?.trim();
      return Number(candidateCount ?? 0) === expectedCount;
    }, expectedFindingCount);
    assert(await page.locator("[data-cleanup-findings]").getAttribute("data-cleanup-findings-virtualized") === String(expectedFindingCount > 20), `${label}: wrong short-list/virtualized mode`);
    if (scenario === "visibility-many") {
      const initialRows = await findings.count();
      assert(initialRows > 0 && initialRows < expectedFindingCount, `${label}: expected virtualization to mount only visible rows, got ${initialRows}`);
      const scrollArea = page.locator("[data-cleanup-findings-scroll-owner='virtualizer']");
      await scrollArea.evaluate((element) => element.scrollTo({ top: element.scrollHeight, behavior: "instant" }));
      const lastRow = page.locator('[data-analysis-finding-id="storage-safe-node-modules-48"]');
      await lastRow.waitFor({ state: "visible" });
      assert(await findings.count() < expectedFindingCount, `${label}: virtualized list mounted all findings at once`);
    }
    const row = scenario === "visibility-many"
      ? page.locator('[data-analysis-finding-id="storage-safe-node-modules-48"]')
      : findings.first();
    assert((await row.textContent())?.includes("deliberately long cleanup reason"), `${label}: long-reason fixture did not load`);
    const findingId = await row.getAttribute("data-analysis-finding-id");
    assert(Boolean(findingId), `${label}: finding row has no stable id`);
    const select = row.locator("button[aria-pressed]");
    assert(await select.count() === 1, `${label}: finding did not expose one selection control`);
    const visibility = await assertVisibleAndReachable(select, label);
    await page.screenshot({ path: `${screenshotPrefix}-finding.png`, fullPage: false });

    await select.click();
    await waitForFindingSelection(page, findingId, false);
    await select.click();
    await waitForFindingSelection(page, findingId, true);
    assert(await select.getAttribute("aria-pressed") === "true", `${label}: selecting the finding did not update button state`);
    const selectionSummary = page.locator("[data-cleanup-selection-summary]");
    await selectionSummary.waitFor({ state: "visible" });
    const preview = selectionSummary.getByRole("button", { name: "Preview selected Safe Trash move", exact: true });
    assert(await preview.isEnabled(), `${label}: selected finding did not expose an enabled Preview action`);
    const assess = selectionSummary.getByRole("button", { name: "Assess selected green items", exact: true });
    assert(await assess.count() === 1, `${label}: selected Safe finding did not expose the AI assessment action`);
    const assessmentVisibility = await assertVisibleAndReachable(assess, `${label}-assessment`);
    if (readiness === "cleanup-local-ready") {
      assert(await assess.isEnabled(), `${label}: ready local presentation fixture disabled the assessment action`);
      await assess.click();
      await page.getByText(/Assessment request finished:/).waitFor({ state: "visible" });
      assert(await page.locator("[data-ai-readiness-feature='cleanup']").getAttribute("data-ai-readiness-source") === "presentation-fixture", `${label}: expected an explicitly presentation-only readiness source`);
      assert(await preview.isEnabled(), `${label}: assessment request removed the separately gated Preview action`);
    } else {
      assert(await assess.isDisabled(), `${label}: assessment action was enabled while provider readiness was disabled`);
      assert(await page.getByRole("button", { name: "Open related settings", exact: true }).count() > 0, `${label}: readiness blocker did not offer its settings route`);
    }
    await page.screenshot({ path: `${screenshotPrefix}-selected.png`, fullPage: false });
    if (readiness === "cleanup-local-ready") await page.screenshot({ path: `${screenshotPrefix}-assessed.png`, fullPage: false });

    const identity = await page.evaluate(() => ({
      title: document.title,
      visibleText: document.body.innerText.includes("Storage Cleanup"),
      errorOverlay: Boolean(document.querySelector("vite-error-overlay, [data-view-error-boundary]"))
    }));
    assert(identity.visibleText && !identity.errorOverlay, `${label}: blank page or framework/error overlay ${JSON.stringify(identity)}`);
    assert(consoleErrors.length === 0 && pageErrors.length === 0, `${label}: browser errors ${JSON.stringify({ consoleErrors, pageErrors })}`);
    console.log(`[cleanup-visibility] PASS ${label} reachability=${JSON.stringify({ selection: visibility, assessment: assessmentVisibility })} sha=${ACTUAL_CHECKOUT_SHA} tree=${ACTUAL_CHECKOUT_TREE} worktreeDirty=${Boolean(WORKTREE_STATUS)}`);
  } catch (error) {
    await page.screenshot({ path: `${screenshotPrefix}-failure.png`, fullPage: false }).catch(() => undefined);
    throw error;
  } finally {
    await context.close();
  }
}

try {
  for (const viewport of [{ width: 1920, height: 1080 }, { width: 960, height: 720 }]) {
    for (const density of ["default", "compact"]) await runCase(viewport, density);
  }
  await runCase({ width: 1920, height: 1080 }, "default", "visibility-many");
  await runCase({ width: 960, height: 720 }, "compact", "visibility-many");
  await runCase({ width: 960, height: 720 }, "compact", "visibility", "cleanup-local-ready");
  console.log(`[cleanup-visibility] screenshots=${ARTIFACT_DIR}`);
} finally {
  await browser.close();
  await server.close();
  await rm(TASK_TEMP_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
}
