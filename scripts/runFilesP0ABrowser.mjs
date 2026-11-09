/** Files P0-A rendered regression. Browser mock only; never native acceptance. */
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const output = process.env.FILES_P0A_OUTPUT_DIR ?? "/tmp/fylura-files-p0a-browser";
await mkdir(output, { recursive: true });
const server = await createServer({ server: { host: "127.0.0.1", port: 0 } });
await server.listen();
const browser = await chromium.launch({ headless: true });
const errors = [];
try {
  const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "Win32" });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "en");
  });
  await page.goto(`${server.resolvedUrls.local[0]}?browser-mock=1&u3-folder-move=1`);
  assert.equal(await page.title(), "Fylura");
  await page.getByRole("button", { name: "Files", exact: true }).click();
  const pane = page.locator("[data-pane-id]").first();
  await pane.getByRole("button", { name: "Edit path (Ctrl/Cmd+L)", exact: true }).click();
  await pane.getByRole("textbox", { name: "Folder path", exact: true }).fill("C:/U3/Left");
  await pane.getByRole("textbox", { name: "Folder path", exact: true }).press("Enter");
  const header = (field) => pane.locator(`[data-browse-sort-column="${field}"]`);
  for (const field of ["name", "type", "size", "created", "modified"]) {
    await header(field).click();
    await page.waitForFunction((field) => document.querySelector(`[data-browse-sort-column="${field}"]`)?.getAttribute("aria-pressed") === "true", field);
    assert.match(await header(field).getAttribute("aria-label"), /Ascending/);
    await header(field).press("Enter");
    await page.waitForFunction((field) => document.querySelector(`[data-browse-sort-column="${field}"]`)?.textContent.includes("↓"), field);
    assert.match(await header(field).getAttribute("aria-label"), /Descending/);
  }
  await header("created").click();
  await page.getByRole("button", { name: "New tab", exact: true }).click();
  await page.getByRole("tab", { name: "Tab: Left", exact: true }).click();
  assert.equal(await header("created").getAttribute("aria-pressed"), "true");
  await header("name").click();
  const row = pane.locator('[data-browse-entry][data-file-entry-kind="file"]').first();
  await row.waitFor();
  const list = pane.locator('[data-shared-file-list-source="browse"]');
  await row.dblclick();
  await pane.getByText(/Cannot open this file/).first().waitFor();
  // The mock explicitly refuses OS launch. Failure feedback proves activation
  // reaches launch rather than silently doing nothing or opening Quick Preview.
  assert.equal(await page.locator('[data-preview-host]').count(), 0);
  await row.click();
  await list.press("Enter");
  await pane.getByText(/Cannot open this file/).first().waitFor();
  for (const label of ["Open", "Open with…", "Open file location"]) {
    await row.click({ button: "right" });
    const menu = page.getByRole("menu");
    await menu.getByRole("menuitem", { name: label, exact: true }).click();
    await pane.getByText(label === "Open file location" ? /Cannot show this item/ : /Cannot open this file/).first().waitFor();
  }
  await row.click();
  await list.press("Space");
  // Existing Preview surface remains separately keyboard-invoked.
  await page.locator('[data-preview-host]').first().waitFor();
  await page.keyboard.press("Escape");
  await page.screenshot({ path: path.join(output, "desktop.png") });
  await page.setViewportSize({ width: 900, height: 700 });
  await row.click({ button: "right" });
  const menu = page.getByRole("menu");
  const bounds = await menu.boundingBox();
  assert.ok(bounds.y >= 0 && bounds.y + bounds.height <= 701, "context menu must fit viewport");
  await menu.press("End");
  assert.equal(await menu.getByRole("menuitem").last().evaluate((el) => el === document.activeElement), true);
  await page.screenshot({ path: path.join(output, "narrow-menu.png") });
  await menu.press("Escape");
  assert.equal(await list.evaluate((el) => el === document.activeElement), true);
  const folder = pane.locator('[data-browse-entry][data-file-entry-kind="directory"]').first();
  await folder.dblclick();
  await pane.locator('[data-file-entry-name="nested"]').waitFor();
  assert.deepEqual(errors, []);
  console.log("PASS: five sort headers/directions, keyboard activation, launch failure feedback, context actions, Space preview, folder activation, narrow menu and focus. Native Windows/macOS GUI UNVERIFIED.");
} finally {
  await browser.close();
  await server.close();
}
