/**
 * Keyframe timeline evidence for the core v3.40 animations.
 * Captures start/middle/end frames and the animation ids/durations that were
 * actually running, for Owner review without shipping large videos.
 */
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { chromium } from "playwright";
import { createServer } from "vite";

const outDir = path.join(process.cwd(), "docs/design/v340-evidence/parity/motion-frames");
await mkdir(outDir, { recursive: true });
const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 5200 + Math.floor(Math.random() * 400) } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
const browser = await chromium.launch({ headless: true });
const records = [];

async function pageWith(theme = "light") {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, colorScheme: theme });
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

async function files(page) {
  await page.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
  await page.waitForTimeout(900);
  await page.getByText("Managed mock root", { exact: false }).first().click();
  await page.waitForTimeout(1200);
}

const animations = (page, selector) => page.evaluate((target) => {
  const element = document.querySelector(target);
  return element ? element.getAnimations().map((animation) => ({
    id: animation.id || animation.animationName || "",
    duration: animation.effect?.getTiming().duration ?? null,
    easing: animation.effect?.getTiming().easing ?? null,
    state: animation.playState
  })) : [];
}, selector);

async function captureScenario(id, label, selector, prepare, trigger) {
  const { page, context } = await pageWith("light");
  try {
    await prepare(page);
    await trigger(page);
    const frames = [];
    for (const [name, delay] of [["start", 20], ["mid", 90], ["end", 220]]) {
      await page.waitForTimeout(delay);
      const file = `${id}-${name}.png`;
      await page.screenshot({ path: path.join(outDir, file) });
      frames.push({ phase: name, file, animations: await animations(page, selector) });
    }
    records.push({ id, label, frames });
  } finally {
    await context.close();
  }
}

await captureScenario(
  "pane-menu", "多窗格布局菜单 240ms",
  ".wb-pane-menu-panel",
  files,
  async (page) => { await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click(); }
);

await captureScenario(
  "sidebar", "应用侧栏收展 200ms",
  ".zc-sidebar",
  async () => {},
  async (page) => { await page.locator(".zc-titlebar-toggle").click(); }
);

await captureScenario(
  "quick-preview", "Quick Preview 260ms",
  ".zc-quick-preview-card",
  async (page) => {
    await files(page);
    await page.locator("[role='option']").first().click();
  },
  async (page) => { await page.keyboard.press("Space"); }
);

await captureScenario(
  "confirm", "确认层 240ms + 3px 遮罩模糊",
  "[data-confirm-card]",
  async (page) => {
    await files(page);
    await page.waitForTimeout(200);
  },
  async (page) => { await page.locator("[data-fm-primary-action]").first().click(); }
);

await writeFile(path.join(outDir, "timeline.json"), `${JSON.stringify({ chrome: browser.version(), viewport: "1440x900", records }, null, 2)}\n`, "utf8");
await browser.close();
await server.close();
console.log(`[v340-motion-frames] captured ${records.length} timelines`);
