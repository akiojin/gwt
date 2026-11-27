/* SPEC #7007 T-022, Issue #7640: Home decisions and direct Notifications on an isolated checkout. */
import { expect, test } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { startExactRelaunchFixture } from './_helpers/exact-relaunch';
import { gotoLiveGwt } from './_helpers/live-gwt';

const SESSION = 'task-experience-pm';
const AGENT = 'task-experience-agent';
const NATIVE_SESSION = 'bc221a3c-2174-4f0a-ac47-a03e4b2aaf78';

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
    + `last_activity_at = "${at}"\nagent_session_id = "${NATIVE_SESSION}"\ndisplay_name = "PM"\n`);
  const rollouts = join(home, '.codex/sessions');
  mkdirSync(rollouts, { recursive: true });
  const transcript = readFileSync(join(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd(), 'crates/gwt/src/pm_conversation/fixtures/codex-chat.jsonl'), 'utf8')
    .replace('"id":"conversation","cwd":"/work/project"', `"id":"${NATIVE_SESSION}","cwd":${JSON.stringify(project)}`);
  writeFileSync(join(rollouts, `rollout-${at.replaceAll(':', '-')}-${NATIVE_SESSION}.jsonl`), transcript);
}

test.describe('T-022 PM view retirement (isolated checkout)', () => {
  test.skip(process.env.GWT_PLAYWRIGHT_TASK_EXPERIENCE !== '1', 'requires checkout binaries (target/debug/gwt, gwtd)');
  test.skip(process.platform === 'win32', 'the fixture PM session record uses POSIX paths');
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(120_000);

  test('Project PM entry opens Home decisions and Notifications remains directly accessible', async ({ page }, info) => {
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
    const { project } = target;
    const projectKey = basename(dirname(target.preferences));
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
    const errors: string[] = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
    try {
      const first = register('Task list layout', 'Where should the task list live?');
      await gotoLiveGwt(page, base, { enableTestBridge: true, projectKey });
      const projectUrl = page.url();
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(page.locator('.workspace-window .pm-chat')).toHaveCount(0);
      const pmEntry = page.locator('#app-head-nav').getByRole('button', { name: /^Project Manager/ });
      await expect(pmEntry).toBeVisible();
      await pmEntry.click({ timeout: 15_000 });
      await expect(page).toHaveURL(/\?view=decisions$/);
      await expect(page.getByRole('heading', { name: 'Decisions' })).toBeVisible();
      const decision = page.locator('[data-home-topic]').filter({ hasText: first.question });
      await expect(decision).toHaveCount(1);
      await decision.getByRole('button', { name: 'Keep the current layout', exact: true }).click();
      await expect(decision).toHaveCount(0);
      await expect(page.locator('[data-home-waitjump]')).toHaveText('No decisions waiting');
      const answered = gwtdCall('decision.list', { include_closed: true }).decisions
        .find((row: { id: string }) => row.id === first.id);
      expect(answered.answer.text).toBe('1. Keep the current layout');

      await page.getByRole('link', { name: 'Back to Home' }).click();
      await expect(page.getByRole('log', { name: 'Conversation with the Project Manager' })).toContainText('Assistant reply');
      await expect(page.getByRole('log', { name: 'Conversation with the Project Manager' })).not.toContainText('internal tool output');

      await gotoLiveGwt(page, projectUrl, { enableTestBridge: true });
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
      expect(errors, 'console / page errors').toEqual([]);
    } finally {
      await target.stop();
    }
  });
});
