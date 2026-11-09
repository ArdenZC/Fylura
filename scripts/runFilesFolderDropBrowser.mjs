/** Rendered pointer regression only. Native filesystem/GUI remains UNVERIFIED. */
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
const output = process.env.FILES_DROP_OUTPUT_DIR ?? "/tmp/fylura-folder-drop-browser";
await mkdir(output, { recursive: true });
const server = await createServer({ server: { host: "127.0.0.1", port: 0 } }); await server.listen();
const browser = await chromium.launch({ headless: true });
const errors = [];
try {
  const page = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (msg) => { if (msg.type() === "error") errors.push(msg.text()); });
  await page.addInitScript(() => { Object.defineProperty(navigator, "platform", { get: () => "Win32" }); localStorage.setItem("zc-onboarding-complete", "true"); localStorage.setItem("zc-language", "en"); });
  await page.goto(`${server.resolvedUrls.local[0]}?browser-mock=1`);
  assert.equal(await page.title(), "Fylura");
  await page.getByRole("button", { name: "Files", exact: true }).click();
  const pane = page.locator('[role="tabpanel"]:not([hidden]) [data-pane-id]').first();
  async function navigate(folder) {
    await pane.getByRole("button", { name: "Edit path (Ctrl/Cmd+L)", exact: true }).click();
    await pane.getByRole("textbox", { name: "Folder path", exact: true }).fill(folder);
    await pane.getByRole("textbox", { name: "Folder path", exact: true }).press("Enter");
    await pane.locator("[data-file-library-query-menu] > summary").click();
    await pane.locator("[data-browse-sort]").selectOption("name");
    await pane.locator("[data-file-library-query-menu] > summary").click();
  }
  const row = (name) => pane.locator(`[data-file-entry-name="${name}"]`);
  async function drag(name, shift = false) {
    const source = row(name), target = row("Subfolder");
    await source.click();
    const a = await source.boundingBox(), b = await target.boundingBox();
    if (shift) await page.keyboard.down("Shift");
    await page.mouse.move(a.x + 90, a.y + a.height / 2); await page.mouse.down();
    await page.mouse.move(b.x + 90, b.y + b.height / 2, { steps: 12 });
    assert.equal(await pane.getAttribute("data-internal-drop"), "active");
    await page.mouse.up();
    if (shift) await page.keyboard.up("Shift");
  }
  await navigate("C:/U3/Left");
  await row("Subfolder").waitFor();
  await drag("Case.txt");
  // Wait for the shared task center to report completion before changing source.
  await pane.getByRole("button", { name: /Operation results.*1\/1/ }).waitFor();
  await navigate("C:/U3/Left/Subfolder");
  await row("Case.txt").waitFor();
  await page.screenshot({ path: path.join(output, "copy-result.png") });
  await navigate("C:/U3/Left");
  await drag("Document-000.md", true);
  await row("Document-000.md").waitFor({ state: "detached" });
  await navigate("C:/U3/Left/Subfolder");
  await row("Document-000.md").waitFor();
  await page.screenshot({ path: path.join(output, "move-result.png") });
  assert.deepEqual(errors, []);
  console.log("PASS: real single-pane pointer Copy and Shift-Move onto a folder use the existing task center; destination rows and source removal verified. Browser mock only.");
} finally { await browser.close(); await server.close(); }
