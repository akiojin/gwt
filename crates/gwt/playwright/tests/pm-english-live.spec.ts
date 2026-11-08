/* Issue #5349 AC-EN4: fixed UI text against the isolated checkout backend. */
import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { gotoLiveGwt, withLiveGwtBackendLock } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? '';
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? '';
const ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? '';
const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;

test.describe('English application UI', () => {
  test.skip(!BASE || !CHECK_HOME || !ROOT, 'requires an isolated checkout fixture');
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(90_000);

  test('PM, Issues, Settings and notifications keep fixed labels in English', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const fixture = JSON.parse(readFileSync(`${CHECK_HOME}/issue-5023-isolated.json`, 'utf8'));
      expect(fixture.project).toBe(ROOT);
      expect(new URL(readFileSync(`${CHECK_HOME}/url`, 'utf8').trim()).origin).toBe(new URL(BASE).origin);
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);

      const pm = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      await expect(pm.getByRole('log', { name: 'PM reports', exact: true })).toBeVisible({ timeout: 30_000 });
      await expect(pm.getByRole('button', { name: 'Reports', exact: true })).toBeVisible();
      await expect(pm.getByRole('button', { name: 'Execution log', exact: true })).toBeVisible();
      await expect(pm.getByRole('combobox', { name: 'Report kind', exact: true })).toBeVisible();
      const status = pm.getByRole('region', { name: 'PM status', exact: true });
      await expect(status.locator('summary strong')).toHaveText('PM status');
      if ((await status.locator('details').getAttribute('open')) === null) await status.locator('summary').click();
      await expect(status.getByRole('heading', { name: 'PM notes', exact: true })).toBeVisible();
      await expect(status.getByRole('heading', { name: 'Live facts', exact: true })).toBeVisible();
      // Authored PM status/report bodies and Issue titles may contain any
      // language. Inspect only fixed chrome around those original contents.
      const fixedPm = await pm.locator('.pm-chat__toolbar, .pm-chat__history-header, .pm-chat__current-heading, .pm-chat__current-meta, .pm-chat h3')
        .allTextContents();
      expect(fixedPm.join('\n')).not.toMatch(CJK);
      const pmScreenshot = info.outputPath(`english-pm-${theme}.png`);
      await pm.screenshot({ path: pmScreenshot });
      await info.attach(`english-pm-${theme}`, { path: pmScreenshot, contentType: 'image/png' });

      await page.locator('[data-surface="issues"]').click();
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const issues = page.locator('.workspace-window[data-preset="issue"]');
      await expect(issues.locator('.settings-root')).toHaveCount(0);
      await expect(issues).toHaveCount(1);
      await expect(issues.locator('.issue-bridge-root')).toBeVisible();
      await expect(issues.getByRole('group', { name: 'Issue view mode', exact: true })).toBeVisible();
      await expect(issues.getByRole('button', { name: 'Refresh cached work items', exact: true })).toBeVisible();
      await expect(issues.getByRole('switch', { name: 'Autonomous mode', exact: true })).toBeVisible();
      const fixedIssues = await issues.locator('.workspace-toolbar, .knowledge-monitor-bar').allTextContents();
      expect(fixedIssues.join('\n')).not.toMatch(CJK);
      expect(await issues.locator('.knowledge-search').getAttribute('placeholder')).not.toMatch(CJK);
      const issuesScreenshot = info.outputPath(`english-issues-${theme}.png`);
      await issues.screenshot({ path: issuesScreenshot });
      await info.attach(`english-issues-${theme}`, { path: issuesScreenshot, contentType: 'image/png' });

      await issues.getByRole('button', { name: 'Issues settings', exact: true }).click();
      const issuesSettings = settings.locator('[data-settings-panel="issues"]');
      await expect(settings.locator('[data-settings-tab="issues"]')).toHaveAttribute('aria-selected', 'true');
      await expect(issuesSettings).toBeVisible();
      await expect(issuesSettings.getByLabel('Max active', { exact: true })).toHaveCount(0);
      await expect(issuesSettings.getByLabel('Autonomous', { exact: true })).toBeVisible();
      await expect(issuesSettings.getByLabel('Allowed labels', { exact: true })).toBeVisible();
      expect(await issuesSettings.innerText()).not.toMatch(CJK);
      await settings.locator('[data-settings-tab="system"]').click();
      const system = settings.locator('[data-settings-panel="system"]');
      await expect(system).toBeVisible();
      await expect(system.getByLabel('Output Language', { exact: true })).toBeVisible();
      await expect(system.locator('option[value="ja"]')).toHaveText('日本語');
      expect((await system.innerText()).replace('日本語', '')).not.toMatch(CJK);
      expect((await settings.locator('[data-settings-tab]').allTextContents()).join('\n')).not.toMatch(CJK);
      const settingsScreenshot = info.outputPath(`english-settings-${theme}.png`);
      await settings.screenshot({ path: settingsScreenshot });
      await info.attach(`english-settings-${theme}`, { path: settingsScreenshot, contentType: 'image/png' });

      await page.locator('#op-notifications-button').click();
      const notifications = page.getByRole('dialog', { name: 'Notifications', exact: true });
      await expect(notifications).toBeVisible();
      await notifications.getByRole('button', { name: 'Clear all', exact: true }).click();
      await expect(notifications.getByText('No notifications yet', { exact: true })).toBeVisible();
      await expect(notifications.getByRole('button', { name: 'Close notifications', exact: true })).toBeVisible();
      expect(await notifications.innerText()).not.toMatch(CJK);
      const notificationsScreenshot = info.outputPath(`english-notifications-${theme}.png`);
      await notifications.screenshot({ path: notificationsScreenshot });
      await info.attach(`english-notifications-${theme}`, { path: notificationsScreenshot, contentType: 'image/png' });
      expect(errors).toEqual([]);
    });
  });
});
