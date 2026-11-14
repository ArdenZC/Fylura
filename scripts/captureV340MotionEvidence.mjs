/**
 * Captures the PR 1 motion evidence bundle:
 * docs/design/v340-evidence/motion-pr1/{metrics.json,*.png}
 */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const outDir = path.join(process.cwd(), "docs/design/v340-evidence/motion-pr1");
await mkdir(outDir, { recursive: true });
const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 0 } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
const browser = await chromium.launch({ headless: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
await page.addInitScript(() => {
  Object.defineProperty(navigator, "platform", { get: () => "Win32" });
  localStorage.setItem("zc-onboarding-complete", "true");
  localStorage.setItem("zc-language", "zh");
});
await page.goto(`${baseUrl}/?browser-mock=1`);
await page.waitForTimeout(1600);

const metrics = {};
const sidebarWidth = () => page.evaluate(() => document.querySelector(".zc-sidebar")?.getBoundingClientRect().width ?? -1);
const clips = { x: 0, y: 0, width: 460, height: 900 };

await page.screenshot({ path: path.join(outDir, "sidebar-expanded.png"), clip: clips });
metrics.sidebarExpanded = await sidebarWidth();
await page.locator(".zc-titlebar-toggle").click();
await page.waitForTimeout(80);
metrics.sidebarRailMid = await sidebarWidth();
metrics.railTextMidOpacity = await page.locator(".zc-nav-text").first().evaluate((el) => getComputedStyle(el).opacity);
await page.screenshot({ path: path.join(outDir, "sidebar-rail-mid.png"), clip: clips });
await page.waitForTimeout(260);
metrics.sidebarRail = await sidebarWidth();
await page.screenshot({ path: path.join(outDir, "sidebar-rail.png"), clip: clips });
await page.locator(".zc-titlebar-toggle").click();
await page.waitForTimeout(80);
metrics.sidebarHiddenMid = await sidebarWidth();
await page.screenshot({ path: path.join(outDir, "sidebar-hidden-mid.png"), clip: clips });
await page.waitForTimeout(260);
metrics.sidebarHidden = await sidebarWidth();
await page.screenshot({ path: path.join(outDir, "sidebar-hidden.png"), clip: clips });

await page.evaluate(() => document.querySelector(".zc-titlebar-toggle")?.click());
await page.waitForTimeout(320);
await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
await page.waitForTimeout(1200);
await page.getByText("Managed mock root", { exact: false }).first().click();
await page.waitForTimeout(1200);
metrics.treeOpenWidth = await page.locator(".file-library-navigation-slot").evaluate((el) => el.getBoundingClientRect().width);
await page.screenshot({ path: path.join(outDir, "tree-open.png"), clip: { x: 300, y: 100, width: 700, height: 760 } });
await page.locator("[data-file-library-nav-toggle]").click();
await page.waitForTimeout(80);
metrics.treeClosingWidth = await page.locator(".file-library-navigation-slot").evaluate((el) => el.getBoundingClientRect().width);
metrics.treeClosingPanelWidth = await page.locator(".file-library-navigation-panel").evaluate((el) => el.getBoundingClientRect().width);
await page.screenshot({ path: path.join(outDir, "tree-closing-mid.png"), clip: { x: 300, y: 100, width: 700, height: 760 } });
await page.waitForTimeout(260);
metrics.treeClosedWidth = await page.locator(".file-library-navigation-slot").evaluate((el) => el.getBoundingClientRect().width);
await page.locator("[data-file-library-nav-toggle]").click();
await page.waitForTimeout(80);
metrics.treeOpeningWidth = await page.locator(".file-library-navigation-slot").evaluate((el) => el.getBoundingClientRect().width);
await page.waitForTimeout(260);
await page.screenshot({ path: path.join(outDir, "tree-reopened.png"), clip: { x: 300, y: 100, width: 700, height: 760 } });

const row = page.locator("[role='option']").first();
await row.click();
await page.waitForTimeout(20);
metrics.selection = await row.evaluate((el) => ({
  transitionDuration: getComputedStyle(el).transitionDuration,
  checkAnimations: el.querySelector(".shared-file-list-checkbox svg")?.getAnimations().map((animation) => ({
    duration: animation.effect?.getTiming().duration,
    id: animation.id
  })) ?? []
}));
await page.screenshot({ path: path.join(outDir, "file-selected.png"), clip: { x: 380, y: 120, width: 900, height: 500 } });

await page.locator('[data-file-library-view="grid"]').click();
await page.waitForTimeout(30);
metrics.viewSwap = await page.evaluate(() => document.querySelector(".file-library-content-slot")?.getAnimations().map((animation) => ({ id: animation.id, duration: animation.effect?.getTiming().duration })) ?? []);
await page.locator("[data-pane-menu] > summary").click();
await page.waitForTimeout(120);
await page.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^2$/ }).first().click();
await page.waitForTimeout(30);
metrics.paneEnter = await page.evaluate(() => [...document.querySelectorAll(".wb-pane")]
  .filter((pane) => pane.getClientRects().length)
  .flatMap((pane) => pane.getAnimations().filter((animation) => animation.id === "zc-pane-in").map((animation) => animation.effect?.getTiming().duration)));
await page.locator(".zc-sidebar").getByRole("button", { name: "概览", exact: true }).click();
await page.waitForTimeout(30);
metrics.pageEnter = await page.evaluate(() => document.querySelector("main > div.min-h-0")?.getAnimations().filter((animation) => animation.id === "zc-page-in").map((animation) => animation.effect?.getTiming().duration) ?? []);

const reduced = await browser.newContext({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });
const reducedPage = await reduced.newPage();
await reducedPage.addInitScript(() => {
  Object.defineProperty(navigator, "platform", { get: () => "Win32" });
  localStorage.setItem("zc-onboarding-complete", "true");
  localStorage.setItem("zc-language", "zh");
});
await reducedPage.goto(`${baseUrl}/?browser-mock=1`);
await reducedPage.waitForTimeout(1500);
await reducedPage.locator(".zc-titlebar-toggle").click();
await reducedPage.waitForTimeout(30);
metrics.reducedMotion = {
  sidebarWidthAfter30ms: await reducedPage.locator(".zc-sidebar").evaluate((el) => el.getBoundingClientRect().width),
  transitions: await reducedPage.locator(".zc-sidebar").evaluate((el) => getComputedStyle(el).transitionDuration)
};
await reduced.close();

await writeFile(path.join(outDir, "metrics.json"), JSON.stringify(metrics, null, 2) + "\n", "utf8");
await browser.close();
await server.close();
console.log(JSON.stringify(metrics, null, 2));
