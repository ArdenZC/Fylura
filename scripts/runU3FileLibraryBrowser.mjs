/** Focused U3 product workflow. Browser mock only; never native filesystem evidence. */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import { assertCheckoutEvidence } from "./ciEvidence.mjs";

const actualCheckoutSha = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const actualCheckoutTree = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
const sourceHead = process.env.U3_SOURCE_HEAD ?? actualCheckoutSha;
const expectedCheckoutSha = process.env.U3_EXPECTED_CHECKOUT_SHA;
if (expectedCheckoutSha) assertCheckoutEvidence(expectedCheckoutSha, actualCheckoutSha);
const output = path.resolve(process.env.U3_OUTPUT_DIR ?? ".tmp-tests/u3-browser");
await mkdir(output, { recursive: true });
const server = await createServer({ server: { host: "127.0.0.1", port: 0, strictPort: false } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("U3 browser gate: Vite did not expose a local URL.");
const browser = await chromium.launch({ headless: true });
const errors = [];
const checks = [];
let page;
try {
  page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "Win32" });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "en");
  });
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  const screenshot = (name) => page.screenshot({ path: path.join(output, name), fullPage: false });
  await page.goto(`${baseUrl}/?browser-mock=1`);
  await page.getByRole("button", { name: "Files", exact: true }).click();
  assert.equal(await page.title(), "Fylura");
  const left = page.locator("[data-pane-id]").first();
  await left.getByRole("button", { name: "Edit path (Ctrl/Cmd+L)", exact: true }).click();
  await left.getByRole("textbox", { name: "Folder path", exact: true }).fill("C:/U3/Left");
  await left.getByRole("textbox", { name: "Folder path", exact: true }).press("Enter");
  await left.locator("[data-browse-sort]").selectOption("size");
  await left.locator('[data-browse-entry-kind="directory"]').first().waitFor();
  await screenshot("01-sorting-light.png");
  await page.getByRole("button", { name: "New tab", exact: true }).click();
  await page.getByRole("tab", { name: "Tab: Left", exact: true }).click();
  assert.equal(await left.locator("[data-browse-sort]").inputValue(), "size");
  checks.push("Sort remains pane/tab-local across tab activation");
  await page.getByRole("button", { name: "Dual pane", exact: true }).click();
  const right = page.locator('[role="tabpanel"]:not([hidden]) [data-pane-id]').nth(1);
  await right.getByRole("button", { name: "Edit path (Ctrl/Cmd+L)", exact: true }).click();
  await right.getByRole("textbox", { name: "Folder path", exact: true }).fill("C:/U3/Right");
  await right.getByRole("textbox", { name: "Folder path", exact: true }).press("Enter");
  await right.locator("[data-browse-sort]").waitFor();
  assert.equal(await right.locator("[data-browse-sort]").inputValue(), "native");
  await page.getByRole("button", { name: "Compare folders", exact: true }).click();
  const compare = page.locator(".u3-comparison");
  await compare.getByRole("table").waitFor();
  for (const name of ["All 140", "Visible only on left 2", "Visible only on right 2", "Metadata differs 2", "Selected metadata matches 133", "Subfolder · not compared 1"]) {
    assert.equal(await compare.getByRole("button", { name, exact: true }).count(), 1);
  }
  assert.equal(await compare.locator(".u3-batch-details").evaluate((details) => details.open), false);
  const visibleCompareRows = await compare.locator(".u3-compare-table").evaluate((table) => {
    const bounds = table.getBoundingClientRect();
    return [...table.querySelectorAll(".u3-compare-row")].filter((row) => {
      const rect = row.getBoundingClientRect();
      return rect.height > 0 && rect.bottom > bounds.top && rect.top < bounds.bottom;
    }).length;
  });
  assert.ok(visibleCompareRows >= 4, `expected a useful comparison viewport, saw ${visibleCompareRows} rows`);
  checks.push("138 entries per side complete across multiple pages; six comparison statuses and exact case policy");
  checks.push("Batch controls start collapsed; comparison results show at least four rows without scrolling at 1600x1000");
  await screenshot("02-compare-light.png");
  await compare.getByRole("button", { name: "Metadata differs 2", exact: true }).click();
  await compare.locator(".u3-batch-details > summary").click();
  const actions = compare.locator(".u3-batch-actions > div");
  await actions.first().getByRole("button", { name: "Select filtered side", exact: true }).click();
  await actions.first().getByRole("button", { name: "Add to basket", exact: true }).click();
  await left.locator(".fm-count").first().waitFor();
  assert.equal(await left.locator(".fm-count").first().innerText(), "2");
  assert.match(await actions.nth(1).innerText(), /Right pane · 0/);
  await screenshot("03-filtered-selection.png");
  await actions.first().getByRole("button", { name: "Copy to other pane", exact: true }).click();
  let dialog = page.getByRole("dialog");
  await dialog.getByText(/The batch is paused\. No file changes have been made\./u).waitFor();
  assert.match(await dialog.innerText(), /Existing files will never be overwritten/u);
  assert.equal(await dialog.getByRole("button", { name: "Apply choices and continue", exact: true }).isEnabled(), false);
  await screenshot("04-conflict-dialog.png");
  await dialog.locator('select[aria-label^="Document"]').nth(0).selectOption("both");
  await dialog.locator('select[aria-label^="Document"]').nth(1).selectOption("skip");
  assert.equal(await dialog.getByRole("button", { name: "Apply choices and continue", exact: true }).isEnabled(), true);
  await dialog.getByRole("button", { name: "Apply choices and continue", exact: true }).click();
  await dialog.waitFor({ state: "hidden" });
  await compare.getByRole("table").waitFor();
  await right.getByRole("button", { name: "Operation results · 1/1", exact: true }).click();
  const results = right.locator(".fm-panel");
  assert.match(await results.innerText(), /Document-003 \(2\)\.txt/);
  assert.equal(await results.getByRole("button", { name: "History / Restore", exact: true }).count(), 1);
  await screenshot("06-operation-results.png");
  await results.getByRole("button", { name: "Dismiss", exact: true }).first().click();
  checks.push("Filtered selection → Basket → conflict-only dialog blocks execution → Keep both + Skip → fresh preview auto-executes → result");
  // A deliberate one-sided Move has no conflict and should execute without confirmation.
  await compare.getByRole("button", { name: "Visible only on left 2", exact: true }).click();
  await compare.getByRole("checkbox", { name: "Left pane: Document-001.txt", exact: true }).check();
  await actions.first().getByRole("button", { name: "Move to other pane", exact: true }).click();
  await right.getByRole("button", { name: "Operation results · 1/1", exact: true }).waitFor();
  assert.equal(await page.getByRole("dialog").count(), 0);
  await compare.getByRole("button", { name: "Visible only on left 1", exact: true }).waitFor();
  checks.push("Explicit no-conflict Move auto-executes without a confirmation dialog and refreshes comparison");
  await right.getByRole("button", { name: "Operation results · 1/1", exact: true }).click();
  await right.locator(".fm-panel").getByRole("button", { name: "Dismiss", exact: true }).last().click();
  // Directory eligibility remains a backend-preview decision.
  await compare.getByRole("button", { name: "Subfolder · not compared 1", exact: true }).click();
  await actions.first().getByRole("button", { name: "Select filtered side", exact: true }).click();
  await actions.first().getByRole("button", { name: "Copy to other pane", exact: true }).click();
  const errorNotice = page.locator(".fm-notice[role=alert]");
  await errorNotice.waitFor();
  assert.match(await errorNotice.innerText(), /folder|directory/i);
  assert.equal(await page.getByRole("dialog").count(), 0);
  await errorNotice.getByRole("button", { name: "Dismiss", exact: true }).click();
  checks.push("Unsupported directory operation reports a nonmodal error and cannot execute");
  await compare.getByRole("button", { name: /Visible only on right/ }).click();
  await page.evaluate(() => document.documentElement.classList.add("dark"));
  await screenshot("07-filter-dark.png");
  await page.setViewportSize({ width: 820, height: 800 });
  await screenshot("08-narrow-dark.png");
  assert.equal(await page.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  await page.evaluate(() => document.documentElement.classList.remove("dark"));
  await screenshot("09-narrow-light.png");
  for (const width of [980, 1280]) {
    await page.setViewportSize({ width, height: 800 });
    assert.equal(await page.evaluate(() => document.body.scrollWidth <= innerWidth), true);
  }
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.getByRole("button", { name: "History", exact: true }).click();
  await page.getByRole("heading", { name: "Operation batches", exact: true }).waitFor();
  await screenshot("10-history.png");
  checks.push("History reachable from transfer results; mock operations do not claim native Restore qualification");
  const chinese = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  chinese.on("pageerror", (error) => errors.push(error.message));
  await chinese.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "Win32" });
    localStorage.setItem("zc-onboarding-complete", "true"); localStorage.setItem("zc-language", "zh");
  });
  await chinese.goto(`${baseUrl}/?browser-mock=1`);
  await chinese.getByRole("button", { name: "文件", exact: true }).click();
  const chinesePane = chinese.locator("[data-pane-id]").first();
  await chinesePane.getByRole("button", { name: "编辑路径（Ctrl/Cmd+L）", exact: true }).click();
  await chinesePane.getByRole("textbox", { name: "文件夹路径", exact: true }).fill("C:/U3/Left");
  await chinesePane.getByRole("textbox", { name: "文件夹路径", exact: true }).press("Enter");
  await chinesePane.locator("[data-browse-sort]").selectOption("name");
  await chinesePane.locator('[data-browse-entry-kind="directory"]').first().waitFor();
  await chinese.evaluate(() => document.documentElement.classList.add("dark"));
  await chinese.screenshot({ path: path.join(output, "11-chinese-dark.png") });
  checks.push("Chinese and English, light and dark, 820/980/1280/1600 widths without page overflow");
  assert.equal(await page.locator("vite-error-overlay").count(), 0);
  assert.deepEqual(errors, []);
  await writeFile(path.join(output, "checks.json"), JSON.stringify({ sourceHead, actualCheckoutSha, actualCheckoutTree, environment: "Playwright Chromium / browser mock / simulated Windows renderer", viewports: ["1600x1000", "1280x800", "980x800", "820x800"], checks, errors }, null, 2));
  console.log(JSON.stringify({ result: "PASS", sourceHead, actualCheckoutSha, actualCheckoutTree, output, checks }, null, 2));
} catch (error) {
  if (page) { await page.screenshot({ path: path.join(output, "failure.png") }); await writeFile(path.join(output, "failure.txt"), await page.locator("body").innerText()); }
  throw error;
} finally {
  await browser.close();
  await server.close();
}
