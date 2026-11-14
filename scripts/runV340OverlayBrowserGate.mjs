/**
 * v3.40 overlay motion gate (PR 2).
 * Real Chromium interactions for settings select, pane menu, context menu and
 * the command modal. No CSS-string assertions.
 */
import assert from "node:assert/strict";
import { chromium } from "playwright";
import { createServer } from "vite";

const server = await createServer({ root: process.cwd(), server: { host: "127.0.0.1", port: 0 } });
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
const browser = await chromium.launch({ headless: true });
const failures = [];
let step = "start";

async function open(options = {}) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, ...options });
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

async function animations(page, selector, id) {
  return page.evaluate(([targetSelector, animationId]) => {
    const element = document.querySelector(targetSelector);
    return element?.getAnimations()
      .filter((animation) => !animationId || animation.id === animationId)
      .map((animation) => ({ id: animation.id, duration: animation.effect?.getTiming().duration, state: animation.playState })) ?? [];
  }, [selector, id]);
}

try {
  const { page, context } = await open();
  const sidebar = page.locator(".zc-sidebar");

  // Settings select: five mounts, each animated, Escape exit observable.
  step = "settings-select";
  await sidebar.getByRole("button", { name: "设置", exact: true }).click();
  await page.waitForTimeout(900);
  for (let index = 0; index < 5; index += 1) {
    await page.locator("#settings-close-behavior-trigger").click();
    await page.waitForTimeout(25);
    const enter = await animations(page, "[data-settings-select-menu]", "zc-menu-in");
    assert.ok(enter.some((animation) => animation.duration === 240), `select enter ${index}: ${JSON.stringify(enter)}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(25);
    assert.equal(await page.locator("[data-settings-select-menu]").getAttribute("data-closing"), "true");
    const exit = await animations(page, "[data-settings-select-menu]", "zc-menu-out");
    assert.ok(exit.some((animation) => animation.duration === 130), `select exit ${index}: ${JSON.stringify(exit)}`);
    await page.waitForTimeout(140);
    assert.equal(await page.locator("[data-settings-select-menu]").count(), 0);
  }

  // Re-opening during the exit must cancel the pending close instead of
  // stacking a second entrance on a half-faded menu.
  step = "settings-select-reopen";
  await page.locator("#settings-close-behavior-trigger").click();
  await page.waitForTimeout(25);
  await page.keyboard.press("Escape");
  await page.waitForTimeout(40);
  await page.locator("#settings-close-behavior-trigger").click();
  await page.waitForTimeout(25);
  assert.equal(await page.locator("[data-settings-select-menu]").count(), 1);
  assert.equal(await page.locator("[data-settings-select-menu]").getAttribute("data-closing"), null);
  const reopen = await animations(page, "[data-settings-select-menu]", "zc-menu-in");
  assert.ok(reopen.some((animation) => animation.duration === 240), JSON.stringify(reopen));
  await page.keyboard.press("Escape");
  await page.waitForTimeout(150);

  // Pane menu: five mounts, Escape exit, action closes in the same frame.
  step = "pane-menu";
  await sidebar.getByRole("button", { name: "文件", exact: true }).click();
  await page.waitForTimeout(1000);
  for (let index = 0; index < 5; index += 1) {
    await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
    await page.waitForTimeout(25);
    const enter = await animations(page, ".wb-pane-menu-panel", "zc-menu-in");
    assert.ok(enter.some((animation) => animation.duration === 240), `pane enter ${index}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(25);
    const exit = await animations(page, ".wb-pane-menu-panel", "zc-menu-out");
    assert.ok(exit.some((animation) => animation.duration === 130), `pane exit ${index}`);
    await page.waitForTimeout(140);
  }

  // Browse context menu: repeated open, Escape exit, menu removed after exit.
  step = "context-menu";
  await page.getByText("Managed mock root", { exact: false }).first().click();
  await page.waitForTimeout(1100);
  for (let index = 0; index < 5; index += 1) {
    await page.locator("[role='option']").first().click({ button: "right" });
    await page.waitForTimeout(25);
    const enter = await animations(page, "[data-context-menu]", "zc-menu-in");
    assert.ok(enter.some((animation) => animation.duration === 240), `context enter ${index}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(25);
    const exit = await animations(page, "[data-context-menu]", "zc-menu-out");
    assert.ok(exit.some((animation) => animation.duration === 130), `context exit ${index}`);
    await page.waitForTimeout(140);
    assert.equal(await page.locator("[data-context-menu]").count(), 0);
  }

  // Command modal: 240ms card, 130ms exit, Escape owned by the dialog.
  step = "command-modal";
  await page.keyboard.press("Control+k");
  await page.waitForTimeout(30);
  const modalEnter = await animations(page, "[data-command-modal]");
  assert.ok(modalEnter.some((animation) => animation.duration === 240), JSON.stringify(modalEnter));
  await page.keyboard.press("Escape");
  await page.waitForTimeout(30);
  assert.equal(await page.locator("[data-command-modal]").count(), 1, "modal must stay mounted during exit");
  const modalExit = await animations(page, "[data-command-modal]", "zc-modal-out");
  assert.ok(modalExit.some((animation) => animation.duration === 130));
  await page.waitForFunction(() => !document.querySelector("[data-command-modal]"), null, { timeout: 1500 });

  // Toolbar More panel: five mounts, Escape runs the 130ms exit.
  step = "toolbar-panel";
  const moreTrigger = page.locator(".fm-more-trigger").first();
  for (let index = 0; index < 5; index += 1) {
    await moreTrigger.click();
    await page.waitForTimeout(25);
    const enter = await animations(page, "[data-fm-panel]", "zc-menu-in");
    assert.ok(enter.some((animation) => animation.duration === 240), `toolbar enter ${index}: ${JSON.stringify(enter)}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(25);
    assert.equal(await page.locator("[data-fm-panel]").getAttribute("data-closing"), "true");
    const exit = await animations(page, "[data-fm-panel]", "zc-menu-out");
    assert.ok(exit.some((animation) => animation.duration === 130), `toolbar exit ${index}`);
    await page.waitForTimeout(140);
    assert.equal(await page.locator("[data-fm-panel]").count(), 0);
  }

  // New-folder dialog: scrim + card enter, Escape closes without delaying.
  step = "confirm-dialog";
  await page.locator("[data-fm-primary-action]").first().click();
  await page.waitForTimeout(30);
  const dialog = await page.evaluate(() => {
    const card = document.querySelector("[data-confirm-card]");
    const backdrop = document.querySelector("[data-confirm-backdrop]");
    return {
      card: card ? getComputedStyle(card).animationName : null,
      cardDuration: card ? getComputedStyle(card).animationDuration : null,
      backdrop: backdrop ? getComputedStyle(backdrop).animationName : null,
      focused: document.activeElement?.tagName
    };
  });
  assert.equal(dialog.card, "zc-dialog-card-in");
  assert.equal(dialog.cardDuration, "0.24s");
  assert.ok(dialog.backdrop?.includes("zc-scrim-in"), JSON.stringify(dialog));
  await page.keyboard.press("Escape");
  await page.waitForTimeout(40);
  assert.equal(await page.locator("[data-confirm-card]").count(), 0);

  // Quick Preview: every mount replays the 260ms card entrance and closes
  // synchronously (resource release is never delayed by motion).
  step = "quick-preview";
  const fileRow = page.locator("[role='option']").first();
  await fileRow.click();
  for (let index = 0; index < 5; index += 1) {
    await page.keyboard.press("Space");
    await page.waitForTimeout(30);
    const preview = await page.evaluate(() => {
      const card = document.querySelector(".zc-quick-preview-card");
      const animation = card?.getAnimations().find((entry) => entry.animationName === "zc-preview-in");
      return { present: Boolean(card), duration: animation?.effect?.getTiming().duration ?? null };
    });
    assert.equal(preview.present, true, `preview mount ${index}`);
    assert.equal(preview.duration, 260, `preview animation ${index}: ${JSON.stringify(preview)}`);
    await page.keyboard.press("Escape");
    await page.waitForTimeout(80);
    assert.equal(await page.locator("[data-preview-shell]").count(), 0, `preview closed ${index}`);
  }

  // A committed pane-layout action closes immediately: the layout change must
  // not wait for an exit animation.
  step = "pane-menu-action";
  await page.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
  await page.waitForTimeout(80);
  await page.locator("[data-pane-menu] .wb-seg button").filter({ hasText: /^2$/ }).first().click();
  await page.waitForTimeout(25);
  assert.equal(await page.locator(".wb-pane-menu-panel").count(), 0);
  assert.equal(await page.evaluate(() => document.querySelector("[data-pane-count]")?.getAttribute("data-pane-count")), "2");

  // Pin / Unpin keeps a single preview identity and does not stack shells.
  step = "quick-preview-pin";
  await fileRow.click();
  await page.keyboard.press("Space");
  await page.waitForTimeout(120);
  const pinButton = page.locator("[data-preview-shell] button[aria-pressed]").first();
  await pinButton.click();
  await page.waitForTimeout(200);
  const pinned = await page.evaluate(() => ({
    floating: Boolean(document.querySelector("[data-preview-host='zen-floating']")),
    shells: document.querySelectorAll("[data-preview-shell]").length,
    pressed: document.querySelector("[data-preview-shell] button[aria-pressed]")?.getAttribute("aria-pressed")
  }));
  assert.equal(pinned.floating, false);
  assert.equal(pinned.shells, 1);
  assert.equal(pinned.pressed, "true");
  await page.locator("[data-preview-shell] button[aria-pressed]").first().click();
  await page.waitForTimeout(220);
  const unpinned = await page.evaluate(() => ({
    floating: Boolean(document.querySelector("[data-preview-host='zen-floating']")),
    shells: document.querySelectorAll("[data-preview-shell]").length,
    pressed: document.querySelector("[data-preview-shell] button[aria-pressed]")?.getAttribute("aria-pressed")
  }));
  assert.equal(unpinned.shells, 1);
  assert.equal(unpinned.pressed, "false");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(80);

  await context.close();

  // Reduced motion: no entry animation, close lands immediately.
  step = "reduced-motion";
  const reduced = await open({ reducedMotion: "reduce" });
  await reduced.page.locator(".zc-sidebar").getByRole("button", { name: "设置", exact: true }).click();
  await reduced.page.waitForTimeout(800);
  await reduced.page.locator("#settings-close-behavior-trigger").click();
  await reduced.page.waitForTimeout(20);
  assert.equal(await animations(reduced.page, "[data-settings-select-menu]").then((items) => items.length), 0);
  await reduced.page.keyboard.press("Escape");
  await reduced.page.waitForTimeout(20);
  assert.equal(await reduced.page.locator("[data-settings-select-menu]").count(), 0);
  await reduced.context.close();

  // macOS presentation uses the same overlay semantics (window chrome differs,
  // menu geometry and motion must not).
  step = "macos-overlay";
  const mac = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  const macPage = await mac.newPage();
  await macPage.addInitScript(() => {
    Object.defineProperty(navigator, "platform", { get: () => "MacIntel" });
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "zh");
  });
  await macPage.goto(`${baseUrl}/?browser-mock=1`);
  await macPage.waitForTimeout(1500);
  await macPage.locator(".zc-sidebar").getByRole("button", { name: "文件", exact: true }).click();
  await macPage.waitForTimeout(900);
  await macPage.locator("[data-pane-menu] .wb-pane-menu-trigger").click();
  await macPage.waitForTimeout(25);
  const macMenu = await macPage.evaluate(() => document.querySelector(".wb-pane-menu-panel")?.getAnimations().map((animation) => ({ id: animation.id, duration: animation.effect?.getTiming().duration })) ?? []);
  assert.ok(macMenu.some((animation) => animation.id === "zc-menu-in" && animation.duration === 240), JSON.stringify(macMenu));
  await macPage.keyboard.press("Escape");
  await macPage.waitForTimeout(160);
  assert.equal(await macPage.locator(".wb-pane-menu-panel").count(), 0);
  await mac.close();
} catch (error) {
  failures.push(`${step}: ${error instanceof Error ? error.message : String(error)}`);
} finally {
  await browser.close();
  await server.close();
}

if (failures.length) {
  console.error("[v340-overlay] FAIL\n" + failures.join("\n"));
  process.exit(1);
}
console.log("[v340-overlay] PASS settings select (+reopen mid-exit), pane menu (+action), context menu, command modal, toolbar panel, confirm dialog, quick preview (+pin/unpin), reduced motion, macOS");
