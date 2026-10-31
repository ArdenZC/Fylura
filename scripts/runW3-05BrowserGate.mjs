import { mkdir, rm, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { chromium } from "playwright";
import { createServer } from "vite";
import { assertCheckoutEvidence } from "./ciEvidence.mjs";

const SOURCE_HEAD = process.env.W305_SOURCE_HEAD ?? process.env.GITHUB_SHA ?? "local";
const EXPECTED_CHECKOUT_SHA = process.env.W305_EXPECTED_CHECKOUT_SHA ?? null;
const ACTUAL_CHECKOUT_SHA = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const ACTUAL_CHECKOUT_TREE = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
if (EXPECTED_CHECKOUT_SHA) assertCheckoutEvidence(EXPECTED_CHECKOUT_SHA, ACTUAL_CHECKOUT_SHA);

const TASK_TEMP_DIR = path.resolve(".tmp-tests/w3-05-browser-runtime");
const ARTIFACT_DIR = path.resolve(".tmp-tests/w3-05-browser-gate");
const FIXTURE_QUERY = "w3-05-browser-fixture=providers&w3-04-browser-fixture=providers&w3-02-browser-fixture=preview&w3-03-browser-fixture=pinned";

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
  ...(process.env.W305_CHROMIUM_EXECUTABLE ? { executablePath: process.env.W305_CHROMIUM_EXECUTABLE } : {})
});

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function tick(page) {
  await page.evaluate(() => Promise.resolve());
}

async function assertPageIdentity(page, label) {
  await page.waitForSelector("#root");
  await page.waitForFunction(() => document.title.trim().length > 0 && document.body.textContent?.trim().length > 0);
  const identity = await page.evaluate(() => ({
    title: document.title,
    bodyTextLength: document.body.textContent?.trim().length ?? 0,
    frameworkOverlay: Boolean(document.querySelector("vite-error-overlay, .vite-error-overlay"))
  }));
  assert(identity.title.includes("Zen"), `${label}: unexpected page title ${identity.title}`);
  assert(identity.bodyTextLength > 0, `${label}: blank application DOM`);
  assert(!identity.frameworkOverlay, `${label}: framework error overlay mounted`);
}

async function assertNoHorizontalOverflow(page, label) {
  const overflow = await page.evaluate(() => ({
    document: document.documentElement.scrollWidth > window.innerWidth + 1,
    body: document.body.scrollWidth > window.innerWidth + 1,
    workspace: [...document.querySelectorAll(".file-library-workspace")].some((element) => element.scrollWidth > element.clientWidth + 1),
    preview: [...document.querySelectorAll('[data-preview-shell="true"]')].some((element) => element.scrollWidth > element.clientWidth + 1)
  }));
  assert(!Object.values(overflow).some(Boolean), `${label}: horizontal overflow ${JSON.stringify(overflow)}`);
}

async function waitForLibrary(page) {
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
  const allIndexedFiles = page.getByRole("button", { name: "View all indexed files", exact: true });
  if (await allIndexedFiles.count() > 0 && await allIndexedFiles.first().isVisible()) await allIndexedFiles.first().click();
  const list = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="library"]');
  await list.waitFor({ state: "visible" });
  await list.locator('[role="option"]').first().waitFor({ state: "visible" });
  return list;
}

async function choose(page, surface, text, role = "option") {
  const item = surface.locator(`[role="${role}"]`).filter({ hasText: text }).first();
  await item.waitFor({ state: "visible" });
  const itemId = await item.getAttribute("id");
  await item.click();
  if (role === "option") {
    assert(itemId, `Could not identify selected ${text} row`);
    await page.waitForFunction((id) => {
      const row = id === null ? null : document.getElementById(id);
      return row?.getAttribute("aria-selected") === "true"
        && row.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") === id;
    }, itemId);
  }
  return item;
}

async function chooseLibraryFile(page, name) {
  const list = await waitForLibrary(page);
  const search = page.locator('[data-file-library-local-search="true"]');
  await search.fill(name);
  await page.waitForFunction((expectedName) => {
    const rows = [...document.querySelectorAll('[data-shared-file-list="true"][data-shared-file-list-source="library"] [role="option"]')];
    return document.querySelector('[data-file-library-local-search="true"]')?.value === expectedName
      && rows.length === 1
      && rows[0]?.textContent?.includes(expectedName) === true;
  }, name);
  await page.waitForFunction(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-provenance") === "query-v2-snapshot");
  await list.locator('[role="option"]').filter({ hasText: name }).first().waitFor({ state: "visible" });
  await choose(page, list, name);
  await page.waitForFunction(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-provenance") === "query-v2-snapshot");
  return list;
}

async function resolveDeferred(page, label) {
  await page.waitForFunction(() => {
    const shell = document.querySelector('[data-preview-shell="true"]');
    const pending = window.__zcW302?.pendingStartCount ?? 0;
    const state = shell?.getAttribute("data-preview-state");
    return pending > 0 && (state === "resolving" || state === "loading");
  });
  for (let attempt = 0; attempt < 40; attempt += 1) {
    await page.evaluate(() => window.__zcW302?.resolveAll());
    await tick(page);
    await page.waitForTimeout(0);
    const settled = await page.evaluate(() => ({
      pending: window.__zcW302?.pendingStartCount ?? 0,
      phase: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
      rendered: document.querySelector('[data-preview-representation], [data-preview-metadata="true"]') !== null
    }));
    if (settled.pending === 0
      && ["content", "metadata_fallback", "unsupported_representation"].includes(settled.phase ?? "")
      && (settled.phase !== "content" || settled.rendered)) return;
  }
  const stats = await page.evaluate(() => ({
    w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
    w304: window.__zcW304 ? JSON.parse(JSON.stringify(window.__zcW304)) : null
  }));
  throw new Error(`${label}: deferred Preview did not settle ${JSON.stringify(stats)}`);
}

async function openFloating(page, surface, label) {
  await surface.focus();
  if (await surface.getAttribute("aria-activedescendant") === null) await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Space");
  await page.waitForSelector('[data-preview-host="zen-floating"]');
  const identity = await page.locator('[data-preview-host="zen-floating"] [data-preview-card="true"]').getAttribute("data-preview-identity");
  assert(identity && identity !== "none", `${label}: Floating Preview has no source identity`);
  await resolveDeferred(page, label);
  assert(await page.locator('[data-preview-host="zen-floating"]').count() === 1, `${label}: duplicate Floating hosts`);
  return identity;
}

async function closeFloating(page, label) {
  await page.locator('[data-preview-host="zen-floating"] [aria-label="Close preview"]').click();
  await page.waitForSelector('[data-preview-shell="true"]', { state: "detached" });
  assert(await page.locator('[data-preview-shell="true"]').count() === 0, `${label}: Floating Preview remained mounted`);
}

async function assertRepresentation(page, family, label, host = "zen-floating") {
  const representation = page.locator(`[data-preview-host="${host}"] [data-preview-representation="${family}"]`);
  try {
    await representation.waitFor({ state: "visible" });
  } catch (error) {
    const diagnostics = await page.evaluate((hostName) => ({
      phase: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
      hostText: document.querySelector(`[data-preview-host="${hostName}"]`)?.textContent ?? null,
      representations: [...document.querySelectorAll('[data-preview-representation]')].map((element) => element.getAttribute("data-preview-representation")),
      w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
      w304: window.__zcW304 ? JSON.parse(JSON.stringify(window.__zcW304)) : null
    }), host);
    throw new Error(`${label}: representation wait failed ${JSON.stringify(diagnostics)} (${String(error)})`);
  }
  assert(await representation.getAttribute("data-preview-completeness") !== "unknown", `${label}: completeness was unknown`);
  return representation;
}

async function pin(page, viewport, label) {
  await page.locator('[data-preview-pin="true"]').click();
  await page.waitForFunction(() => document.querySelector('[data-preview-host="zen-pinned"]') !== null
    && document.querySelectorAll('[data-preview-shell="true"]').length === 1
    && document.querySelector('[data-preview-host="zen-floating"]') === null);
  assert(await page.locator('[data-preview-host="zen-pinned"] [data-preview-card="true"]').count() === 1, `${label}: pinned card was not mounted in the shared surface`);
  if (viewport.width <= 980) {
    assert(await page.locator('[data-side-sheet="true"]').count() === 0, `${label}: compact Pinned handoff left a modal SideSheet mounted`);
    assert(await page.locator('[data-modal-layer="true"]').count() === 0, `${label}: compact Pinned Preview created a modal layer`);
  } else {
    assert(await page.locator('[data-modal-layer="true"]').count() === 0, `${label}: large Pinned Preview opened a modal layer`);
  }
  await page.waitForFunction(() => {
    const phase = document.querySelector('[data-preview-host="zen-pinned"]')?.getAttribute("data-preview-state");
    return (window.__zcW302?.pendingStartCount ?? 0) > 0 || ["content", "metadata_fallback", "unsupported_representation", "no_source"].includes(phase ?? "");
  });
  if ((await page.evaluate(() => window.__zcW302?.pendingStartCount ?? 0)) > 0) await resolveDeferred(page, `${label} staged Pinned Preview`);
}

async function unpin(page, label) {
  await page.locator('[data-preview-pin="true"][data-preview-pin-state="pinned"]').click();
  await page.waitForFunction(() => document.querySelector('[data-preview-host="zen-pinned"]') === null
    && document.querySelector('[data-preview-host="zen-floating"]') !== null
    && document.querySelectorAll('[data-preview-shell="true"]').length === 1);
  assert(await page.locator('[data-file-library-context-content]').count() === 0, `${label}: Unpin left a Context panel mounted`);
}

async function openBrowse(page) {
  if (await page.getByRole("tab", { name: "Browse Folder", exact: true }).count() === 0) await waitForLibrary(page);
  await page.getByRole("tab", { name: "Browse Folder", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="browse"]');
  if (await page.locator('[data-browse-state="current-folder"]').count() === 0) {
    const openable = page.locator('[data-browse-location-openable="true"] [data-browse-location-action="open"]');
    await openable.first().waitFor({ state: "visible" });
    await openable.first().click();
  }
  await page.locator('[data-browse-state="current-folder"]').waitFor({ state: "visible" });
  const list = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="browse"]');
  await list.locator('[role="option"]').first().waitFor({ state: "visible" });
  return list;
}

async function chooseBrowseFile(page, name) {
  const list = await openBrowse(page);
  const search = page.locator('[data-browse-query-controls="true"] input');
  await search.fill(name);
  await list.locator('[role="option"]').filter({ hasText: name }).first().waitFor({ state: "visible" });
  await choose(page, list, name);
  return list;
}

async function runScenario(context, viewport, label, scenario, evidence) {
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  page.setDefaultNavigationTimeout(60_000);
  const errors = [];
  const networkViolations = [];
  page.on("console", (message) => { if (message.type() === "error") errors.push({ kind: "console", text: message.text() }); });
  page.on("pageerror", (error) => errors.push({ kind: "pageerror", text: String(error) }));
  page.on("request", (request) => {
    const url = request.url();
    const isAppResource = url.startsWith(`${appOrigin}/`) || url === appOrigin || url.startsWith("ws:") || url.startsWith("wss:");
    if (!isAppResource || /^(file:|data:|blob:)/u.test(url)) networkViolations.push({ kind: "request", url });
  });
  page.on("framenavigated", (frame) => {
    if (frame !== page.mainFrame()) return;
    const url = frame.url();
    if (!url.startsWith(`${appOrigin}/`) && url !== appOrigin) networkViolations.push({ kind: "navigation", url });
  });
  try {
    await page.goto(`${baseUrl}?${FIXTURE_QUERY}`, { waitUntil: "commit" });
    await assertPageIdentity(page, label);
    await scenario(page);
    assert(networkViolations.length === 0, `${label}: unexpected external/resource navigation ${JSON.stringify(networkViolations)}`);
    assert(errors.length === 0, `${label}: console/page errors ${JSON.stringify(errors)}`);
    await assertNoHorizontalOverflow(page, `${label} ${viewport.width}x${viewport.height}`);
    evidence.push({ label, viewport, networkViolations, w302: await page.evaluate(() => window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null) });
    await page.screenshot({ path: path.join(ARTIFACT_DIR, `${label}-${viewport.width}x${viewport.height}.png`), fullPage: true });
  } finally {
    await page.close();
  }
}

async function exerciseViewport(viewport) {
  const context = await browser.newContext({ viewport, deviceScaleFactor: 1 });
  await context.addInitScript(() => {
    window.localStorage.setItem("zc-onboarding-complete", "true");
    window.localStorage.setItem("zc-language", "en");
  });
  const evidence = [];
  try {
    await runScenario(context, viewport, "library-json-floating-pinned", async (page) => {
      const list = await waitForLibrary(page);
      await choose(page, list, "structured-sample.json");
      await page.waitForFunction(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-provenance") === "query-v2-snapshot");
      await openFloating(page, list, "Library JSON Floating");
      const tree = await assertRepresentation(page, "structured_tree", "Library JSON tree");
      assert(await tree.getAttribute("data-preview-structured-format") === "json", "JSON format was not rendered");
      assert((await tree.textContent())?.includes("Fylura") === true, "JSON scalar was not rendered");
      await pin(page, viewport, "Library JSON Pin");
      await assertRepresentation(page, "structured_tree", "Pinned JSON tree", "zen-pinned");
      await unpin(page, "Library JSON Unpin");
    }, evidence);

    await runScenario(context, viewport, "library-yaml-xml-csv-tsv", async (page) => {
      for (const [index, [name, family, format]] of [
        ["structured-config.yaml", "structured_tree", "yaml"],
        ["structured-markup.xml", "structured_tree", "xml"],
        ["structured-records.csv", "table", "csv"],
        ["structured-records.tsv", "table", "tsv"]
      ].entries()) {
        if (index > 0) {
          await page.reload({ waitUntil: "commit" });
          await assertPageIdentity(page, `${name} reload`);
        }
        const list = await chooseLibraryFile(page, name);
        await openFloating(page, list, `${name} Floating`);
        const representation = await assertRepresentation(page, family, `${name} representation`);
        assert(await representation.getAttribute(family === "table" ? "data-preview-table-format" : "data-preview-structured-format") === format, `${name}: format mismatch`);
        if (name.endsWith(".xml")) {
          assert((await representation.textContent())?.includes("<script>inert text</script>") === true, "XML markup was not inert text");
          assert(await representation.locator("script").count() === 0, "XML fixture mounted an executable script");
        }
        if (name.endsWith(".csv")) {
          assert((await representation.textContent())?.includes("=SUM(A1:A2)") === true, "CSV formula was evaluated or lost");
        }
        await closeFloating(page, `${name} close`);
      }
    }, evidence);

    await runScenario(context, viewport, "library-partial-and-fallback", async (page) => {
      let list = await chooseLibraryFile(page, "structured-partial.json");
      await openFloating(page, list, "Partial structured Floating");
      const structured = await assertRepresentation(page, "structured_tree", "Partial structured");
      assert(await structured.locator('[data-preview-partial="true"]').count() === 1, "Structured Partial disclosure was not visible");
      await closeFloating(page, "Partial structured close");
      list = await chooseLibraryFile(page, "table-partial.csv");
      await openFloating(page, list, "Partial table Floating");
      const table = await assertRepresentation(page, "table", "Partial table");
      assert(await table.locator('[data-preview-partial="true"]').count() === 1, "Table Partial disclosure was not visible");
      await closeFloating(page, "Partial table close");
      list = await chooseLibraryFile(page, "malformed-structured.json");
      await openFloating(page, list, "Malformed structured fallback");
      await page.locator('[data-preview-metadata="true"]').waitFor({ state: "visible" });
      await closeFloating(page, "Malformed fallback close");
      list = await chooseLibraryFile(page, "unsupported-structured.bin");
      await openFloating(page, list, "Unsupported structured fallback");
      await page.locator('[data-preview-metadata="true"]').waitFor({ state: "visible" });
      await closeFloating(page, "Unsupported fallback close");
    }, evidence);

    await runScenario(context, viewport, "browse-json-floating-pinned", async (page) => {
      const browse = await openBrowse(page);
      await choose(page, browse, "structured-sample.json");
      await openFloating(page, browse, "Browse JSON Floating");
      await assertRepresentation(page, "structured_tree", "Browse JSON tree");
      await pin(page, viewport, "Browse JSON Pin");
      await assertRepresentation(page, "structured_tree", "Browse JSON pinned tree", "zen-pinned");
      await unpin(page, "Browse JSON Unpin");
    }, evidence);

    await runScenario(context, viewport, "structured-latest-wins-and-no-source", async (page) => {
      const list = await waitForLibrary(page);
      await choose(page, list, "structured-sample.json");
      await page.waitForFunction(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-provenance") === "query-v2-snapshot");
      await openFloating(page, list, "Latest-wins JSON Floating");
      await pin(page, viewport, "Latest-wins Pin");
      await page.locator('[data-file-library-local-search="true"]').fill("");
      await page.waitForFunction(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-provenance") === "query-v2-snapshot");
      const baselineLate = await page.evaluate(() => window.__zcW302?.lateStarts ?? 0);
      const baselineSwitches = await page.evaluate(() => window.__zcW302?.switchCalls ?? 0);
      const pinnedIdentity = await page.locator('[data-preview-host="zen-pinned"] [data-preview-card="true"]').getAttribute("data-preview-identity");
      await list.focus();
      await list.press("ArrowDown");
      await list.press("ArrowDown");
      await page.waitForTimeout(0);
      const stats = await page.evaluate(() => ({
        late: window.__zcW302?.lateStarts ?? 0,
        switches: window.__zcW302?.switchCalls ?? 0,
        identity: document.querySelector('[data-preview-host="zen-pinned"] [data-preview-card="true"]')?.getAttribute("data-preview-identity"),
        hosts: document.querySelectorAll('[data-preview-shell="true"]').length
      }));
      assert(stats.late === baselineLate, `Pinned source isolation started a stale structured result ${JSON.stringify(stats)}`);
      assert(stats.switches === baselineSwitches, `Pinned source isolation triggered a structured owner switch ${JSON.stringify(stats)}`);
      assert(stats.identity === pinnedIdentity && stats.hosts === 1, `Pinned structured source changed with external focus ${JSON.stringify(stats)}`);
      await page.locator('[data-file-library-mode="browse"]').evaluate((element) => element instanceof HTMLElement && element.click());
      await page.waitForFunction((previous) => document.querySelector('[data-preview-host="zen-pinned"] [data-preview-card="true"]')?.getAttribute("data-preview-identity") === previous
        && document.querySelector('[data-preview-host="zen-pinned"]') !== null
        && document.querySelector('[data-preview-host="zen-floating"]') === null, pinnedIdentity);
      await page.keyboard.press("Escape");
      await page.waitForSelector('[data-preview-shell="true"]', { state: "detached" });
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
  console.log(`[w3-05-real] PASS ${viewport.width}x${viewport.height} sourceHead=${SOURCE_HEAD} actualSha=${ACTUAL_CHECKOUT_SHA} tree=${ACTUAL_CHECKOUT_TREE}`);
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
