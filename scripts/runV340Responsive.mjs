/**
 * v3.40 responsive / DPI runner (browser simulation).
 *
 * For every display profile and logical window it checks the real DOM for
 * horizontal overflow, clipped panels, overlapping toolbar controls, unreadable
 * file names, off-screen dialogs/previews and excessive reading width. It does
 * not claim native acceptance; the installed-app matrix stays in #487.
 *
 *   node scripts/runV340Responsive.mjs
 *   node scripts/runV340Responsive.mjs --quick
 */
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const outDir = path.join(process.cwd(), "docs/design/v340-evidence/responsive");
await mkdir(outDir, { recursive: true });
const quick = process.argv.includes("--quick");

const DISPLAYS = [
  { id: "fhd-100", label: "1080P 100%", width: 1920, height: 1080, dpr: 1, platform: "Win32" },
  { id: "fhd-125", label: "1080P 125%", width: 1536, height: 864, dpr: 1.25, platform: "Win32" },
  { id: "fhd-150", label: "1080P 150%", width: 1280, height: 720, dpr: 1.5, platform: "Win32" },
  { id: "qhd-100", label: "2K 100%", width: 2560, height: 1440, dpr: 1, platform: "Win32" },
  { id: "qhd-125", label: "2K 125%", width: 2048, height: 1152, dpr: 1.25, platform: "Win32" },
  { id: "qhd-150", label: "2K 150%", width: 1707, height: 960, dpr: 1.5, platform: "Win32" },
  { id: "uhd-150", label: "4K 150%", width: 2560, height: 1440, dpr: 1.5, platform: "Win32" },
  { id: "uhd-200", label: "4K 200%", width: 1920, height: 1080, dpr: 2, platform: "Win32" },
  { id: "uhd-250", label: "4K 250%", width: 1536, height: 864, dpr: 2.5, platform: "Win32" },
  { id: "mac-air13", label: "MacBook Air 13 Retina", width: 1280, height: 832, dpr: 2, platform: "MacIntel" },
  { id: "mac-air15", label: "MacBook Air 15 Retina", width: 1440, height: 932, dpr: 2, platform: "MacIntel" },
  { id: "mac-pro14", label: "MacBook Pro 14 Retina", width: 1512, height: 982, dpr: 2, platform: "MacIntel" },
  { id: "mac-pro16", label: "MacBook Pro 16 Retina", width: 1728, height: 1117, dpr: 2, platform: "MacIntel" },
  { id: "mac-5k", label: "Studio Display 5K", width: 2560, height: 1440, dpr: 2, platform: "MacIntel" }
];

const WINDOWS = [
  { id: "980x680", width: 980, height: 680, state: "minimum" },
  { id: "1080x740", width: 1080, height: 740, state: "normal" },
  { id: "1280x720", width: 1280, height: 720, state: "normal" },
  { id: "1366x768", width: 1366, height: 768, state: "normal" },
  { id: "1440x900", width: 1440, height: 900, state: "normal" },
  { id: "1536x864", width: 1536, height: 864, state: "normal" },
  { id: "1920x1080", width: 1920, height: 1080, state: "maximized" },
  { id: "2560x1440", width: 2560, height: 1440, state: "maximized" }
];

const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 5200 + Math.floor(Math.random() * 400) } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
const browser = await chromium.launch({ headless: true });

const results = [];
const failures = [];

const MEASURE = `(() => {
  const rect = (el) => {
    if (!el) return null;
    const r = el.getBoundingClientRect();
    return { x: +r.x.toFixed(1), y: +r.y.toFixed(1), w: +r.width.toFixed(1), h: +r.height.toFixed(1), right: +r.right.toFixed(1), bottom: +r.bottom.toFixed(1) };
  };
  const visible = (el) => Boolean(el && el.getClientRects().length && getComputedStyle(el).visibility !== "hidden");
  const overflow = document.documentElement.scrollWidth - document.documentElement.clientWidth;
  const bodyOverflow = document.body.scrollWidth - document.body.clientWidth;
  const workspace = rect(document.querySelector(".zc-app-workspace"));
  const list = rect(document.querySelector(".shared-file-list-shell"));
  const nameCell = rect(document.querySelector(".shared-file-list-name"));
  const header = document.querySelector(".shared-file-list-header");
  const headerOverflow = header ? header.scrollWidth - header.clientWidth : 0;
  const toolbar = document.querySelector(".file-library-command-bar, .file-library-location-row");
  const toolbarOverflow = toolbar ? toolbar.scrollWidth - toolbar.clientWidth : 0;
  const panes = [...document.querySelectorAll(".wb-pane")].filter(visible).map(rect);
  const clippedRect = (el) => {
    let box = el.getBoundingClientRect();
    let node = el.parentElement;
    while (node && node !== document.body) {
      const style = getComputedStyle(node);
      if (style.overflow === "hidden" || style.overflow === "clip" || style.overflowX === "hidden") {
        const parent = node.getBoundingClientRect();
        const left = Math.max(box.left, parent.left);
        const top = Math.max(box.top, parent.top);
        const right = Math.min(box.right, parent.right);
        const bottom = Math.min(box.bottom, parent.bottom);
        if (right <= left || bottom <= top) return null;
        box = { left, top, right, bottom, width: right - left, height: bottom - top, x: left, y: top };
      }
      node = node.parentElement;
    }
    return { x: +box.x.toFixed(1), y: +box.y.toFixed(1), w: +box.width.toFixed(1), h: +box.height.toFixed(1), right: +box.right.toFixed(1), bottom: +box.bottom.toFixed(1) };
  };
  const buttons = [...document.querySelectorAll(".file-library-command-bar button, .file-library-location-row button, .zc-titlebar-tool")]
    .filter(visible).map((el) => ({ label: (el.getAttribute("aria-label") || el.textContent || "").trim().slice(0, 32), ...(clippedRect(el) ?? {}) }))
    .filter((item) => item.w > 0);
  const overlaps = [];
  for (let i = 0; i < buttons.length; i += 1) {
    for (let j = i + 1; j < buttons.length; j += 1) {
      const a = buttons[i]; const b = buttons[j];
      if (!a.label || !b.label) continue;
      const x = Math.min(a.right, b.right) - Math.max(a.x, b.x);
      const y = Math.min(a.bottom, b.bottom) - Math.max(a.y, b.y);
      if (x > 2 && y > 2) overlaps.push([a.label, b.label, Math.round(x), Math.round(y)]);
    }
  }
  const settingsContent = rect(document.querySelector("[data-settings-sections]"));
  const grid = document.querySelector(".shared-file-grid-row");
  const gridColumns = grid ? getComputedStyle(grid).gridTemplateColumns.split(" ").filter(Boolean).length : 0;
  const columnsView = rect(document.querySelector("[data-browse-column-view], .browse-columns"));
  return {
    viewport: { w: innerWidth, h: innerHeight, dpr: devicePixelRatio },
    overflow,
    bodyOverflow,
    workspace,
    list,
    nameCell,
    headerOverflow,
    toolbarOverflow,
    panes,
    layout: document.querySelector(".file-library-workspace")?.getAttribute("data-layout") ?? null,
    paneCount: document.querySelector("[data-pane-menu] .wb-pane-menu-trigger")?.getAttribute("data-pane-count") ?? null,
    overlaps: overlaps.slice(0, 6),
    settingsContent,
    pageContent: rect(document.querySelector(".v340-page")),
    gridColumns,
    columnsView,
    documentScroll: document.documentElement.scrollHeight - document.documentElement.clientHeight
  };
})()`;

async function open(display, windowSpec) {
  const context = await browser.newContext({
    viewport: { width: windowSpec.width, height: windowSpec.height },
    deviceScaleFactor: display.dpr,
    colorScheme: "light",
    ...(display.platform === "MacIntel"
      ? { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36" }
      : {})
  });
  const page = await context.newPage();
  await page.addInitScript(({ platform }) => {
    Object.defineProperty(navigator, "platform", { get: () => platform });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "zh");
  }, { platform: display.platform });
  await page.goto(`${baseUrl}/?browser-mock=1&u3-folder-move=1&devices-fixture=1`);
  await page.waitForTimeout(1500);
  return { page, context };
}

async function openRoot(page) {
  await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
  await page.waitForTimeout(800);
  await page.getByText("Managed mock root", { exact: false }).first().click();
  await page.waitForTimeout(900);
  if ((await page.locator("[role='option']").count()) === 0) {
    const card = page.locator(".file-library-content-slot article.browse-location-card").filter({ hasText: "Managed mock root" }).first();
    if (await card.count()) {
      await card.getByRole("button", { name: "打开位置" }).click();
      await page.waitForTimeout(1000);
    }
  }
}

const STATES = [
  { id: "files", prepare: async (page) => { await openRoot(page); await page.locator("[role='option']").first().click(); } },
  { id: "files-inspector", prepare: async (page) => { await openRoot(page); await page.locator("[role='option']").first().click(); await page.locator("[data-file-library-context-toggle]").click(); await page.waitForTimeout(350); } },
  { id: "files-grid", prepare: async (page) => { await openRoot(page); await page.locator('[data-file-library-view="grid"]').click(); await page.waitForTimeout(350); } },
  { id: "files-columns", prepare: async (page) => { await openRoot(page); await page.locator('[data-file-library-view="columns"]').click(); await page.waitForTimeout(400); } },
  {
    id: "files-quad",
    prepare: async (page) => {
      await openRoot(page);
      await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
      await page.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^4$/ }).first().click();
      await page.waitForTimeout(500);
    }
  },
  { id: "overview", prepare: async (page) => { await page.locator(".zc-sidebar").getByRole("button", { name: "概览", exact: true }).click(); await page.waitForTimeout(500); } },
  { id: "organize", prepare: async (page) => { await page.locator(".zc-sidebar").getByRole("button", { name: "整理文件", exact: true }).click(); await page.waitForTimeout(500); } },
  { id: "cleanup", prepare: async (page) => { await page.locator(".zc-sidebar").getByRole("button", { name: "空间清理", exact: true }).click(); await page.waitForTimeout(500); } },
  { id: "history", prepare: async (page) => { await page.locator(".zc-sidebar").getByRole("button", { name: "历史记录", exact: true }).click(); await page.waitForTimeout(500); } },
  { id: "settings", prepare: async (page) => { await page.locator(".zc-sidebar").getByRole("button", { name: "设置", exact: true }).click(); await page.waitForTimeout(500); } },
  { id: "search", prepare: async (page) => {
    const mac = await page.evaluate(() => navigator.platform.startsWith("Mac"));
    await page.keyboard.press(mac ? "Meta+k" : "Control+k");
    await page.waitForTimeout(400);
  } },
  {
    id: "preview",
    prepare: async (page) => {
      await openRoot(page);
      await page.locator("[role='option']").first().click();
      await page.keyboard.press("Space");
      await page.waitForTimeout(400);
    }
  }
];

const DISPLAY_STATES = STATES.filter((state) => ["files", "files-inspector", "files-grid", "files-columns", "files-quad", "settings", "search", "preview"].includes(state.id));
const WINDOW_STATES = STATES.filter((state) => ["files", "files-quad", "settings", "files-columns"].includes(state.id));
const PAGE_STATES = STATES.filter((state) => ["overview", "organize", "cleanup", "history"].includes(state.id));

const cases = [];
for (const display of DISPLAYS) {
  // Large displays keep their real logical width (up to 2560), so the matrix
  // also proves the 1180px reading cap instead of only testing a 1440 window.
  const fit = {
    id: `fit-${Math.min(display.width - 48, 2560)}x${Math.min(display.height - 48, 1440)}`,
    width: Math.min(display.width - 48, 2560),
    height: Math.min(display.height - 48, 1440),
    state: "normal"
  };
  for (const state of DISPLAY_STATES) {
    if (state.id === "files-columns" && display.platform !== "MacIntel") continue;
    cases.push({ display, windowSpec: fit, state });
  }
}
for (const display of DISPLAYS.filter((item) => ["fhd-100", "mac-air13"].includes(item.id))) {
  for (const windowSpec of WINDOWS.filter((item) => item.width <= display.width && item.height <= display.height)) {
    for (const state of WINDOW_STATES) {
      if (state.id === "files-columns" && display.platform !== "MacIntel") continue;
      cases.push({ display, windowSpec, state });
    }
  }
}
for (const display of DISPLAYS.filter((item) => ["fhd-100", "mac-pro14"].includes(item.id))) {
  for (const state of PAGE_STATES) cases.push({ display, windowSpec: WINDOWS.find((item) => item.id === "1440x900"), state });
}

for (const item of quick ? cases.slice(0, 8) : cases) {
  const { display, windowSpec, state } = item;
  console.log(`[v340-responsive] ${display.id} ${windowSpec.id} ${state.id}`);
  const { page, context } = await open(display, windowSpec);
  try {
    await state.prepare(page);
    const metrics = await page.evaluate(MEASURE);
    const pageFailures = [];
    if (metrics.overflow > 1 || metrics.bodyOverflow > 1) pageFailures.push(`horizontal overflow ${Math.max(metrics.overflow, metrics.bodyOverflow)}px`);
    if (metrics.workspace && (metrics.workspace.right > metrics.viewport.w + 1 || metrics.workspace.bottom > metrics.viewport.h + 1)) pageFailures.push("workspace outside viewport");
    if (metrics.list && metrics.list.w < 220) pageFailures.push(`file list ${metrics.list.w}px < 220px`);
    if (metrics.nameCell && metrics.nameCell.w < 90) pageFailures.push(`name cell ${metrics.nameCell.w}px < 90px`);
    if (metrics.headerOverflow > 1) pageFailures.push(`header overflow ${metrics.headerOverflow}px`);
    if (metrics.toolbarOverflow > 1) pageFailures.push(`toolbar overflow ${metrics.toolbarOverflow}px`);
    if (metrics.overlaps.length) pageFailures.push(`toolbar overlap ${JSON.stringify(metrics.overlaps[0])}`);
    if (state.id === "settings" && metrics.settingsContent && metrics.settingsContent.w > 821) pageFailures.push(`settings content ${metrics.settingsContent.w}px > 820px`);
    if (["overview", "organize", "cleanup", "history"].includes(state.id) && metrics.pageContent && metrics.pageContent.w > 1181) pageFailures.push(`page content ${metrics.pageContent.w}px > 1180px`);
    if (state.id === "files-grid" && metrics.viewport.w >= 1280 && metrics.gridColumns < 3) pageFailures.push(`grid columns ${metrics.gridColumns} < 3`);
    if (state.id === "files-quad") {
      const visiblePanes = metrics.panes.length;
      if (visiblePanes !== 4 && !(visiblePanes === 1 && metrics.paneCount === "4")) pageFailures.push(`quad panes ${visiblePanes}/${metrics.paneCount}`);
      if (visiblePanes === 4 && metrics.panes.some((pane) => pane.w < 160 || pane.h < 160)) pageFailures.push("quad pane below usable size");
    }
    if (state.id === "search") {
      const dialog = await page.locator("[data-command-modal]").evaluate((el) => {
        const r = el.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom, left: r.left, right: r.right };
      });
      if (dialog.left < 0 || dialog.right > metrics.viewport.w + 1 || dialog.top < 0 || dialog.bottom > metrics.viewport.h + 1) pageFailures.push("search outside viewport");
    }
    if (state.id === "preview") {
      const card = await page.locator(".zc-quick-preview-card").evaluate((el) => {
        const r = el.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom, left: r.left, right: r.right };
      });
      if (card.left < 0 || card.right > metrics.viewport.w + 1 || card.top < 0 || card.bottom > metrics.viewport.h + 1) pageFailures.push("preview outside viewport");
    }
    const status = pageFailures.length ? "FAIL" : "PASS";
    if (pageFailures.length) failures.push({ display: display.id, window: windowSpec.id, state: state.id, failures: pageFailures });
    results.push({
      display: display.id,
      displayLabel: display.label,
      platform: display.platform,
      dpr: display.dpr,
      window: windowSpec.id,
      windowState: windowSpec.state,
      state: state.id,
      status,
      failures: pageFailures,
      metrics
    });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    failures.push({ display: display.id, window: windowSpec.id, state: state.id, failures: [message] });
    results.push({ display: display.id, window: windowSpec.id, state: state.id, status: "FAIL", failures: [message] });
  } finally {
    await context.close();
  }
}

// Representative screenshots only: minimum, 1080P@150, 4K@250, Air 13, 5K.
const SHOTS = [
  { display: "fhd-150", window: "1280x720", state: "files" },
  { display: "uhd-250", window: "1536x864", state: "files-quad" },
  { display: "mac-air13", window: "980x680", state: "files-columns" },
  { display: "mac-5k", window: "2560x1440", state: "settings" }
];
for (const shot of SHOTS) {
  const display = DISPLAYS.find((item) => item.id === shot.display);
  const windowSpec = WINDOWS.find((item) => item.id === shot.window);
  const state = STATES.find((item) => item.id === shot.state);
  if (!display || !windowSpec || !state) continue;
  const { page, context } = await open(display, windowSpec);
  try {
    await state.prepare(page);
    await page.waitForTimeout(300);
    await page.screenshot({ path: path.join(outDir, `${shot.display}-${shot.window}-${shot.state}.png`) });
  } finally {
    await context.close();
  }
}

const summary = {
  chrome: browser.version(),
  generatedAt: new Date().toISOString(),
  cases: results.length,
  pass: results.filter((item) => item.status === "PASS").length,
  fail: results.filter((item) => item.status === "FAIL").length,
  failures,
  results
};
await writeFile(path.join(outDir, "responsive.json"), `${JSON.stringify(summary, null, 2)}\n`, "utf8");
await browser.close();
await server.close();
console.log(`[v340-responsive] ${summary.pass}/${summary.cases} PASS · ${summary.fail} FAIL`);
if (failures.length) {
  for (const failure of failures.slice(0, 20)) console.error(`[${failure.display}/${failure.window}/${failure.state}] ${failure.failures.join("; ")}`);
  process.exit(1);
}
