/* SPEC #7007 T-036, Issue #7654: the whole v3 GUI on an isolated checkout.
 *
 * One real gwt (checkout binaries, fresh HOME) is driven through every v3
 * screen in both themes: Home and its decisions, the header destinations,
 * and every surface of the project page. Decisions are registered through
 * real `decision.*` gwtd operations, and their waiting count must read the
 * same in the header, the Decisions view, Home and the status bar, before
 * and after one is answered. The retired PM view must not be reachable, and
 * the run must end with no console or page error.
 */
import { expect, test, type Page } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { startExactRelaunchFixture } from './_helpers/exact-relaunch';
import { gotoLiveGwt } from './_helpers/live-gwt';

const QUESTIONS = ['Which storage should the queue use?', 'Where should the task list live?', 'Should the release wait?'];

// The project page's surfaces, opened from the rail, and what each shows.
const RAIL_SURFACES: [string, RegExp][] = [
  ['[data-surface="themes"]', /Themes/],
  ['[data-surface="issues"]', /Issues/i],
  ['[data-surface="agents"]', /Agent history & plans/],
  ['[data-surface="board"]', /Board/i],
  ['[data-surface="settings"]', /Settings/i],
  ['[data-cmd="open-logs"]', /Logs/i],
  ['[data-cmd="open-index"]', /Index/i],
  ['[data-cmd="open-profile"]', /Environment variables/i],
];

// The shared header (Issue #7740) on every page; its first entry is the PM.
const appNav = (page: Page) => page.getByRole('navigation', { name: 'App', exact: true });
const pmEntry = (page: Page) => appNav(page).getByRole('button').first();

test.describe('T-036 v3 regression (isolated checkout)', () => {
  test.skip(process.env.GWT_PLAYWRIGHT_TASK_EXPERIENCE !== '1', 'requires checkout binaries (target/debug/gwt, gwtd)');
  test.skip(process.platform === 'win32', 'the fixture project uses POSIX paths');
  test.use({ viewport: { width: 1440, height: 1000 }, actionTimeout: 20_000 });
  test.setTimeout(240_000);

  test('every v3 screen opens, decision counts agree, and the PM view is gone', async ({ page }, info) => {
    const theme = info.project.name.includes('light') ? 'light' : 'dark';
    const gwtd = join(resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd()), 'target/debug/gwtd');
    let childEnv: NodeJS.ProcessEnv = {};
    const target = await startExactRelaunchFixture(info, {
      page,
      async beforeStart({ env, preferences }) {
        childEnv = { ...env };
        mkdirSync(preferences, { recursive: true });
        writeFileSync(join(preferences, 'issue-monitor.json'), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] }));
      },
    });
    for (const key of Object.keys(childEnv)) {
      if (key.startsWith('GWT_SESSION_') || key === 'GWT_PANE_WS_URL' || key === 'GWT_BIN_PATH') delete childEnv[key];
    }
    // An agent registers each decision and waits; answers come from the page.
    const register = (question: string) => {
      const result = spawnSync(gwtd, [], {
        cwd: target.project, encoding: 'utf8', timeout: 30_000,
        env: { ...childEnv, GWT_PROJECT_ROOT: target.project, GWT_SESSION_ID: 'v3-regression-agent' },
        input: JSON.stringify({ schema_version: 1, operation: 'decision.register', params: {
          issue: 7654, kind: 'product', target: question, question, materials: 'Both options keep the product working.',
          options: ['Keep it', 'Change it'], recommendation: 'Keep it',
        } }),
      });
      expect(result.status, result.stderr || result.stdout).toBe(0);
      expect(JSON.parse(result.stdout).ok, result.stdout).toBe(true);
    };
    const projectKey = basename(dirname(target.preferences));
    const errors: string[] = [];
    const watchErrors = (view: Page) => {
      view.on('pageerror', error => errors.push(error.message));
      view.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    };
    watchErrors(page);
    // Every tab of the run uses the theme; a new tab's about:blank has no storage.
    await page.context().addInitScript(value => {
      if (location.protocol.startsWith('http')) localStorage.setItem('gwt:ui:theme', value);
    }, theme);
    const openProject = () => gotoLiveGwt(page, target.url, { enableTestBridge: true, projectKey });
    const expectWaiting = async (count: number, view: Page = page) => {
      await expect(pmEntry(view)).toHaveAttribute('aria-label',
        count ? new RegExp(`: ${count} decisions? waiting$`) : /: no decisions waiting$/, { timeout: 20_000 });
    };
    try {
      QUESTIONS.forEach(register);

      // Project page: the header and the status bar count the project's queue
      // with no PM window open.
      await openProject();
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expectWaiting(3);
      await expect(page.locator('#op-strip-issue-monitor')).toHaveAttribute('title', /PM decisions 3/, { timeout: 20_000 });

      // The PM entry opens the Decisions view on the page at `/`.
      await pmEntry(page).click();
      await expect(page).toHaveURL(/\?view=decisions$/);
      await expect(page.getByRole('heading', { name: 'Decisions' })).toBeVisible();
      await expect(page.locator('[data-home-waitjump]')).toHaveText('3 decisions waiting');
      for (const question of QUESTIONS) {
        await expect(page.locator('[data-home-topic]').filter({ hasText: question })).toHaveCount(1);
      }
      await expectWaiting(3);

      // Home is the conversation with the PM and holds the same three topics.
      await page.getByRole('link', { name: 'Back to Home' }).click();
      await expect(page.getByRole('log', { name: 'Conversation with the Project Manager' })).toBeVisible();
      for (const question of QUESTIONS) {
        await expect(page.locator('[data-home-topic]').filter({ hasText: question })).toHaveCount(1);
      }
      // Answering one from Home resolves it everywhere.
      const answered = page.locator('[data-home-topic]').filter({ hasText: QUESTIONS[0] });
      await answered.getByRole('button', { name: 'Keep it', exact: true }).click();
      await expectWaiting(2);

      // Header destinations on the page at `/`.
      await appNav(page).getByRole('button', { name: 'Projects', exact: true }).click();
      await expect(page.getByRole('heading', { name: 'Projects', level: 1 })).toBeVisible();
      await expectWaiting(2);

      // The project opens from the list in its own browser tab (SPEC-3287);
      // its count followed the answer.
      const opened = page.context().waitForEvent('page');
      await page.getByRole('region', { name: 'Open' }).getByRole('link').first().click();
      const project = await opened;
      watchErrors(project);
      await expect(project).toHaveURL(new RegExp(`/p/${projectKey}$`));
      await expect(project.locator('html')).toHaveAttribute('data-theme', theme);
      await expectWaiting(2, project);
      await expect(project.locator('#op-strip-issue-monitor')).toHaveAttribute('title', /PM decisions 2/, { timeout: 20_000 });

      // Header destinations on the project page.
      const header = appNav(project);
      await header.getByRole('button', { name: 'Agents', exact: true }).click();
      await expect(project.getByRole('heading', { name: 'Agents', exact: true })).toBeVisible();
      await expect(project.getByText('registered members')).toBeVisible();
      await header.getByRole('button', { name: 'Environments', exact: true }).click();
      await expect(project.getByRole('heading', { name: 'Execution environments', exact: true })).toBeVisible();
      await project.getByRole('button', { name: 'Accounts', exact: true }).click();
      // Design item 234: no tool-side account rows on the accounts screen.
      const accounts = project.getByRole('region', { name: 'Execution environments' });
      await expect(accounts.getByRole('heading', { name: 'Accounts', level: 1 })).toBeVisible();
      await expect(accounts).not.toContainText(/Hermes Agent|OpenClaw|OpenCode/);
      await header.getByRole('button', { name: 'Menu ▾' }).click();
      await project.getByRole('menuitem', { name: 'Settings' }).click();
      await expect(project.locator('.surface-area')).toContainText(/Settings/i);
      await project.locator('#op-notifications-button').click();
      await expect(project.locator('#notification-center')).toBeVisible();
      await project.keyboard.press('Escape');

      // Every surface of the project page, from the rail.
      for (const [selector, content] of RAIL_SURFACES) {
        await project.locator(`.op-rail ${selector}`).click();
        await expect(project.locator('.surface-area'), selector).toContainText(content);
      }

      // The Works screen (design item 137) is folded into Issues: the rail
      // carries no entry to it (Issue #7880).
      await expect(project.locator('.op-rail [data-surface="works"]')).toHaveCount(0);

      // The PM view (design decision 29) is retired: no PM chat window, no
      // tab of it, and the rail's PM entry opens the Agents ledger.
      await project.locator('#op-pm-entry').click();
      await expect(project.getByText('registered members')).toBeVisible();
      await expect(project.locator('.pm-chat')).toHaveCount(0);
      await expect(project.getByRole('tab', { name: /^(Reports|Chat|Log|Progress|Decide)$/ })).toHaveCount(0);

      const screenshot = info.outputPath(`v3-regression-${theme}.png`);
      await project.screenshot({ path: screenshot });
      await info.attach(`v3-regression-${theme}`, { path: screenshot, contentType: 'image/png' });
      expect(errors, 'console / page errors').toEqual([]);
    } finally {
      await target.stop();
    }
  });
});
