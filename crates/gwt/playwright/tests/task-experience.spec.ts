/* SPEC #7007 T-016 (Issue #7634): regression of the task-centered foundation
 * surfaces (T-001..T-015) on the checkout binary with a fresh HOME.
 *
 * One run per Operator theme walks Home, Decide, Progress and the Log
 * (notification history) of the PM window, and checks that they read one
 * decision snapshot through a WebSocket reconnect, a project switch and a
 * decision answered elsewhere first. Decisions are registered and answered
 * through the real `decision.*` gwtd operations; nothing is replayed from the
 * design mock. The only injected frame is the window command error used for the
 * Log: the real backend emits it only when a remote runtime refuses a command,
 * which a fresh fixture cannot produce on demand.
 */
import { expect, test, type Page } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { startExactRelaunchFixture } from './_helpers/exact-relaunch';
import { gotoLiveGwt, readLiveHubCatalog, sendLiveGwtEvent } from './_helpers/live-gwt';

const SESSION = 'task-experience-pm';
const AGENT = 'task-experience-agent';

type Fixture = { home: string; project: string; preferences: string };

function seedStoppedPm({ home, project, preferences }: Fixture, session: string, windowId: string) {
  const at = new Date().toISOString();
  mkdirSync(preferences, { recursive: true });
  writeFileSync(join(preferences, 'pm.json'), JSON.stringify({ settings: { auto_start: false }, registration: {
    session_id: session, agent_id: 'codex', worktree_path: project, created_at: at,
  } }));
  writeFileSync(join(preferences, 'issue-monitor.json'), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] }));
  writeFileSync(join(preferences, '../workspace.json'), JSON.stringify({ viewport: { x: 0, y: 0, zoom: 1 },
    windows: [{ id: windowId, title: 'PM', preset: 'agent', geometry: { x: 30, y: 30, width: 1050, height: 720 },
      z_index: 1, status: 'stopped', session_id: session, agent_id: 'codex', persist: true }], next_z_index: 2 }));
  writeFileSync(join(preferences, '../pr-inventory-cache.json'), JSON.stringify({ fetched_at: at, rows: [] }));
  mkdirSync(join(home, '.gwt/sessions'), { recursive: true });
  writeFileSync(join(home, '.gwt/sessions', `${session}.toml`), `id = "${session}"\nworktree_path = ${JSON.stringify(project)}\n`
    + `branch = "main"\nagent_id = { type = "Codex" }\nstatus = "Stopped"\ncreated_at = "${at}"\nupdated_at = "${at}"\n`
    + `last_activity_at = "${at}"\n`);
}

async function openPm(page: Page, base: string, projectKey?: string) {
  await gotoLiveGwt(page, base, { enableTestBridge: true, projectKey });
  await expect.poll(async () => {
    await sendLiveGwtEvent(page, { kind: 'frontend_ready' });
    return page.locator('.workspace-window .pm-chat').count();
  }, { timeout: 30_000 }).toBe(1);
  await expect(page.locator('#op-pm-entry')).toHaveAttribute('data-pm-state', 'stopped');
  await page.locator('#op-pm-entry').click();
  return page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
}

const refresh = (page: Page) => page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));

test.describe('T-016 task experience regression (isolated checkout)', () => {
  test.skip(process.env.GWT_PLAYWRIGHT_TASK_EXPERIENCE !== '1', 'requires checkout binaries (target/debug/gwt, gwtd)');
  test.skip(process.platform === 'win32', 'the fixture PM session record uses POSIX paths');
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(240_000);

  test('Home, Decide, Progress and Log share one snapshot through reconnect, project switch and a decision conflict', async ({ page }, info) => {
    const theme = info.project.name.includes('light') ? 'light' : 'dark';
    const checkout = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
    const gwtd = join(checkout, 'target/debug/gwtd');
    let childEnv: NodeJS.ProcessEnv = {};
    const target = await startExactRelaunchFixture(info, {
      page,
      async beforeStart({ home, project, env, preferences }) {
        childEnv = { ...env };
        seedStoppedPm({ home, project, preferences }, SESSION, 'pm-task-experience');
      },
    });
    const { home, project } = target;
    const base = target.url;
    for (const key of Object.keys(childEnv)) {
      if (key.startsWith('GWT_SESSION_') || key === 'GWT_PANE_WS_URL' || key === 'GWT_BIN_PATH') delete childEnv[key];
    }
    // A direct user CLI call: no session identity, so answering is the user's.
    const gwtdEnvelope = (operation: string, params: Record<string, unknown>, root = project, session?: string) => {
      const env = { ...childEnv, GWT_PROJECT_ROOT: root, ...(session ? { GWT_SESSION_ID: session } : {}) };
      const result = spawnSync(gwtd, [], { cwd: root, env, encoding: 'utf8',
        timeout: 30_000, input: JSON.stringify({ schema_version: 1, operation, params }) });
      expect(result.status, result.stderr || result.stdout).toBe(0);
      const envelope = JSON.parse(result.stdout);
      expect(envelope.ok, result.stdout).toBe(true);
      return { envelope, output: JSON.parse(envelope.output) };
    };
    const gwtdCall = (operation: string, params: Record<string, unknown>, root = project, session?: string) =>
      gwtdEnvelope(operation, params, root, session).output;
    const register = (target: string, question: string, root = project) => gwtdCall('decision.register', {
      issue: 7634, kind: 'product', target, question,
      materials: `Both options keep ${target} working.`,
      options: ['Keep the current layout', 'Move it to the sidebar'],
      recommendation: 'Keep the current layout',
      // An agent registers and waits; the answers below are direct user calls.
    }, root, AGENT).decision;
    const evidence = async (name: string, body: Buffer | string, contentType = 'image/png') => {
      const path = info.outputPath(`task-experience-${theme}-${name}.${contentType === 'image/png' ? 'png' : 'json'}`);
      mkdirSync(info.outputDir, { recursive: true });
      writeFileSync(path, body);
      await info.attach(`task experience ${name} ${theme}`, { path, contentType });
    };

    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
    // The real backend stays behind every socket. While `hold` is set, queue
    // snapshots stop reaching the page so a decision answered elsewhere is
    // still on screen when the user acts on it (the conflict case).
    let hold = false;
    const sockets: { close: () => Promise<void> }[] = [];
    await page.routeWebSocket(/\/ws(\?|$)/, ws => {
      const server = ws.connectToServer();
      sockets.push({ close: () => ws.close() });
      ws.onMessage(message => server.send(message));
      server.onMessage(message => {
        if (hold && /"kind"\s*:\s*"pm_reports"/.test(String(message))) return;
        ws.send(message);
      });
    });

    try {
      const first = register('Task list layout', 'Where should the task list live?');
      const second = register('Decision badge colour', 'Which colour marks a waiting decision?');
      const internalIds = [first.id, second.id, SESSION, AGENT];

      const pane = await openPm(page, base);
      const primaryKey = new URL(page.url()).pathname.split('/').at(-1)!;
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const view = (name: string) => pane.getByRole('button', { name, exact: true }).click();
      const homeView = pane.getByRole('region', { name: 'Home' });
      const decide = pane.getByRole('region', { name: 'Decide' });
      const cards = decide.locator('.task-decisions__card');
      const count = decide.locator('.task-decisions__count');

      // AC-1 / AC-2 / AC-6: Home names the next step from the same queue
      // snapshot that Decide lists; both read the queue's own waiting count.
      await view('Home');
      await expect(homeView.getByRole('heading', { level: 2 })).toHaveText('2 decisions need your attention');
      await homeView.getByRole('button', { name: 'Review 2 decisions →' }).click();
      await expect(decide).toBeVisible();
      await expect(count).toHaveText('2 decisions waiting');
      await expect(cards).toHaveCount(2);
      const firstCard = cards.filter({ hasText: first.question });
      await expect(firstCard.locator('.task-decisions__recommendation')).toHaveText('Recommended: Keep the current layout');
      await expect(firstCard.getByRole('button')).toHaveText(['1. Keep the current layout', '2. Move it to the sidebar',
        'Reply in chat', 'Decide later']);
      await evidence('decide', await pane.screenshot());

      // US-3: "Decide later" keeps the decision waiting at the end of the queue.
      await firstCard.getByRole('button', { name: 'Decide later', exact: true }).click();
      await expect(decide.locator('.task-decisions__notice')).toHaveText('Kept waiting and moved to the end of the queue.');
      await expect(cards.last()).toHaveAttribute('data-decision-id', first.id);
      await expect(firstCard.locator('.task-decisions__chip')).toContainText('Deferred');
      await expect(count).toHaveText('2 decisions waiting');

      // AC-6: a WebSocket drop reconnects to the same backend and restores the
      // same queue snapshot on every surface.
      const opened = sockets.length;
      await Promise.all(sockets.map(socket => socket.close()));
      await expect.poll(() => sockets.length, { timeout: 30_000 }).toBeGreaterThan(opened);
      await expect.poll(async () => {
        await refresh(page);
        return count.textContent();
      }, { timeout: 30_000 }).toBe('2 decisions waiting');
      await view('Home');
      await expect(homeView.getByRole('heading', { level: 2 })).toHaveText('2 decisions need your attention');

      // Conflict: the user answers `second` from the CLI while the GUI still
      // shows it (queue snapshots are held). The GUI answer is refused; the
      // refusal itself carries the current queue, so the stale card leaves at
      // once and the notice says what happened without internal wording (AC-5,
      // AC-7).
      await view('Decide');
      const secondCard = cards.filter({ hasText: second.question });
      await expect(secondCard).toHaveCount(1);
      hold = true;
      gwtdCall('decision.answer', { id: second.id, answer: '1. Keep the current layout' });
      await secondCard.getByRole('button', { name: '2. Move it to the sidebar', exact: true }).click();
      const notice = decide.locator('.task-decisions__notice');
      await expect(notice).toHaveText('This decision was already settled elsewhere. Your answer was not recorded.');
      await expect(secondCard).toHaveCount(0);
      await expect(count).toHaveText('1 decision waiting');
      await evidence('conflict', await pane.screenshot());
      const refusalText = await decide.innerText();
      hold = false;
      const answered = gwtdCall('decision.list', { include_closed: true }).decisions
        .find((row: { id: string }) => row.id === second.id);
      expect(answered.answer.text, 'the first answer stands; the refused GUI answer did not overwrite it')
        .toBe('1. Keep the current layout');

      // Answer the remaining decision from the GUI.
      await cards.first().getByRole('button', { name: '1. Keep the current layout', exact: true }).click();
      await expect(decide.locator('.task-decisions__notice')).toHaveText('Answer sent. The waiting session continues with it.');
      await expect(decide).toContainText('No decisions waiting');
      await view('Home');
      await expect(homeView.getByRole('heading', { level: 2 })).toHaveText('No decisions waiting');

      // US-2: Create task goes to the existing request-task intake (the PM
      // composer), reachable from the keyboard.
      await homeView.getByRole('button', { name: '+ Create task' }).focus();
      await page.keyboard.press('Enter');
      await expect(pane.getByRole('textbox', { name: 'Message to PM' })).toBeFocused();

      // US-4 / AC-8: Progress for a project without request tasks says so,
      // instead of a zero count or a "metric unavailable" note.
      await view('Progress');
      const progress = pane.locator('.task-progress');
      await expect(progress.getByRole('heading', { name: 'No request tasks yet' })).toBeVisible();
      await expect(progress).not.toContainText(/unavailable|unknown|N\/A/i);
      await evidence('progress', await pane.screenshot());

      // AC-7: internal identifiers stay off the main surfaces.
      for (const name of ['Home', 'Decide', 'Progress']) {
        await view(name);
        const text = await pane.locator('.pm-chat').innerText();
        for (const id of internalIds) expect(text, `${name} shows the internal id ${id}`).not.toContain(id);
      }

      // AC-3 / AC-4 / AC-5: the Log (notification history) keeps one row per
      // cause with its occurrence count, a formatted time, the impact and the
      // next action as the headline, and the original backend text as details.
      const raw = `runtime refused resize for ${SESSION}: owner lease is held by another host`;
      const inject = () => page.evaluate(detail => window.dispatchEvent(new CustomEvent('__gwt_test_inject', { detail })),
        { kind: 'window_command_error', id: 'pm-task-experience', operation: 'resize', message: raw });
      await page.locator('#op-notifications-button').click();
      const rows = page.locator('#notification-center .notification-center__item').filter({ hasText: 'Resize failed' });
      const badge = rows.locator('.notification-center__count');
      // One occurrence shows no badge; later ones show ×N on the same row.
      const occurrences = async () => (await badge.count())
        ? Number((await badge.textContent())!.replace('×', '')) : 1;
      // The test bridge may hand one injected frame to the page more than once;
      // the aggregation contract is that every delivery is counted on one row.
      await inject();
      await expect(rows).toHaveCount(1);
      const once = await occurrences();
      await inject();
      await inject();
      await expect(rows).toHaveCount(1);
      await expect.poll(occurrences).toBe(once * 3);
      await expect(rows).toContainText('The terminal size may be out of date. Reopen the window to refresh it.');
      const time = rows.locator('time');
      expect(Number.isFinite(Date.parse((await time.getAttribute('datetime'))!))).toBe(true);
      await expect(time).not.toContainText('T');
      const details = rows.locator('.notification-center__details');
      await details.locator('summary').click();
      await expect(details.locator('.notification-center__details-content')).toHaveText(raw);
      const headline = await rows.innerText();
      await evidence('log', await page.screenshot());
      await page.keyboard.press('Escape');

      // Project switch: another project's queue never mixes into this one.
      const secondRoot = join(home, 'second-project');
      mkdirSync(secondRoot);
      // A wrapper holding a bare repository opens directly (a plain checkout
      // asks for the bare + worktree migration first), as in RPT-F.
      const git = spawnSync('git', ['init', '--bare', '--quiet', join(secondRoot, 'repo.git')],
        { cwd: secondRoot, env: childEnv, encoding: 'utf8' });
      expect(git.status, git.stderr).toBe(0);
      const secondHash = gwtdEnvelope('issue.monitor.status', {}, secondRoot).envelope.project_store.hash;
      const secondState = join(home, '.gwt/projects', secondHash, 'project-state');
      seedStoppedPm({ home, project: secondRoot, preferences: secondState }, `${SESSION}-second`, 'pm-task-experience-second');
      const foreign = register('Second project release', 'Ship the second project now?', secondRoot);
      const catalog = await readLiveHubCatalog(page, base, { send: { kind: 'reopen_recent_project', path: secondRoot },
        until: (hub, root) => hub.recent_projects.some(entry => entry.path === root && entry.project_key
          && hub.projects.some(project => project.project_key === entry.project_key)), arg: secondRoot });
      const secondKey = catalog.recent_projects.find(entry => entry.path === secondRoot)!.project_key!;
      expect(secondKey, 'the Hub opens the project store seeded above').toBe(secondHash);
      const secondPane = await openPm(page, base, secondKey);
      await secondPane.getByRole('button', { name: 'Decide', exact: true }).click();
      await expect(secondPane.locator('.task-decisions__card')).toHaveCount(1);
      await expect(secondPane.locator('.task-decisions__card')).toContainText(foreign.question);
      await expect(secondPane.locator('.task-decisions__count')).toHaveText('1 decision waiting');
      await evidence('second-project', await secondPane.screenshot());
      const back = await openPm(page, base, primaryKey);
      await back.getByRole('button', { name: 'Home', exact: true }).click();
      await expect(back.getByRole('region', { name: 'Home' }).getByRole('heading', { level: 2 })).toHaveText('No decisions waiting');
      await back.getByRole('button', { name: 'Decide', exact: true }).click();
      await expect(back.locator('.task-decisions')).not.toContainText(foreign.question);

      await evidence('observations', JSON.stringify({ theme, refusalText, headline, internalIds, primaryKey, secondKey }, null, 2),
        'application/json');
      for (const id of internalIds) expect(refusalText, 'the conflict refusal shows an internal id').not.toContain(id);
      expect(refusalText).not.toMatch(/Config error|no longer open/);
      expect(errors, 'console / page errors').toEqual([]);
    } finally {
      await target.stop();
    }
  });
});
