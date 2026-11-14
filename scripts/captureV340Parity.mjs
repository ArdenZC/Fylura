/**
 * v3.40 same-state visual parity capture.
 *
 * Pairs the prototype (file://) and production (Vite + browser mock) at the
 * same viewport, theme and interaction state. Data differs by construction —
 * the prototype has demo files, production has the managed mock root — so the
 * images are for structure/material comparison, not pixel diffing.
 */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const outDir = path.join(process.cwd(), "docs/design/v340-evidence/parity");
await mkdir(outDir, { recursive: true });
const protoFile = "F:/Coding/_proto340/p340/fylura-proto-v3.40/redesign-v2.html";
const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 0 } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
const browser = await chromium.launch({ headless: true });
const manifest = [];

async function protoPage(theme, width, height) {
  const page = await browser.newPage({ viewport: { width, height } });
  await page.goto("file:///" + protoFile.replace(/\\/g, "/"));
  await page.waitForTimeout(600);
  if (theme === "dark") await page.evaluate(() => document.documentElement.classList.add("dark"));
  return page;
}

async function prodPage(theme, width, height) {
  const context = await browser.newContext({ viewport: { width, height }, colorScheme: theme });
  const page = await context.newPage();
  await page.addInitScript(({ mode }) => {
    Object.defineProperty(navigator, "platform", { get: () => "Win32" });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "zh");
    localStorage.setItem("zc-theme", mode);
  }, { mode: theme });
  await page.goto(`${baseUrl}/?browser-mock=1`);
  await page.waitForTimeout(1600);
  return { page, context };
}

async function prodFiles(page, { inspector = true } = {}) {
  await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
  await page.waitForTimeout(900);
  await page.getByText("Managed mock root", { exact: false }).first().click();
  await page.waitForTimeout(1200);
  if ((await page.locator("[role='option']").count()) === 0) {
    // Medium/compact layouts first land on the places start page.
    const card = page.locator(".file-library-content-slot article.browse-location-card")
      .filter({ hasText: "Managed mock root" })
      .first();
    await card.getByRole("button", { name: "打开位置" }).click();
    await page.waitForTimeout(1200);
  }
  await page.locator("[role='option']").first().click();
  if (inspector) {
    await page.locator("[data-file-library-context-toggle]").click();
    await page.waitForTimeout(400);
  }
}

async function shot(page, name) {
  await page.screenshot({ path: path.join(outDir, name) });
}

const pages = [
  { id: "files", proto: async () => {}, prod: (p) => prodFiles(p) },
  { id: "overview", proto: (p) => p.locator('.nav-item[data-page="overview"]').first().click(), prod: (p) => p.locator(".zc-sidebar").getByRole("button", { name: "概览", exact: true }).click() },
  { id: "organize", proto: (p) => p.locator('.nav-item[data-page="organize"]').first().click(), prod: (p) => p.locator(".zc-sidebar").getByRole("button", { name: "整理文件", exact: true }).click() },
  { id: "cleanup", proto: (p) => p.locator('.nav-item[data-page="cleanup"]').first().click(), prod: (p) => p.locator(".zc-sidebar").getByRole("button", { name: "空间清理", exact: true }).click() },
  { id: "history", proto: (p) => p.locator('.nav-item[data-page="history"]').first().click(), prod: (p) => p.locator(".zc-sidebar").getByRole("button", { name: "历史记录", exact: true }).click() },
  { id: "settings", proto: (p) => p.locator('.nav-item[data-page="settings"]').first().click(), prod: (p) => p.locator(".zc-sidebar").getByRole("button", { name: "设置", exact: true }).click() },
  {
    id: "panes4",
    proto: async (p) => { await p.evaluate(() => window.zcPanes.setLayout("4")); },
    prod: async (p) => {
      await prodFiles(p, { inspector: false });
      await p.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
      await p.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^4$/ }).first().click();
    }
  },
  {
    id: "menu",
    proto: (p) => p.locator("#layoutBtn").click(),
    prod: async (p) => {
      await prodFiles(p, { inspector: false });
      await p.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
    }
  },
  {
    id: "search",
    proto: async (p) => {
      await p.evaluate(() => { const spot = document.getElementById("spot"); if (spot) spot.hidden = false; });
    },
    prod: (p) => p.keyboard.press("Control+k")
  }
];

for (const theme of ["light", "dark"]) {
  for (const scenario of pages) {
    const { page: prod, context } = await prodPage(theme, 1440, 900);
    const proto = await protoPage(theme, 1440, 900);
    try {
      await scenario.prod(prod);
      await prod.waitForTimeout(650);
      await scenario.proto(proto);
      await proto.waitForTimeout(650);
      const base = `${scenario.id}-${theme}-1440x900`;
      await shot(proto, `proto-${base}.png`);
      await shot(prod, `prod-${base}.png`);
      manifest.push({ scenario: scenario.id, theme, viewport: "1440x900", proto: `proto-${base}.png`, prod: `prod-${base}.png` });
    } catch (error) {
      await shot(prod, `error-${scenario.id}-${theme}.png`).catch(() => undefined);
      console.error(`[v340-parity] failed ${scenario.id}/${theme}`, error instanceof Error ? error.message : error);
      throw error;
    } finally {
      await context.close();
      await proto.close();
    }
  }
}

for (const size of [{ w: 1080, h: 740 }, { w: 850, h: 680 }]) {
  const { page: prod, context } = await prodPage("light", size.w, size.h);
  const proto = await protoPage("light", size.w, size.h);
  try {
    await prodFiles(prod, { inspector: false });
    await prod.waitForTimeout(650);
    await shot(proto, `proto-files-light-${size.w}x${size.h}.png`);
    await shot(prod, `prod-files-light-${size.w}x${size.h}.png`);
    manifest.push({ scenario: "files", theme: "light", viewport: `${size.w}x${size.h}`, proto: `proto-files-light-${size.w}x${size.h}.png`, prod: `prod-files-light-${size.w}x${size.h}.png` });
  } finally {
    await context.close();
    await proto.close();
  }
}

await writeFile(path.join(outDir, "manifest.json"), `${JSON.stringify({ chrome: browser.version(), pairs: manifest }, null, 2)}\n`, "utf8");
await browser.close();
await server.close();
console.log(`[v340-parity] captured ${manifest.length} pairs`);
