/* Issue #5286: actual token/XHR -> backend save -> provider PTY input.
 * The wrapper owns a fresh Windows gwt and throwaway project. Only Codex is
 * replaced with an input recorder; no upload or backend result is mocked.
 */
import { spawnSync } from 'node:child_process';
import { readdir, readFile, rename, unlink, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { expect, test, type Page, type TestInfo } from '@playwright/test';
import { gotoLiveGwt, openLiveLaunchWizardForBranch, sendLiveGwtEvent } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL!;
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME!;
const CHECKOUT = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!;
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!;
const PROVIDER_LOG = process.env.GWT_PLAYWRIGHT_PROVIDER_LOG!;
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jH1kAAAAASUVORK5CYII=', 'base64');
type Receipt = { kind: string; cwd: string; session: string; text?: string };

async function receipts(): Promise<Receipt[]> {
  return (await readFile(PROVIDER_LOG, 'utf8')).split('\n').slice(0, -1).filter(Boolean).map(line => JSON.parse(line));
}
async function wizard(page: Page, after: number) {
  return (await page.waitForFunction(after => {
    const entry = (window as any).__gwtPlaywrightMessages.findLast((entry: any) =>
      entry.sequence > after && entry.payload.kind === 'launch_wizard_state'
      && (!entry.payload.wizard || (!entry.payload.wizard.is_hydrating
        && !entry.payload.wizard.runtime_resolution_pending && !entry.payload.wizard.launch_materialization_pending)));
    return entry ? { wizard: entry.payload.wizard } : null;
  }, after, { timeout: 60_000 })).jsonValue();
}
async function action(page: Page, action: unknown) {
  const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
  await sendLiveGwtEvent(page, { kind: 'launch_wizard_action', action, bounds: { x: 60, y: 60, width: 900, height: 700 } });
  return wizard(page, after);
}
async function panes(page: Page) {
  await sendLiveGwtEvent(page, { kind: 'frontend_ready' });
  return (await page.waitForFunction(() => {
    const workspace = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === 'workspace_state')?.payload.workspace;
    return workspace ? workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .filter((pane: any) => pane.agent_id === 'codex') : null;
  })).jsonValue() as Promise<Array<{ id: string; session_id: string; is_pm: boolean }>>;
}
async function setup(page: Page, info: TestInfo) {
  const errors = { page: [] as string[], console: [] as Array<{ message: string; url: string }> };
  page.on('pageerror', error => errors.page.push(error.message));
  page.on('console', message => {
    if (message.type() === 'error') errors.console.push({ message: message.text(), url: message.location().url });
  });
  page.on('dialog', dialog => void dialog.accept());
  const theme = info.project.name.endsWith('light') ? 'light' : 'dark';
  await page.addInitScript(theme => localStorage.setItem('gwt:ui:theme', theme), theme);
  await gotoLiveGwt(page, BASE, { enableTestBridge: true });
  await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
  let current = await panes(page);
  if (!current.some(pane => !pane.is_pm)) {
    const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
    await openLiveLaunchWizardForBranch(page, 'main');
    await wizard(page, after);
    await action(page, { kind: 'set_launch_path', path: 'manual_setup' });
    let state = await action(page, { kind: 'set_agent', agent_id: 'codex' });
    for (let step = 0; step < 12 && state.wizard; step++) {
      expect(state.wizard.error).toBeFalsy();
      if (state.wizard.selected_runtime_target !== 'host'
        && state.wizard.runtime_target_options?.some((option: any) => option.value === 'host')) {
        state = await action(page, { kind: 'set_runtime_target', target: 'Host' });
      } else {
        expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
        state = await action(page, { kind: 'submit' });
      }
    }
    expect(state.wizard).toBeNull();
  }
  if (!current.some(pane => pane.is_pm)) {
    await sendLiveGwtEvent(page, { kind: 'open_pm_agent', bounds: { x: 20, y: 20, width: 1200, height: 800 } });
  }
  await expect.poll(async () => (await panes(page)).filter(pane => pane.is_pm).length, { timeout: 90_000 }).toBe(1);
  current = await panes(page);
  const pm = current.find(pane => pane.is_pm)!;
  const agent = current.find(pane => !pane.is_pm)!;
  for (const pane of [pm, agent]) {
    await expect.poll(async () => (await receipts()).some(receipt => receipt.kind === 'launch' && receipt.session === pane.session_id), { timeout: 60_000 }).toBe(true);
  }
  const pmElement = page.locator(`.workspace-window[data-id="${pm.id}"]`);
  await expect(pmElement.locator('.pm-chat').locator('textarea, form, [contenteditable]')).toHaveCount(0);
  await pmElement.getByRole('button', { name: 'Execution log', exact: true }).click();
  await expect(pmElement.locator('.terminal-root')).toBeVisible();
  return { errors, theme, pm, agent };
}
async function paste(page: Page, id: string, filename: string) {
  await page.evaluate(({ id, filename, bytes }) => {
    const target = document.querySelector(`.workspace-window[data-id="${id}"] .terminal-root`)!;
    const transfer = new DataTransfer();
    transfer.items.add(new File([new Uint8Array(bytes)], filename, { type: 'image/png' }));
    target.dispatchEvent(new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData: transfer }));
  }, { id, filename, bytes: [...PNG] });
}
async function assertSavedAndInjected(page: Page, pane: { id: string; session_id: string; is_pm: boolean }, filename: string) {
  const launch = (await receipts()).find(receipt => receipt.kind === 'launch' && receipt.session === pane.session_id)!;
  let worktree = launch.cwd;
  if (pane.is_pm) {
    const hash = (await readdir(join(HOME, '.gwt', 'projects')))[0];
    const prefs = JSON.parse(await readFile(join(HOME, '.gwt', 'projects', hash, 'project-state', 'pm.json'), 'utf8'));
    expect(prefs.registration.session_id).toBe(pane.session_id);
    worktree = prefs.registration.worktree_path;
  }
  const directory = join(worktree, '.gwt', 'drop-files');
  await expect.poll(async () => (await readdir(directory).catch(() => [])).find(file => file.endsWith(`-${filename}`)), { timeout: 30_000 }).toBeTruthy();
  const saved = (await readdir(directory)).find(file => file.endsWith(`-${filename}`))!;
  expect(await readFile(join(directory, saved))).toEqual(PNG);
  await expect.poll(async () => (await receipts()).filter(receipt => receipt.kind === 'input' && receipt.session === pane.session_id)
    .map(receipt => receipt.text).join(''), { timeout: 30_000 }).toContain(saved);
  const input = (await receipts()).filter(receipt => receipt.kind === 'input' && receipt.session === pane.session_id)
    .map(receipt => receipt.text).join('');
  const start = input.lastIndexOf('Image file: ') + 'Image file: '.length;
  const inputPath = input.slice(start, input.indexOf(saved, start) + saved.length);
  expect(await readFile(resolve(launch.cwd, inputPath))).toEqual(PNG);
  await expect(page.locator(`.workspace-window[data-id="${pane.id}"] .attachment-progress`)).toContainText('Attached');
}
function ledger() {
  const env = { ...process.env, HOME, USERPROFILE: HOME };
  for (const key of Object.keys(env)) if (key.startsWith('GWT_')) delete env[key];
  const result = spawnSync(join(CHECKOUT, 'target/debug/gwtd.exe'), [], { cwd: PROJECT, env, encoding: 'utf8',
    input: JSON.stringify({ schema_version: 1, operation: 'errors.list', params: { scope: 'all' } }) });
  expect(result.status, result.stderr).toBe(0);
  const envelope = JSON.parse(result.stdout);
  expect(envelope.ok).toBe(true);
  return JSON.parse(envelope.output).errors;
}

test.describe('Clipboard image live upload (isolated Windows checkout)', () => {
  test.skip(process.env.GWT_PLAYWRIGHT_CLIPBOARD !== '1', 'requires scripts/verify-issue-5286.ps1');
  test.use({ viewport: { width: 1600, height: 1100 } });
  test.setTimeout(240_000);

  test('PM execution log and Agent use actual upload, save identical PNG bytes, and inject paths', async ({ page }, info) => {
    const { errors, theme, pm, agent } = await setup(page, info);
    const statuses: number[] = [];
    page.on('response', response => { if (new URL(response.url()).pathname === '/internal/attachments/upload') statuses.push(response.status()); });
    for (const [role, pane] of [['pm', pm], ['agent', agent]] as const) {
      const filename = `${role}-${theme}.png`;
      await paste(page, pane.id, filename);
      await assertSavedAndInjected(page, pane, filename);
    }
    expect(statuses).toEqual([200, 200]);
    await info.attach(`clipboard-${theme}`, { body: await page.screenshot(), contentType: 'image/png' });
    expect(errors).toEqual({ page: [], console: [] });
  });

  test('token failure can be retried, and actual HTTP storage failure keeps its cause in UI and errors.list', async ({ page }, info) => {
    const { errors, theme, pm } = await setup(page, info);
    // Fail the first token response at the Fetch boundary without generating
    // an unrelated browser network-error console entry. The retry uses HTTP.
    await page.evaluate(() => {
      const original = window.fetch.bind(window);
      let first = true;
      window.fetch = (...args) => {
        if (first && String(args[0]).includes('/internal/attachment-upload-token')) {
          first = false;
          return Promise.resolve(new Response('token temporarily unavailable', { status: 503 }));
        }
        return original(...args);
      };
    });
    await paste(page, pm.id, `token-failed-${theme}.png`);
    const progress = page.locator(`.workspace-window[data-id="${pm.id}"] .attachment-progress`);
    await expect(progress).toContainText('503');
    await expect.poll(() => ledger().some((error: any) => error.kind === 'attachment_failure' && error.message.includes('503'))).toBe(true);
    const retried = `token-retry-${theme}.png`;
    await paste(page, pm.id, retried);
    await assertSavedAndInjected(page, pm, retried);

    // This owned temp directory is the actual backend upload store. Replacing
    // it with a file forces its create_dir_all failure and an actual HTTP 500.
    const storeParent = join(HOME, 'tmp', 'gwt-attachment-uploads');
    const store = join(storeParent, (await readdir(storeParent))[0]);
    const backup = `${store}.fixture-backup`;
    await rename(store, backup);
    await writeFile(store, 'fixture blocks upload directory');
    try {
      await paste(page, pm.id, `storage-failed-${theme}.png`);
      await expect(progress).toContainText('500');
      await expect(progress).toContainText('failed to create upload directory');
      await expect.poll(() => ledger().some((error: any) => error.kind === 'attachment_failure'
        && error.message.includes('500') && error.message.includes('failed to create upload directory'))).toBe(true);
      await info.attach(`clipboard-failure-${theme}`, { body: await page.screenshot(), contentType: 'image/png' });
    } finally {
      await unlink(store);
      await rename(backup, store);
    }
    // Chromium may emit its own resource diagnostic for the deliberately
    // failed HTTP request. Assert its exact source and wording, retain it in
    // evidence, and reject every application console or page error.
    expect(errors.page).toEqual([]);
    expect(errors.console.length).toBeLessThanOrEqual(1);
    for (const diagnostic of errors.console) {
      expect(new URL(diagnostic.url).pathname).toBe('/internal/attachments/upload');
      expect(diagnostic.message).toBe('Failed to load resource: the server responded with a status of 500 (Internal Server Error)');
    }
    await info.attach('expected-http-500-resource-diagnostics', {
      body: JSON.stringify(errors.console), contentType: 'application/json',
    });
  });
});
