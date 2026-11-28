/* SPEC #7007 T-036, Issue #7654: the whole v3 GUI on an isolated checkout.
 *
 * One real gwt (checkout binaries, fresh HOME) is driven through every v3
 * screen in both themes: Home and its decisions, the header destinations,
 * and every surface of the project page. Decisions are registered through
 * real `decision.*` gwtd operations, and their waiting count must read the
 * same in the header, the Decisions view, Home and the status bar, before
 * and after answers from either view, including a bundled decision (#7641).
 * The retired PM view must not be reachable, and
 * the run must end with no console or page error.
 */
import { expect, test, type Page } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
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
    const decisionOperation = (operation: string, params: object, sessionId = 'v3-regression-agent') => {
      const result = spawnSync(gwtd, [], {
        cwd: target.project, encoding: 'utf8', timeout: 30_000,
        env: { ...childEnv, GWT_PROJECT_ROOT: target.project, GWT_SESSION_ID: sessionId },
        input: JSON.stringify({ schema_version: 1, operation, params }),
      });
      expect(result.status, result.stderr || result.stdout).toBe(0);
      const envelope = JSON.parse(result.stdout);
      expect(envelope.ok, result.stdout).toBe(true);
      return JSON.parse(envelope.output);
    };
    const register = (question: string, sessionId?: string) => decisionOperation('decision.register', {
      issue: 7654, kind: 'product', target: question, question, materials: 'Both options keep the product working.',
      options: ['Keep it', 'Change it'], recommendation: 'Keep it',
    }, sessionId);
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
    const mock = await page.context().newPage();
    const comparison = await page.context().newPage();
    const referencePath = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd(),
      'docs/design/issue-7007/mock-full-refresh-v3.html');
    const compare = async (product: Page, name: string, mockPage: string, mockScreen?: string) => {
      await mock.goto(`${pathToFileURL(referencePath).href}?full=1`);
      // Pin the mock's active pane without invoking menu or dialog actions.
      await mock.evaluate(({ mockPage, mockScreen }) => {
        for (const group of document.querySelectorAll<HTMLElement>('.pagegroup')) {
          const active = group.dataset.page === mockPage;
          group.classList.toggle('on', active);
          if (!active || !mockScreen) continue;
          for (const pane of group.querySelectorAll<HTMLElement>('.screen-pane')) {
            pane.classList.toggle('on', pane.dataset.screen === mockScreen);
          }
          const title = group.querySelector(`.screen-pane.on :is(.h-big, h1, h2)`);
          if (title) document.getElementById('appTitle')!.textContent = title.textContent;
        }
      }, { mockPage, mockScreen });
      await expect(mock.locator(`.pagegroup.on[data-page="${mockPage}"]`)).toBeVisible();
      if (mockScreen) {
        await expect(mock.locator(`.screen-pane.on[data-screen="${mockScreen}"]`)).toBeVisible();
      }
      const canvas = theme === 'light' ? 'rgb(242, 243, 245)' : 'rgb(25, 27, 32)';
      await expect(product.locator('body')).toHaveCSS('background-color', canvas);
      const typography = (view: Page) => view.locator('body').evaluate(body => {
        const style = getComputedStyle(body);
        return [style.fontFamily, style.fontSize, style.lineHeight];
      });
      expect(await typography(product)).toEqual(await typography(mock));
      expect(await product.locator('body').evaluate(body => [
        getComputedStyle(body, '::before').backgroundImage,
        getComputedStyle(body, '::after').backgroundImage,
      ])).toEqual(['none', 'none']);
      const [reference, actual] = await Promise.all([mock, product].map(view => view.screenshot({ animations: 'disabled' })));
      const image = (buffer: Buffer, label: string) =>
        `<figure style="margin:0"><figcaption>${label}</figcaption><img style="display:block;width:100%" src="data:image/png;base64,${buffer.toString('base64')}" alt=""></figure>`;
      await comparison.setContent(`<main style="display:grid;grid-template-columns:1fr 1fr;gap:8px;padding:8px;background:white;color:black;font:16px system-ui">
        ${image(reference, 'Mock v3 · Light')}${image(actual, `Product · ${theme}`)}</main>`);
      const path = info.outputPath(`${name}-${theme}-comparison.png`);
      await comparison.locator('main').screenshot({ path });
      await info.attach(`${name}-${theme}-comparison`, { path, contentType: 'image/png' });
    };
    const openProject = () => gotoLiveGwt(page, target.url, { enableTestBridge: true, projectKey });
    const expectWaiting = async (count: number, view: Page = page) => {
      await expect(pmEntry(view)).toHaveAttribute('aria-label',
        count ? new RegExp(`: ${count} decisions? waiting$`) : /: no decisions waiting$/, { timeout: 20_000 });
    };
    try {
      const decisions = QUESTIONS.map(question => register(question).decision);
      const bundled = decisions[1];
      const duplicate = register(QUESTIONS[1], 'v3-regression-second-agent');
      expect(duplicate).toMatchObject({ created: false, decision: { id: bundled.id } });
      const bundledTopic = page.locator(`article[data-home-topic="${projectKey}:${bundled.id}"]`);

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
      await compare(page, 'decisions', 'gwt', 'g-decide');
      for (const question of QUESTIONS) {
        await expect(page.locator('[data-home-topic]').filter({ hasText: question })).toHaveCount(1);
      }
      await expectWaiting(3);

      // Answering the bundled queue item from Decisions schedules delivery to both agents.
      await bundledTopic.getByRole('button', { name: 'Change it', exact: true }).click();
      await expect(bundledTopic).toHaveCount(0);
      await expectWaiting(2);

      // Home is the conversation with the PM and reflects the same queue update.
      await page.getByRole('link', { name: 'Back to Home' }).click();
      await expect(page.getByRole('log', { name: 'Conversation with the Project Manager' })).toBeVisible();
      await expect(bundledTopic).toHaveCount(0);
      await expect(page.locator('[data-home-waitjump]')).toHaveText('2 decisions waiting');
      await expect(page.locator('article[data-home-topic]').filter({ hasText: QUESTIONS[2] })).toHaveCount(1);

      await compare(page, 'home', 'start');

      // Answering one from Home resolves it everywhere.
      const answered = page.locator('[data-home-topic]').filter({ hasText: QUESTIONS[0] });
      await answered.getByRole('button', { name: 'Keep it', exact: true }).click();
      await expectWaiting(1);
      const queue = decisionOperation('decision.list', { include_closed: true });
      expect(queue.decisions).toHaveLength(3);
      expect(queue.decisions).toContainEqual(expect.objectContaining({
        id: bundled.id,
        status: 'answered',
        answer: expect.objectContaining({ text: '2. Change it' }),
        deliveries: expect.arrayContaining([
          expect.objectContaining({ waiter: { role: 'session', session_id: 'v3-regression-agent' } }),
          expect.objectContaining({ waiter: { role: 'session', session_id: 'v3-regression-second-agent' } }),
        ]),
      }));
      expect(queue.decisions).toContainEqual(expect.objectContaining({
        id: decisions[0].id, status: 'answered', answer: expect.objectContaining({ text: '1. Keep it' }),
      }));

      // Header destinations on the page at `/`.
      await appNav(page).getByRole('button', { name: 'Projects', exact: true }).click();
      await expect(page.getByRole('heading', { name: 'Projects', level: 1 })).toBeVisible();
      await expectWaiting(1);

      await compare(page, 'projects', 'projects', 'projects-main');

      // The project opens from the list in its own browser tab (SPEC-3287);
      // its count followed the answer.
      const opened = page.context().waitForEvent('page');
      await page.getByRole('region', { name: 'Open' }).getByRole('link').first().click();
      const project = await opened;
      watchErrors(project);
      await expect(project).toHaveURL(new RegExp(`/p/${projectKey}$`));
      await expect(project.locator('html')).toHaveAttribute('data-theme', theme);
      await expectWaiting(1, project);
      await expect(project.locator('#op-strip-issue-monitor')).toHaveAttribute('title', /PM decisions 1/, { timeout: 20_000 });
      const bodyFont = await project.locator('body').evaluate(body => getComputedStyle(body).fontFamily);
      await expect(project.locator('.op-status-strip')).toHaveCSS('font-family', bodyFont);
      const surface = await project.locator('.app-head').evaluate(header => getComputedStyle(header).backgroundColor);
      for (const name of ['Split view', 'Close Project']) {
        const button = project.getByRole('button', { name, exact: true });
        await expect(button).toHaveCSS('background-color', surface);
        await expect(button).toHaveCSS('font-family', bodyFont);
        await expect(button).toHaveCSS('text-transform', 'none');
      }

      // Header destinations on the project page.
      const header = appNav(project);
      await header.getByRole('button', { name: 'Agents', exact: true }).click();
      await expect(project.getByRole('heading', { name: 'Agents', exact: true })).toBeVisible();
      const registeredMembers = project.getByRole('meter', { name: 'Registered members', exact: true });
      await expect(registeredMembers).toBeVisible();
      await compare(project, 'agent-ledger', 'agents', 'agents-ledger');
      await header.getByRole('button', { name: 'Environments', exact: true }).click();
      await expect(project.getByRole('heading', { name: 'Execution environments', exact: true })).toBeVisible();
      await compare(project, 'environments', 'tools', 'tools-env');
      await project.emulateMedia({ forcedColors: 'active' });
      const environmentAction = project.getByRole('button', { name: 'Add an execution environment' });
      try {
        await environmentAction.hover();
        const { backgroundColor, borderTopColor } = await environmentAction.evaluate(button => {
          const style = getComputedStyle(button);
          return { backgroundColor: style.backgroundColor, borderTopColor: style.borderTopColor };
        });
        expect(borderTopColor).not.toBe(backgroundColor);
      } finally {
        await project.emulateMedia({ forcedColors: null });
      }
      await project.getByRole('button', { name: 'Accounts', exact: true }).click();
      // Tool-side providers may appear in the usage summary, but cannot have account pools.
      const accounts = project.getByRole('region', { name: 'Execution environments' });
      await expect(accounts.getByRole('heading', { name: 'Accounts', level: 1 })).toBeVisible();
      await expect(accounts.getByRole('heading', { name: 'Usage by provider' })).toBeVisible();
      await expect(accounts.getByRole('region', { name: /^(Hermes Agent|OpenClaw|OpenCode) accounts$/ })).toHaveCount(0);
      await compare(project, 'accounts', 'tools', 'tools-pool');
      await accounts.getByRole('button', { name: 'Tool management' }).click();
      await expect(project.locator('.surface-area')).toContainText(/Tools/i);
      await compare(project, 'tool-management', 'tools', 'tools-supported');
      await header.getByRole('button', { name: 'Menu ▾' }).click();
      await project.getByRole('menuitem', { name: 'Settings' }).click();
      await expect(project.locator('.surface-area')).toContainText(/Settings/i);
      await project.locator('#op-notifications-button').click();
      await expect(project.locator('#notification-center')).toBeVisible();
      await compare(project, 'notifications', 'notifications', 'notifications-main');
      await project.keyboard.press('Escape');

      // Every surface of the project page, from the rail.
      const mockScreens = ['g-tasks', 'g-issues', 'g-agents', 'g-board', 'g-settings', 'g-logs', 'g-index', 'profile-main'];
      for (const [index, [selector, content]] of RAIL_SURFACES.entries()) {
        await project.locator(`.op-rail ${selector}`).click();
        await expect(project.locator('.surface-area'), selector).toContainText(content);
        const mockScreen = mockScreens[index];
        if (mockScreen === 'g-board') {
          await expect(project.locator('.board-root .knowledge-heading')).toHaveCSS('font-size', '20px');
        }
        if (mockScreen === 'g-settings') {
          await project.getByRole('tab', { name: 'Issues execution settings' }).click();
          await expect(project.locator('.settings-heading')).toHaveCSS('font-size', '20px');
        }
        await compare(project, mockScreen, mockScreen === 'profile-main' ? 'profile' : 'gwt', mockScreen);
      }

      // The Works screen (design item 137) is folded into Issues: the rail
      // carries no entry to it (Issue #7880).
      await expect(project.locator('.op-rail [data-surface="works"]')).toHaveCount(0);

      // The PM view (design decision 29) is retired: no PM chat window, no
      // tab of it, and the rail's PM entry opens the Agents ledger.
      await project.locator('#op-pm-entry').click();
      await expect(registeredMembers).toBeVisible();
      await expect(project.locator('.pm-chat')).toHaveCount(0);
      await expect(project.getByRole('tab', { name: /^(Reports|Chat|Log|Progress|Decide)$/ })).toHaveCount(0);

      const screenshot = info.outputPath(`v3-regression-${theme}.png`);
      await project.screenshot({ path: screenshot });
      await info.attach(`v3-regression-${theme}`, { path: screenshot, contentType: 'image/png' });
      expect(errors, 'console / page errors').toEqual([]);
    } finally {
      await comparison.close();
      await mock.close();
      await target.stop();
    }
  });
});
