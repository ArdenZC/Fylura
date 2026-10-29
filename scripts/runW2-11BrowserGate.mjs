import { mkdir, rm, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { assertCheckoutEvidence } from "./ciEvidence.mjs";
import { chromium } from "playwright";
import { createServer } from "vite";

const SOURCE_HEAD = process.env.W211_SOURCE_HEAD ?? process.env.GITHUB_SHA ?? "local";
const EXPECTED_CHECKOUT_SHA = process.env.W211_EXPECTED_CHECKOUT_SHA ?? null;
const ACTUAL_CHECKOUT_SHA = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
const ACTUAL_CHECKOUT_TREE = execFileSync("git", ["rev-parse", "HEAD^{tree}"], { encoding: "utf8" }).trim();
if (EXPECTED_CHECKOUT_SHA) assertCheckoutEvidence(EXPECTED_CHECKOUT_SHA, ACTUAL_CHECKOUT_SHA);

const FIXTURE_QUERY = "w2-11-browser-fixture=integrated";
const LIBRARY_TOTAL = 100_000;
const BROWSE_TOTAL = 100_000;
const BROWSE_SCAN_BUDGET = 1_024;
const BROWSE_LATE_SENTINEL_INDEX = 99_000;
const MAX_MOUNTED_CELLS = 240;
const MAX_FAR_DRAG_PAGE_DELTA = 1;
const PLATEAU_CYCLE_COUNT = 8;
const PLATEAU_IGNORED_CYCLES = 2;
const PLATEAU_MAX_CONSECUTIVE_POSITIVE_LISTENER_DELTAS = 2;
const PLATEAU_MAX_LATER_LISTENER_SPREAD = 48;
const PLATEAU_MAX_LATER_LISTENER_NET_INCREASE = 32;
const PLATEAU_SETTLE_MS = 250;
const TASK_TEMP_DIR = path.resolve(".tmp-tests/w2-11-browser-runtime");
const ARTIFACT_DIR = path.resolve(".tmp-tests/w2-11-browser-gate");
const MAX_NETWORK_DIAGNOSTIC_ITEMS = 64;

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

const browser = await chromium.launch({
  headless: true,
  ...(process.env.W211_CHROMIUM_EXECUTABLE ? { executablePath: process.env.W211_CHROMIUM_EXECUTABLE } : {})
});

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function sanitizeRequestUrl(value) {
  if (!value) return "<no-url>";
  try {
    const url = new URL(value);
    url.username = "";
    url.password = "";
    url.search = "";
    url.hash = "";
    if (url.protocol === "data:" || url.protocol === "blob:") return `${url.protocol}<inline-resource>`;
    if (url.protocol === "file:") {
      const fileName = url.pathname.split("/").filter(Boolean).at(-1) ?? "<resource>";
      return `file:///<local-path>/${fileName.slice(-128)}`;
    }
    const localHost = ["localhost", "127.0.0.1", "[::1]"].includes(url.hostname);
    if (localHost) {
      let pathname = url.pathname;
      try { pathname = decodeURIComponent(pathname); } catch {}
      const localFilePath = pathname.match(/(?:^|\/)(?:@fs\/)?(?:[A-Z]:\/|Users\/|home\/|root\/)(.*)$/iu);
      if (localFilePath) {
        const fileName = localFilePath[1].split(/[\\/]/u).filter(Boolean).at(-1) ?? "<resource>";
        return `${url.origin}/<local-path>/${fileName.slice(-128)}`;
      }
    }
    if (!localHost) {
      const resourceName = url.pathname.split("/").filter(Boolean).at(-1) ?? "<resource>";
      return `${url.protocol}//${url.host}/<external-resource>/${resourceName.slice(-128)}`;
    }
    return `${url.origin}${url.pathname}`.slice(0, 512);
  } catch {
    return "<unparseable-url>";
  }
}

function sanitizeDiagnostic(value) {
  return String(value)
    .replace(/https?:\/\/[^\s"'<>]+/gu, (url) => sanitizeRequestUrl(url))
    .replace(/((?:authorization|token|password|cookie|secret)\s*[:=]\s*)\S+/giu, "$1<redacted>")
    .replace(/(?:[A-Z]:\\|\/Users\/|\/home\/|\/root\/)[^\s"'<>]*/giu, "<local-path>")
    .slice(0, 512);
}

function pushBoundedDiagnostic(diagnostics, key, value) {
  const list = diagnostics[key];
  if (list.length < MAX_NETWORK_DIAGNOSTIC_ITEMS) list.push(value);
  else diagnostics[`${key}Dropped`] += 1;
}

async function installInstrumentation(context) {
  await context.addInitScript(() => {
    localStorage.setItem("zc-onboarding-complete", "true");
    localStorage.setItem("zc-language", "en");
    const state = {
      ...{
        browsePageCalls: 0,
        browsePageLengths: [],
        browseScanEnds: [],
        browseQueries: [],
        browseFirstPageSnapshots: [],
        browseSessionsCreated: 0,
        browseSessionsDisposed: 0,
        libraryPageCalls: 0,
        libraryPageLengths: [],
        libraryQueries: [],
        libraryFingerprints: [],
        librarySnapshotRevisions: [],
        selectionSummaries: [],
        thumbnailRequests: 0,
        thumbnailCancels: 0,
        activeThumbnailRequests: 0,
        thumbnailVariants: []
      },
      lifecycle: {
        listenerAdds: 0,
        listenerRemoves: 0,
        durableListenerAdds: 0,
        durableListenerRemoves: 0,
        resizeObservers: 0,
        resizeObserverCreates: 0,
        resizeObserverDisconnects: 0,
        mutationObservers: 0,
        intersectionObservers: 0,
        activeTimers: 0,
        objectUrlsCreated: 0,
        objectUrlsRevoked: 0
      }
    };
    Object.defineProperty(window, "__zcW211", { value: state, writable: true, configurable: true });
    const lifecycle = state.lifecycle;
    const originalAdd = EventTarget.prototype.addEventListener;
    const originalRemove = EventTarget.prototype.removeEventListener;
    // Keep all registrations as raw observation data, but use only durable
    // global targets for the growth signal. React 19's non-delegated image
    // listeners and the virtualizer's element listeners are attached to
    // ephemeral DOM nodes during each deterministic view cycle. This does
    // not attempt an exact active-listener shim or alter native semantics.
    const isDurableListenerTarget = (target) => target === window
      || target === document
      || (typeof MediaQueryList === "function" && target instanceof MediaQueryList);
    EventTarget.prototype.addEventListener = function (...args) {
      lifecycle.listenerAdds += 1;
      if (isDurableListenerTarget(this)) lifecycle.durableListenerAdds += 1;
      return originalAdd.apply(this, args);
    };
    EventTarget.prototype.removeEventListener = function (...args) {
      lifecycle.listenerRemoves += 1;
      if (isDurableListenerTarget(this)) lifecycle.durableListenerRemoves += 1;
      return originalRemove.apply(this, args);
    };

    const wrapObserverMethods = (name, counter) => {
      const Original = window[name];
      if (typeof Original !== "function" || !Original.prototype?.observe || !Original.prototype?.disconnect) return;
      const originalObserve = Original.prototype.observe;
      const originalDisconnect = Original.prototype.disconnect;
      const originalUnobserve = Original.prototype.unobserve;
      const observedTargets = new WeakMap();
      const adjustActive = (delta) => {
        if (delta > 0) {
          lifecycle[counter] += 1;
          if (counter === "resizeObservers") lifecycle.resizeObserverCreates += 1;
        } else {
          lifecycle[counter] = Math.max(0, lifecycle[counter] - 1);
          if (counter === "resizeObservers") lifecycle.resizeObserverDisconnects += 1;
        }
      };
      Original.prototype.observe = function (...args) {
        const result = originalObserve.apply(this, args);
        const target = args[0];
        if (target && typeof target === "object") {
          let targets = observedTargets.get(this);
          if (!targets) {
            targets = new Set();
            observedTargets.set(this, targets);
          }
          if (targets.size === 0) adjustActive(1);
          targets.add(target);
        }
        return result;
      };
      if (typeof originalUnobserve === "function") {
        Original.prototype.unobserve = function (...args) {
          const result = originalUnobserve.apply(this, args);
          const targets = observedTargets.get(this);
          if (targets) {
            targets.delete(args[0]);
            if (targets.size === 0) adjustActive(-1);
          }
          return result;
        };
      }
      Original.prototype.disconnect = function (...args) {
        const targets = observedTargets.get(this);
        if (targets?.size) {
          targets.clear();
          adjustActive(-1);
        }
        return originalDisconnect.apply(this, args);
      };
    };
    wrapObserverMethods("ResizeObserver", "resizeObservers");
    wrapObserverMethods("MutationObserver", "mutationObservers");
    wrapObserverMethods("IntersectionObserver", "intersectionObservers");

    const originalSetTimeout = window.setTimeout.bind(window);
    const originalClearTimeout = window.clearTimeout.bind(window);
    const activeTimers = new Set();
    window.setTimeout = (callback, delay, ...args) => {
      let timerId;
      timerId = originalSetTimeout(() => {
        activeTimers.delete(timerId);
        lifecycle.activeTimers = activeTimers.size;
        callback(...args);
      }, delay);
      activeTimers.add(timerId);
      lifecycle.activeTimers = activeTimers.size;
      return timerId;
    };
    window.clearTimeout = (timerId) => {
      activeTimers.delete(timerId);
      lifecycle.activeTimers = activeTimers.size;
      return originalClearTimeout(timerId);
    };

    if (typeof URL.createObjectURL === "function" && typeof URL.revokeObjectURL === "function") {
      const originalCreate = URL.createObjectURL.bind(URL);
      const originalRevoke = URL.revokeObjectURL.bind(URL);
      // Keep raw Blob URLs only inside this browser context. Evidence contains
      // bounded numeric IDs/timestamps, never the Blob URL or file content.
      const createdUrls = new Map();
      const maxRetainedUrls = 2048;
      let nextUrlId = 0;
      window.__zcW211TraceBlobUrls = (urls) => urls.slice(-16).map((url) => {
        const row = createdUrls.get(url);
        return row ? { matched: true, ...row } : { matched: false };
      });
      URL.createObjectURL = (value) => {
        const url = originalCreate(value);
        lifecycle.objectUrlsCreated += 1;
        createdUrls.set(url, {
          id: ++nextUrlId,
          createdMs: Math.round(performance.now()),
          revokedMs: null,
          byteSize: value instanceof Blob ? value.size : null,
          mimeType: value instanceof Blob ? value.type : null
        });
        if (createdUrls.size > maxRetainedUrls) {
          createdUrls.delete(createdUrls.keys().next().value);
        }
        return url;
      };
      URL.revokeObjectURL = (url) => {
        lifecycle.objectUrlsRevoked += 1;
        const row = createdUrls.get(url);
        if (row && row.revokedMs === null) row.revokedMs = Math.round(performance.now());
        return originalRevoke(url);
      };
    }
  });
}

async function readStats(page) {
  return page.evaluate(() => JSON.parse(JSON.stringify(window.__zcW211 ?? {})));
}

async function readResourceSnapshot(page) {
  return page.evaluate(() => {
    const state = window.__zcW211 ?? {};
    const lifecycle = state.lifecycle ?? {};
    return {
      domNodes: document.querySelectorAll("*").length,
      listenerAdds: lifecycle.listenerAdds ?? 0,
      listenerRemoves: lifecycle.listenerRemoves ?? 0,
      listenerNet: (lifecycle.listenerAdds ?? 0) - (lifecycle.listenerRemoves ?? 0),
      durableListenerAdds: lifecycle.durableListenerAdds ?? 0,
      durableListenerRemoves: lifecycle.durableListenerRemoves ?? 0,
      durableListenerNet: (lifecycle.durableListenerAdds ?? 0) - (lifecycle.durableListenerRemoves ?? 0),
      resizeObservers: lifecycle.resizeObservers ?? 0,
      resizeObserverCreates: lifecycle.resizeObserverCreates ?? 0,
      resizeObserverDisconnects: lifecycle.resizeObserverDisconnects ?? 0,
      mutationObservers: lifecycle.mutationObservers ?? 0,
      intersectionObservers: lifecycle.intersectionObservers ?? 0,
      activeTimers: lifecycle.activeTimers ?? 0,
      objectUrlsCreated: lifecycle.objectUrlsCreated ?? 0,
      objectUrlsRevoked: lifecycle.objectUrlsRevoked ?? 0,
      activeThumbnailRequests: state.activeThumbnailRequests ?? 0
    };
  });
}

async function assertNoHorizontalOverflow(page, label) {
  const overflow = await page.evaluate(() => ({
    document: document.documentElement.scrollWidth > window.innerWidth + 1,
    body: document.body.scrollWidth > window.innerWidth + 1,
    workspaces: [...document.querySelectorAll(".file-library-workspace")]
      .some((element) => element.scrollWidth > element.clientWidth + 1),
    grids: [...document.querySelectorAll('[data-shared-file-grid="true"]')]
      .some((element) => element.scrollWidth > element.clientWidth + 1)
  }));
  assert(!overflow.document && !overflow.body && !overflow.workspaces && !overflow.grids,
    `${label}: horizontal overflow ${JSON.stringify(overflow)}`);
  return overflow;
}

async function openLibrary(page) {
  await page.getByRole("button", { name: "Files", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
  const allIndexedFiles = page.getByRole("button", { name: "View all indexed files", exact: true });
  if (await allIndexedFiles.count() > 0 && await allIndexedFiles.first().isVisible()) {
    await allIndexedFiles.first().click();
  }
  const search = page.locator('[data-file-library-command-search="true"] [data-file-library-local-search="true"]');
  await search.waitFor({ state: "visible" });
  const list = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="library"]');
  await list.waitFor({ state: "visible" });
  await page.waitForFunction((total) => document.querySelector('[data-shared-file-list-source="library"]')
    ?.getAttribute("data-file-library-logical-count") === String(total), LIBRARY_TOTAL);
  return { search, list };
}

async function waitForLibraryCount(page, count) {
  await page.waitForFunction((expected) => document.querySelector('[data-shared-file-list-source="library"]')
    ?.getAttribute("data-file-library-logical-count") === String(expected), count);
}

async function switchView(page, view) {
  await page.getByRole("button", { name: view === "grid" ? "Grid" : "List", exact: true }).click();
  const selector = view === "grid"
    ? '[data-shared-file-grid="true"]'
    : '[data-shared-file-list="true"]';
  await page.locator(selector).filter({ visible: true }).first().waitFor({ state: "visible" });
}

async function closeContextPanel(page) {
  const compactContext = await page.evaluate(() => window.innerWidth < 1120);
  if (compactContext) {
    const sideSheet = page.locator('[data-side-sheet="true"]');
    if (await sideSheet.count() > 0) {
      await sideSheet.waitFor({ state: "visible" });
      await page.keyboard.press("Escape");
    } else {
      await page.locator('[data-file-library-context-toggle="true"]').click();
    }
  } else {
    await page.locator('[data-file-library-context-toggle="true"]').click();
  }
  await page.waitForFunction(() => document.querySelector('.file-library-workspace')
    ?.getAttribute("data-context-open") === "false");
  if (compactContext) {
    await page.waitForFunction(() => document.activeElement
      ?.matches('[data-file-library-context-toggle="true"]') === true);
  }
}

async function warmUpInteractionSurface(page, search, list) {
  await list.waitFor({ state: "visible" });
  await switchView(page, "grid");
  await switchView(page, "list");

  await search.fill("late");
  await waitForLibraryCount(page, 256);
  await search.fill("");
  await waitForLibraryCount(page, LIBRARY_TOTAL);

  const navigationToggle = page.locator('[data-file-library-nav-toggle="true"]');
  const compactNavigation = await page.evaluate(() => window.innerWidth < 1120);
  if (compactNavigation && await navigationToggle.count() > 0 && await navigationToggle.isVisible()) {
    await navigationToggle.click();
    await page.waitForFunction(() => document.querySelector('[data-file-library-nav-toggle="true"]')
      ?.getAttribute("aria-expanded") === "true");
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => document.querySelector('[data-file-library-nav-toggle="true"]')
      ?.getAttribute("aria-expanded") === "false");
  }

  const libraryContextToggle = page.locator('[data-file-library-context-toggle="true"]');
  await list.locator('[role="option"]').first().click();
  await libraryContextToggle.click();
  await page.waitForFunction(() => document.querySelector('.file-library-workspace')
    ?.getAttribute("data-context-open") === "true");
  await closeContextPanel(page);

  const libraryMenuSurface = page.locator('[data-shared-file-list-source="library"]');
  await libraryMenuSurface.locator('[role="option"]').first().click({ button: "right" });
  const libraryMenu = page.locator('[role="menu"][aria-label="File actions menu"]');
  await libraryMenu.waitFor({ state: "visible" });
  await page.keyboard.press("Escape");
  await libraryMenu.waitFor({ state: "detached" });

  await page.getByRole("tab", { name: "Browse Folder", exact: true }).click();
  const browseList = await openBrowseRoot(page);
  await switchView(page, "grid");
  await switchView(page, "list");
  const browseSearch = page.locator('[data-file-library-command-search="true"] [data-file-library-local-search="true"]');
  await browseSearch.fill("slow-b");
  await page.waitForFunction(() => document.querySelector('[data-browse-query="slow-b"]') !== null);
  await page.getByText(/slow-b-\d+\.txt/).first().waitFor({ state: "visible" });
  await browseSearch.fill("");
  await page.waitForTimeout(100);

  const browseContextToggle = page.locator('[data-file-library-context-toggle="true"]');
  await browseList.locator('[role="option"]').first().click();
  await browseContextToggle.click();
  await page.waitForFunction(() => document.querySelector('.file-library-workspace')
    ?.getAttribute("data-context-open") === "true");
  await closeContextPanel(page);

  await browseList.locator('[role="option"]').first().click({ button: "right" });
  const browseMenu = page.locator('[role="menu"][aria-label="Browse item menu"]');
  await browseMenu.waitFor({ state: "visible" });
  await page.keyboard.press("Escape");
  await browseMenu.waitFor({ state: "detached" });

  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await waitForLibraryCount(page, LIBRARY_TOTAL);
  await switchView(page, "list");
  const back = page.getByRole("button", { name: "Back", exact: true });
  const forward = page.getByRole("button", { name: "Forward", exact: true });
  assert(await back.isEnabled(), "Warm-up did not establish a usable Back history transition");
  await back.click();
  await page.waitForSelector('.file-library-workspace[data-mode="browse"]');
  assert(await forward.isEnabled(), "Warm-up did not retain a usable Forward history transition");
  await forward.click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
  await switchView(page, "list");
  await waitForLibraryCount(page, LIBRARY_TOTAL);
  await page.waitForTimeout(PLATEAU_SETTLE_MS);
}

async function runResourcePlateauCycle(page) {
  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
  const librarySearch = page.locator('.file-library-workspace[data-mode="library"] [data-file-library-local-search="true"]');
  await librarySearch.waitFor({ state: "visible" });
  await switchView(page, "list");
  await librarySearch.fill("");
  await waitForLibraryCount(page, LIBRARY_TOTAL);
  await switchView(page, "grid");

  await page.getByRole("tab", { name: "Browse Folder", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="browse"]');
  const browseSearch = page.locator('[data-file-library-command-search="true"] [data-file-library-local-search="true"]');
  await browseSearch.fill("");
  await page.waitForTimeout(50);
  await browseSearch.fill("slow-b");
  await page.waitForFunction(() => document.querySelector('[data-browse-query="slow-b"]') !== null);
  await page.getByText(/slow-b-\d+\.txt/).first().waitFor({ state: "visible" });
  await switchView(page, "list");
  await switchView(page, "grid");

  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
  await librarySearch.waitFor({ state: "visible" });
  await switchView(page, "list");
  await librarySearch.fill("");
  await waitForLibraryCount(page, LIBRARY_TOTAL);
  const back = page.getByRole("button", { name: "Back", exact: true });
  const forward = page.getByRole("button", { name: "Forward", exact: true });
  assert(await back.isEnabled(), "Plateau cycle did not establish a usable Back transition");
  await back.click();
  await page.waitForSelector('.file-library-workspace[data-mode="browse"]');
  assert(await forward.isEnabled(), "Plateau cycle did not retain a usable Forward transition");
  await forward.click();
  await page.waitForSelector('.file-library-workspace[data-mode="library"]');
  await librarySearch.waitFor({ state: "visible" });
  await switchView(page, "list");
  await librarySearch.fill("");
  await waitForLibraryCount(page, LIBRARY_TOTAL);
  await switchView(page, "list");
}

function evaluateResourcePlateau(cycles) {
  assert(cycles.length === PLATEAU_CYCLE_COUNT,
    `Resource plateau recorded ${cycles.length} cycles instead of ${PLATEAU_CYCLE_COUNT}`);
  const laterCycles = cycles.slice(PLATEAU_IGNORED_CYCLES);
  const listenerValues = laterCycles.map((cycle) => cycle.durableListenerNet);
  const listenerDeltas = listenerValues.slice(1).map((value, index) => value - listenerValues[index]);
  let currentPositiveRun = 0;
  let maxConsecutivePositiveDeltas = 0;
  for (const delta of listenerDeltas) {
    currentPositiveRun = delta > 0 ? currentPositiveRun + 1 : 0;
    maxConsecutivePositiveDeltas = Math.max(maxConsecutivePositiveDeltas, currentPositiveRun);
  }
  const listenerNetSpread = Math.max(...listenerValues) - Math.min(...listenerValues);
  const listenerNetIncrease = listenerValues.at(-1) - listenerValues[0];
  const evaluation = {
    ignoredCycleIndices: cycles.slice(0, PLATEAU_IGNORED_CYCLES).map((cycle) => cycle.cycleIndex),
    laterCycleIndices: laterCycles.map((cycle) => cycle.cycleIndex),
    listenerDeltas,
    listenerNetSpread,
    listenerNetIncrease,
    maxConsecutivePositiveDeltas,
    growthSignal: "durableListenerNet (window/document/MediaQueryList only); raw listenerNet remains observation data",
    passed: maxConsecutivePositiveDeltas <= PLATEAU_MAX_CONSECUTIVE_POSITIVE_LISTENER_DELTAS
      && listenerNetSpread <= PLATEAU_MAX_LATER_LISTENER_SPREAD
      && listenerNetIncrease <= PLATEAU_MAX_LATER_LISTENER_NET_INCREASE
  };
  assert(evaluation.passed,
    `Listener growth did not plateau: ${JSON.stringify({ evaluation, cycles })}`);
  return evaluation;
}

async function runResourcePlateau(page) {
  const cycles = [];
  for (let cycleIndex = 0; cycleIndex < PLATEAU_CYCLE_COUNT; cycleIndex += 1) {
    await runResourcePlateauCycle(page);
    await page.waitForTimeout(PLATEAU_SETTLE_MS);
    const resource = await readResourceSnapshot(page);
    cycles.push({
      cycleIndex,
      listenerAdds: resource.listenerAdds,
      listenerRemoves: resource.listenerRemoves,
      listenerNet: resource.listenerNet,
      durableListenerAdds: resource.durableListenerAdds,
      durableListenerRemoves: resource.durableListenerRemoves,
      durableListenerNet: resource.durableListenerNet,
      domNodes: resource.domNodes,
      resizeObservers: resource.resizeObservers,
      mutationObservers: resource.mutationObservers,
      intersectionObservers: resource.intersectionObservers,
      activeTimers: resource.activeTimers,
      activeThumbnails: resource.activeThumbnailRequests,
      liveObjectUrls: resource.objectUrlsCreated - resource.objectUrlsRevoked
    });
  }
  return {
    rule: {
      cycleCount: PLATEAU_CYCLE_COUNT,
      ignoredCycles: PLATEAU_IGNORED_CYCLES,
      maxConsecutivePositiveListenerDeltas: PLATEAU_MAX_CONSECUTIVE_POSITIVE_LISTENER_DELTAS,
      maxLaterListenerSpread: PLATEAU_MAX_LATER_LISTENER_SPREAD,
      maxLaterListenerNetIncrease: PLATEAU_MAX_LATER_LISTENER_NET_INCREASE,
      settleMs: PLATEAU_SETTLE_MS,
      growthSignal: "durableListenerNet",
      interpretation: "ignore initial settling cycles; durable global listener growth must not sustain positive linear growth or exceed the bounded spread/increase; raw listenerAdds/removes/net remains observation data; active-resource hard assertions remain in force"
    },
    cycles,
    evaluation: evaluateResourcePlateau(cycles)
  };
}

async function openBrowseRoot(page) {
  await page.getByRole("tab", { name: "Browse Folder", exact: true }).click();
  await page.waitForSelector('.file-library-workspace[data-mode="browse"]');
  const currentFolder = page.locator('[data-browse-state="current-folder"]');
  if (await currentFolder.count() === 0) {
    const openLocation = page.locator('[data-browse-location-action="open"]');
    await openLocation.first().click();
  }
  await currentFolder.waitFor({ state: "visible" });
  const list = page.locator('[data-shared-file-list="true"][data-shared-file-list-source="browse"]');
  await list.waitFor({ state: "visible" });
  return list;
}

async function browseLoadMoreButton(list) {
  return list.locator("xpath=..")
    .getByRole("button", { name: /Load/i })
    .first();
}

// Optional qualification instrumentation over the existing integrated fixture.
// The browser boundary is accepted dblclick -> visible target / child batch;
// real filesystem enumeration remains covered by Workspace Foundation.
async function qualifyBrowseTransition(page) {
  const list = await openBrowseRoot(page);
  await switchView(page, 'list');
  const search = page.locator('[data-file-library-local-search="true"]');
  await search.fill('');
  const samples = [];
  for (let index = 0; index < 23; index++) {
    const rootCrumb = page.locator('[data-browse-breadcrumbs="true"] button').first();
    if (await rootCrumb.isEnabled()) await rootCrumb.click();
    const child = list.locator('[role="option"]').filter({ hasText: 'w2-11-child-folder' }).first();
    await child.waitFor({ state: 'visible' });
    await page.evaluate(() => {
      const sample = { acceptedAt: null, feedbackMs: null, usefulMs: null };
      window.__zcQualificationBrowse = sample;
      const check = () => {
        if (sample.acceptedAt === null) return;
        const crumbs = document.querySelector('[data-browse-breadcrumbs="true"]');
        if (!crumbs?.textContent?.includes('w2-11-child-folder')) return;
        if (sample.feedbackMs === null) sample.feedbackMs = performance.now() - sample.acceptedAt;
        const workspace = document.querySelector('[data-browse-known-count="8"]');
        if (workspace?.querySelector('[data-shared-file-list-source="browse"] [role="option"]')) {
          sample.usefulMs = performance.now() - sample.acceptedAt;
          observer.disconnect();
        }
      };
      const observer = new MutationObserver(check);
      observer.observe(document.body, { subtree: true, childList: true, attributes: true });
      document.addEventListener('dblclick', () => { sample.acceptedAt = performance.now(); check(); }, { capture: true, once: true });
    });
    await child.dblclick();
    await page.waitForFunction(() => window.__zcQualificationBrowse?.usefulMs !== null);
    const sample = await page.evaluate(() => ({ ...window.__zcQualificationBrowse }));
    if (index >= 3) samples.push(sample);
  }
  const p95 = field => samples.map(sample => sample[field]).sort((a, b) => a - b)[Math.ceil((samples.length - 1) * 0.95)];
  // Restore the existing plateau scenario's root/query before its independent
  // resource lifecycle checks; timing setup is not part of that workload.
  await page.locator('[data-browse-breadcrumbs="true"] button').first().click();
  await search.fill('slow-b');
  await page.getByText(/slow-b-\d+\.txt/).first().waitFor({ state: 'visible' });
  return { measurementBoundary: 'browser_accepted_dblclick_to_visible_target_and_mock_entry_batch',
    warmupCount: 3, sampleCount: samples.length, feedbackP95Ms: p95('feedbackMs'),
    usefulP95Ms: p95('usefulMs'), feedbackTargetP95Ms: 100, usefulTargetP95Ms: 250, samples };
}

async function runIntegratedScene(viewport, deviceScaleFactor) {
  const context = await browser.newContext({ viewport, deviceScaleFactor });
  await installInstrumentation(context);
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  page.setDefaultNavigationTimeout(60_000);
  const networkDiagnostics = {
    pageUrl: "<not-navigated>",
    requestFailures: [],
    requestFailuresDropped: 0,
    consoleErrors: [],
    consoleErrorsDropped: 0,
    pageErrors: [],
    pageErrorsDropped: 0,
    responseStatuses: [],
    responseStatusesDropped: 0,
    responseStatusCounts: {}
  };
  const consoleErrors = networkDiagnostics.consoleErrors;
  const pageErrors = networkDiagnostics.pageErrors;
  const failedBlobUrls = new Set();
  const captureBlobLifecycle = async () => {
    if (failedBlobUrls.size === 0) return;
    const ids = [...failedBlobUrls].slice(-16);
    networkDiagnostics.blobLifecycle = await page.evaluate(
      (urls) => window.__zcW211TraceBlobUrls?.(urls) ?? urls.map(() => ({ matched: false })),
      ids
    );
    // IDs and timestamps are safe to emit. Never emit the raw blob: URL.
    console.log(`[w2-11-real] BLOB_LIFECYCLE ${JSON.stringify(networkDiagnostics.blobLifecycle)}`);
  };
  page.on("console", (message) => {
    if (message.type() !== "error") return;
    const location = message.location();
    if (location.url?.startsWith("blob:")) failedBlobUrls.add(location.url);
    pushBoundedDiagnostic(networkDiagnostics, "consoleErrors", {
      text: sanitizeDiagnostic(message.text()),
      location: {
        url: sanitizeRequestUrl(location.url),
        lineNumber: location.lineNumber,
        columnNumber: location.columnNumber
      }
    });
  });
  page.on("pageerror", (error) => {
    pushBoundedDiagnostic(networkDiagnostics, "pageErrors", sanitizeDiagnostic(error?.stack ?? error));
  });
  page.on("requestfailed", (request) => {
    if (request.url().startsWith("blob:")) failedBlobUrls.add(request.url());
    pushBoundedDiagnostic(networkDiagnostics, "requestFailures", {
      url: sanitizeRequestUrl(request.url()),
      method: sanitizeDiagnostic(request.method()),
      resourceType: sanitizeDiagnostic(request.resourceType()),
      failure: sanitizeDiagnostic(request.failure()?.errorText ?? "<unknown-failure>")
    });
  });
  page.on("response", (response) => {
    const status = response.status();
    const statusKey = String(status);
    networkDiagnostics.responseStatusCounts[statusKey] = (networkDiagnostics.responseStatusCounts[statusKey] ?? 0) + 1;
    if (status < 200 || status >= 300) {
      pushBoundedDiagnostic(networkDiagnostics, "responseStatuses", {
        url: sanitizeRequestUrl(response.url()),
        resourceType: sanitizeDiagnostic(response.request().resourceType()),
        status
      });
    }
  });
  const metrics = {
    viewport,
    deviceScaleFactor,
    sourceHead: SOURCE_HEAD,
    actualSha: ACTUAL_CHECKOUT_SHA,
    actualTree: ACTUAL_CHECKOUT_TREE,
    fixture: {
      seed: "w2-11-fixed-index-v1",
      libraryTotal: LIBRARY_TOTAL,
      browseTotal: BROWSE_TOTAL,
      browseScanBudget: BROWSE_SCAN_BUDGET,
      lateSentinelIndex: BROWSE_LATE_SENTINEL_INDEX
    },
    firstUsefulContentMs: {},
    virtualization: {},
    browse: {},
    history: {},
    resources: {},
    networkDiagnostics
  };

  try {
    await page.goto(`${baseUrl}?${FIXTURE_QUERY}`, { waitUntil: "commit" });
    networkDiagnostics.pageUrl = sanitizeRequestUrl(page.url());
    const libraryStart = Date.now();
    const { search, list } = await openLibrary(page);
    await list.locator('[role="option"]').first().waitFor({ state: "visible" });
    metrics.firstUsefulContentMs.library = Date.now() - libraryStart;
    assert(metrics.firstUsefulContentMs.library >= 0, "Library first useful content measurement was invalid");
    assert(await list.locator('[role="option"]').count() > 0, "Library did not expose a first usable row");
    await warmUpInteractionSurface(page, search, list);
    await page.waitForTimeout(150);
    const resourceBaseline = await readResourceSnapshot(page);
    metrics.resources.baseline = resourceBaseline;

    const initialListRows = await list.locator('[role="option"]').count();
    assert(initialListRows < MAX_MOUNTED_CELLS, `Library List mounted too many rows: ${initialListRows}`);
    await list.focus();
    await page.keyboard.press("Control+A");
    await page.waitForSelector('[data-library-selection-kind="all_matching"]');
    await page.waitForFunction(() => (window.__zcW211?.selectionSummaries ?? [])
      .some((summary) => summary.count === 100000));
    const selectionStats = await readStats(page);
    assert(selectionStats.selectionSummaries?.at(-1)?.count === LIBRARY_TOTAL,
      `all_matching summary was not exact 100k: ${JSON.stringify(selectionStats.selectionSummaries)}`);
    assert(selectionStats.selectionSummaries?.at(-1)?.queryFingerprint,
      "all_matching selection summary did not retain query fingerprint");
    assert(typeof selectionStats.selectionSummaries?.at(-1)?.snapshotRevision === "number",
      "all_matching selection summary did not retain snapshot revision");

    await switchView(page, "grid");
    const grid = page.locator('[data-shared-file-grid="true"][data-shared-file-grid-source="library"]');
    await page.waitForFunction((total) => document.querySelector('[data-shared-file-grid-source="library"]')
      ?.getAttribute("data-file-library-grid-logical-count") === String(total), LIBRARY_TOTAL);
    const initialGridCells = await grid.locator('[data-grid-cell="true"]').count();
    const initialGridRows = Number(await grid.getAttribute("data-file-library-grid-mounted-rows") ?? "0");
    assert(initialGridCells > 0 && initialGridCells < MAX_MOUNTED_CELLS,
      `Library Grid mounted-cell bound failed: ${initialGridCells}`);
    metrics.virtualization.library = { initialListRows, initialGridCells, initialGridRows };

    const statsBeforeFarDrag = await readStats(page);
    const pageCallsBeforeFarDrag = statsBeforeFarDrag.libraryPageCalls ?? 0;
    await grid.evaluate((element) => {
      element.scrollTop = 204 * 80_000;
      element.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await page.waitForTimeout(120);
    const statsAfterFarDrag = await readStats(page);
    const farDragDelta = (statsAfterFarDrag.libraryPageCalls ?? 0) - pageCallsBeforeFarDrag;
    assert(farDragDelta <= MAX_FAR_DRAG_PAGE_DELTA,
      `Library Grid far jump drained pages: before=${pageCallsBeforeFarDrag} after=${statsAfterFarDrag.libraryPageCalls}`);
    const scrolledGridCells = await grid.locator('[data-grid-cell="true"]').count();
    assert(scrolledGridCells < MAX_MOUNTED_CELLS, `Library Grid mounted too many scrolled cells: ${scrolledGridCells}`);

    await switchView(page, "list");
    await page.waitForSelector('[data-library-selection-kind="all_matching"]');
    await search.fill("late");
    await page.waitForFunction(() => document.querySelector('[data-shared-file-list-source="library"]')
      ?.getAttribute("data-file-library-logical-count") === "256");
    const filteredStats = await readStats(page);
    assert(filteredStats.libraryQueries?.includes("late"), "Library filter query was not observed by the fixture");
    assert(await page.locator('[data-library-selection-kind="none"]').count() === 1,
      "LibrarySelectionV1 all_matching survived a query boundary change");
    assert(new Set(filteredStats.libraryFingerprints ?? []).size >= 2,
      "Query V2 fingerprint did not change across the filter boundary");
    await search.fill("");
    await waitForLibraryCount(page, LIBRARY_TOTAL);

    await search.fill("slow-a");
    await page.waitForTimeout(350);
    await search.fill("slow-b");
    await page.waitForFunction(() => document.querySelector('[data-file-library-local-search="true"]')?.value === "slow-b");
    await page.waitForFunction(() => (window.__zcW211?.libraryQueries ?? []).includes("slow-b"));
    await page.waitForTimeout(500);
    const rapidLibraryRows = await list.locator('[role="option"]').allTextContents();
    assert(rapidLibraryRows.some((row) => row.toLowerCase().includes("slow-b")),
      `Library query B did not publish a usable row: ${JSON.stringify(rapidLibraryRows)}`);
    assert(!rapidLibraryRows.some((row) => row.toLowerCase().includes("slow-a")),
      "Stale Library query A published into query B");
    await search.fill("");
    await waitForLibraryCount(page, LIBRARY_TOTAL);

    const browseStart = Date.now();
    const browseList = await openBrowseRoot(page);
    await browseList.locator('[role="option"]').first().waitFor({ state: "visible" });
    metrics.firstUsefulContentMs.browse = Date.now() - browseStart;
    const initialBrowseCells = await browseList.locator('[role="option"]').count();
    assert(initialBrowseCells > 0 && initialBrowseCells < MAX_MOUNTED_CELLS,
      `Browse first page mounted too many rows: ${initialBrowseCells}`);
    const initialBrowseStats = await readStats(page);
    const ordinarySnapshot = (initialBrowseStats.browseFirstPageSnapshots ?? []).find((item) => item.query === "");
    const ordinaryCallsBeforeFarDrag = (initialBrowseStats.browseQueries ?? []).filter((query) => query === "").length;
    assert(ordinarySnapshot?.completion === "partial" && ordinarySnapshot.hasCursor,
      `Browse 100k did not publish a progressive partial first page: ${JSON.stringify(ordinarySnapshot)}`);
    assert(ordinarySnapshot.entries <= 32, `Browse first page exceeded page bound: ${ordinarySnapshot.entries}`);
    assert(Number(await browseList.getAttribute("data-browse-logical-count")) <= 32,
      "Browse exposed a non-authoritative large count before completion");

    await browseList.evaluate((element) => {
      element.scrollTop = 44 * 100_000;
      element.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await page.waitForTimeout(100);
    const afterBrowseFarDrag = await readStats(page);
    const ordinaryCallsAfterFarDrag = (afterBrowseFarDrag.browseQueries ?? []).filter((query) => query === "").length;
    const ordinaryCalls = ordinaryCallsAfterFarDrag - ordinaryCallsBeforeFarDrag;
    assert(ordinaryCalls <= 2, `Browse far jump requested unbounded pages: ${ordinaryCalls}`);

    await switchView(page, "grid");
    const browseGrid = page.locator('[data-shared-file-grid="true"][data-shared-file-grid-source="browse"]');
    const browseGridCells = await browseGrid.locator('[data-grid-cell="true"]').count();
    assert(browseGridCells > 0 && browseGridCells < MAX_MOUNTED_CELLS,
      `Browse Grid mounted too many cells: ${browseGridCells}`);
    const childFolder = page.getByRole("gridcell", { name: /w2-11-child-folder/ }).first();
    await childFolder.dblclick();
    await page.waitForFunction(() => document.querySelectorAll('[data-browse-breadcrumbs="true"] button').length >= 2);
    await page.waitForSelector('[data-browse-known-count="8"]');
    const childKnownCount = await page.locator('[data-browse-known-count]').getAttribute("data-browse-known-count").catch(() => null);
    assert(childKnownCount === "8", `Browse child did not restore exact known count: ${childKnownCount}`);
    const breadcrumbs = page.locator('[data-browse-breadcrumbs="true"] button');
    await breadcrumbs.first().click();
    await page.waitForFunction(() => document.querySelector('[data-browse-enumeration-completion]')?.getAttribute("data-browse-enumeration-completion") === "partial");

    await switchView(page, "list");
    const browseSearch = page.locator('[data-file-library-command-search="true"] [data-file-library-local-search="true"]');
    await browseSearch.fill("impossible-match");
    await page.waitForFunction(() => document.querySelector('[data-browse-query="impossible-match"]') !== null);
    await page.waitForFunction(() => document.querySelector('[data-browse-enumeration-completion]')
      ?.getAttribute("data-browse-enumeration-completion") === "complete");
    const impossibleStats = await readStats(page);
    const impossibleFirst = (impossibleStats.browseFirstPageSnapshots ?? [])
      .find((item) => item.query === "impossible-match");
    assert(impossibleFirst?.completion === "partial" && impossibleFirst.entries === 0 && impossibleFirst.hasCursor,
      `Impossible Browse query was not bounded/partial on its first response: ${JSON.stringify(impossibleFirst)}`);
    assert(impossibleStats.browseScanEnds?.filter((end, index) => impossibleStats.browseQueries?.[index] === "impossible-match")
      .every((end) => end <= BROWSE_TOTAL), "Impossible Browse scan crossed the logical fixture boundary");

    await browseSearch.fill("late-sentinel");
    await page.waitForFunction(() => document.querySelector('[data-browse-query="late-sentinel"]') !== null);
    await page.getByText(/late-sentinel-\d+\.txt/).first().waitFor({ state: "visible" });
    const lateStats = await readStats(page);
    const lateSnapshots = (lateStats.browseFirstPageSnapshots ?? []).filter((item) => item.query === "late-sentinel");
    assert(lateSnapshots.some((item) => item.completion === "partial" && item.entries === 0 && item.hasCursor),
      "Late-sentinel query did not show a bounded empty partial turn");
    assert(lateSnapshots.some((item) => item.completion === "partial" && item.entries === 1),
      "Late-sentinel query did not publish the sentinel progressively");
    const lateLoadMore = await browseLoadMoreButton(browseList);
    for (let turn = 0; turn < 3; turn += 1) {
      const completion = await page.locator('[data-browse-enumeration-completion]').getAttribute("data-browse-enumeration-completion");
      if (completion === "complete") break;
      await lateLoadMore.click();
      await page.waitForTimeout(60);
    }
    await page.waitForFunction(() => document.querySelector('[data-browse-enumeration-completion]')
      ?.getAttribute("data-browse-enumeration-completion") === "complete");
    assert(await page.locator('[data-browse-known-count="1"]').count() === 1,
      "Browse knownCount was not exact only at late-sentinel EOF");

    await browseSearch.fill("slow-a");
    await page.waitForTimeout(350);
    await browseSearch.fill("slow-b");
    await page.waitForFunction(() => document.querySelector('[data-browse-query="slow-b"]') !== null);
    await page.getByText(/slow-b-\d+\.txt/).first().waitFor({ state: "visible" });
    await page.waitForTimeout(600);
    const rapidBrowseRows = await page.locator('[data-shared-file-list-source="browse"] [role="option"]').allTextContents();
    assert(!rapidBrowseRows.some((row) => row.toLowerCase().includes("slow-a")),
      "Stale Browse query A published into query B");
    assert(rapidBrowseRows.some((row) => row.toLowerCase().includes("slow-b")),
      "Browse query B did not remain current after query A completed late");

    await page.getByRole("button", { name: "Grid", exact: true }).click();
    await page.locator('[data-shared-file-grid="true"][data-shared-file-grid-source="browse"]').waitFor({ state: "visible" });
    await page.getByRole("button", { name: "List", exact: true }).click();
    await page.getByRole("tab", { name: "Library", exact: true }).click();
    await waitForLibraryCount(page, LIBRARY_TOTAL);
    await page.getByRole("tab", { name: "Browse Folder", exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[data-browse-query]')?.getAttribute("data-browse-query") === "slow-b");
    const backButton = page.getByRole("button", { name: "Back", exact: true });
    const forwardButton = page.getByRole("button", { name: "Forward", exact: true });
    if (await backButton.isEnabled()) {
      await backButton.click();
      await page.waitForTimeout(100);
      if (await forwardButton.isEnabled()) await forwardButton.click();
      await page.waitForFunction(() => document.querySelector('.file-library-workspace[data-mode="browse"]') !== null);
    }
    metrics.history = { browseQueryAfterSwitch: await page.locator('[data-browse-query]').getAttribute("data-browse-query") };

    for (let cycle = 0; cycle < 3; cycle += 1) {
      await page.getByRole("button", { name: cycle % 2 ? "Grid" : "List", exact: true }).click();
      await page.getByRole("tab", { name: "Library", exact: true }).click();
      await waitForLibraryCount(page, LIBRARY_TOTAL);
      await page.getByRole("tab", { name: "Browse Folder", exact: true }).click();
      await page.waitForFunction(() => document.querySelector('.file-library-workspace[data-mode="browse"]') !== null);
    }
    if (process.env.ZC_BROWSE_QUALIFICATION_OUTPUT) metrics.browseQualification = await qualifyBrowseTransition(page);
    const resourcePlateau = await runResourcePlateau(page);
    const finalResources = await readResourceSnapshot(page);
    metrics.resources = {
      baseline: resourceBaseline,
      plateau: resourcePlateau,
      final: finalResources,
      stats: await readStats(page)
    };
    assert(finalResources.activeThumbnailRequests === 0,
      `Thumbnail work did not quiesce: ${JSON.stringify(finalResources)}`);
    assert(finalResources.domNodes <= resourceBaseline.domNodes + 400,
      `DOM grew beyond the fixed baseline tolerance: baseline=${resourceBaseline.domNodes} final=${finalResources.domNodes}`);
    assert(finalResources.resizeObservers <= resourceBaseline.resizeObservers + 6
      && finalResources.mutationObservers <= resourceBaseline.mutationObservers + 6
      && finalResources.intersectionObservers <= resourceBaseline.intersectionObservers + 6,
    `Observer counts grew beyond fixed baseline tolerance: baseline=${JSON.stringify(resourceBaseline)} final=${JSON.stringify(finalResources)}`);
    assert(finalResources.activeTimers <= resourceBaseline.activeTimers + 20,
      `Timer count grew beyond fixed baseline tolerance: baseline=${resourceBaseline.activeTimers} final=${finalResources.activeTimers}`);
    assert(finalResources.objectUrlsCreated - finalResources.objectUrlsRevoked < 40,
      `Object URL steady state exceeded fixed tolerance: ${JSON.stringify(finalResources)}`);
    assert((metrics.resources.stats.thumbnailRequests ?? 0) > 0,
      "Integrated stress scene did not exercise thumbnail requests");
    assert((metrics.resources.stats.thumbnailCancels ?? 0) > 0,
      "Integrated stress scene did not exercise thumbnail cancellation");
    assert((metrics.resources.stats.thumbnailVariants ?? []).length > 0,
      "Integrated stress scene did not record thumbnail variants");
    await assertNoHorizontalOverflow(page, `${viewport.width}x${viewport.height}@${deviceScaleFactor}`);
    await captureBlobLifecycle();
    assert(consoleErrors.length === 0 && pageErrors.length === 0,
      `Browser console/page errors: ${JSON.stringify({ consoleErrors, pageErrors })}`);
    console.log(`[w2-11-real] NETWORK ${viewport.width}x${viewport.height}@${deviceScaleFactor} ${JSON.stringify(networkDiagnostics)}`);
    return metrics;
  } catch (error) {
    networkDiagnostics.pageUrl = sanitizeRequestUrl(page.url());
    await captureBlobLifecycle().catch(() => undefined);
    metrics.failure = sanitizeDiagnostic(error?.stack ?? error);
    metrics.resources = await readResourceSnapshot(page).catch(() => ({}));
    metrics.fixtureStats = await readStats(page).catch(() => ({}));
    const artifactStem = `w2-11-${viewport.width}x${viewport.height}-dpr-${String(deviceScaleFactor).replace(".", "_")}`;
    await writeFile(path.join(ARTIFACT_DIR, `${artifactStem}.json`), JSON.stringify(metrics, null, 2), "utf8");
    await page.screenshot({ path: path.join(ARTIFACT_DIR, `${artifactStem}.png`), fullPage: false }).catch(() => undefined);
    throw error;
  } finally {
    await context.close();
  }
}

async function runDprProbe(viewport, deviceScaleFactor) {
  const context = await browser.newContext({ viewport, deviceScaleFactor });
  await installInstrumentation(context);
  const page = await context.newPage();
  page.setDefaultTimeout(60_000);
  try {
    await page.goto(`${baseUrl}?${FIXTURE_QUERY}`, { waitUntil: "commit" });
    await openLibrary(page);
    await page.getByRole("button", { name: "Grid", exact: true }).click();
    const grid = page.locator('[data-shared-file-grid="true"][data-shared-file-grid-source="library"]');
    await grid.waitFor({ state: "visible" });
    await page.waitForFunction((total) => document.querySelector('[data-shared-file-grid-source="library"]')
      ?.getAttribute("data-file-library-grid-logical-count") === String(total), LIBRARY_TOTAL);
    const mountedCells = await grid.locator('[data-grid-cell="true"]').count();
    const columns = Number(await grid.getAttribute("data-file-library-grid-columns") ?? "0");
    assert(columns > 0, `DPR probe did not calculate grid columns at ${deviceScaleFactor}`);
    assert(mountedCells > 0 && mountedCells < MAX_MOUNTED_CELLS,
      `DPR probe mounted-cell bound failed at ${deviceScaleFactor}: ${mountedCells}`);
    const stats = await readStats(page);
    assert((stats.thumbnailVariants ?? []).length > 0, `DPR probe did not request a thumbnail at ${deviceScaleFactor}`);
    const overflow = await assertNoHorizontalOverflow(page, `${viewport.width}x${viewport.height}@${deviceScaleFactor}`);
    return { viewport, deviceScaleFactor, columns, mountedCells, variants: stats.thumbnailVariants, overflow };
  } finally {
    await context.close();
  }
}

let passed = false;
try {
  const results = [
    await runIntegratedScene({ width: 1600, height: 900 }, 1),
    await runIntegratedScene({ width: 980, height: 680 }, 1),
    await runDprProbe({ width: 1600, height: 900 }, 1.25),
    await runDprProbe({ width: 980, height: 680 }, 2)
  ];
  passed = true;
  if (process.env.ZC_BROWSE_QUALIFICATION_OUTPUT) {
    const output = path.resolve(process.env.ZC_BROWSE_QUALIFICATION_OUTPUT);
    assert(output.startsWith(path.resolve('.performance-artifacts/qualification') + path.sep), 'Qualification output escapes evidence boundary');
    await mkdir(path.dirname(output), { recursive: true });
    await writeFile(output, JSON.stringify({ sourceHead: SOURCE_HEAD, actualSha: ACTUAL_CHECKOUT_SHA, actualTree: ACTUAL_CHECKOUT_TREE,
      evidence: results.slice(0, 2).map(result => ({ viewport: result.viewport, ...result.browseQualification })) }, null, 2));
  }
  const integratedSummary = results.slice(0, 2).map((result) => {
    const stats = result.resources.stats;
    return {
      viewport: result.viewport,
      deviceScaleFactor: result.deviceScaleFactor,
      firstUsefulContentMs: result.firstUsefulContentMs,
      virtualization: result.virtualization,
      resources: {
        baseline: result.resources.baseline,
        plateau: result.resources.plateau,
        final: result.resources.final
      },
      fixture: {
        libraryPageCalls: stats.libraryPageCalls,
        libraryPageLengths: stats.libraryPageLengths,
        browsePageCalls: stats.browsePageCalls,
        maxBrowsePageLength: Math.max(0, ...(stats.browsePageLengths ?? [])),
        maxBrowseScanEnd: Math.max(0, ...(stats.browseScanEnds ?? [])),
        browseSessionsCreated: stats.browseSessionsCreated,
        browseSessionsDisposed: stats.browseSessionsDisposed,
        thumbnailRequests: stats.thumbnailRequests,
        thumbnailCancels: stats.thumbnailCancels,
        activeThumbnailRequests: stats.activeThumbnailRequests,
        thumbnailVariants: [...new Set(stats.thumbnailVariants ?? [])]
      }
    };
  });
  const plateauSummary = results.slice(0, 2).map((result) => ({
    viewport: result.viewport,
    rule: result.resources.plateau.rule,
    cycles: result.resources.plateau.cycles,
    evaluation: result.resources.plateau.evaluation
  }));
  console.log(`[w2-11-real] PLATEAU ${JSON.stringify(plateauSummary)}`);
  console.log(`[w2-11-real] METRICS ${JSON.stringify({ integrated: integratedSummary, dpr: results.slice(2) })}`);
  for (const result of results) {
    console.log(`[w2-11-real] PASS ${result.viewport.width}x${result.viewport.height}@${result.deviceScaleFactor} sourceHead=${result.sourceHead ?? SOURCE_HEAD} actualSha=${result.actualSha ?? ACTUAL_CHECKOUT_SHA} tree=${result.actualTree ?? ACTUAL_CHECKOUT_TREE}`);
  }
} finally {
  await browser.close();
  await server.close();
  await rm(TASK_TEMP_DIR, { recursive: true, force: true });
  if (passed) await rm(ARTIFACT_DIR, { recursive: true, force: true });
}
