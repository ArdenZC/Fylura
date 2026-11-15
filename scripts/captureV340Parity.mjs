/**
 * v3.40 same-state visual parity capture.
 *
 * Pairs the prototype (file://) and production (Vite + browser mock) at the
 * same viewport, DPR, theme, platform and interaction state. Data differs by
 * construction — the prototype has demo files, production has the managed mock
 * root / Finder fixture — so the images are for structure/material comparison,
 * not pixel diffing. Every pair is rejected when the two PNGs do not have
 * identical physical pixel dimensions.
 */
import { mkdir, readFile, readdir, unlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";
import { ensureProto340 } from "./lib/protoV340.mjs";

const outDir = path.join(process.cwd(), "docs/design/v340-evidence/parity");
await mkdir(outDir, { recursive: true });
const proto = await ensureProto340(process.cwd());
const protoWinFile = proto.html;
const protoMacFile = path.join(proto.root, "redesign-v2-macos.html");
const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 5200 + Math.floor(Math.random() * 400) } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("vite server did not start");
const browser = await chromium.launch({ headless: true });
const manifest = [];
const MAC_UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const WIN_UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

// Drop previous pairs so a failed/partial run can never leave a stale image
// next to a fresh manifest entry.
for (const name of await readdir(outDir)) {
  if (/^(proto|prod|error)-.*\.(png|webp)$/u.test(name)) await unlink(path.join(outDir, name)).catch(() => undefined);
}

function platformOptions(platform) {
  const mac = platform === "MacIntel";
  return {
    userAgent: mac ? MAC_UA : WIN_UA,
    locale: "zh-CN",
    timezoneId: "Asia/Shanghai"
  };
}

async function protoPage({ theme, width, height, dpr, platform }) {
  const context = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: dpr,
    colorScheme: theme,
    ...platformOptions(platform)
  });
  const page = await context.newPage();
  const source = platform === "MacIntel" ? protoMacFile : protoWinFile;
  await page.goto(`file:///${source.replace(/\\/g, "/")}`);
  await page.waitForTimeout(700);
  if (theme === "dark") await page.evaluate(() => document.documentElement.classList.add("dark"));
  return { page, context };
}

async function prodPage({ theme, width, height, dpr, platform, query = "" }) {
  const context = await browser.newContext({
    viewport: { width, height },
    deviceScaleFactor: dpr,
    colorScheme: theme,
    ...platformOptions(platform)
  });
  const page = await context.newPage();
  await page.addInitScript(({ mode, platform: platformName }) => {
    Object.defineProperty(navigator, "platform", { get: () => platformName });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "zh");
    localStorage.setItem("zc-theme", mode);
  }, { mode: theme, platform });
  await page.goto(`${baseUrl}/?browser-mock=1&u3-folder-move=1&devices-fixture=1&finder-fixture=1${query}`);
  await page.waitForTimeout(1600);
  return { page, context };
}

async function browseTo(page, folder) {
  const pane = page.locator("[data-pane-id]").first();
  await pane.click({ position: { x: 40, y: 40 } });
  await pane.getByRole("button", { name: "编辑路径（Ctrl/Cmd+L）", exact: true }).click();
  const input = pane.getByRole("textbox", { name: "文件夹路径", exact: true });
  await input.fill(folder);
  await input.press("Enter");
  await page.waitForTimeout(1100);
}

async function browsePane(page, index, folder) {
  const pane = page.locator("[data-pane-id]").nth(index);
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

async function prodMacColumns(page, { select = "assets-2025.zip", deep = false, inspect = false } = {}) {
  await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
  await page.waitForTimeout(900);
  await page.locator('[data-file-library-view="columns"]').click();
  await page.waitForTimeout(300);
  await browseTo(page, "C:/U3/Finder");
  const row = (name) => page.locator('.browse-column[data-browse-column-current="true"] .browse-column-row', { hasText: name }).first();
  await row("Documents").click();
  await page.waitForTimeout(500);
  await row("Workspace").click();
  await page.waitForTimeout(600);
  if (deep) {
    await row("Assets").click();
    await page.waitForTimeout(500);
  }
  if (select) {
    await page.locator('.browse-column[data-browse-column-current="true"] .browse-column-row', { hasText: select }).first().click();
    await page.waitForTimeout(450);
  }
  if (inspect) {
    await page.locator("[data-file-library-context-toggle]").click();
    await page.waitForTimeout(400);
  }
}

function pngSize(buffer) {
  if (buffer.length < 24 || buffer.toString("ascii", 1, 4) !== "PNG") throw new Error("not a PNG");
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

const converter = await browser.newPage();
async function toWebp(pngPath, webpPath) {
  const base64 = (await readFile(pngPath)).toString("base64");
  const encoded = await converter.evaluate(async (data) => {
    const response = await fetch(`data:image/png;base64,${data}`);
    const bitmap = await createImageBitmap(await response.blob());
    const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("canvas unavailable");
    context.drawImage(bitmap, 0, 0);
    const blob = await canvas.convertToBlob({ type: "image/webp", quality: 0.9 });
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    const chunk = 0x8000;
    for (let index = 0; index < bytes.length; index += chunk) {
      binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
    }
    return btoa(binary);
  }, base64);
  await writeFile(webpPath, Buffer.from(encoded, "base64"));
  await unlink(pngPath).catch(() => undefined);
}

async function capturePair({ id, label, theme, width, height, dpr = 1, platform = "Win32", query = "", protoPrep = async () => {}, prodPrep = async () => {} }) {
  const protoSession = await protoPage({ theme, width, height, dpr, platform });
  const prodSession = await prodPage({ theme, width, height, dpr, platform, query });
  try {
    await prodPrep(prodSession.page);
    await prodSession.page.waitForTimeout(650);
    await protoPrep(protoSession.page);
    await protoSession.page.waitForTimeout(650);
    const base = `${id}-${theme}-${width}x${height}@${dpr}x`;
    const protoPng = path.join(outDir, `proto-${base}.png`);
    const prodPng = path.join(outDir, `prod-${base}.png`);
    await protoSession.page.screenshot({ path: protoPng });
    await prodSession.page.screenshot({ path: prodPng });
    const protoSize = pngSize(await readFile(protoPng));
    const prodSize = pngSize(await readFile(prodPng));
    const expected = { width: width * dpr, height: height * dpr };
    if (protoSize.width !== prodSize.width || protoSize.height !== prodSize.height
      || protoSize.width !== expected.width || protoSize.height !== expected.height) {
      throw new Error(`PAIR DIMENSION MISMATCH ${id}/${theme}: proto ${protoSize.width}x${protoSize.height}, prod ${prodSize.width}x${prodSize.height}, expected ${expected.width}x${expected.height}`);
    }
    const protoWebp = `proto-${base}.webp`;
    const prodWebp = `prod-${base}.webp`;
    await toWebp(protoPng, path.join(outDir, protoWebp));
    await toWebp(prodPng, path.join(outDir, prodWebp));
    manifest.push({
      scenario: id,
      label,
      theme,
      platform,
      viewport: { width, height },
      dpr,
      physical: protoSize,
      proto: protoWebp,
      prod: prodWebp
    });
    console.log(`[v340-parity] ${id}/${theme} ${width}x${height}@${dpr}x → ${protoSize.width}x${protoSize.height}`);
  } finally {
    await protoSession.context.close();
    await prodSession.context.close();
  }
}

const clickNav = (page, selector) => page.locator(selector).first().click();
const prodNav = (name) => async (page) => {
  await page.locator(".zc-sidebar").getByRole("button", { name, exact: true }).click();
};

const winScenarios = [
  { id: "files", label: "Files · list + Inspector", protoPrep: async () => {}, prodPrep: (page) => prodFiles(page) },
  { id: "overview", label: "Overview", protoPrep: (page) => clickNav(page, '.nav-item[data-page="overview"]'), prodPrep: prodNav("概览") },
  { id: "organize-plan", label: "Organize · 8-item plan / AI ready", query: "&pm01-organize=rich&pm01-readiness=ready", protoPrep: (page) => clickNav(page, '.nav-item[data-page="organize"]'), prodPrep: prodNav("整理文件") },
  { id: "cleanup", label: "Cleanup · findings", query: "&pm01-cleanup=rich", protoPrep: (page) => clickNav(page, '.nav-item[data-page="cleanup"]'), prodPrep: async (page) => {
    await prodNav("空间清理")(page);
    await page.waitForTimeout(400);
    const documents = page.getByText("Documents", { exact: true }).first();
    if (await documents.count()) { await documents.click(); await page.waitForTimeout(300); }
    const scan = page.getByRole("button", { name: "扫描选中范围", exact: true }).last();
    if (await scan.count() && await scan.isEnabled()) { await scan.click(); await page.waitForTimeout(1200); }
  } },
  { id: "history", label: "History / Restore", protoPrep: (page) => clickNav(page, '.nav-item[data-page="history"]'), prodPrep: prodNav("历史记录") },
  { id: "settings", label: "Settings", protoPrep: (page) => clickNav(page, '.nav-item[data-page="settings"]'), prodPrep: prodNav("设置") },
  { id: "panes4", label: "Files · 4 panes / 4 folders", protoPrep: (page) => page.evaluate(() => window.zcPanes.setLayout("4")), prodPrep: async (page) => {
    await prodFiles(page, { inspector: false });
    await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
    await page.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^4$/ }).first().click();
    await page.waitForTimeout(400);
    await browsePane(page, 1, "C:/U3/Right");
    await browsePane(page, 2, "C:/U1/Source");
    await browsePane(page, 3, "C:/U1/Organize");
  } },
  { id: "menu", label: "Files · pane layout menu", protoPrep: (page) => page.locator("#layoutBtn").click(), prodPrep: async (page) => {
    await prodFiles(page, { inspector: false });
    await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
  } },
  { id: "search", label: "Global Search", protoPrep: async (page) => {
    await page.evaluate(() => { const spot = document.getElementById("spot"); if (spot) spot.hidden = false; });
  }, prodPrep: (page) => page.keyboard.press("Control+k") }
];

const macOnly = process.argv.includes("--mac-only");
if (!macOnly) {
  for (const theme of ["light", "dark"]) {
    for (const scenario of winScenarios) {
      await capturePair({ ...scenario, theme, width: 1440, height: 900, dpr: 1, platform: "Win32" });
    }
  }
}

if (!macOnly) for (const size of [{ w: 1080, h: 740 }, { w: 850, h: 680 }]) {
  await capturePair({
    id: "files",
    label: "Files · list + Inspector",
    theme: "light",
    width: size.w,
    height: size.h,
    dpr: 1,
    platform: "Win32",
    prodPrep: (page) => prodFiles(page, { inspector: size.w >= 1080 })
  });
}

// P0: macOS Retina. Both sides use the same MacIntel UA, viewport and DPR;
// the pair fails if the physical PNG dimensions differ.
const macSizes = [
  { id: "mac-columns-1440", label: "Finder Columns · 3 levels + file preview", width: 1440, height: 900, dpr: 2, themes: ["light", "dark"], deep: false, select: "assets-2025.zip" },
  { id: "mac-columns-air13", label: "Finder Columns · Air 13", width: 1280, height: 832, dpr: 2, themes: ["light", "dark"], deep: false, select: "assets-2025.zip" },
  { id: "mac-columns-pro14", label: "Finder Columns · Pro 14", width: 1512, height: 982, dpr: 2, themes: ["light"], deep: false, select: "Design tokens v2.pdf" },
  { id: "mac-columns-pro16", label: "Finder Columns · Pro 16", width: 1728, height: 1117, dpr: 2, themes: ["light"], deep: false, select: "2026 年第四季度设计系统交付清单与验收记录（最终版）.pdf" },
  { id: "mac-columns-5k", label: "Finder Columns · 5K", width: 2560, height: 1440, dpr: 2, themes: ["light"], deep: false, select: "2026 年第四季度设计系统交付清单与验收记录（最终版）.pdf" },
  { id: "mac-columns-min", label: "Finder Columns · minimum window", width: 980, height: 680, dpr: 2, themes: ["light"], deep: false, select: "assets-2025.zip" },
  { id: "mac-columns-dpr1", label: "Finder Columns · DPR1 control (not Retina evidence)", width: 1440, height: 900, dpr: 1, themes: ["light"], deep: false, select: "assets-2025.zip" }
];

for (const size of macSizes) {
  for (const theme of size.themes) {
    await capturePair({
      id: size.id,
      label: size.label,
      theme,
      width: size.width,
      height: size.height,
      dpr: size.dpr,
      platform: "MacIntel",
      prodPrep: (page) => prodMacColumns(page, { select: size.select, deep: size.deep })
    });
  }
}

// State A is a production-only state (AI provider unavailable). It is captured
// separately and never paired with the prototype's ready plan.
if (!macOnly) await capturePair({
  id: "organize-unavailable",
  label: "Organize · AI unavailable (state A)",
  theme: "light",
  width: 1440,
  height: 900,
  dpr: 1,
  platform: "Win32",
  query: "&pm01-organize=rich&pm01-readiness=disconnected",
  protoPrep: (page) => clickNav(page, '.nav-item[data-page="organize"]'),
  prodPrep: prodNav("整理文件")
});

// 4 Finder panes on macOS is a Fylura-only capability (the prototype has no
// 4-pane Finder). Capture it without pretending it is a prototype pair.
await (async () => {
  const session = await prodPage({ theme: "light", width: 1512, height: 982, dpr: 2, platform: "MacIntel" });
  try {
    const page = session.page;
    await prodMacColumns(page, { select: "assets-2025.zip" });
    await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
    await page.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^4$/ }).first().click();
    await page.waitForTimeout(500);
    await browsePane(page, 1, "C:/U3/Finder/Documents/Archive");
    await browsePane(page, 2, "C:/U3/Right");
    await browsePane(page, 3, "C:/U1/Organize");
    await page.waitForTimeout(600);
    const file = path.join(outDir, "prod-mac-panes4-light-1512x982@2x.png");
    await page.screenshot({ path: file });
    const size = pngSize(await readFile(file));
    await toWebp(file, path.join(outDir, "prod-mac-panes4-light-1512x982@2x.webp"));
    manifest.push({
      scenario: "mac-panes4",
      label: "Finder Columns · 4 panes (production-only)",
      theme: "light",
      platform: "MacIntel",
      viewport: { width: 1512, height: 982 },
      dpr: 2,
      physical: size,
      proto: null,
      prod: "prod-mac-panes4-light-1512x982@2x.webp"
    });
  } finally {
    await session.context.close();
  }
})();

// Four Finder columns have no prototype equivalent (the prototype only ships
// a three-column demo). Capture the production state on its own and label it,
// instead of calling a 4-vs-3 pair "same state".
for (const spec of [
  { id: "mac-columns-deep-980", label: "Finder Columns · 4 levels / min window (production-only)", width: 980, height: 680 },
  { id: "mac-columns-deep-pro14", label: "Finder Columns · 4 levels / horizontal scroll (production-only)", width: 1512, height: 982 }
]) {
  const session = await prodPage({ theme: "light", width: spec.width, height: spec.height, dpr: 2, platform: "MacIntel" });
  try {
    await prodMacColumns(session.page, { deep: true, select: "poster.png" });
    await session.page.waitForTimeout(500);
    const base = `prod-${spec.id}-light-${spec.width}x${spec.height}@2x`;
    const png = path.join(outDir, `${base}.png`);
    await session.page.screenshot({ path: png });
    const size = pngSize(await readFile(png));
    await toWebp(png, path.join(outDir, `${base}.webp`));
    manifest.push({
      scenario: spec.id,
      label: spec.label,
      theme: "light",
      platform: "MacIntel",
      viewport: { width: spec.width, height: spec.height },
      dpr: 2,
      physical: size,
      proto: null,
      prod: `${base}.webp`
    });
  } finally {
    await session.context.close();
  }
}

await writeFile(path.join(outDir, "manifest.json"), `${JSON.stringify({
  chrome: browser.version(),
  capturedAt: new Date().toISOString(),
  rule: "proto and prod must share viewport, DPR, theme, platform; physical PNG dimensions are asserted equal",
  dprNote: "DPR1 mac-columns-dpr1 is a separate control and is never mixed into Retina evidence",
  pairs: manifest
}, null, 2)}\n`, "utf8");

const describe = (entry) => `${entry.label} · ${entry.theme} · ${entry.viewport.width}×${entry.viewport.height} · DPR${entry.dpr} · ${entry.platform}`;
const card = (src, alt) => `<figure><img src="${src}" loading="lazy" alt="${alt}"><figcaption>${alt}</figcaption></figure>`;
const sections = manifest.map((entry) => `<section class="pair"><h2>${escapeHtml(describe(entry))}</h2><p class="meta">physical ${entry.physical.width}×${entry.physical.height}</p><div class="imgs">${
  entry.proto ? card(entry.proto, "原型 v3.40") : ""
}${card(entry.prod, entry.proto ? "生产前端" : "生产前端（原型无此状态）")}</div></section>`).join("\n");
await writeFile(path.join(outDir, "index.html"), `<!doctype html><html lang="zh"><head><meta charset="utf-8"><title>Fylura v3.40 同状态视觉对照</title><style>:root{color-scheme:light;font-family:"Segoe UI","PingFang SC",sans-serif;background:#eff3f9;color:#16202c}body{margin:0;padding:24px 28px 64px}h1{font-size:20px;margin:0 0 6px}p.lead{max-width:1100px;margin:0 0 22px;color:#58697c;font-size:13px;line-height:1.7}.pair{margin:0 0 28px}.pair h2{font-size:14px;margin:0 0 2px;font-weight:600}.pair p.meta{margin:0 0 8px;color:#64758a;font-size:11.5px}.imgs{display:grid;grid-template-columns:1fr 1fr;gap:12px}figure{margin:0;background:#fff;border-radius:10px;box-shadow:0 0 0 1px #c3cedd,0 8px 24px -18px rgba(24,48,80,.4);overflow:hidden}img{display:block;width:100%;height:auto}figcaption{padding:7px 10px;font-size:11.5px;color:#64758a;border-top:1px solid #e6ecf4}@media(max-width:900px){.imgs{grid-template-columns:1fr}}</style></head><body><h1>Fylura v3.40 · 同状态视觉对照</h1><p class="lead">原型与生产使用相同 viewport、DPR、主题与平台；每一对截图在写入前校验物理像素尺寸，不一致直接失败。原型从仓库 ZIP 解压，生产为 browser fixture，比较结构/密度/材质，不做像素差。macOS 为 MacIntel UA + DPR2 的 Chromium 模拟，不是原生 WKWebView 验收。Chromium ${escapeHtml(browser.version())}。</p>
${sections}</body></html>
`, "utf8");

// PNGs are only an intermediate for the dimension assertion + WebP encode.
for (const name of await readdir(outDir)) {
  if (name.endsWith(".png")) await unlink(path.join(outDir, name)).catch(() => undefined);
}

// PNGs are only an intermediate for the dimension assertion + WebP encode.
for (const name of await readdir(outDir)) {
  if (name.endsWith(".png")) await unlink(path.join(outDir, name)).catch(() => undefined);
}

await converter.close();
await browser.close();
// Windows can hold the screenshot handle briefly; retry after the browser is
// fully closed so no intermediate PNG is ever committed.
for (let attempt = 0; attempt < 3; attempt += 1) {
  const leftovers = (await readdir(outDir)).filter((name) => name.endsWith(".png"));
  if (!leftovers.length) break;
  for (const name of leftovers) await unlink(path.join(outDir, name)).catch(() => undefined);
  if (attempt < 2) await new Promise((resolve) => setTimeout(resolve, 100));
}
await server.close();
console.log(`[v340-parity] captured ${manifest.length} pairs`);

function escapeHtml(value) {
  return String(value).replace(/[&<>"]/gu, (character) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[character]));
}
