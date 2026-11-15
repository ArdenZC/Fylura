/**
 * v3.40 long-frame locator — 4× CPU paired runs (5 per scenario).
 *
 * Captures the real Chrome CPU profile and Long Animation Frame attribution for
 * the two remaining long-frame paths (four-pane layout, sidebar resize). The
 * report states Script vs Layout/Style for the longest task instead of guessing
 * from CSS. Usage:
 *   node scripts/profileV340LongFrames.mjs
 */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer, preview } from "vite";

const outDir = path.join(process.cwd(), "docs/design/v340-evidence/parity/perf");
await mkdir(outDir, { recursive: true });
const usePreview = process.argv.includes("--preview");
const withProfiler = process.argv.includes("--profile");
const port = 5200 + Math.floor(Math.random() * 400);
const server = usePreview
  ? await preview({ root: process.cwd(), preview: { host: "127.0.0.1", port, strictPort: true } })
  : await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port } });
if (!usePreview) await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("vite server did not start");
const browser = await chromium.launch({ headless: true });

const INSTRUMENT = `(() => {
  window.__perf = { frames: [], loaf: [] };
  let last = 0;
  const loop = (t) => {
    if (last) window.__perf.frames.push({ dt: +(t - last).toFixed(2), t, active: document.getAnimations().length > 0 });
    last = t;
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
  new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) {
      window.__perf.loaf.push({
        duration: Math.round(entry.duration),
        blocking: Math.round(entry.blockingDuration || 0),
        styleAndLayout: entry.styleAndLayoutStart ? Math.round(entry.styleAndLayoutStart - entry.startTime) : 0,
        renderStart: entry.renderStart ? Math.round(entry.renderStart - entry.startTime) : 0,
        scripts: (entry.scripts || []).map((script) => ({
          name: script.sourceFunctionName || script.invoker || "(anonymous)",
          invoker: script.invoker || "",
          url: script.sourceURL || "",
          duration: Math.round(script.duration),
          forcedStyleAndLayout: Math.round(script.forcedStyleAndLayoutDuration || 0)
        })).sort((a, b) => b.duration - a.duration).slice(0, 8)
      });
    }
  }).observe({ type: "long-animation-frame", buffered: false });
  return "armed";
})()`;

const STATS = `(() => {
  const all = window.__perf.frames.slice(2, Math.max(3, window.__perf.frames.length - 2));
  const active = all.filter((frame) => frame.active);
  const sample = (active.length >= 8 ? active : all).map((frame) => frame.dt).sort((a, b) => a - b);
  const pct = (q) => sample.length ? sample[Math.min(sample.length - 1, Math.floor(sample.length * q))] : 0;
  const budget = 1000 / 60;
  return {
    frames: sample.length,
    idleFrames: all.length - active.length,
    p50: +pct(0.5).toFixed(1),
    p95: +pct(0.95).toFixed(1),
    max: sample.length ? Math.max(...sample) : 0,
    dropped: sample.filter((value) => value > budget * 1.6).length,
    loaf: window.__perf.loaf
  };
})()`;

async function openPage() {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1, colorScheme: "light" });
  const page = await context.newPage();
  const cdp = await context.newCDPSession(page);
  await cdp.send("Performance.enable");
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  if (withProfiler) {
    await cdp.send("Profiler.enable");
    await cdp.send("Profiler.setSamplingInterval", { interval: 200 });
  }
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "Win32" });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "zh");
  });
  await page.goto(`${baseUrl}/?browser-mock=1`);
  await page.waitForTimeout(1500);
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

function topFunctions(profile) {
  const byId = new Map(profile.nodes.map((node) => [node.id, node]));
  const self = new Map();
  let sampled = 0;
  profile.samples.forEach((id, index) => {
    const delta = profile.timeDeltas[index] ?? 0;
    sampled += delta;
    const node = byId.get(id);
    if (!node) return;
    const key = `${node.callFrame.functionName || "(anonymous)"} @ ${node.callFrame.url || "native"}:${node.callFrame.lineNumber + 1}`;
    self.set(key, (self.get(key) ?? 0) + delta);
  });
  return {
    sampledMs: Math.round(sampled / 1000),
    top: [...self.entries()]
      .sort((left, right) => right[1] - left[1])
      .slice(0, 18)
      .map(([key, micros]) => ({ frame: key, selfMs: +(micros / 1000).toFixed(1) }))
  };
}

const SCENARIOS = {
  layout: async (page) => {
    await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
    await page.waitForTimeout(120);
    await page.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^4$/ }).first().click();
    await page.waitForTimeout(900);
    return (await page.locator("[data-pane-id]").count()) === 4;
  },
  rail: async (page) => {
    const toggle = page.locator(".zc-titlebar-toggle");
    for (let index = 0; index < 3; index += 1) {
      await toggle.click();
      await page.waitForTimeout(320);
    }
    const width = await page.evaluate(() => Math.round(document.querySelector(".zc-sidebar")?.getBoundingClientRect().width ?? -1));
    return width === 216;
  }
};

const report = { chrome: browser.version(), mode: usePreview ? "production-build" : "vite-dev", profiler: withProfiler, cpu: 4, viewport: "1440x900", dpr: 1, runs: [] };
for (const [id, action] of Object.entries(SCENARIOS)) {
  const samples = [];
  for (let iteration = 1; iteration <= 5; iteration += 1) {
    const { context, page, cdp } = await openPage();
    try {
      await page.evaluate(INSTRUMENT);
      const before = await metrics(cdp);
      if (withProfiler) await cdp.send("Profiler.start");
      const started = Date.now();
      const verified = await action(page);
      const elapsed = Date.now() - started;
      const profile = withProfiler
        ? topFunctions((await cdp.send("Profiler.stop")).profile)
        : { sampledMs: 0, top: [], note: "run with --profile for CPU attribution" };
      const after = await metrics(cdp);
      const stats = await page.evaluate(STATS);
      const loaf = [...(stats.loaf ?? [])].sort((left, right) => right.duration - left.duration);
      samples.push({
        iteration,
        durationMs: elapsed,
        verified,
        frames: stats.frames,
        idleFrames: stats.idleFrames,
        p50: stats.p50,
        p95: stats.p95,
        max: stats.max,
        dropped: stats.dropped,
        layout: Math.round((after.LayoutCount || 0) - (before.LayoutCount || 0)),
        layoutMs: Math.round(((after.LayoutDuration || 0) - (before.LayoutDuration || 0)) * 1000),
        styleMs: Math.round(((after.RecalcStyleDuration || 0) - (before.RecalcStyleDuration || 0)) * 1000),
        scriptMs: Math.round(((after.ScriptDuration || 0) - (before.ScriptDuration || 0)) * 1000),
        taskMs: Math.round(((after.TaskDuration || 0) - (before.TaskDuration || 0)) * 1000),
        loafMax: loaf[0]?.duration ?? 0,
        longestLoaf: loaf[0] ?? null,
        otherLoaf: loaf.slice(1, 5),
        profile
      });
    } finally {
      await context.close();
    }
  }
  const pct = (values, q) => {
    const sorted = [...values].sort((a, b) => a - b);
    return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))];
  };
  report.runs.push({
    id,
    verified: samples.every((sample) => sample.verified),
    samples,
    summary: {
      p95Median: pct(samples.map((sample) => sample.p95), 0.5),
      p95Worst: Math.max(...samples.map((sample) => sample.p95)),
      loafMaxMedian: pct(samples.map((sample) => sample.loafMax), 0.5),
      loafMaxWorst: Math.max(...samples.map((sample) => sample.loafMax)),
      layoutMsAverage: Math.round(samples.reduce((sum, sample) => sum + sample.layoutMs, 0) / samples.length),
      styleMsAverage: Math.round(samples.reduce((sum, sample) => sum + sample.styleMs, 0) / samples.length),
      scriptMsAverage: Math.round(samples.reduce((sum, sample) => sum + sample.scriptMs, 0) / samples.length)
    }
  });
  console.log(`[long-frame] ${id} verified=${report.runs.at(-1).verified} p95 median ${report.runs.at(-1).summary.p95Median} · LoAF max median ${report.runs.at(-1).summary.loafMaxMedian}ms (worst ${report.runs.at(-1).summary.loafMaxWorst}ms)`);
  const longest = samples.map((sample) => sample.longestLoaf).filter(Boolean).sort((a, b) => b.duration - a.duration)[0];
  if (longest) {
    const totalScript = longest.scripts.reduce((sum, script) => sum + script.duration, 0);
    console.log(`  longest ${longest.duration}ms · script ${totalScript}ms · layout/style ${longest.styleAndLayout}ms · render ${longest.renderStart}ms`);
    console.log(`  top script ${longest.scripts.slice(0, 3).map((script) => `${script.name} ${script.duration}ms`).join(" | ") || "(none)"}`);
  }
  const profile = samples[0].profile;
  console.log(`  CPU profile total ${profile.sampledMs}ms · top ${profile.top.slice(0, 3).map((item) => `${item.frame.split(" @ ")[0]} ${item.selfMs}ms`).join(" | ")}`);
}

await writeFile(path.join(outDir, `long-frames-4x${withProfiler ? "-profile" : ""}.json`), `${JSON.stringify(report, null, 2)}\n`, "utf8");
await browser.close();
await server.close();
const invalid = report.runs.filter((run) => !run.verified);
if (invalid.length) {
  console.error(`[long-frame] INVALID: ${invalid.map((run) => run.id).join(", ")}`);
  process.exit(1);
}
console.log(`[long-frame] PASS 2 scenarios × 5 runs → long-frames-4x${withProfiler ? "-profile" : ""}.json`);
