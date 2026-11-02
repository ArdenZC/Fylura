import { mkdir, rm, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";
import { createServer } from "vite";
import { assertCheckoutEvidence } from "./ciEvidence.mjs";
import {
  assert,
  assertArchivePreview,
  assertFolderPreview,
  assertNoHorizontalOverflow,
  assertPreviewSecurity,
  assertPreviewMetadataFallback,
  chooseBrowseFile,
  chooseLibraryFile,
  closeFloating,
  closeFloatingWithSpace,
  dispatchFloatingSpace,
  openFloating,
  openFloatingWhileStarting,
  pinPreview,
  pressPreviewNavigationSpace,
  resolveDeferredPreview,
  trackPageSecurity,
  unpinPreview,
  waitForApp
} from "./w3PreviewBrowserHarness.mjs";

const SOURCE_HEAD = process.env.W309_SOURCE_HEAD ?? process.env.GITHUB_SHA ?? "local";
const EXPECTED_CHECKOUT_SHA = process.env.W309_EXPECTED_CHECKOUT_SHA ?? null;
const ACTUAL_CHECKOUT_SHA = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const ACTUAL_CHECKOUT_TREE = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
if (EXPECTED_CHECKOUT_SHA) assertCheckoutEvidence(EXPECTED_CHECKOUT_SHA, ACTUAL_CHECKOUT_SHA);

const TASK_TEMP_DIR = path.resolve(".tmp-tests/w3-09-browser-runtime");
const ARTIFACT_DIR = path.resolve(".tmp-tests/w3-09-browser-gate");
const BASE_FIXTURE_QUERY = "w3-09-browser-fixture=integration&w3-02-browser-fixture=preview&w3-03-browser-fixture=pinned";
const FOLDER_ARCHIVE_FIXTURE_QUERY = `${BASE_FIXTURE_QUERY}&w3-07-browser-fixture=folders&w3-08-browser-fixture=archives`;
process.env.TEMP = TASK_TEMP_DIR;
process.env.TMP = TASK_TEMP_DIR;
process.env.TMPDIR = TASK_TEMP_DIR;
await mkdir(TASK_TEMP_DIR, { recursive: true });
await mkdir(ARTIFACT_DIR, { recursive: true });

const server = await createServer({
  configFile: path.resolve("vite.config.ts"),
  server: { host: "127.0.0.1", port: 0, strictPort: false }
});
await server.listen();
const baseUrl = server.resolvedUrls?.local?.[0]?.replace(/\/$/u, "");
if (!baseUrl) throw new Error("Vite did not expose a local browser URL.");
const appOrigin = new URL(baseUrl).origin;
const browser = await chromium.launch({
  headless: true,
  ...(process.env.W309_CHROMIUM_EXECUTABLE ? { executablePath: process.env.W309_CHROMIUM_EXECUTABLE } : {})
});

async function runScenario(context, viewport, label, scenario, evidence, fixtureQuery = BASE_FIXTURE_QUERY) {
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  page.setDefaultNavigationTimeout(60_000);
  const security = trackPageSecurity(page, appOrigin);
  await page.addInitScript(() => {
    window.localStorage.setItem("zc-onboarding-complete", "true");
    window.localStorage.setItem("zc-language", "en");
    const nativeCreate = URL.createObjectURL.bind(URL);
    const nativeRevoke = URL.revokeObjectURL.bind(URL);
    const created = [];
    const revoked = [];
    const live = new Set();
    URL.createObjectURL = (blob) => {
      const url = nativeCreate(blob);
      created.push(url);
      live.add(url);
      return url;
    };
    URL.revokeObjectURL = (url) => {
      revoked.push(url);
      live.delete(url);
      nativeRevoke(url);
    };
    window.__zcW309Browser = { created, revoked, get live() { return [...live]; } };
  });
  try {
    await page.goto(`${baseUrl}?${fixtureQuery}&managed-library-test=1`, { waitUntil: "commit" });
    await waitForApp(page, label);
    await scenario(page);
    const lifecycle = await page.evaluate(() => JSON.parse(JSON.stringify(window.__zcW309Browser ?? {})));
    assert((lifecycle.live ?? []).length === 0, `${label}: live image object URLs remained ${JSON.stringify(lifecycle)}`);
    assert(security.networkViolations.length === 0, `${label}: unexpected external/resource navigation ${JSON.stringify(security.networkViolations)}`);
    assert(security.errors.length === 0, `${label}: console/page errors ${JSON.stringify(security.errors)}`);
    await assertNoHorizontalOverflow(page, `${label} ${viewport.width}x${viewport.height}`);
    evidence.push({
      label,
      viewport,
      networkViolations: security.networkViolations,
      blobRequests: security.blobRequests,
      lifecycle,
      w309: await page.evaluate(() => window.__zcW309 ? JSON.parse(JSON.stringify(window.__zcW309)) : null)
    });
    await page.screenshot({ path: path.join(ARTIFACT_DIR, `${label}-${viewport.width}x${viewport.height}.png`), fullPage: true });
  } finally {
    await page.close();
  }
}

async function exerciseViewport(viewport) {
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  const evidence = [];
  try {
    await runScenario(context, viewport, "merged-provider-security", async (page) => {
      let selected = await chooseLibraryFile(page, "W3-04-hostile.md");
      await openFloating(page, selected.list, "Markdown");
      const markdown = page.locator('[data-preview-host="zen-floating"] [data-preview-representation="safe_html"]');
      await markdown.waitFor({ state: "visible", timeout: 5_000 });
      assert(await markdown.locator("h1").count() >= 1, "Markdown: heading was not rendered");
      assert(await markdown.locator("strong,b").count() >= 1, "Markdown: bold text was not rendered");
      assert(await markdown.locator("ul,ol").count() >= 1, "Markdown: list was not rendered");
      assert(await markdown.locator("blockquote").count() >= 1, "Markdown: blockquote was not rendered");
      assert(await markdown.locator("pre code, pre").count() >= 1, "Markdown: code block was not rendered");
      assert(await markdown.locator("table").count() >= 1, "Markdown: table was not rendered");
      assert(await markdown.locator("script,iframe,object,embed,img,a,[src],[href],[action]").count() === 0, "Markdown: unsafe element survived rendering");
      const markdownText = await markdown.textContent();
      assert(markdownText?.includes("W3-09 rendered Markdown") === true, "Markdown: rendered heading text missing");
      assert(!markdownText?.includes("# W3-09"), "Markdown: raw heading syntax was presented");
      await assertPreviewSecurity(page, "Markdown");
      assert(await page.locator('[data-preview-state-announcement="true"]').count() === 1, "Markdown: status announcement missing");
      await closeFloating(page, "Markdown");

      selected = await chooseLibraryFile(page, "structured-markup.xml");
      await openFloating(page, selected.list, "XML");
      await page.locator('[data-preview-representation="structured_tree"]').waitFor({ state: "visible" });
      await assertPreviewSecurity(page, "XML");
      assert(await page.locator('[data-preview-structured-format="xml"]').count() === 1, "XML: structured format missing");
      await closeFloating(page, "XML");

      selected = await chooseLibraryFile(page, "structured-records.csv");
      await openFloating(page, selected.list, "CSV");
      await page.locator('[data-preview-representation="table"]').waitFor({ state: "visible" });
      await assertPreviewSecurity(page, "CSV");
      assert((await page.locator('[data-preview-content]').textContent()).includes("=SUM(A1:A2)"), "CSV: formula-looking value was not literal");
      await closeFloating(page, "CSV");

      selected = await chooseLibraryFile(page, "image-sample.png");
      await openFloating(page, selected.list, "Image");
      const image = page.locator('[data-preview-representation="image"]');
      await image.waitFor({ state: "visible" });
      for (let attempt = 0; attempt < 50; attempt += 1) {
        await page.evaluate(() => window.__zcW306?.resolveAllAssets());
        await page.evaluate(() => Promise.resolve());
        if (await image.getAttribute("data-preview-image-status") === "ready") break;
      }
      assert(await image.getAttribute("data-preview-image-status") === "ready", "Image: bounded asset did not settle");
      await assertPreviewSecurity(page, "Image", true);
      assert((await image.locator("img").getAttribute("src"))?.startsWith("blob:"), "Image: transport was not an opaque Blob URL");
      await closeFloating(page, "Image");
      assert((await page.evaluate(() => window.__zcW309Browser?.live.length ?? 0)) === 0, "Image: object URL was not revoked on close");
    }, evidence);

    await runScenario(context, viewport, "folder-archive-convergence", async (page) => {
      let selected = await chooseBrowseFile(page, "w3-07-mixed-folder");
      await openFloating(page, selected.list, "Browse Folder");
      await assertFolderPreview(page, "Browse Folder", "zen-floating", "complete");
      await pinPreview(page, viewport, "Browse Folder Pin");
      await assertFolderPreview(page, "Pinned Folder", "zen-pinned", "complete");
      await unpinPreview(page, "Pinned Folder Unpin");

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "Progressive Browse Folder reload");
      selected = await chooseBrowseFile(page, "w3-07-mixed-folder");
      await openFloatingWhileStarting(page, selected.list, "Progressive Browse Folder");
      await resolveDeferredPreview(page, "Progressive Browse Folder settle");
      await assertFolderPreview(page, "Progressive Browse Folder", "zen-floating");
      await closeFloating(page, "Progressive Browse Folder close");

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "Archive reload");
      selected = await chooseLibraryFile(page, "archive-hostile.zip");
      await openFloating(page, selected.list, "Hostile ZIP");
      await assertArchivePreview(page, "Hostile ZIP");
      assert(await page.locator('[data-preview-archive-unsafe="true"]').count() >= 2, "Hostile ZIP names were not marked");
      await closeFloating(page, "Hostile ZIP close");

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "Corrupt ZIP reload");
      selected = await chooseLibraryFile(page, "archive-corrupt.zip");
      await openFloating(page, selected.list, "Corrupt ZIP");
      await assertPreviewMetadataFallback(page, "Corrupt ZIP");
      await closeFloating(page, "Corrupt ZIP close");

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "No-source ZIP reload");
      selected = await chooseLibraryFile(page, "archive-sample.zip");
      await openFloating(page, selected.list, "No-source ZIP");
      await assertArchivePreview(page, "No-source ZIP Floating");
      await pinPreview(page, viewport, "No-source ZIP Pin");
      await assertArchivePreview(page, "No-source ZIP Pinned", "zen-pinned");
      const pinnedArchiveIdentity = await page.locator('[data-preview-host="zen-pinned"] [data-preview-card="true"]').getAttribute("data-preview-identity");
      await page.locator('[data-file-library-mode="browse"]').evaluate((element) => element instanceof HTMLElement && element.click());
      await page.waitForFunction((identity) => document.querySelector('[data-preview-host="zen-pinned"] [data-preview-card="true"]')?.getAttribute("data-preview-identity") === identity, pinnedArchiveIdentity);
      assert(await page.locator('[data-preview-host="zen-pinned"] [data-preview-representation="archive_tree"]').count() === 1, "No-source ZIP pinned source changed with external mode selection");
      await unpinPreview(page, "No-source ZIP Unpin");

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "Browse ZIP reload");
      selected = await chooseBrowseFile(page, "archive-sample.zip");
      await openFloating(page, selected.list, "Browse ZIP");
      await assertArchivePreview(page, "Browse ZIP");
      await closeFloating(page, "Browse ZIP close");
    }, evidence, FOLDER_ARCHIVE_FIXTURE_QUERY);

    await runScenario(context, viewport, "terminal-materialization-and-permission", async (page) => {
      const cases = [
        ["materialization-required.txt", "materialization_required", "preview_materialization_required"],
        ["permission-denied.txt", "permission_denied", "preview_permission_denied"],
        ["source-unavailable.txt", "source_unavailable", "preview_source_unavailable"],
        ["identity-changed.txt", "identity_changed", "preview_source_identity_changed"]
      ];
      for (const [name, phase, code] of cases) {
        await page.reload({ waitUntil: "commit" });
        await waitForApp(page, `${name} reload`);
        const selected = await chooseLibraryFile(page, name);
        await openFloating(page, selected.list, name);
        await page.waitForSelector(`[data-preview-state="${phase}"]`);
        assert(await page.locator('[data-preview-content] button').count() === 0, `${name}: terminal content offered an action`);
        assert(!/download|fetch|hydrate/iu.test(await page.locator('[data-preview-content]').textContent() ?? ""), `${name}: materialization action text leaked into UI`);
        const codes = await page.evaluate(() => window.__zcW309?.terminalCodes ?? []);
        assert(codes.includes(code), `${name}: terminal code was not exercised ${JSON.stringify(codes)}`);
        await closeFloating(page, name);
      }
    }, evidence);

    await runScenario(context, viewport, "keyboard-focus-and-pinned-owner", async (page) => {
      const selected = await chooseLibraryFile(page, "W3-04-hostile.md");
      await openFloating(page, selected.list, "Accessibility");
      const dialog = page.getByRole("dialog");
      assert(await dialog.count() === 1, "Accessibility: Floating Preview dialog missing");
      assert(await dialog.getAttribute("aria-labelledby"), "Accessibility: dialog label missing");
      assert(await dialog.getAttribute("aria-describedby"), "Accessibility: dialog description missing");
      assert(await page.locator('[data-preview-state-announcement="true"]').count() === 1, "Accessibility: live status missing");

      await dispatchFloatingSpace(page, "Accessibility repeated Space", { repeat: true });
      await dispatchFloatingSpace(page, "Accessibility composing Space", { isComposing: true });
      await dispatchFloatingSpace(page, "Accessibility Alt+Space", { altKey: true });
      await page.locator('[data-preview-host="zen-floating"] [data-preview-content]').evaluate((content) => {
        const input = document.createElement("input");
        input.dataset.w309TestInput = "true";
        content.append(input);
        input.focus();
        input.dispatchEvent(new KeyboardEvent("keydown", { key: " ", code: "Space", bubbles: true, cancelable: true }));
      });
      assert(await page.locator('[data-preview-host="zen-floating"]').count() === 1, "Accessibility: input Space closed Floating Preview");
      await page.keyboard.press("Escape");
      await page.waitForSelector('[data-preview-shell="true"]', { state: "detached" });
      await page.waitForFunction(() => document.activeElement?.closest('[data-shared-file-list="true"]')?.getAttribute("data-shared-file-list-source") === "library", undefined, { timeout: 5_000 });
      const restoredFocus = await page.evaluate(() => ({
        activeTag: document.activeElement?.tagName ?? null,
        activeId: document.activeElement?.id ?? null,
        list: document.activeElement?.closest('[data-shared-file-list="true"]')?.getAttribute("data-shared-file-list-source") ?? null,
        activeDescendant: document.activeElement?.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") ?? null
      }));
      assert(restoredFocus.list === "library", `Accessibility: focus did not return to originating list ${JSON.stringify(restoredFocus)}`);

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "Pinned reload");
      const pinnedSelected = await chooseLibraryFile(page, "W3-04-hostile.md");
      await openFloating(page, pinnedSelected.list, "Pinned");
      await page.locator('[data-preview-pin="true"]').click();
      await page.waitForSelector('[data-preview-host="zen-pinned"]');
      assert(await page.locator('[data-preview-shell="true"]').count() === 1, "Pinned: duplicate Preview shell");
      assert(await page.locator('[data-preview-state-announcement="true"]').count() === 1, "Pinned: duplicate live status");
      const pinnedIdentity = await page.locator('[data-preview-host="zen-pinned"] [data-preview-card="true"]').getAttribute("data-preview-identity");
      const pinnedExternalSelection = await chooseLibraryFile(page, "bounded-prefix.txt");
      await page.waitForFunction((identity) => document.querySelector('[data-preview-host="zen-pinned"] [data-preview-card="true"]')?.getAttribute("data-preview-identity") === identity, pinnedIdentity);
      assert(await page.locator('[data-preview-host="zen-pinned"]').count() === 1, "Pinned: external Files selection closed the pinned host");
      assert(await pinnedExternalSelection.item.getAttribute("aria-selected") === "true", "Pinned: external Files row was not interactable");
      assert(await page.locator('[data-modal-layer="true"]').count() === 0, "Pinned: non-modal surface created a modal owner");
      await unpinPreview(page, "Pinned Unpin");

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "Floating Space ownership reload");
      const spaceSelected = await chooseLibraryFile(page, "W3-04-hostile.md");
      await openFloating(page, spaceSelected.list, "Floating Space ownership");
      await closeFloatingWithSpace(page, "Normal Floating Space close");

      const repeatSelected = await chooseLibraryFile(page, "W3-04-hostile.md");
      await openFloating(page, repeatSelected.list, "Repeat Floating Space");
      await dispatchFloatingSpace(page, "Repeat Floating Space", { repeat: true });
      await closeFloating(page, "Repeat Floating Space cleanup");

      const pinSelected = await chooseLibraryFile(page, "W3-04-hostile.md");
      await openFloating(page, pinSelected.list, "Pin Space ownership");
      const pin = page.locator('[data-preview-pin="true"]');
      await pin.focus();
      await page.keyboard.press("Space");
      await page.waitForFunction(() => document.querySelector('[data-preview-host="zen-pinned"]') !== null
        && document.querySelector('[data-preview-host="zen-floating"]') === null
        && document.querySelectorAll('[data-preview-shell="true"]').length === 1);
      assert(await page.locator('[data-file-library-context-content]').count() === 0, "Pin Space: non-modal handoff left a Context panel mounted");
      await unpinPreview(page, "Pin Space cleanup");

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "Sibling Space ownership reload");
      const navigationSelected = await chooseLibraryFile(page, "W3-04-hostile.md", { unfiltered: true });
      await openFloating(page, navigationSelected.list, "Sibling Space ownership");
      await pressPreviewNavigationSpace(page, "next", "Next Space ownership");
      await pressPreviewNavigationSpace(page, "previous", "Previous Space ownership");

      await page.reload({ waitUntil: "commit" });
      await waitForApp(page, "Close button Space ownership reload");
      const closeSelected = await chooseLibraryFile(page, "W3-04-hostile.md");
      await openFloating(page, closeSelected.list, "Close button Space ownership");
      const closeButton = page.locator('[data-preview-host="zen-floating"] .zc-quick-preview-close');
      await closeButton.focus();
      await page.keyboard.press("Space");
      await page.waitForSelector('[data-preview-shell="true"]', { state: "detached" });
      assert(await page.locator('[data-preview-shell="true"]').count() === 0, "Close button Space: host-level handler duplicated button close");
    }, evidence);
  } finally {
    await context.close();
  }
  await writeFile(path.join(ARTIFACT_DIR, `viewport-${viewport.width}x${viewport.height}.json`), JSON.stringify({
    sourceHead: SOURCE_HEAD,
    actualCheckoutSha: ACTUAL_CHECKOUT_SHA,
    actualCheckoutTree: ACTUAL_CHECKOUT_TREE,
    viewport,
    evidence
  }, null, 2));
  console.log(`[w3-09-real] PASS ${viewport.width}x${viewport.height} sourceHead=${SOURCE_HEAD} actualSha=${ACTUAL_CHECKOUT_SHA} tree=${ACTUAL_CHECKOUT_TREE}`);
}

try {
  await exerciseViewport({ width: 1600, height: 900 });
  await exerciseViewport({ width: 980, height: 680 });
} finally {
  await browser.close();
  await server.close();
  await rm(ARTIFACT_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
  await rm(TASK_TEMP_DIR, { recursive: true, force: true, maxRetries: 20, retryDelay: 250 });
}
