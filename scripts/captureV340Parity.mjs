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
import { ensureProto340 } from "./lib/protoV340.mjs";

const outDir = path.join(process.cwd(), "docs/design/v340-evidence/parity");
await mkdir(outDir, { recursive: true });
const proto = await ensureProto340(process.cwd());
const protoFile = proto.html;
const protoMacFile = path.join(proto.root, "redesign-v2-macos.html");
const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 5200 + Math.floor(Math.random() * 400) } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
const browser = await chromium.launch({ headless: true });
const manifest = [];

async function protoPage(theme, width, height, platform = "Win32") {
  const page = await browser.newPage({ viewport: { width, height } });
  const source = platform === "MacIntel" ? protoMacFile : protoFile;
  await page.goto("file:///" + source.replace(/\\/g, "/"));
  await page.waitForTimeout(600);
  if (theme === "dark") await page.evaluate(() => document.documentElement.classList.add("dark"));
  return page;
}

async function prodPage(theme, width, height, query = "", platform = "Win32") {
  const context = await browser.newContext({ viewport: { width, height }, colorScheme: theme });
  const page = await context.newPage();
  await page.addInitScript(({ mode, platform: platformName }) => {
    Object.defineProperty(navigator, "platform", { get: () => platformName });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "zh");
    localStorage.setItem("zc-theme", mode);
  }, { mode: theme, platform: platform });
  await page.goto(`${baseUrl}/?browser-mock=1&u3-folder-move=1&devices-fixture=1${query}`);
  await page.waitForTimeout(1600);
  return { page, context };
}

async function browseTo(page, folder, paneIndex = 0) {
  const pane = page.locator("[data-pane-id]").nth(paneIndex);
  await pane.click({ position: { x: 40, y: 40 } });
  await pane.getByRole("button", { name: "编辑路径（Ctrl/Cmd+L）", exact: true }).click();
  const input = pane.getByRole("textbox", { name: "文件夹路径", exact: true });
  await input.fill(folder);
  await input.press("Enter");
  await page.waitForTimeout(1100);
}

async function prodFiles(page, { inspector = true } = {}) {
  await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
  await page.waitForTimeout(900);
  await browseTo(page, "C:/U3/Left");
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
  { id: "organize", query: "&pm01-organize=rich", proto: (p) => p.locator('.nav-item[data-page="organize"]').first().click(), prod: (p) => p.locator(".zc-sidebar").getByRole("button", { name: "整理文件", exact: true }).click() },
  { id: "cleanup", query: "&pm01-cleanup=rich", proto: (p) => p.locator('.nav-item[data-page="cleanup"]').first().click(), prod: async (p) => {
    await p.locator(".zc-sidebar").getByRole("button", { name: "空间清理", exact: true }).click();
    await p.waitForTimeout(400);
    const documents = p.getByText("Documents", { exact: true }).first();
    if (await documents.count()) { await documents.click(); await p.waitForTimeout(300); }
    const scan = p.getByRole("button", { name: "扫描选中范围", exact: true }).last();
    if (await scan.count() && await scan.isEnabled()) { await scan.click(); await p.waitForTimeout(1200); }
  } },
  { id: "history", proto: (p) => p.locator('.nav-item[data-page="history"]').first().click(), prod: (p) => p.locator(".zc-sidebar").getByRole("button", { name: "历史记录", exact: true }).click() },
  { id: "settings", proto: (p) => p.locator('.nav-item[data-page="settings"]').first().click(), prod: (p) => p.locator(".zc-sidebar").getByRole("button", { name: "设置", exact: true }).click() },
  {
    id: "panes4",
    proto: async (p) => { await p.evaluate(() => window.zcPanes.setLayout("4")); },
    prod: async (p) => {
      await prodFiles(p, { inspector: false });
      await p.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
      await p.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^4$/ }).first().click();
      await p.waitForTimeout(400);
      await browseTo(p, "C:/U3/Right", 1);
      await browseTo(p, "C:/U1/Source", 2);
      await browseTo(p, "C:/U1/Organize", 3);
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
    const { page: prod, context } = await prodPage(theme, 1440, 900, scenario.query ?? "");
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
    await prodFiles(prod, { inspector: size.w >= 1080 });
    await prod.waitForTimeout(650);
    await shot(proto, `proto-files-light-${size.w}x${size.h}.png`);
    await shot(prod, `prod-files-light-${size.w}x${size.h}.png`);
    manifest.push({ scenario: "files", theme: "light", viewport: `${size.w}x${size.h}`, proto: `proto-files-light-${size.w}x${size.h}.png`, prod: `prod-files-light-${size.w}x${size.h}.png` });
  } finally {
    await context.close();
    await proto.close();
  }
}

// macOS Retina pairs: Finder-style prototype vs MacIntel production at DPR 2.
const macScenarios = [
  { id: "files", prep: async (page) => { await prodFiles(page); }, width: 1440, height: 900, dpr: 2 },
  { id: "columns", prep: async (page) => {
    await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
    await page.waitForTimeout(900);
    await browseTo(page, "C:/U3/Left/MoveFolder");
    await page.locator('[data-file-library-view="columns"]').click();
    await page.waitForTimeout(500);
    const nested = page.locator(".browse-column-row", { hasText: "nested" }).first();
    if (await nested.count()) { await nested.click(); await page.waitForTimeout(700); }
  }, width: 1440, height: 900, dpr: 2 },
  { id: "files-air13", prep: async (page) => { await prodFiles(page); }, width: 1280, height: 832, dpr: 2 }
];
for (const theme of ["light", "dark"]) {
  for (const scenario of macScenarios) {
    if (scenario.id === "files-air13" && theme === "dark") continue;
    const context = await browser.newContext({
      viewport: { width: scenario.width, height: scenario.height },
      deviceScaleFactor: scenario.dpr,
      colorScheme: theme,
      userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36"
    });
    const prod = await context.newPage();
    await prod.addInitScript(({ mode }) => {
      Object.defineProperty(navigator, "platform", { get: () => "MacIntel" });
      localStorage.setItem("zc-onboarding-complete", "true");
      localStorage.setItem("zc-language", "zh");
      localStorage.setItem("zc-theme", mode);
    }, { mode: theme });
    await prod.goto(`${baseUrl}/?browser-mock=1&u3-folder-move=1&devices-fixture=1`);
    await prod.waitForTimeout(1600);
    const protoPageMac = await protoPage(theme, scenario.width, scenario.height, "MacIntel");
    try {
      await scenario.prep(prod);
      await prod.waitForTimeout(650);
      const base = `mac-${scenario.id}-${theme}-${scenario.width}x${scenario.height}`;
      await shot(protoPageMac, `proto-${base}.png`);
      await shot(prod, `prod-${base}.png`);
      manifest.push({ scenario: `mac-${scenario.id}`, theme, viewport: `${scenario.width}x${scenario.height}`, dpr: scenario.dpr, platform: "MacIntel", proto: `proto-${base}.png`, prod: `prod-${base}.png` });
    } finally {
      await context.close();
      await protoPageMac.close();
    }
  }
}

await writeFile(path.join(outDir, "manifest.json"), `${JSON.stringify({ chrome: browser.version(), pairs: manifest }, null, 2)}\n`, "utf8");
await browser.close();
await server.close();
console.log(`[v340-parity] captured ${manifest.length} pairs`);
