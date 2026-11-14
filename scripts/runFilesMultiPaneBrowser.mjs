/** Files #553 browser fixture evidence only; native GUI belongs to #487. */
import { chromium } from 'playwright';
import { createServer } from 'vite';
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

/** v3.40 pane controls live in one menu; these helpers keep the gates readable. */
async function setPaneCount(page, count) {
  const menu = page.locator('[data-pane-menu]');
  if ((await menu.locator('.wb-pane-menu-trigger').getAttribute('aria-expanded')) !== 'true') {
    await menu.locator('.wb-pane-menu-trigger').click();
  }
  await menu.getByRole('button', { name: `Pane \u00d7 ${count}`, exact: true }).click();
}
async function readPaneCount(page) {
  return page.locator('[data-pane-menu] .wb-pane-menu-trigger').getAttribute('data-pane-count');
}
async function openPaneMenu(page) {
  const menu = page.locator('[data-pane-menu]');
  if ((await menu.locator('.wb-pane-menu-trigger').getAttribute('aria-expanded')) !== 'true') {
    await menu.locator('.wb-pane-menu-trigger').click();
  }
  return menu;
}
async function focusPane(page, index) {
  const menu = await openPaneMenu(page);
  await menu.getByRole('button', { name: `Pane ${index}`, exact: true }).click();
}
async function setDestinationPane(page, paneId) {
  const menu = await openPaneMenu(page);
  // The destination picker keeps the menu open so the transfer action in the
  // same menu applies to it; choosing that action dismisses the menu.
  await menu.getByRole('combobox', { name: 'Transfer destination pane', exact: true }).selectOption(paneId);
}

const output = process.env.FILES_MULTIPANE_OUTPUT_DIR ?? path.join(tmpdir(), 'fylura-multipane');
await mkdir(output, { recursive: true });
const server = await createServer({ server: { host: '127.0.0.1', port: 0, strictPort: false } });
await server.listen();
const browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH } : {}) });
const page = await browser.newPage({ viewport: { width: 1800, height: 1100 } });
const errors = [];
page.on('pageerror', e => errors.push(e.message));
page.on('console', e => { if (e.type() === 'error')
    errors.push(e.text()); });
try {
    await page.addInitScript(() => { Object.defineProperty(navigator, 'platform', { get: () => 'Win32' }); localStorage.setItem('zc-onboarding-complete', 'true'); localStorage.setItem('zc-language', 'en'); });
    await page.goto(server.resolvedUrls.local[0] + '?browser-mock=1&u3-folder-move=1');
    await page.getByRole('button', { name: 'Files', exact: true }).click();
    await page.locator('[data-workbench-ready="true"]').waitFor();
    assert.equal(await page.title(), 'Fylura');
    await setPaneCount(page, 4);
    const panes = page.locator('[role="tabpanel"]:not([hidden]) [data-pane-id]');
    for (let i = 0; i < 4; i++) {
        const pane = panes.nth(i);
        await pane.getByRole('button', { name: 'Edit path (Ctrl/Cmd+L)', exact: true }).click();
        await pane.getByRole('textbox', { name: 'Folder path', exact: true }).fill(i === 0 ? 'C:/U3/Left' : i === 1 ? 'C:/U3/Right' : `C:/Pane${i + 1}`);
        await pane.getByRole('textbox', { name: 'Folder path', exact: true }).press('Enter');
        await pane.locator('[data-file-entry-kind]').first().waitFor();
    }
    assert.equal(await panes.count(), 4);
    await panes.first().locator('[data-file-library-query-menu] > summary').click();
    await panes.first().locator('[data-browse-sort]').selectOption('size');
    await panes.first().locator('[data-file-library-query-menu] > summary').click();
    await panes.nth(2).locator('[data-file-entry-name="notes.md"]').click();
    assert.equal(await panes.nth(2).locator('[aria-selected="true"]').count(), 1);
    await setDestinationPane(page, await panes.nth(3).getAttribute('data-pane-id'));
    await page.getByRole('button', { name: 'Copy to other pane', exact: true }).click();
    await page.getByText('Operation results · 1/1', { exact: true }).first().waitFor();
    await panes.nth(2).locator('[data-file-entry-name="notes.md"]').click();
    await page.screenshot({ path: path.join(output, 'four-pane.png') });
    assert.equal(await page.locator('.wb-layout[data-rows="2"]').count(), 1);
    await page.getByRole('separator', { name: 'Resize panes 1', exact: true }).first().focus();
    await page.keyboard.press('ArrowRight');
    await page.getByRole('button', { name: 'New tab', exact: true }).click();
    await setPaneCount(page, 3);
    await page.getByRole('tab').first().click();
    assert.equal(await readPaneCount(page), '4');
    await page.setViewportSize({ width: 800, height: 800 });
    await page.waitForTimeout(200);
    assert.equal(await page.locator('[data-pane-id]:visible').count(), 1);
    await focusPane(page, 3);
    assert.equal(await page.locator('[data-pane-id][data-focused="true"]:visible').count(), 1);
    await page.screenshot({ path: path.join(output, 'compact-pane.png') });
    await page.setViewportSize({ width: 1800, height: 1100 });
    await page.waitForTimeout(300);
    assert.equal(await page.locator('[data-pane-id]:visible').count(), 4);
    assert.equal(await panes.nth(2).locator('[aria-selected="true"]').count(), 1);
    await page.reload();
    await page.locator('[data-workbench-ready="true"]').waitFor();
    assert.equal(await readPaneCount(page), '4');
    assert.equal(await page.getByRole('tab').count(), 2);
    assert.equal(await panes.first().locator('[data-browse-sort]').inputValue(), 'size');
    await panes.first().getByRole('button', { name: 'C:/U3/Left', exact: true }).waitFor();
    await page.waitForTimeout(300);
    assert.equal(await page.locator('[data-pane-id]:visible').count(), 4);
    // A real legacy stored session keeps its hidden second pane through migration.
    const legacy = await page.evaluate(() => {
        const saved = JSON.parse(localStorage.getItem('zen-canvas:file-library-workbench:v1'));
        const tab = saved.tabs[0];
        return { version: 1, activeTabIndex: 0,
            tabs: [{ id: tab.id, panes: tab.panes.slice(0, 2), activePaneIndex: 0, dual: false }],
        };
    });
    await page.addInitScript((saved) => localStorage.setItem('zen-canvas:file-library-workbench:v1', JSON.stringify(saved)), legacy);
    await page.reload();
    await page.locator('[data-workbench-ready="true"]').waitFor();
    assert.equal(await readPaneCount(page), '1');
    await setPaneCount(page, 2);
    await panes.nth(1).getByRole('button', { name: 'C:/U3/Right', exact: true }).waitFor();
    console.log(JSON.stringify({ ok: true, output, checks: ['four independent navigation paths', 'pane-local sorting', 'explicit pane 3→4 copy', 'separator keyboard resize', 'per-tab count', 'compact selection retention', 'restart paths/sort/splits', 'legacy single with hidden second pane'], errors }));
    assert.deepEqual(errors, []);
}
catch (e) {
    await page.screenshot({ path: path.join(output, 'failure.png') });
    console.log((await page.locator('body').innerText()).slice(0, 2500));
    throw e;
}
finally {
    await browser.close();
    await server.close();
}
