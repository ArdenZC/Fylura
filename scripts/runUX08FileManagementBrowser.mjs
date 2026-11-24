import assert from 'node:assert/strict';
import { chromium } from 'playwright';
import { createServer } from 'vite';

/** Browser-mock workflow evidence for Files search, transfers, undo and History restore. */
const server = await createServer({ server: { host: '127.0.0.1', port: 0, strictPort: false } });
await server.listen();
const browser = await chromium.launch({ headless: true, executablePath: '/usr/bin/chromium' });
const page = await browser.newPage({ viewport: { width: 1500, height: 900 } });
const errors = [];
page.on('pageerror', error => errors.push(error.message));
page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
async function setPaneCount(count) {
  const menu = page.locator('[data-pane-menu]');
  if ((await menu.locator('.wb-pane-menu-trigger').getAttribute('aria-expanded')) !== 'true') await menu.locator('.wb-pane-menu-trigger').click();
  await menu.getByRole('button', { name: `Pane × ${count}`, exact: true }).click();
}
async function focusPane(index) {
  const menu = page.locator('[data-pane-menu]');
  if ((await menu.locator('.wb-pane-menu-trigger').getAttribute('aria-expanded')) !== 'true') await menu.locator('.wb-pane-menu-trigger').click();
  await menu.getByRole('button', { name: `Pane ${index}`, exact: true }).click();
}
async function setDestinationPane(paneId) {
  const menu = page.locator('[data-pane-menu]');
  if ((await menu.locator('.wb-pane-menu-trigger').getAttribute('aria-expanded')) !== 'true') await menu.locator('.wb-pane-menu-trigger').click();
  await menu.getByRole('combobox', { name: 'Transfer destination pane', exact: true }).selectOption(paneId);
}
async function openPath(pane, path) {
  await pane.getByRole('button', { name: 'Edit path (Ctrl/Cmd+L)', exact: true }).click();
  await pane.getByRole('textbox', { name: 'Folder path', exact: true }).fill(path);
  await pane.getByRole('textbox', { name: 'Folder path', exact: true }).press('Enter');
  await pane.locator('[data-file-entry-kind]').first().waitFor();
}
try {
  await page.addInitScript(() => { Object.defineProperty(navigator, 'platform', { get: () => 'Win32' }); localStorage.setItem('zc-onboarding-complete', 'true'); localStorage.setItem('zc-language', 'en'); });
  await page.goto(`${server.resolvedUrls.local[0]}?browser-mock=1`);
  await page.getByRole('button', { name: 'Files', exact: true }).click();
  await page.locator('[data-workbench-ready="true"]').waitFor();
  await setPaneCount(2);
  const panes = page.locator('[role="tabpanel"]:not([hidden]) [data-pane-id]');
  assert.equal(await panes.count(), 2);
  await openPath(panes.nth(0), 'C:/U1/Source');
  await openPath(panes.nth(1), 'C:/U1/Target');
  console.log('opened source and target');
  const folderSearch = panes.nth(0).getByRole('textbox', { name: 'Search this folder', exact: true });
  await folderSearch.fill('one.txt');
  await panes.nth(0).locator('[data-file-entry-name="one.txt"]').waitFor();
  assert.equal(await panes.nth(0).locator('[data-file-entry-name]').count(), 1);
  await folderSearch.fill('');
  await panes.nth(0).locator('[data-file-entry-name="two.txt"]').waitFor();
  console.log('folder search filters the source path');

  await panes.nth(0).locator('[data-file-entry-name="one.txt"]').click();
  await setDestinationPane(page, await panes.nth(1).getAttribute('data-pane-id'));
  await page.getByRole('button', { name: 'Copy to other pane', exact: true }).click();
  const conflict = page.locator('[data-fm-dialog="conflict"]');
  await conflict.waitFor();
  await conflict.getByRole('button', { name: 'Keep both', exact: true }).click();
  await conflict.getByRole('button', { name: 'Apply choices and continue', exact: true }).click();
  await page.getByText(/Operation (finished|results)/).first().waitFor();
  await panes.nth(1).locator('[data-file-entry-name="one (2).txt"]').waitFor();
  console.log('conflict resolved with Keep Both');

  const operationResults = page.getByRole('button', { name: /Operation results/ });
  await operationResults.click();
  await page.locator('.fm-results strong').filter({ hasText: 'one (2).txt' }).waitFor();
  console.log('operation results show the completed copy');
  await operationResults.click();

  await panes.nth(0).locator('[data-file-entry-name="two.txt"]').click();
  await setDestinationPane(page, await panes.nth(1).getAttribute('data-pane-id'));
  await page.getByRole('button', { name: 'Move to other pane', exact: true }).click();
  await page.getByText(/Operation (finished|results)/).first().waitFor();
  await panes.nth(1).locator('[data-file-entry-name="two.txt"]').waitFor();
  await panes.nth(0).locator('[data-file-entry-name="two.txt"]').waitFor({ state: 'detached' });
  console.log('cross-pane move finished');

  await focusPane(1);
  await page.keyboard.press('Control+z');
  await page.getByRole('button', { name: 'History', exact: true }).click();
  await page.getByRole('listbox', { name: 'Operation batches' }).waitFor();
  const undoStatus = await page.getByRole('option').filter({ hasText: 'two.txt' }).first().innerText();
  assert.ok(undoStatus.includes('Restored'), `Ctrl+Z batch status: ${undoStatus}`);
  console.log('Ctrl+Z restored the latest move batch');
  await page.getByRole('button', { name: 'Files', exact: true }).click();
  await openPath(panes.nth(0), 'C:/U1/Source');
  await openPath(panes.nth(1), 'C:/U1/Target');
  await panes.nth(0).locator('[data-file-entry-name="two.txt"]').waitFor();
  await panes.nth(1).locator('[data-file-entry-name="two.txt"]').waitFor({ state: 'detached' });
  console.log('reopened source and target reflect the undo');

  const sourceOne = panes.nth(0).locator('[data-file-entry-name="one.txt"]');
  const sourceTwo = panes.nth(0).locator('[data-file-entry-name="two.txt"]');
  await sourceOne.click();
  await sourceTwo.click({ modifiers: ['Control'] });
  assert.equal((await page.locator('.fm-selection strong').innerText()).trim(), '2');
  console.log('multi-selection follows focused source pane');
  await sourceTwo.click({ modifiers: ['Control'] });
  assert.equal((await page.locator('.fm-selection strong').innerText()).trim(), '1');
  await page.getByRole('button', { name: 'More actions', exact: true }).click();
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  const rename = page.locator('[data-fm-dialog="name"]');
  await rename.waitFor();
  await rename.getByRole('textbox').fill('renamed-one.txt');
  await rename.getByRole('button', { name: 'Rename', exact: true }).click();
  await panes.nth(0).locator('[data-file-entry-name="renamed-one.txt"]').waitFor();
  await sourceOne.waitFor({ state: 'detached' });
  console.log('rename completed');

  await panes.nth(0).locator('[data-file-entry-name="renamed-one.txt"]').click();
  await setDestinationPane(page, await panes.nth(1).getAttribute('data-pane-id'));
  await page.getByRole('button', { name: 'Move to other pane', exact: true }).click();
  await page.getByText(/Operation (finished|results)/).first().waitFor();
  await panes.nth(1).locator('[data-file-entry-name="renamed-one.txt"]').waitFor();
  await panes.nth(0).locator('[data-file-entry-name="renamed-one.txt"]').waitFor({ state: 'detached' });
  console.log('renamed item moved to target pane');

  await page.getByRole('button', { name: 'History', exact: true }).click();
  await page.getByRole('listbox', { name: 'Operation batches' }).waitFor();
  const movedBatch = page.getByRole('option').filter({ hasText: 'renamed-one.txt' }).first();
  await movedBatch.getByRole('checkbox', { name: 'Select restorable items in this batch' }).check();
  await page.getByRole('button', { name: 'Restore 1 items', exact: true }).click();
  await page.getByRole('dialog', { name: 'Restore selected items?' }).getByRole('button', { name: 'Restore 1 items', exact: true }).click();
  await movedBatch.getByText('Restored', { exact: false }).waitFor();
  await page.getByRole('dialog', { name: 'Restore selected items?' }).waitFor({ state: 'detached' });
  const historyText = (await movedBatch.innerText()).replaceAll("\\", "/");
  assert.ok(historyText.includes("C:/U1/Source/renamed-one.txt"));
  assert.ok(historyText.includes("C:/U1/Target/renamed-one.txt"));
  console.log('History shows the restored result and source → target paths');

  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ ok: true, title: await page.title(), checks: ['open source and destination folders', 'current directory search', 'copy conflict Keep Both', 'operation result visibility', 'cross-pane Move', 'Ctrl+Z undo', 'multi-selection in focused pane', 'rename selected file', 'History restore returns item to source'], errors }));
} catch (error) {
  console.log((await page.locator('body').innerText()).slice(0, 5000));
  console.log(JSON.stringify({ ok: false, errors, failure: String(error?.stack ?? error) }));
  process.exitCode = 1;
} finally { await browser.close(); await server.close(); }
