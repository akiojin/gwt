import { expect, test } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? '';
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? '';
const ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? '';

test('compact PM Reports shares one scroll at 600, 1280 and 400 pixels', async ({ page }, info) => {
  test.skip(!BASE || !HOME, 'requires isolated checkout fixture');
  test.setTimeout(90_000);
  await withLiveGwtBackendLock(BASE, info, async () => {
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    const theme = info.project.name.includes('light') ? 'light' : 'dark';
    await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
    await page.setViewportSize({ width: 1500, height: 1000 });
    const fixture = JSON.parse(readFileSync(`${HOME}/issue-5023-isolated.json`, 'utf8'));
    // Replace only this disposable fixture's report ledger with concise reports.
    const ledger = `${HOME}/.gwt/projects/${fixture.repo_hash}/project-state/pm-reports.jsonl`;
    writeFileSync(ledger, '');
    const env = { ...process.env, HOME, USERPROFILE: HOME };
    for (const key of Object.keys(env)) {
      if (key.startsWith('GWT_SESSION_') || key.startsWith('GWT_HOOK_FORWARD_')) delete env[key];
    }
    const post = (operation: string, params: Record<string, unknown>) => {
      const result = spawnSync(`${ROOT}/target/debug/gwtd`, [], { cwd: ROOT, env, encoding: 'utf8',
        input: JSON.stringify({ schema_version: 1, operation, params }) });
      expect(result.status, result.stderr).toBe(0);
      expect(JSON.parse(result.stdout).ok).toBe(true);
    };
    for (let index = 1; index <= 6; index++) {
      post('pm.report.post', { kind: 'progress', body: `Report ${index}: first body line.\n\nA concise update.` });
    }
    post('pm.status.set', { items: [{ kind: 'waiting', text: 'Waiting for required CI', started_at: new Date().toISOString() }],
      next_trigger: 'Required CI passes, then verify every deployment result. '.repeat(12), user_action: null });
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
    const card = pane.getByRole('region', { name: 'PM status', exact: true });
    await expect(card).toBeVisible({ timeout: 30_000 });
    const id = await pane.getAttribute('data-id');
    const body = pane.locator('.pm-chat__body');
    const details = card.locator('details');
    await expect(details).not.toHaveAttribute('open');
    for (const width of [600, 1280, 400]) {
      await sendLiveGwtEvent(page, { kind: 'update_window_geometry', id,
        geometry: { x: 20, y: 20, width, height: 700 }, cols: 80, rows: 24 });
      await expect.poll(() => pane.evaluate(node => Math.round(node.getBoundingClientRect().width))).toBe(width);
      await body.evaluate(node => { node.scrollTop = 0; });
      await expect(card.locator('summary')).toContainText('0 working · 1 waiting');
      const summaryRows = await card.locator('summary > span').evaluateAll(nodes => nodes.map(node => ({ height: node.getBoundingClientRect().height, line: parseFloat(getComputedStyle(node).lineHeight) })));
      expect(summaryRows).toHaveLength(3);
      for (const row of summaryRows) expect(row.height).toBeLessThanOrEqual(row.line + 1);
      expect(await card.locator('summary').evaluate(node => node.getBoundingClientRect().height)).toBeLessThanOrEqual(summaryRows.reduce((height, row) => height + row.line, 0) + 1);
      expect(await body.evaluate(node => node.scrollWidth <= node.clientWidth)).toBe(true);
      const scrollOwners = await body.evaluate(node => [node, ...node.querySelectorAll('*')]
        .filter(element => ['auto', 'scroll'].includes(getComputedStyle(element).overflowY))
        .map(element => element.className));
      expect(scrollOwners).toEqual(['pm-chat__body']);
      if (width >= 600) {
        for (let index = 0; index < 3; index++) {
          const line = pane.locator('.pm-chat__message .pm-chat__text p').filter({ hasText: `Report ${index + 1}:` });
          const lineBox = await line.boundingBox();
          const bodyBox = await body.boundingBox();
          expect(lineBox!.y + lineBox!.height).toBeLessThanOrEqual(bodyBox!.y + bodyBox!.height);
        }
        const screenshot = info.outputPath(`pm-reports-${width}-${theme}.png`);
        await pane.screenshot({ path: screenshot });
        await info.attach(`PM Reports ${width}px ${theme}`, { path: screenshot, contentType: 'image/png' });
      }
    }
    await details.locator('summary').click();
    await expect(details).toHaveAttribute('open');
    await sendLiveGwtEvent(page, { kind: 'load_pm_reports', id });
    await expect(details).toHaveAttribute('open');
    await pane.locator('.pm-chat__message').last().scrollIntoViewIfNeeded();
    expect(await body.evaluate(node => node.scrollTop)).toBeGreaterThan(0);
    await expect(pane.locator('.pm-chat__message').last()).toBeInViewport();
    await pane.getByRole('button', { name: 'Execution log', exact: true }).click();
    await pane.getByRole('button', { name: 'Reports', exact: true }).click();
    await expect(details).toHaveAttribute('open');
    expect(errors).toEqual([]);
  });
});
