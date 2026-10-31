export function assert(condition, message) {
  if (!condition) throw new Error(message);
}

export function trackPageSecurity(page, appOrigin) {
  const errors = [];
  const networkViolations = [];
  const blobRequests = [];
  page.on("console", (message) => {
    if (message.type() === "error") errors.push({ kind: "console", text: message.text() });
  });
  page.on("pageerror", (error) => errors.push({ kind: "pageerror", text: String(error) }));
  page.on("request", (request) => {
    const url = request.url();
    if (url.startsWith("blob:")) {
      blobRequests.push(url);
      return;
    }
    if (!url.startsWith(`${appOrigin}/`) && url !== appOrigin && !url.startsWith("ws:") && !url.startsWith("wss:")) {
      networkViolations.push({ kind: "request", url });
    }
  });
  page.on("framenavigated", (frame) => {
    if (frame !== page.mainFrame()) return;
    const url = frame.url();
    if (!url.startsWith(`${appOrigin}/`) && url !== appOrigin) networkViolations.push({ kind: "navigation", url });
  });
  return { errors, networkViolations, blobRequests };
}

export async function waitForApp(page, label) {
  await page.waitForSelector("#root");
  await page.waitForFunction(() => document.title.trim().length > 0 && document.body.textContent?.trim().length > 0);
  const identity = await page.evaluate(() => ({
    title: document.title,
    frameworkOverlay: Boolean(document.querySelector("vite-error-overlay, .vite-error-overlay"))
  }));
  assert(identity.title.includes("Fylura"), `${label}: unexpected page title ${identity.title}`);
  assert(!identity.frameworkOverlay, `${label}: framework error overlay mounted`);
}

export async function assertNoHorizontalOverflow(page, label) {
  const overflow = await page.evaluate(() => ({
    document: document.documentElement.scrollWidth > window.innerWidth + 1,
    body: document.body.scrollWidth > window.innerWidth + 1,
    workspace: [...document.querySelectorAll(".file-library-workspace")].some((element) => element.scrollWidth > element.clientWidth + 1),
    preview: [...document.querySelectorAll('[data-preview-shell="true"]')].some((element) => element.scrollWidth > element.clientWidth + 1)
  }));
  assert(!Object.values(overflow).some(Boolean), `${label}: horizontal overflow ${JSON.stringify(overflow)}`);
}

export async function openLibrary(page) {
  if (await page.locator(".file-library-workspace").count() === 0) {
    await page.getByRole("button", { name: "Files", exact: true }).click();
    await page.locator(".file-library-workspace").waitFor({ state: "visible" });
  }
  const libraryTab = page.locator('[role="tab"][data-file-library-mode="library"]');
  await libraryTab.waitFor({ state: "visible" });
  if (await page.locator('.file-library-workspace[data-mode="library"]').count() === 0) {
    await libraryTab.click();
    await page.waitForSelector('.file-library-workspace[data-mode="library"]', { state: "visible" });
  }
  const allIndexed = page.getByRole("button", { name: "View all indexed files", exact: true });
  if (await allIndexed.count() > 0 && await allIndexed.first().isVisible()) await allIndexed.first().click();
  const list = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="library"]');
  await list.waitFor({ state: "visible" });
  await list.locator('[role="option"]').first().waitFor({ state: "visible" });
  return list;
}

export async function chooseLibraryFile(page, name, { unfiltered = false } = {}) {
  const list = await openLibrary(page);
  const search = page.locator('[data-file-library-local-search="true"]');
  await search.fill(unfiltered ? "" : name);
  await page.waitForFunction((expected) => {
    const input = document.querySelector('[data-file-library-local-search="true"]');
    const owner = document.querySelector('[data-library-source-owner="query-v2"]');
    return input?.value === expected && owner?.getAttribute("data-library-provenance") === "query-v2-snapshot";
  }, unfiltered ? "" : name);
  const item = list.locator('[role="option"]').filter({ hasText: name }).first();
  await item.waitFor({ state: "visible" });
  await page.waitForFunction((expectMany) => {
    const count = Number(document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("data-file-library-logical-count") ?? 0);
    return expectMany ? count > 1 : count === 1;
  }, unfiltered);
  const itemId = await item.getAttribute("id");
  const alreadyActive = itemId !== null
    && await item.getAttribute("aria-selected") === "true"
    && await list.getAttribute("aria-activedescendant") === itemId;
  if (alreadyActive) {
    await list.focus();
    await list.press("Escape");
    await page.waitForFunction(() => document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-selection-kind") === "none"
      && document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") === null);
  }
  await item.focus();
  await item.evaluate((element) => element instanceof HTMLElement && element.click());
  await page.waitForFunction((id) => {
    const row = id === null ? null : document.getElementById(id);
    return row?.getAttribute("aria-selected") === "true"
      && row.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") === id;
  }, itemId);
  return { list, item, itemId };
}

export async function choosePreviewItem(page, surface, text, role = "option", domClick = false) {
  const item = surface.locator(`[role="${role}"]`).filter({ hasText: text }).first();
  await item.waitFor({ state: "visible" });
  const itemId = await item.getAttribute("id");
  if (domClick) await item.evaluate((element) => element instanceof HTMLElement && element.click());
  else await item.click();
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

export async function openBrowse(page) {
  if (await page.getByRole("tab", { name: "Browse Folder", exact: true }).count() === 0) {
    await page.getByRole("button", { name: "Files", exact: true }).click();
  }
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

export async function chooseBrowseFile(page, name, domClick = false) {
  const list = await openBrowse(page);
  const search = page.locator('[data-file-library-local-search="true"]');
  await search.fill(name);
  await list.locator('[role="option"]').filter({ hasText: name }).first().waitFor({ state: "visible" });
  await choosePreviewItem(page, list, name, "option", domClick);
  return { list };
}

export async function resolveDeferredPreview(page, label) {
  await page.waitForFunction(() => (window.__zcW302?.pendingStartCount ?? 0) > 0);
  for (let attempt = 0; attempt < 60; attempt += 1) {
    await page.evaluate(() => window.__zcW302?.resolveAll());
    await page.evaluate(() => Promise.resolve());
    const settled = await page.evaluate(() => ({
      pending: window.__zcW302?.pendingStartCount ?? 0,
      phase: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null
    }));
    if (settled.pending === 0 && ["content", "metadata_fallback", "unsupported_representation"].includes(settled.phase ?? "")) return;
  }
  const stats = await page.evaluate(() => ({
    w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
    w307: window.__zcW307 ? JSON.parse(JSON.stringify(window.__zcW307)) : null
  }));
  throw new Error(`${label}: deferred Preview did not settle ${JSON.stringify(stats)}`);
}

export async function openFloatingWhileStarting(page, surface, label) {
  await surface.focus();
  if (await surface.getAttribute("aria-activedescendant") === null) await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Space");
  await page.waitForSelector('[data-preview-host="zen-floating"]');
  await page.waitForFunction(() => {
    const representation = document.querySelector('[data-preview-host="zen-floating"] [data-preview-representation="folder_summary"]');
    return (window.__zcW302?.pendingStartCount ?? 0) > 0
      && (window.__zcW307?.snapshotCalls ?? 0) >= 1
      && representation?.getAttribute("data-preview-completeness") === "partial"
      && representation?.getAttribute("data-preview-folder-state") === "partial";
  });
  const firstInspected = Number(await page.locator('[data-preview-host="zen-floating"] [data-preview-representation="folder_summary"]').getAttribute("data-preview-inspected-entries"));
  await page.waitForFunction((count) => {
    const representation = document.querySelector('[data-preview-host="zen-floating"] [data-preview-representation="folder_summary"]');
    return (window.__zcW307?.snapshotCalls ?? 0) >= 2
      && Number(representation?.getAttribute("data-preview-inspected-entries")) > count;
  }, firstInspected);
  assert(await page.evaluate(() => (window.__zcW302?.pendingStartCount ?? 0) > 0), `${label}: final previewStart resolved too early`);
}

export async function settlePreview(page, label) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    await page.evaluate(() => window.__zcW302?.resolveAll());
    await page.evaluate(() => window.__zcW306?.resolveAllAssets());
    await page.evaluate(() => Promise.resolve());
    const state = await page.locator('[data-preview-shell="true"]').getAttribute("data-preview-state").catch(() => null);
    const representationReady = state !== "content"
      || await page.locator("[data-preview-representation]").count() > 0;
    if (state !== null && !["resolving", "loading"].includes(state) && representationReady) {
      if (state !== "error") await waitForPreviewHandoffReady(page, label, state);
      return state;
    }
  }
  const diagnostics = await page.evaluate(() => ({
    phase: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
    w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null,
    w306: window.__zcW306 ? JSON.parse(JSON.stringify(window.__zcW306)) : null,
    w309: window.__zcW309 ? JSON.parse(JSON.stringify(window.__zcW309)) : null
  }));
  throw new Error(`${label}: Preview did not settle ${JSON.stringify(diagnostics)}`);
}

async function waitForPreviewHandoffReady(page, label, expectedState) {
  await page.waitForFunction((state) => {
    const shell = document.querySelector('[data-preview-shell="true"]');
    if (shell?.getAttribute("data-preview-state") !== state) return false;
    if (state === "content" && shell.querySelector("[data-preview-representation]") === null) return false;
    const pin = shell.querySelector('[data-preview-pin="true"]');
    return pin instanceof HTMLButtonElement && !pin.disabled;
  }, expectedState, { timeout: 5_000 }).catch(async (error) => {
    const diagnostics = await page.evaluate(() => ({
      phase: document.querySelector('[data-preview-shell="true"]')?.getAttribute("data-preview-state") ?? null,
      pinDisabled: document.querySelector('[data-preview-pin="true"]')?.getAttribute("disabled") ?? null,
      owner: document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-selection-kind") ?? null,
      provenance: document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-provenance") ?? null,
      activeDescendant: document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") ?? null,
      selected: document.querySelector('[data-shared-file-list-source="library"] [aria-selected="true"]')?.getAttribute("id") ?? null,
      w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null
    }));
    throw new Error(`${label}: Preview did not expose an enabled handoff control ${JSON.stringify(diagnostics)} (${String(error)})`);
  });
}

export async function openFloating(page, surface, label) {
  await surface.focus();
  if (await surface.getAttribute("aria-activedescendant") === null) await surface.press("ArrowDown");
  const activeId = await surface.getAttribute("aria-activedescendant");
  if (activeId !== null) {
    const activeRow = page.locator(`#${activeId}`);
    if (await activeRow.getAttribute("aria-selected") !== "true") {
      await activeRow.focus();
      await activeRow.evaluate((element) => element instanceof HTMLElement && element.click());
      await page.waitForFunction((id) => {
        const row = id === null ? null : document.getElementById(id);
        return row?.getAttribute("aria-selected") === "true"
          && row.closest('[role="listbox"]')?.getAttribute("aria-activedescendant") === id;
      }, activeId);
    }
  }
  await surface.press("Space");
  await page.waitForSelector('[data-preview-host="zen-floating"]').catch(async (error) => {
    const diagnostics = await page.evaluate(() => ({
      owner: document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-selection-kind") ?? null,
      provenance: document.querySelector('[data-library-source-owner="query-v2"]')?.getAttribute("data-library-provenance") ?? null,
      activeDescendant: document.querySelector('[data-shared-file-list-source="library"]')?.getAttribute("aria-activedescendant") ?? null,
      selected: document.querySelector('[data-shared-file-list-source="library"] [aria-selected="true"]')?.getAttribute("id") ?? null,
      w302: window.__zcW302 ? JSON.parse(JSON.stringify(window.__zcW302)) : null
    }));
    throw new Error(`${label}: Floating Preview did not open ${JSON.stringify(diagnostics)} (${String(error)})`);
  });
  const state = await settlePreview(page, label);
  assert(state !== "error", `${label}: Preview entered generic error`);
  return state;
}

export async function focusFloatingContent(page, label) {
  const content = page.locator('[data-preview-host="zen-floating"] [data-preview-content]').first();
  await content.waitFor({ state: "visible" });
  await content.evaluate((element) => {
    if (!(element instanceof HTMLElement)) throw new Error("Floating Preview content is not an HTMLElement");
    element.tabIndex = 0;
    element.focus();
  });
  assert(await page.evaluate(() => document.activeElement?.matches('[data-preview-host="zen-floating"] [data-preview-content]') === true),
    `${label}: non-interactive Preview content did not receive focus`);
  return content;
}

export async function closeFloatingWithSpace(page, label) {
  assert(await page.locator('[data-preview-shell="true"]').count() === 1, `${label}: expected one Floating Preview shell before Space close`);
  await focusFloatingContent(page, label);
  await page.keyboard.press("Space");
  await page.waitForSelector('[data-preview-shell="true"]', { state: "detached" });
  assert(await page.locator('[data-preview-shell="true"]').count() === 0, `${label}: Space left a duplicate Preview shell`);
  await page.waitForFunction(() => document.activeElement?.closest('[data-shared-file-list="true"]')?.getAttribute("data-shared-file-list-source") === "library");
}

export async function dispatchFloatingSpace(page, label, {
  targetSelector = '[data-preview-host="zen-floating"] [data-preview-content]',
  repeat = false,
  isComposing = false,
  altKey = false
} = {}) {
  const target = page.locator(targetSelector).first();
  await target.waitFor({ state: "visible" });
  await target.evaluate((element, options) => {
    element.dispatchEvent(new KeyboardEvent("keydown", {
      key: " ",
      code: "Space",
      repeat: options.repeat,
      isComposing: options.isComposing,
      altKey: options.altKey,
      bubbles: true,
      cancelable: true
    }));
  }, { repeat, isComposing, altKey });
  assert(await page.locator('[data-preview-shell="true"]').count() === 1, `${label}: guarded Space changed Preview shell ownership`);
}

export async function pressPreviewNavigationSpace(page, direction, label) {
  const button = page.locator(`[data-preview-navigation="${direction}"]:not([disabled])`).first();
  await button.waitFor({ state: "visible" });
  const beforeEpoch = await page.locator('[data-preview-host="zen-floating"] [data-preview-card="true"]').getAttribute("data-preview-epoch");
  await button.press("Space");
  await page.waitForFunction((epoch) => {
    const card = document.querySelector('[data-preview-host="zen-floating"] [data-preview-card="true"]');
    return card !== null && card.getAttribute("data-preview-epoch") !== epoch;
  }, beforeEpoch);
  assert(await page.locator('[data-preview-shell="true"]').count() === 1, `${label}: ${direction} Space closed or duplicated Preview`);
}

export async function assertFolderPreview(page, label, host = "zen-floating", state = null) {
  const representation = page.locator(`[data-preview-host="${host}"] [data-preview-representation="folder_summary"]`);
  await representation.waitFor({ state: "visible" });
  const actualState = await representation.getAttribute("data-preview-folder-state");
  assert(["complete", "partial"].includes(actualState ?? ""), `${label}: invalid FolderSummary state ${actualState}`);
  if (state !== null) assert(actualState === state, `${label}: FolderSummary state mismatch ${actualState}`);
  const limitReason = await representation.getAttribute("data-preview-limit-reason");
  if (actualState === "complete") assert(limitReason === "none", `${label}: Complete disclosed limit ${limitReason}`);
  else assert(["none", "entry_limit", "deadline"].includes(limitReason ?? ""), `${label}: invalid Partial limit ${limitReason}`);
  assert((await representation.textContent())?.includes("Inspected") === true, `${label}: Folder progress missing`);
  assert((await representation.textContent())?.includes("Accepted children") === true, `${label}: Folder accepted count missing`);
  assert((await representation.textContent())?.includes("C:\\") !== true, `${label}: Folder rendered a path-like value`);
  assert(await representation.locator("a").count() === 0, `${label}: Folder rendered navigation links`);
  assert(await representation.locator("[aria-live]").count() === 0, `${label}: Folder progress became a live-region stream`);
  return representation;
}

export async function assertArchivePreview(page, label, host = "zen-floating", state = "complete") {
  const representation = page.locator(`[data-preview-host="${host}"] [data-preview-representation="archive_tree"]`);
  await representation.waitFor({ state: "visible" });
  assert(await representation.getAttribute("data-preview-archive-state") === state, `${label}: ArchiveTree state mismatch`);
  assert(await representation.getAttribute("data-preview-archive-inspected") !== null, `${label}: archive inspected count missing`);
  assert(await representation.getAttribute("data-preview-archive-observed") !== null, `${label}: archive observed count missing`);
  assert(await representation.getAttribute("data-preview-selectable") === "false", `${label}: ArchiveTree became selectable`);
  assert(await representation.locator("a,button,input,select,textarea,img,video,audio,iframe,object,embed").count() === 0,
    `${label}: ArchiveTree mounted an interactive/resource element`);
  assert(await representation.locator("[data-preview-archive-kind]").count() <= 2_000, `${label}: ArchiveTree node cap exceeded`);
  assert(await representation.locator("[href],[src],[action]").count() === 0, `${label}: ArchiveTree exposed a resource attribute`);
  return representation;
}

export async function assertPreviewMetadataFallback(page, label) {
  await page.locator('[data-preview-metadata="true"]').waitFor({ state: "visible" });
  assert(await page.locator('[data-preview-representation="archive_tree"]').count() === 0, `${label}: corrupt archive published ArchiveTree`);
  assert(await page.locator('[data-preview-representation="folder_summary"]').count() === 0, `${label}: fallback published FolderSummary`);
}

export async function pinPreview(page, viewport, label, resolveStart = true) {
  const pin = page.locator('[data-preview-pin="true"]');
  await pin.waitFor({ state: "visible" });
  await page.waitForFunction(() => {
    const button = document.querySelector('[data-preview-pin="true"]');
    return button instanceof HTMLButtonElement && !button.disabled;
  });
  await pin.click();
  await page.waitForFunction(() => document.querySelector('[data-preview-host="zen-pinned"]') !== null
    && document.querySelectorAll('[data-preview-shell="true"]').length === 1
    && document.querySelector('[data-preview-host="zen-floating"]') === null);
  assert(await page.locator('[data-file-library-context-content]').count() === 0, `${label}: pinned handoff left a Context panel mounted`);
  assert(await page.locator('[data-side-sheet="true"]').count() === 0, `${label}: pinned handoff left a modal SideSheet mounted`);
  assert(await page.locator('[data-modal-layer="true"]').count() === 0, `${label}: pinned Preview opened a modal layer`);
  assert(await page.locator('[data-preview-host="zen-pinned"] [data-preview-card="true"]').count() === 1, `${label}: pinned card was not mounted in the shared surface`);
  if (resolveStart && (await page.evaluate(() => window.__zcW302?.pendingStartCount ?? 0)) > 0) await resolveDeferredPreview(page, `${label} staged Pinned Preview`);
}

export async function unpinPreview(page, label) {
  await page.locator('[data-preview-pin="true"][data-preview-pin-state="pinned"]').click();
  await page.waitForFunction(() => document.querySelector('[data-preview-host="zen-pinned"]') === null
    && document.querySelector('[data-preview-host="zen-floating"]') !== null
    && document.querySelectorAll('[data-preview-shell="true"]').length === 1);
  assert(await page.locator('[data-file-library-context-content]').count() === 0, `${label}: Unpin left a Context panel mounted`);
}

export async function closeFloating(page, label) {
  await page.keyboard.press("Escape");
  await page.waitForSelector('[data-preview-shell="true"]', { state: "detached" });
  assert(await page.locator('[data-preview-shell="true"]').count() === 0, `${label}: Preview remained mounted`);
}

export async function assertPreviewSecurity(page, label, allowBlob = false) {
  const attributes = await page.locator('[data-preview-content] [href], [data-preview-content] [src], [data-preview-content] [action]')
    .evaluateAll((elements) => elements.map((element) => ({
      tag: element.tagName,
      value: element.getAttribute("href") ?? element.getAttribute("src") ?? element.getAttribute("action") ?? ""
    })));
  const violations = allowBlob ? attributes.filter(({ value }) => !value.startsWith("blob:")) : attributes;
  assert(violations.length === 0, `${label}: resource-bearing Preview attributes ${JSON.stringify(violations)}`);
  assert(await page.locator('[data-preview-content] script, [data-preview-content] iframe, [data-preview-content] object, [data-preview-content] embed').count() === 0,
    `${label}: executable/resource element mounted`);
  if (!allowBlob) {
    assert(await page.locator('[data-preview-content] img').count() === 0, `${label}: textual Preview mounted an image`);
  }
}
