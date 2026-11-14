/**
 * v3.40 motion gate (PR 1).
 *
 * Verifies real browser behaviour instead of CSS string presence:
 * press → release, sidebar interpolation, tree width interpolation,
 * view/pane WAAPI ids, instant file selection and reduced-motion parity.
 */
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { createServer } from "vite";

const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 0 } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
assert.ok(baseUrl, "vite server did not start");

const browser = await chromium.launch({ headless: true });
const failures = [];
let step = "start";

async function openPage(options = {}) {
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 },
    reducedMotion: options.reducedMotion ?? "no-preference"
  });
  const page = await context.newPage();
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "Win32" });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "zh");
  });
  await page.goto(`${baseUrl}/?browser-mock=1`);
  await page.waitForTimeout(1500);
  return { page, context };
}

try {
  const { page, context } = await openPage();

  // Press / release on a control that does not change the sidebar state.
  const pressTarget = page.locator(".zc-sidebar .zc-nav-item").first();
  const pressBox = await pressTarget.boundingBox();
  assert.ok(pressBox, "nav item not found");
  await page.mouse.move(pressBox.x + pressBox.width / 2, pressBox.y + pressBox.height / 2);
  await page.mouse.down();
  await page.waitForTimeout(70);
  assert.equal(await pressTarget.getAttribute("data-zc-press"), "active");
  assert.notEqual(await pressTarget.evaluate((el) => getComputedStyle(el).transform), "none");
  await page.mouse.up();
  await page.waitForTimeout(340);
  assert.equal(await pressTarget.getAttribute("data-zc-press"), null);

  // Sidebar expanded → rail interpolation.
  step = "sidebar-rail";
  const toggle = page.locator(".zc-titlebar-toggle");
  const sidebar = page.locator(".zc-sidebar");
  const expandedWidth = await sidebar.evaluate((el) => el.getBoundingClientRect().width);
  await toggle.click();
  await page.waitForFunction(() => {
    const width = document.querySelector(".zc-sidebar")?.getBoundingClientRect().width ?? 0;
    return width > 64 && width < 216;
  }, null, { timeout: 1500 });
  await page.waitForFunction(() => Math.round(document.querySelector(".zc-sidebar")?.getBoundingClientRect().width ?? 0) === 64, null, { timeout: 1500 });
  const railText = await page.locator(".zc-nav-text").first().evaluate((el) => getComputedStyle(el).opacity);
  assert.equal(railText, "0", "rail text should fade rather than unmount");
  assert.ok(expandedWidth > 200);

  // Rail → hidden interpolation, sidebar stays mounted but inert.
  step = "sidebar-hidden";
  await toggle.click();
  await page.waitForFunction(() => {
    const width = document.querySelector(".zc-sidebar")?.getBoundingClientRect().width ?? 1;
    return width >= 0 && width < 64;
  }, null, { timeout: 1500 });
  await page.waitForFunction(() => Math.round(document.querySelector(".zc-sidebar")?.getBoundingClientRect().width ?? -1) === 0, null, { timeout: 1500 });
  assert.equal(await sidebar.getAttribute("aria-hidden"), "true");
  assert.ok(await sidebar.evaluate((el) => el.hasAttribute("inert")));

  // Restore expanded for the file-library checks.
  await page.evaluate(() => document.querySelector(".zc-titlebar-toggle")?.click());
  await page.waitForTimeout(350);
  await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
  await page.waitForTimeout(1200);
  await page.getByText("Managed mock root", { exact: false }).first().click();
  await page.waitForTimeout(1200);

  // Tree width interpolation; the panel keeps its measured width while clipped.
  step = "tree-close";
  const treeToggle = page.locator("[data-file-library-nav-toggle]");
  const treeOpenWidth = await page.locator(".file-library-navigation-slot").evaluate((el) => el.getBoundingClientRect().width);
  await treeToggle.click();
  await page.waitForFunction((openWidth) => {
    const slot = document.querySelector(".file-library-navigation-slot");
    const width = slot?.getBoundingClientRect().width ?? -1;
    return width > 0 && width < openWidth;
  }, treeOpenWidth, { timeout: 1500 });
  const closingPanelWidth = await page.locator(".file-library-navigation-panel").evaluate((el) => el.getBoundingClientRect().width);
  assert.ok(closingPanelWidth >= 190, `panel reflowed during close: ${closingPanelWidth}`);
  await page.waitForFunction(() => document.querySelector(".file-library-navigation-slot")?.getBoundingClientRect().width === 0, null, { timeout: 1500 });
  await treeToggle.click();
  await page.waitForFunction((openWidth) => {
    const width = document.querySelector(".file-library-navigation-slot")?.getBoundingClientRect().width ?? 0;
    return width > 0 && width < openWidth;
  }, treeOpenWidth, { timeout: 1500 });
  assert.equal(await page.locator(".file-library-navigation-slot").getAttribute("aria-hidden"), "false");

  // File selection is instantaneous; the check mark animates separately.
  step = "file-selection";
  const row = page.locator("[role='option']").first();
  await row.click();
  await page.waitForTimeout(20);
  const selection = await row.evaluate((el) => ({
    duration: getComputedStyle(el).transitionDuration,
    checked: el.querySelector(".shared-file-list-checkbox svg")?.getAnimations().map((animation) => animation.effect?.getTiming().duration) ?? []
  }));
  assert.equal(selection.duration, "0s");
  assert.ok(selection.checked.includes(260));

  // View swap uses one 240ms WAAPI run.
  step = "view-swap";
  await page.locator('[data-file-library-view="grid"]').click();
  await page.waitForTimeout(30);
  const viewAnimations = await page.evaluate(() => document.querySelector(".file-library-content-slot")?.getAnimations().map((animation) => ({ id: animation.id, duration: animation.effect?.getTiming().duration })) ?? []);
  assert.ok(viewAnimations.some((animation) => animation.id === "zc-view-swap" && animation.duration === 240));

  // Pane layout uses per-pane 260ms runs, not a grid-track animation.
  step = "pane-layout";
  await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
  await page.waitForTimeout(120);
  await page.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^2$/ }).first().click();
  await page.waitForTimeout(30);
  const paneAnimations = await page.evaluate(() => [...document.querySelectorAll(".wb-pane")]
    .filter((pane) => pane.getClientRects().length)
    .map((pane) => ({
      id: pane.getAttribute("data-pane-id"),
      enters: pane.getAnimations().filter((animation) => animation.id === "zc-pane-in").map((animation) => animation.effect?.getTiming().duration)
    })));
  // Only the newly revealed pane lands in; the existing pane must not replay.
  assert.equal(paneAnimations.filter((pane) => pane.enters.length > 0).length, 1, JSON.stringify(paneAnimations));
  assert.ok(paneAnimations.some((pane) => pane.enters.includes(260)));

  // Page switch is a 220ms WAAPI run and ends on identity.
  step = "page-switch";
  await page.locator(".zc-sidebar").getByRole("button", { name: "整理文件", exact: true }).click();
  await page.waitForTimeout(30);
  const pageAnimations = await page.evaluate(() => document.querySelector("main > div.min-h-0")?.getAnimations().filter((animation) => animation.id === "zc-page-in").map((animation) => animation.effect?.getTiming().duration) ?? []);
  assert.ok(pageAnimations.includes(220));
  await page.waitForTimeout(300);
  assert.equal(await page.evaluate(() => getComputedStyle(document.querySelector("main > div.min-h-0")).opacity), "1");

  await context.close();

  // Reduced motion: no WAAPI, instant panel states, full functionality.
  step = "reduced-motion";
  const reduced = await openPage({ reducedMotion: "reduce" });
  const reducedPage = reduced.page;
  await reducedPage.locator(".zc-titlebar-toggle").click();
  await reducedPage.waitForTimeout(30);
  assert.ok(Math.round(await reducedPage.locator(".zc-sidebar").evaluate((el) => el.getBoundingClientRect().width)) === 64);
  await reducedPage.locator(".zc-sidebar").getByRole("button", { name: "整理文件", exact: true }).click({ force: true });
  await reducedPage.waitForTimeout(30);
  assert.equal(await reducedPage.evaluate(() => document.querySelector("main > div.min-h-0")?.getAnimations().length ?? 0), 0);
  await reduced.context.close();
} catch (error) {
  failures.push(`${step}: ${error instanceof Error ? error.message : String(error)}`);
} finally {
  await browser.close();
  await server.close();
}

if (failures.length) {
  console.error("[v340-motion] FAIL\n" + failures.join("\n"));
  process.exit(1);
}
console.log("[v340-motion] PASS press/release, sidebar, tree, view, panes, selection, reduced motion");
