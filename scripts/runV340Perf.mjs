/**
 * v3.40 PR 3 performance runner — Playwright + CDP.
 *
 * Seven real production interactions at 1× and 4× CPU. For every scenario it
 * records whether the action actually fired, the animation id/duration, the
 * settled UI state, rAF frame intervals, dropped frames, Layout/Recalc counts,
 * layout/style time and Long Animation Frames.
 *
 * Usage:
 *   node scripts/runV340Perf.mjs            # writes perf-1x.json + perf-4x.json
 *   node scripts/runV340Perf.mjs --cpu 4    # one rate only
 */
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const outDir = path.join(process.cwd(), "docs/design/v340-evidence/parity/perf");
await mkdir(outDir, { recursive: true });
const requestedCpu = process.argv.includes("--cpu")
  ? Number(process.argv[process.argv.indexOf("--cpu") + 1])
  : null;
const rates = requestedCpu ? [requestedCpu] : [1, 4];

const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 0 } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
assert.ok(baseUrl, "vite server did not start");
const browser = await chromium.launch({ headless: true });
const chromeVersion = browser.version();

const INSTRUMENT = `(() => {
  window.__perf = { frames: [], loaf: [] };
  let last = 0;
  const loop = (t) => { if (last) window.__perf.frames.push(+(t - last).toFixed(2)); last = t; requestAnimationFrame(loop); };
  requestAnimationFrame(loop);
  try {
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        window.__perf.loaf.push({
          duration: Math.round(entry.duration),
          styleAndLayout: entry.styleAndLayoutStart ? Math.round(entry.styleAndLayoutStart - entry.startTime) : 0,
          blocking: Math.round(entry.blockingDuration || 0)
        });
      }
    }).observe({ type: "long-animation-frame", buffered: false });
  } catch (_error) {}
  return "armed";
})()`;

const STATS = `(() => {
  const frames = window.__perf.frames.slice(2, Math.max(3, window.__perf.frames.length - 2));
  const sorted = [...frames].sort((a, b) => a - b);
  const pct = (q) => sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))] : 0;
  const budget = 1000 / 60;
  return {
    frames: frames.length,
    p50: +pct(0.5).toFixed(1),
    p95: +pct(0.95).toFixed(1),
    max: frames.length ? Math.max(...frames) : 0,
    dropped: frames.filter((value) => value > budget * 1.6).length,
    loaf: window.__perf.loaf
  };
})()`;

async function openPage(cdpRate) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, colorScheme: "light" });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send("Performance.enable");
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: cdpRate });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "Win32" });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "zh");
  });
  await page.goto(`${baseUrl}/?browser-mock=1`);
  await page.waitForTimeout(1600);
  await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
  await page.waitForTimeout(1200);
  await page.getByText("Managed mock root", { exact: false }).first().click();
  await page.waitForTimeout(1400);
  return { context, page, cdp };
}

async function metrics(cdp) {
  const { metrics: list } = await cdp.send("Performance.getMetrics");
  const map = {};
  for (const entry of list) map[entry.name] = entry.value;
  return map;
}

const animationOf = (page, selector) => page.evaluate((target) => {
  const elements = [...document.querySelectorAll(target)];
  return elements.flatMap((element) => element.getAnimations().map((animation) => ({
    id: animation.id || animation.animationName || "",
    duration: animation.effect?.getTiming().duration ?? null,
    state: animation.playState
  }))).filter((item) => item.id);
}, selector);

const transitionOf = (page, selector) => page.evaluate((target) => {
  const element = document.querySelector(target);
  if (!element) return null;
  const cs = getComputedStyle(element);
  return { property: cs.transitionProperty, duration: cs.transitionDuration, easing: cs.transitionTimingFunction };
}, selector);

const SCENARIOS = [
  {
    id: "page",
    name: "页面切换",
    setup: async () => {},
    run: async (page) => {
      const overview = page.locator(".zc-sidebar").getByRole("button", { name: "概览", exact: true });
      const organize = page.locator(".zc-sidebar").getByRole("button", { name: "整理文件", exact: true });
      await overview.click();
      await page.waitForTimeout(280);
      await organize.click();
      await page.waitForTimeout(280);
      let animations = [];
      for (let index = 0; index < 4; index += 1) {
        await overview.click();
        await page.waitForTimeout(40);
        animations = animations.concat((await animationOf(page, "main > div.min-h-0")) ?? []);
        await page.waitForTimeout(240);
        await organize.click();
        await page.waitForTimeout(240);
      }
      return {
        verified: (await page.locator("#app-shell-content").innerText()).includes("整理"),
        animation: animations.find((animation) => animation.id === "zc-page-in") ?? null,
        final: await page.evaluate(() => {
          const stage = document.querySelector("main > div.min-h-0");
          return stage ? { opacity: getComputedStyle(stage).opacity, transform: getComputedStyle(stage).transform } : null;
        })
      };
    }
  },
  {
    id: "rail",
    name: "侧栏收展",
    setup: async () => {},
    run: async (page) => {
      const toggle = page.locator(".zc-titlebar-toggle");
      const widths = [];
      // Start expanded: expanded → rail → hidden → expanded, twice.
      for (let index = 0; index < 6; index += 1) {
        await toggle.click();
        await page.waitForTimeout(70);
        widths.push(await page.evaluate(() => document.querySelector(".zc-sidebar")?.getBoundingClientRect().width ?? -1));
        await page.waitForTimeout(220);
      }
      const finalWidth = await page.evaluate(() => document.querySelector(".zc-sidebar")?.getBoundingClientRect().width ?? -1);
      return {
        verified: widths.some((width) => width > 0 && width < 216) && Math.round(finalWidth) === 216,
        animation: await transitionOf(page, ".zc-sidebar"),
        final: { widths, finalWidth }
      };
    }
  },
  {
    id: "menu",
    name: "菜单弹出",
    setup: async () => {},
    run: async (page) => {
      const trigger = page.locator("[data-pane-menu] .wb-pane-menu-trigger");
      let animation = null;
      let closing = null;
      for (let index = 0; index < 4; index += 1) {
        await trigger.click();
        await page.waitForTimeout(30);
        animation = (await animationOf(page, ".wb-pane-menu-panel")).find((item) => item.id === "zc-menu-in") ?? animation;
        await page.keyboard.press("Escape");
        await page.waitForTimeout(30);
        closing = (await animationOf(page, ".wb-pane-menu-panel")).find((item) => item.id === "zc-menu-out") ?? closing;
        await page.waitForTimeout(140);
      }
      return {
        verified: animation?.duration === 240 && closing?.duration === 130 && (await page.locator(".wb-pane-menu-panel").count()) === 0,
        animation,
        closing,
        final: { menuCount: await page.locator(".wb-pane-menu-panel").count() }
      };
    }
  },
  {
    id: "sheet",
    name: "确认层",
    setup: async () => {},
    run: async (page) => {
      const trigger = page.locator("[data-fm-primary-action]").first();
      let animation = null;
      let ghost = false;
      for (let index = 0; index < 3; index += 1) {
        await trigger.click();
        await page.waitForTimeout(40);
        animation = await page.evaluate(() => {
          const card = document.querySelector("[data-confirm-card]");
          if (!card) return null;
          const cs = getComputedStyle(card);
          return { id: cs.animationName, duration: cs.animationDuration };
        });
        await page.keyboard.press("Escape");
        await page.waitForTimeout(25);
        ghost = ghost || (await page.locator("[data-zc-exit-ghost]").count()) === 1;
        await page.waitForTimeout(150);
      }
      return {
        verified: animation?.id === "zc-dialog-card-in" && animation?.duration === "0.24s" && ghost,
        animation,
        final: { realDialog: await page.locator("[data-confirm-card]").count(), ghost: await page.locator("[data-zc-exit-ghost]").count() }
      };
    }
  },
  {
    id: "tree",
    name: "目录树折叠",
    setup: async () => {},
    run: async (page) => {
      const toggles = page.locator("[role='treeitem'] button[aria-label='展开目录'], [role='treeitem'] button[aria-label='折叠目录']");
      const count = await toggles.count();
      assert.ok(count > 0, "tree expander not found");
      const first = toggles.first();
      const heights = [];
      const beforeHeight = await page.evaluate(() => Math.round(document.querySelector(".file-library-tree-branch")?.getBoundingClientRect().height ?? 0));
      for (let index = 0; index < 3; index += 1) {
        await first.click();
        await page.waitForTimeout(70);
        heights.push(await page.evaluate(() => {
          const branches = [...document.querySelectorAll(".file-library-tree-branch")];
          return branches.map((branch) => ({ open: branch.getAttribute("data-open"), height: Math.round(branch.getBoundingClientRect().height) }));
        }));
        await page.waitForTimeout(230);
        await first.click();
        await page.waitForTimeout(260);
      }
      const finalHeight = await page.evaluate(() => Math.round(document.querySelector(".file-library-tree-branch")?.getBoundingClientRect().height ?? 0));
      const changed = heights.some((snapshot) => snapshot.some((branch) => branch.height !== beforeHeight)) || finalHeight !== beforeHeight;
      return {
        verified: changed,
        animation: await transitionOf(page, ".file-library-tree-branch"),
        final: { beforeHeight, finalHeight, mid: heights[0] }
      };
    }
  },
  {
    id: "view",
    name: "列表↔网格",
    setup: async () => {},
    run: async (page) => {
      const grid = page.locator('[data-file-library-view="grid"]');
      const list = page.locator('[data-file-library-view="list"]');
      await grid.click();
      await page.waitForTimeout(300);
      await list.click();
      await page.waitForTimeout(300);
      let animation = null;
      for (let index = 0; index < 4; index += 1) {
        await grid.click();
        await page.waitForTimeout(30);
        animation = (await animationOf(page, ".file-library-content-slot")).find((item) => item.id === "zc-view-swap") ?? animation;
        await page.waitForTimeout(270);
        await list.click();
        await page.waitForTimeout(270);
      }
      return {
        verified: animation?.duration === 240 && (await page.locator(".shared-file-list").count()) === 1,
        animation,
        final: { list: await page.locator(".shared-file-list").count(), grid: await page.locator(".shared-file-grid").count() }
      };
    }
  },
  {
    id: "layout",
    name: "窗格布局切换",
    setup: async () => {},
    run: async (page) => {
      const trigger = page.locator("[data-pane-menu] .wb-pane-menu-trigger");
      let animation = null;
      const counts = [];
      for (const count of [2, 4, 3, 1]) {
        await trigger.click();
        await page.waitForTimeout(180);
        await page.locator("[data-pane-menu] .wb-seg button").filter({ hasText: new RegExp(`^${count}$`) }).first().click();
        await page.waitForTimeout(40);
        animation = (await animationOf(page, ".wb-pane")).find((item) => item.id === "zc-pane-in") ?? animation;
        await page.waitForTimeout(300);
        counts.push(await page.evaluate(() => document.querySelector("[data-pane-count]")?.getAttribute("data-pane-count")));
      }
      return {
        verified: animation?.duration === 260 && counts.join(",") === "2,4,3,1",
        animation,
        final: { counts }
      };
    }
  }
];

const report = { chrome: chromeVersion, viewport: "1440x900", dpr: 1, platform: "Win32 mock", fixture: "browser-mock=1 (managed mock root)", runs: [] };

for (const rate of rates) {
  const run = { cpu: rate, scenarios: [] };
  for (const scenario of SCENARIOS) {
    const { context, page, cdp } = await openPage(rate);
    try {
      await scenario.setup(page);
      await page.evaluate(INSTRUMENT);
      const before = await metrics(cdp);
      const started = Date.now();
      const outcome = await scenario.run(page);
      const elapsed = Date.now() - started;
      const after = await metrics(cdp);
      const stats = await page.evaluate(STATS);
      const loaf = stats.loaf ?? [];
      const result = {
        id: scenario.id,
        name: scenario.name,
        durationMs: elapsed,
        verified: Boolean(outcome.verified),
        animation: outcome.animation ?? null,
        closing: outcome.closing ?? null,
        final: outcome.final ?? null,
        frames: stats.frames,
        p50: stats.p50,
        p95: stats.p95,
        max: stats.max,
        dropped: stats.dropped,
        layout: Math.round((after.LayoutCount || 0) - (before.LayoutCount || 0)),
        recalc: Math.round((after.RecalcStyleCount || 0) - (before.RecalcStyleCount || 0)),
        layoutMs: Math.round(((after.LayoutDuration || 0) - (before.LayoutDuration || 0)) * 1000),
        styleMs: Math.round(((after.RecalcStyleDuration || 0) - (before.RecalcStyleDuration || 0)) * 1000),
        scriptMs: Math.round(((after.ScriptDuration || 0) - (before.ScriptDuration || 0)) * 1000),
        loafCount: loaf.length,
        loafMax: loaf.length ? Math.max(...loaf.map((entry) => entry.duration)) : 0,
        loafStyleMs: loaf.length ? Math.round(loaf.reduce((sum, entry) => sum + entry.styleAndLayout, 0) / loaf.length) : 0
      };
      run.scenarios.push(result);
      const flag = result.verified ? "OK" : "INVALID";
      if (!result.verified) console.error(`[${rate}x] invalid outcome`, JSON.stringify(result.final), JSON.stringify(result.animation), JSON.stringify(result.closing));
      console.log(`[${rate}x] ${flag.padEnd(7)} ${scenario.name.padEnd(8)} frames ${String(result.frames).padStart(3)} · p50 ${String(result.p50).padStart(5)} p95 ${String(result.p95).padStart(6)} · dropped ${String(result.dropped).padStart(2)} · layout ${String(result.layout).padStart(4)}/${String(result.layoutMs).padStart(4)}ms · style ${String(result.styleMs).padStart(4)}ms · LoAF ${result.loafCount} (max ${result.loafMax}ms)`);
    } finally {
      await context.close();
    }
  }
  report.runs.push(run);
  await writeFile(path.join(outDir, `perf-${rate}x.json`), `${JSON.stringify({ ...report, cpu: rate, scenarios: run.scenarios }, null, 2)}\n`, "utf8");
}

await browser.close();
await server.close();
const invalid = report.runs.flatMap((run) => run.scenarios.filter((scenario) => !scenario.verified));
if (invalid.length) {
  console.error(`[v340-perf] INVALID scenarios: ${invalid.map((scenario) => `${scenario.name}@${scenario.cpu ?? ""}`).join(", ")}`);
  process.exit(1);
}
console.log(`[v340-perf] PASS 7 scenarios × ${rates.join("/")}x → ${path.relative(process.cwd(), outDir)}`);
