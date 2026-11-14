/* SPEC #5023: explicit durable reports -> backend -> read-only PM view. */
import { expect, test, type Page } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { appendFileSync, existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { startExactRelaunchFixture } from './_helpers/exact-relaunch';
import { gotoLiveGwt, installLiveGwtInitScript, readLiveHubCatalog, sendLiveGwtEvent, withLiveGwtBackendLock } from './_helpers/live-gwt';
import { startManagedBrowserSource } from './_helpers/managed-browser-source';
import { withNativeHiddenPage } from './_helpers/native-chromium';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? 'http://127.0.0.1:0/';
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? '';
const ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? '';
const SESSION = 'pm-reports-fixture';

async function gotoPmReports(page: Page, projectKey?: string, base = BASE, keepTasks = false) {
  await gotoLiveGwt(page, base, { enableTestBridge: true, projectKey });
  // PM registration settles after the first workspace projection in this
  // stopped-PM fixture. Read its current projection before testing views.
  await expect.poll(async () => {
    await sendLiveGwtEvent(page, { kind: 'frontend_ready' });
    return page.locator('.workspace-window .pm-chat').count();
  }, { timeout: 30_000 }).toBe(1);
  // Open the existing stopped PM through its normal rail entry; this
  // selects its surface without starting an agent.
  await expect(page.locator('#op-pm-entry')).toHaveAttribute('data-pm-state', 'stopped');
  await page.locator('#op-pm-entry').click();
  if (!keepTasks) await page.locator('.pm-chat').getByRole('button', { name: 'Reports', exact: true }).click();
}

// A reload reopens the PM window in its default Chat view (T-CHAT-DEFAULT),
// so reselect Reports before asserting the persisted history.
async function reloadPmReports(page: Page) {
  await page.reload();
  await page.locator('.pm-chat').getByRole('button', { name: 'Reports', exact: true }).click();
}

async function refreshPmReports(page: Page) {
  await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
}

test.describe('PM reports', () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL || !HOME, 'requires isolated checkout fixture');
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(90_000);

  test('Issue #6235 Reports follows the tail and preserves reading position through updates and reveal', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      const projects = join(HOME, '.gwt', 'projects');
      const ledgers = readdirSync(projects).map(hash => join(projects, hash, 'project-state', 'pm-reports.jsonl')).filter(existsSync);
      expect(ledgers, 'one owned isolated PM report ledger').toHaveLength(1);
      const history = ledgers[0];
      const original = readFileSync(history);
      const report = (index: number) => JSON.stringify({ id: `scroll-${theme}-${index}`, kind: 'progress',
        body: `Report ${index}\n\nA durable progress update for the scroll regression.`, created_at: '2026-10-24T00:00:00Z' });
      try {
        writeFileSync(history, Array.from({ length: 35 }, (_, index) => report(index)).join('\n') + '\n');
        await gotoPmReports(page);
        const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
        const body = pane.locator('.pm-chat__body');
        const messages = pane.locator('.pm-chat__message');
        const gap = () => body.evaluate(node => node.scrollHeight - node.clientHeight - node.scrollTop);
        await expect(messages).toHaveCount(35);
        await expect.poll(gap, { message: 'first Reports display starts at the latest report' }).toBeLessThanOrEqual(2);
        expect(await body.evaluate(node => node.scrollTop)).toBeGreaterThan(0);
        appendFileSync(history, report(35) + '\n');
        await refreshPmReports(page);
        await expect(messages).toHaveCount(36);
        await expect.poll(gap, { message: 'new Reports follow the tail' }).toBeLessThanOrEqual(2);

        const reading = await body.evaluate(node => { node.scrollTop = 180; node.dispatchEvent(new Event('scroll')); return node.scrollTop; });
        appendFileSync(history, report(36) + '\n');
        await refreshPmReports(page);
        await expect(messages).toHaveCount(37);
        await expect.poll(() => body.evaluate(node => node.scrollTop)).toBe(reading);
        await pane.getByRole('button', { name: 'Execution log', exact: true }).click();
        await pane.getByRole('button', { name: 'Tasks', exact: true }).click();
        await pane.getByRole('button', { name: 'Reports', exact: true }).click();
        await expect.poll(() => body.evaluate(node => node.scrollTop)).toBe(reading);

        await page.getByRole('button', { name: 'Works', exact: true }).click();
        await expect(body).toBeHidden();
        appendFileSync(history, report(37) + '\n');
        await refreshPmReports(page);
        await expect(messages).toHaveCount(38);
        await page.locator('#op-pm-entry').click();
        await expect(body).toBeVisible();
        await expect.poll(() => body.evaluate(node => node.scrollTop)).toBe(reading);
        await body.evaluate(node => { node.scrollTop = node.scrollHeight; node.dispatchEvent(new Event('scroll')); });
        await page.getByRole('button', { name: 'Works', exact: true }).click();
        appendFileSync(history, report(38) + '\n');
        await refreshPmReports(page);
        await expect(messages).toHaveCount(39);
        await page.locator('#op-pm-entry').click();
        await expect.poll(gap, { message: 'hidden tail intent survives new Reports' }).toBeLessThanOrEqual(2);
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        const screenshot = info.outputPath(`reports-scroll-${theme}.png`);
        await pane.screenshot({ path: screenshot });
        await info.attach('Reports scroll', { path: screenshot, contentType: 'image/png' });
        expect(errors).toEqual([]);
      } finally {
        writeFileSync(history, original);
      }
    });
  });

  test('Issue #6950 saved reports stay visible while the PM current status is unreadable', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      const projects = join(HOME, '.gwt', 'projects');
      const ledgers = readdirSync(projects).map(hash => join(projects, hash, 'project-state', 'pm-reports.jsonl')).filter(existsSync);
      expect(ledgers, 'one owned isolated PM report ledger').toHaveLength(1);
      const history = ledgers[0];
      const status = join(history, '..', 'pm-status.jsonl');
      const original = readFileSync(history);
      const originalStatus = existsSync(status) ? readFileSync(status) : null;
      const report = (id: string) => JSON.stringify({ id: `${id}-${theme}`, kind: 'qa', question: `Question ${id}`,
        body: `Answer ${id}`, created_at: '2026-11-09T13:03:00Z' });
      try {
        writeFileSync(history, report('before') + '\n');
        await gotoPmReports(page);
        const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
        const messages = pane.locator('.pm-chat__message');
        await expect(messages).toHaveCount(1);
        writeFileSync(status, 'broken PM status\n');
        appendFileSync(history, report('saved') + '\n');
        await refreshPmReports(page);
        await expect(messages, 'a saved report appears despite the broken current status').toHaveCount(2);
        await expect(pane.locator('.pm-chat__body')).toContainText('Answer saved');
        await expect(pane.locator('.pm-chat__body')).toContainText('PM current status could not be loaded');
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        const screenshot = info.outputPath(`reports-current-error-${theme}.png`);
        await pane.screenshot({ path: screenshot });
        await info.attach('Reports with unreadable current status', { path: screenshot, contentType: 'image/png' });
        expect(errors).toEqual([]);
      } finally {
        writeFileSync(history, original);
        if (originalStatus === null) rmSync(status, { force: true }); else writeFileSync(status, originalStatus);
      }
    });
  });

  test('T-CHAT-VIEW Chat renders the PM conversation as user-right and PM-left bubbles', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      const sendResults: any[] = [];
      page.on('websocket', socket => socket.on('framereceived', ({ payload }) => {
        const event = JSON.parse(String(payload));
        if (event.kind === 'pane_send_result') sendResults.push(event);
      }));
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoPmReports(page, undefined, BASE, true);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      await expect(pane.getByRole('button', { name: 'Home', exact: true })).toHaveAttribute('aria-pressed', 'true');
      await expect(pane.getByRole('region', { name: 'Home', exact: true })).toBeVisible();
      // AC-3431-CHAT-1: one action from the default Home opens Chat.
      await pane.getByRole('button', { name: 'Chat', exact: true }).click();
      await expect(pane.getByRole('button', { name: 'Chat', exact: true })).toHaveAttribute('aria-pressed', 'true');
      await expect(pane.getByRole('region', { name: 'Home', exact: true })).toBeHidden();
      const chat = pane.getByRole('region', { name: 'PM conversation' });
      await expect(chat).toBeVisible();
      // The real host answers for the stopped fixture PM; it has no native record yet.
      await expect(chat.getByRole('status')).not.toHaveText('Loading PM conversation…', { timeout: 30_000 });
      const id = await pane.getAttribute('data-id');
      await page.evaluate(({ id, session }) => {
        const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
          candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
        socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ kind: 'pm_conversation', id, session_id: session,
          snapshot: { conversation_id: 'chat-fixture', availability: 'ready', detail: null, messages: [
            { id: 'u1', role: 'user', text: 'Ship <b>it</b>?' },
            { id: 'a0', role: 'assistant', text: 'Checking the queue.', text_html: '<p>Checking the queue.</p>' },
            { id: 'a1', role: 'assistant', text: '# Plan', text_html: '<h1>Plan</h1><ul><li>Verify</li></ul><table><thead><tr><th>Check</th></tr></thead><tbody><tr><td>CI</td></tr></tbody></table><pre><code>cargo test</code></pre>' },
          ] } }) }));
      }, { id, session: SESSION });
      const user = chat.locator('.pm-chat__bubble--user');
      const assistant = chat.locator('.pm-chat__conversation > .pm-chat__bubble--assistant');
      await expect(user).toHaveText('Ship <b>it</b>?');
      // T-CHAT-COLLAPSE: the earlier PM message of the turn folds behind a disclosure.
      const earlier = chat.locator('details.pm-chat__collapsed');
      const folded = earlier.locator('.pm-chat__bubble--assistant');
      await expect(earlier.locator('summary')).toHaveText('1 earlier message');
      await expect(folded).toBeHidden();
      await earlier.locator('summary').click();
      await expect(folded).toHaveText('Checking the queue.');
      await expect(folded).toBeVisible();
      expect((await earlier.boundingBox())!.y).toBeLessThan((await assistant.boundingBox())!.y);
      await expect(user.locator('b')).toHaveCount(0);
      await expect(assistant.getByRole('heading', { name: 'Plan' })).toBeVisible();
      await expect(assistant.locator('table')).toBeVisible();
      const userBox = (await user.boundingBox())!, assistantBox = (await assistant.boundingBox())!, chatBox = (await chat.boundingBox())!;
      expect(userBox.x + userBox.width).toBeGreaterThan(chatBox.x + chatBox.width - 40);
      expect(assistantBox.x).toBeLessThan(chatBox.x + 40);
      expect(userBox.y).toBeLessThan(assistantBox.y);
      expect(await pane.locator('.pm-chat__transcript').textContent()).not.toContain('Ship');
      // T-CHAT-INPUT: the composer submits through the real host's pane input path.
      await page.evaluate(id => {
        const sent: any[] = (window as any).__gwtChatSent = [];
        for (const socket of (window as any).__gwtPlaywrightSockets as WebSocket[]) {
          const original = socket.send.bind(socket);
          socket.send = (data: any) => {
            const message = JSON.parse(String(data));
            if (message.kind === 'pane_send_input') {
              sent.push(message);
              // A Palette/CLI success on the same pane is unrelated to this draft.
              socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({
                kind: 'pane_send_result', window_id: id, ok: true, error: null }) }));
            }
            original(data);
          };
        }
      }, id);
      const composer = chat.getByRole('form', { name: 'Message PM' });
      const draft = composer.getByRole('textbox', { name: 'Message to PM' });
      await expect(composer.getByRole('button', { name: 'Send' })).toBeDisabled();
      await draft.fill('Plan #3431');
      await draft.press('Shift+Enter');
      await draft.pressSequentially('next');
      await draft.press('Enter');
      await expect.poll(() => page.evaluate(() => (window as any).__gwtChatSent)).toEqual([
        { kind: 'pane_send_input', session_id: SESSION, text: 'Plan #3431\nnext\r', request_id: expect.any(Number) }]);
      const requestId = await page.evaluate(() => (window as any).__gwtChatSent[0].request_id);
      await expect.poll(() => sendResults.find(result => result.request_id === requestId)).toMatchObject({ ok: false, request_id: requestId });
      // The fixture PM has no live runtime, so the real host refuses and the draft is kept.
      await expect(composer.locator('.pm-chat__composer-status')).toHaveText(/^Not sent: .+[^.]\. Your message is kept\.$/, { timeout: 30_000 });
      await expect(draft).toHaveValue('Plan #3431\nnext');
      await expect(composer.getByRole('button', { name: 'Send' })).toBeEnabled();
      const composerBox = (await composer.boundingBox())!;
      expect(composerBox.y + composerBox.height).toBeLessThanOrEqual(chatBox.y + chatBox.height + 1);
      expect(composerBox.y).toBeGreaterThan(assistantBox.y);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const screenshot = info.outputPath(`chat-view-${theme}.png`);
      await pane.screenshot({ path: screenshot });
      await info.attach('PM chat view', { path: screenshot, contentType: 'image/png' });
      expect(errors).toEqual([]);
    });
  });

  test('T-CHAT-WORKS Chat opens the same PM terminal in Works', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      let retainPm = false;
      let retainedWindowId: string | null = null;
      // Preserve the owned stopped fixture through every real focus reply.
      // All requests reach the real host; only this exact display flag changes.
      await page.routeWebSocket(url => url.pathname === '/ws', route => {
        const server = route.connectToServer();
        server.onMessage(message => {
          if (retainPm) {
            const payload = JSON.parse(String(message));
            if (payload.kind === 'workspace_state') {
              const pm = payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
                .find((window: any) => window.id === retainedWindowId && window.session_id === SESSION && window.is_pm);
              if (pm) {
                pm.persist = false;
                route.send(JSON.stringify(payload));
                return;
              }
            }
          }
          route.send(message);
        });
      });
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoPmReports(page, undefined, BASE, true);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const id = await pane.getAttribute('data-id');
      expect(id).toBeTruthy();
      retainedWindowId = id;
      try {
        await pane.getByRole('button', { name: 'Chat', exact: true }).click();
        const initialDraft = pane.getByRole('textbox', { name: 'Message to PM' });
        await initialDraft.fill('Keep this unsent PM draft');
        await pane.getByRole('button', { name: 'Open terminal in Works', exact: true }).click();
        await expect(pane.locator('.pm-chat__terminal-status')).toContainText('Terminal unavailable in Works');
        await expect(initialDraft).toBeVisible();
        await expect(initialDraft).toHaveValue('Keep this unsent PM draft');
        retainPm = true;
        await sendLiveGwtEvent(page, { kind: 'frontend_ready' });
        await expect.poll(() => page.evaluate(id => {
          const workspace = (window as any).__gwtPlaywrightMessages.map((entry: any) => entry.payload)
            .findLast((event: any) => event.kind === 'workspace_state')?.workspace;
          return workspace?.tabs.flatMap((tab: any) => tab.workspace.windows)
            .find((window: any) => window.id === id)?.persist;
        }, id)).toBe(false);
        await page.evaluate(id => {
          const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
            candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
          socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ kind: 'terminal_snapshot', id,
            data_base64: btoa('PM Works terminal identity\r\n') }) }));
        }, id);
        // Capture the native xterm before using the new cross-surface entry.
        await pane.getByRole('button', { name: 'Execution log', exact: true }).click();
        await expect(pane.locator('.xterm')).toHaveCount(1);
        await pane.locator('.xterm').evaluate(node => { (window as any).__gwtChatWorksTerminal = node; });
        await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi!.bufferText(id!), id))
          .toContain('PM Works terminal identity');
        await pane.getByRole('button', { name: 'Chat', exact: true }).click();
        const chat = pane.getByRole('region', { name: 'PM conversation' });
        const draft = chat.getByRole('textbox', { name: 'Message to PM' });
        await draft.fill('Keep this unsent PM draft');
        await chat.getByRole('button', { name: 'Open terminal in Works', exact: true }).click();
        const main = page.getByRole('region', { name: 'Main pane', exact: true });
        const works = main.locator('.works-surface:not(.works-surface--preview)');
        const full = works.getByRole('region', { name: 'Work terminal', exact: true });
        await expect(main).toHaveAttribute('data-surface', 'works');
        await expect(full).toBeVisible();
        await expect(full.locator('.window-role-badge')).toHaveText('PM');
        await expect(full.locator(`.terminal-root[data-window-id="${id}"] .xterm`)).toBeVisible();
        expect(await full.locator('.xterm').evaluate(node => node === (window as any).__gwtChatWorksTerminal)).toBe(true);
        await expect(chat).toBeHidden();
        const screenshot = info.outputPath(`chat-works-${theme}.png`);
        await page.screenshot({ path: screenshot });
        await info.attach('Chat opens the same PM terminal in Works', { path: screenshot, contentType: 'image/png' });

        await page.locator('#op-pm-entry').click();
        await expect(chat).toBeVisible();
        await expect(pane.getByRole('button', { name: 'Chat', exact: true })).toHaveAttribute('aria-pressed', 'true');
        await expect(draft).toHaveValue('Keep this unsent PM draft');
        await info.attach(`Chat terminal action ${theme}`, { body: await pane.screenshot(), contentType: 'image/png' });
        await page.getByRole('button', { name: 'Works', exact: true }).click();
        await full.getByRole('button', { name: 'Back to Works' }).click();
        const row = works.locator(`.work-row[data-window-id="${id}"]`);
        await expect(row.locator('.window-role-badge')).toHaveText('PM');
        await row.getByRole('button', { name: 'Open terminal', exact: true }).click();
        await expect(full).toBeVisible();
        expect(await full.locator('.xterm').evaluate(node => node === (window as any).__gwtChatWorksTerminal)).toBe(true);
        await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi!.bufferText(id!), id))
          .toContain('PM Works terminal identity');
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        expect(errors).toEqual([]);
      } finally {
        retainPm = false;
        if (!page.isClosed()) await sendLiveGwtEvent(page, { kind: 'frontend_ready' })
          .catch(error => { if (!page.isClosed()) throw error; });
        if (!page.isClosed()) await page.evaluate(() => {
          delete (window as any).__gwtChatWorksTerminal;
        }).catch(error => { if (!page.isClosed()) throw error; });
      }
    });
  });

  test('AC-3431-CHAT-2 Chat shows only conversation text from native Claude and Codex records', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      // The checkout host inherits these; a set value would move its native store off the isolated HOME.
      expect(process.env.CLAUDE_CONFIG_DIR ?? '', 'Claude store follows the isolated HOME').toBe('');
      expect(process.env.CODEX_HOME ?? '', 'Codex store follows the isolated HOME').toBe('');
      expect(ROOT, 'native records are bound to the fixture PM checkout').not.toBe('');
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      const sessionPath = join(HOME, '.gwt', 'sessions', `${SESSION}.toml`);
      const originalSession = readFileSync(sessionPath, 'utf8');
      const fixtures = resolve(ROOT, 'crates/gwt/src/pm_conversation/fixtures');
      // Anonymized native records (see fixtures/README.md) bound to this fixture PM's checkout and identity.
      const seed = (agent: 'ClaudeCode' | 'Codex', fixture: string, path: string) => {
        const id = `chat-noise-${agent.toLowerCase()}-${theme}`;
        const records = readFileSync(join(fixtures, fixture), 'utf8')
          .replaceAll('"/work/project"', JSON.stringify(ROOT))
          .replaceAll('"conversation"', JSON.stringify(id));
        const file = join(HOME, path.replace('<id>', id));
        mkdirSync(join(file, '..'), { recursive: true });
        writeFileSync(file, records);
        // The seeded stopped-PM Session is intentionally minimal; the reader needs a loadable one.
        const session = originalSession.replace(/^agent_id = .*$/m, `agent_id = { type = "${agent}" }`);
        writeFileSync(sessionPath, `${session}\ndisplay_name = "PM"\nagent_session_id = "${id}"\n`);
        return file;
      };
      const seeded: string[] = [];
      try {
        seeded.push(seed('ClaudeCode', 'claude-chat.jsonl', '.claude/projects/chat-noise/<id>.jsonl'));
        await gotoPmReports(page, undefined, BASE, true);
        const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
        await pane.getByRole('button', { name: 'Chat', exact: true }).click();
        const chat = pane.getByRole('region', { name: 'PM conversation' });
        const conversation = chat.locator('.pm-chat__conversation');
        const expectOnlyVisibleConversation = async (userText: string) => {
          await expect(chat.locator('.pm-chat__bubble--user')).toHaveText([userText], { timeout: 30_000 });
          await expect(chat.locator('.pm-chat__bubble--assistant')).toHaveText(['Assistant reply']);
          await expect(chat.locator('details.pm-chat__collapsed')).toHaveCount(0);
          const shown = (await conversation.textContent()) ?? '';
          for (const hidden of ['\u001b', '⏺', '⎿', 'Bash(', 'internal tool output', 'fixture command', 'Internal',
            'Stop hook feedback', 'Scheduled supervision tick', 'Private reasoning', 'Duplicate human event']) {
            expect(shown, `chat must not show ${JSON.stringify(hidden)}`).not.toContain(hidden);
          }
        };
        // Claude: thinking, tool use/result (ANSI + terminal decoration), meta, Stop feedback, stop hook summary,
        // the scheduled tick and a task notification are in the record but never in the chat.
        await expectOnlyVisibleConversation('Human question');
        // Codex: developer message, tool call/output, reasoning and the duplicate event record are excluded.
        seeded.push(seed('Codex', 'codex-chat.jsonl', '.codex/sessions/2026/11/12/rollout-2026-11-12T00-00-00-<id>.jsonl'));
        await expectOnlyVisibleConversation('Human question\nAdditional context');
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        const screenshot = info.outputPath(`chat-native-records-${theme}.png`);
        await pane.screenshot({ path: screenshot });
        await info.attach('PM chat from native records', { path: screenshot, contentType: 'image/png' });
        expect(errors).toEqual([]);
      } finally {
        writeFileSync(sessionPath, originalSession);
        for (const file of seeded) rmSync(file, { force: true });
      }
    });
  });

  test('Issue #6235 Execution log preserves scrollback across replacement snapshots and reveal', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoPmReports(page);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const id = await pane.getAttribute('data-id');
      expect(id).toBeTruthy();
      const metrics = () => page.evaluate(id => window.__gwtTerminalTestApi!.metrics(id!), id);
      const emit = (kind: string, lines: number) => page.evaluate(({ id, kind, lines }) => {
        const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
          candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
        if (!socket) throw new Error('Missing live project socket');
        const text = Array.from({ length: lines }, (_, index) => `Execution line ${index + 1}`).join('\r\n') + '\r\n';
        socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ kind, id, data_base64: btoa(text) }) }));
      }, { id, kind, lines });
      await pane.getByRole('button', { name: 'Execution log', exact: true }).click();
      await emit('terminal_snapshot', 160);
      await expect.poll(async () => (await metrics()).baseY).toBeGreaterThan(0);
      await expect.poll(async () => { const value = await metrics(); return value.viewportY === value.baseY; }).toBe(true);
      await emit('terminal_snapshot', 180);
      await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi!.bufferText(id!), id)).toContain('Execution line 180');
      await expect.poll(async () => { const value = await metrics(); return value.viewportY === value.baseY; }).toBe(true);

      await pane.locator('.xterm-screen').hover();
      await page.mouse.wheel(0, -600);
      await expect.poll(async () => { const value = await metrics(); return value.viewportY < value.baseY; }).toBe(true);
      const reading = (await metrics()).viewportY;
      expect(reading).toBeGreaterThan(0);
      await emit('terminal_snapshot', 200);
      await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi!.bufferText(id!), id)).toContain('Execution line 200');
      await expect.poll(async () => (await metrics()).viewportY).toBe(reading);
      const beforeLiveOutput = (await metrics()).baseY;
      await emit('terminal_output', 5);
      await expect.poll(async () => (await metrics()).baseY).toBeGreaterThan(beforeLiveOutput);
      await expect.poll(async () => (await metrics()).viewportY).toBe(reading);
      await pane.getByRole('button', { name: 'Reports', exact: true }).click();
      await emit('terminal_snapshot', 220);
      await pane.getByRole('button', { name: 'Tasks', exact: true }).click();
      await pane.getByRole('button', { name: 'Execution log', exact: true }).click();
      await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi!.bufferText(id!), id)).toContain('Execution line 220');
      await expect.poll(async () => (await metrics()).viewportY).toBe(reading);
      await page.getByRole('button', { name: 'Works', exact: true }).click();
      await emit('terminal_snapshot', 240);
      await page.locator('#op-pm-entry').click();
      await expect(pane.locator('.pm-conversation-log')).toBeVisible();
      await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi!.bufferText(id!), id)).toContain('Execution line 240');
      await expect.poll(async () => (await metrics()).viewportY).toBe(reading);
      await page.evaluate(id => window.__gwtTerminalTestApi!.scrollToBottom(id!), id);
      const previousBase = (await metrics()).baseY;
      await emit('terminal_output', 8);
      await expect.poll(async () => (await metrics()).baseY).toBeGreaterThan(previousBase);
      await expect.poll(async () => { const value = await metrics(); return value.viewportY === value.baseY; }).toBe(true);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const screenshot = info.outputPath(`execution-log-scroll-${theme}.png`);
      await pane.screenshot({ path: screenshot });
      await info.attach('Execution log scroll', { path: screenshot, contentType: 'image/png' });
      expect(errors).toEqual([]);
    });
  });

  test('Reports and Execution log render the same window version for every receive without focus or animation frames', async ({ page, playwright }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoPmReports(page);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const reports = pane.getByRole('log', { name: 'PM reports' });
      await expect(reports.getByRole('heading', { name: 'Persisted before restart', exact: true })).toBeVisible({ timeout: 30_000 });
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const id = await pane.getAttribute('data-id');
      expect(id).toBeTruthy();
      const terminal = await pane.locator('.terminal-root').elementHandle();
      const marker = `Shared version ${theme}`;
      // Complete the normal first-fit handshake before measuring an inactive
      // terminal. A never-revealed log intentionally defers its initial bytes.
      await pane.getByRole('button', { name: 'Execution log', exact: true }).click();
      await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi!.metrics(id!).isReady, id)).toBe(true);
      await pane.getByRole('button', { name: 'Reports', exact: true }).click();
      await expect(reports).toBeVisible();
      await page.locator('#op-pm-entry').focus();
      await expect(page.locator('#op-pm-entry')).toBeFocused();

      // Controlled inbound frames exercise the real checkout receive/model/view
      // deadline with rAF withheld, not real PTY throughput or OS background paint.
      const evidence = await page.evaluate(async ({ id, marker }) => {
        const pane = document.querySelector(`.workspace-window[data-id="${id}"]`)!;
        const reportRoot = pane.querySelector<HTMLElement>('.pm-chat')!;
        const logRoot = pane.querySelector<HTMLElement>('.pm-conversation-log')!;
        const versions = () => [reportRoot.dataset.stateVersion, logRoot.dataset.stateVersion];
        const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
          candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
        const original = (window as any).__gwtPlaywrightMessages.map((entry: any) => entry.payload)
          .findLast((event: any) => event.kind === 'pm_reports' && event.id === id);
        if (!socket || !original) throw new Error('Missing live project socket or hydrated Reports');
        const dispatch = (raw: string, reportText: string, ...events: object[]) => new Promise<{
          before: (string | undefined)[]; after: (string | undefined)[]; elapsed_ms: number;
        }>((resolve, reject) => {
          const before = versions();
          const startedAt = performance.now();
          let after: (string | undefined)[] | undefined;
          const check = () => {
            const elapsed_ms = performance.now() - startedAt;
            if (after && reportRoot.textContent!.includes(reportText)
              && window.__gwtTerminalTestApi!.bufferText(id!).includes(raw)) {
              resolve({ before, after, elapsed_ms });
            } else if (elapsed_ms >= 1_000) {
              observer.disconnect();
              reject(new Error(`Receive did not update both views and RAW buffer within one second: ${JSON.stringify({
                before, after, reportMatched: reportRoot.textContent!.includes(reportText),
                rawMatched: window.__gwtTerminalTestApi!.bufferText(id!).includes(raw),
                terminal: window.__gwtTerminalTestApi!.metrics(id!),
              })}`));
            } else timer = setTimeout(check, 100);
          };
          const observer = new MutationObserver(() => {
            observer.disconnect();
            after = versions();
            clearTimeout(timer);
            check();
          });
          let timer = setTimeout(check, 100);
          observer.observe(reportRoot, { attributes: true, attributeFilter: ['data-state-version'] });
          for (const event of events) socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(event) }));
        });
        // Obtain a real model-issued force token without sending this controlled
        // request to the backend, before withholding animation callbacks.
        let request: any;
        const originalSend = socket.send;
        socket.send = function (data: string) {
          const payload = JSON.parse(data);
          if (payload.kind === 'load_pm_reports' && payload.id === id) request = payload;
          else originalSend.call(this, data);
        };
        try { document.dispatchEvent(new Event('visibilitychange')); }
        finally { socket.send = originalSend; }
        if (!request) throw new Error('Missing model-issued Reports request');
        const originalFrame = window.requestAnimationFrame;
        const originalCancel = window.cancelAnimationFrame;
        const captured = new Map<number, FrameRequestCallback>();
        let requestedFrames = 0;
        let focusEvents = 0;
        const onFocus = () => { focusEvents += 1; };
        pane.addEventListener('focusin', onFocus);
        window.requestAnimationFrame = callback => {
          const handle = -(++requestedFrames);
          captured.set(handle, callback);
          return handle;
        };
        window.cancelAnimationFrame = handle => {
          if (!captured.delete(handle)) originalCancel.call(window, handle);
        };
        try {
          const snapshot = await dispatch('PAIR RAW snapshot', 'Persisted before restart',
            { kind: 'terminal_snapshot', id, data_base64: btoa('PAIR RAW snapshot\r\n') });
          const report = await dispatch('PAIR RAW snapshot', marker, { ...original, request_id: request.request_id, snapshot: true,
            version: { epoch: original.version.epoch, revision: original.version.revision + 1 }, reports: [...original.reports, {
            id: 'shared-version-report', kind: 'progress', created_at: '2026-10-19T00:00:00Z',
            body: `## ${marker}`, body_html: `<h2>${marker}</h2>`,
          }] });
          const reportNodes = [...reportRoot.querySelectorAll('article')];
          const output = await dispatch('PAIR RAW appended', marker,
            { kind: 'terminal_output', id, data_base64: btoa('PAIR RAW appended\r\n') });
          const reportsRetainedOnOutput = reportNodes.every((node, index) => node === reportRoot.querySelectorAll('article')[index]);
          const reportText = reportRoot.textContent;
          // The following valid packet fences dispatcher processing of the stale
          // frame: only that valid packet may advance the shared version.
          const stale = await dispatch('PAIR RAW fence', marker,
            { ...original, session_id: 'stale-session', reports: [], error: 'STALE REPORT ERROR' },
            { kind: 'terminal_output', id, data_base64: btoa('PAIR RAW fence\r\n') },
          );
          return { snapshot, report, output, stale, reportsRetainedOnOutput,
            reportsRetainedOnStale: reportRoot.textContent === reportText,
            requestedFrames, capturedFrames: captured.size, focusEvents,
            windowFocused: pane.contains(document.activeElement) };
        } finally {
          window.requestAnimationFrame = originalFrame;
          window.cancelAnimationFrame = originalCancel;
          pane.removeEventListener('focusin', onFocus);
          for (const callback of captured.values()) originalFrame.call(window, callback);
        }
      }, { id, marker });
      for (const step of [evidence.snapshot, evidence.report, evidence.output, evidence.stale]) {
        expect(step.before[0]).toMatch(/^\d+$/);
        expect(Number(step.before[0])).toBeGreaterThan(0);
        expect(step.before[1], 'both views start at the same window version').toBe(step.before[0]);
        expect(step.after, 'each receive advances both views together').toEqual([
          String(Number(step.before[0]) + 1), String(Number(step.before[0]) + 1),
        ]);
        expect(step.elapsed_ms, 'joint DOM and RAW buffer advance within one second').toBeLessThanOrEqual(1_000);
      }
      expect(evidence.requestedFrames, 'animation callbacks were withheld').toBeGreaterThan(0);
      expect(evidence.capturedFrames).toBeGreaterThan(0);
      expect(evidence.windowFocused).toBe(false);
      expect(evidence.focusEvents, 'no target focus restored updates').toBe(0);
      expect(evidence.reportsRetainedOnOutput).toBe(true);
      expect(evidence.reportsRetainedOnStale).toBe(true);
      await expect(reports.getByRole('heading', { name: marker, exact: true })).toBeVisible();
      await expect(reports).not.toContainText('PAIR RAW');
      await expect(reports).not.toContainText('STALE REPORT ERROR');
      const reportsImage = info.outputPath(`shared-reports-${theme}.png`);
      await pane.screenshot({ path: reportsImage });
      await info.attach('Reports shared version', { path: reportsImage, contentType: 'image/png' });
      await pane.getByRole('button', { name: 'Execution log', exact: true }).click();
      await expect(pane.locator('.xterm')).toContainText('PAIR RAW snapshot');
      await expect(pane.locator('.xterm')).toContainText('PAIR RAW appended');
      await expect(pane.locator('.xterm')).toContainText('PAIR RAW fence');
      await expect(pane.locator('.xterm')).not.toContainText(marker);
      expect(await pane.locator('.terminal-root').evaluate((node, original) => node === original, terminal)).toBe(true);
      const logImage = info.outputPath(`shared-execution-log-${theme}.png`);
      await pane.screenshot({ path: logImage });
      await info.attach('Execution log shared version', { path: logImage, contentType: 'image/png' });
      const versionsFile = info.outputPath(`shared-window-versions-${theme}.json`);
      writeFileSync(versionsFile, JSON.stringify(evidence, null, 2));
      await info.attach('Per-receive window versions (controlled inbound frames)', {
        path: versionsFile, contentType: 'application/json',
      });
      if (info.project.use.headless === false) {
        const native = await withNativeHiddenPage(playwright.chromium, info, async nativePage => {
          await installLiveGwtInitScript(nativePage, { enableTestBridge: true });
          await nativePage.addInitScript(theme => localStorage.setItem('gwt:ui:theme', theme), theme);
          await nativePage.goto(page.url());
          await expect.poll(() => nativePage.evaluate(() => document.documentElement.dataset.theme)).toBe(theme);
          await expect.poll(() => nativePage.evaluate(() => document.querySelectorAll('.pm-chat').length)).toBe(1);
          await expect.poll(() => nativePage.evaluate(() => document.querySelector('#op-pm-entry')?.getAttribute('data-pm-state'))).toBe('stopped');
          await nativePage.evaluate(() => (document.querySelector('#op-pm-entry') as HTMLElement).click());
          await expect.poll(() => nativePage.evaluate(id => (window as any).__gwtPlaywrightMessages
            .some((entry: any) => entry.payload.kind === 'pm_reports' && entry.payload.id === id), id)).toBe(true);
          await expect.poll(() => nativePage.evaluate(() => document.querySelector('.pm-chat')?.textContent
            ?.includes('Persisted before restart'))).toBe(true);
          // Hidden windows intentionally suppress polling. Capture the normal
          // model-issued token while visible, then deliver its response hidden.
          await nativePage.evaluate(id => {
            const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
              candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
            const send = socket.send;
            socket.send = function (data: string) {
              const request = JSON.parse(data);
              if (request.kind === 'load_pm_reports' && request.id === id) (window as any).__nativeReportsRequest = request;
              else send.call(this, data);
            };
            try { document.dispatchEvent(new Event('visibilitychange')); }
            finally { socket.send = send; }
            if (!(window as any).__nativeReportsRequest) throw new Error('Missing visible model-issued Reports token');
          }, id);
        }, async ({ id, marker }) => {
          const pane = document.querySelector(`.workspace-window[data-id="${id}"]`)!;
          const reports = pane.querySelector<HTMLElement>('.pm-chat')!;
          const log = pane.querySelector<HTMLElement>('.pm-conversation-log')!;
          const versions = () => [reports.dataset.stateVersion, log.dataset.stateVersion];
          const before = versions(), hiddenBefore = document.hidden, focusedBefore = document.hasFocus();
          const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
            candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
          const original = (window as any).__gwtPlaywrightMessages.map((entry: any) => entry.payload)
            .findLast((event: any) => event.kind === 'pm_reports' && event.id === id);
          const request = (window as any).__nativeReportsRequest;
          let focusEvents = 0, visibilityChanges = 0;
          const onFocus = () => { focusEvents++; }, onVisibility = () => { visibilityChanges++; };
          window.addEventListener('focus', onFocus);
          pane.addEventListener('focusin', onFocus);
          document.addEventListener('visibilitychange', onVisibility);
          try {
            return await new Promise<any>((resolve, reject) => {
              const startedAt = performance.now();
              const finish = () => { observer.disconnect(); clearTimeout(timer); };
              const observer = new MutationObserver(() => {
                if (!reports.textContent!.includes(marker)) return;
                const elapsedMs = performance.now() - startedAt;
                finish();
                resolve({ before, after: versions(), elapsedMs, hiddenBefore, focusedBefore,
                  hiddenAfter: document.hidden, focusedAfter: document.hasFocus(), focusEvents, visibilityChanges });
              });
              const timer = setTimeout(() => { finish(); reject(new Error('Native hidden Reports did not render within one second')); }, 1000);
              observer.observe(pane, { attributes: true, childList: true, characterData: true, subtree: true });
              socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({ ...original,
                request_id: request.request_id, snapshot: true,
                version: { epoch: original.version.epoch, revision: original.version.revision + 1 },
                reports: [...original.reports, { id: 'native-shared-version-report', kind: 'progress',
                  created_at: '2026-11-05T00:00:00Z', body: `## ${marker}`, body_html: `<h2>${marker}</h2>` }],
              }) }));
            });
          } finally {
            window.removeEventListener('focus', onFocus);
            pane.removeEventListener('focusin', onFocus);
            document.removeEventListener('visibilitychange', onVisibility);
            delete (window as any).__nativeReportsRequest;
          }
        }, { id, marker: `Native shared version ${theme}` });
        await info.attach('native-hidden-shared-window-versions.json', { contentType: 'application/json', body: JSON.stringify(native) });
        expect(native.before[0]).toMatch(/^\d+$/);
        expect(native.before[1]).toBe(native.before[0]);
        expect(native.after).toEqual([String(Number(native.before[0]) + 1), String(Number(native.before[0]) + 1)]);
        expect(native.elapsedMs).toBeGreaterThanOrEqual(0);
        expect(native.elapsedMs).toBeLessThanOrEqual(1000);
        expect([native.hiddenBefore, native.hiddenAfter]).toEqual([true, true]);
        expect([native.focusedBefore, native.focusedAfter]).toEqual([false, false]);
        expect(native.focusEvents).toBe(0);
        expect(native.visibilityChanges).toBe(0);
      }
      expect(errors).toEqual([]);
    });
  });

  test('lost Reports response and revision gap recover through a full snapshot', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      // Install the loss injector when the socket is constructed, before the
      // app registers its receive listener. A later listener cannot undo a
      // frame that an earlier listener has already handed to the dispatcher.
      await page.addInitScript(() => {
        const NativeSocket = window.WebSocket;
        window.WebSocket = new Proxy(NativeSocket, {
          construct(Target, args) {
            const socket = new Target(...args as ConstructorParameters<typeof WebSocket>);
            socket.addEventListener('message', event => {
              const state = (window as any).__reportsRecovery;
              if (!state?.armed) return;
              const payload = JSON.parse(String(event.data));
              if (payload.kind !== 'pm_reports' || payload.id !== state.id || payload.snapshot
                || !payload.reports.some((report: any) => report.body === `# ${state.first}`)) return;
              state.dropped = payload;
              state.armed = false;
              event.stopImmediatePropagation();
            });
            return socket;
          },
        });
      });
      await gotoPmReports(page);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const reports = pane.getByRole('log', { name: 'PM reports' });
      await expect(reports.getByRole('heading', { name: 'Persisted before restart', exact: true })).toBeVisible();
      const id = await pane.getAttribute('data-id');
      const first = `Lost response ${theme}`;
      const second = `Recovered gap ${theme}`;
      await page.evaluate(({ id, first }) => {
        const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
          candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
        const requests: any[] = [];
        (window as any).__reportsRecovery = { requests, dropped: null, armed: true, id, first };
        const send = socket.send;
        socket.send = function (data: string) {
          const request = JSON.parse(data);
          if (request.kind === 'load_pm_reports' && request.id === id) requests.push(request);
          return send.call(this, data);
        };
      }, { id, first });
      // The first render can precede a pending force reply. Observe a normal
      // acknowledged poll before changing the ledger and arming the loss case.
      await expect.poll(() => page.evaluate(() => (window as any).__reportsRecovery.requests
        .some((request: any) => request.version !== null)), { timeout: 20_000 }).toBe(true);
      const fixture = JSON.parse(readFileSync(`${HOME}/issue-5023-isolated.json`, 'utf8'));
      const history = `${HOME}/.gwt/projects/${fixture.repo_hash}/project-state/pm-reports.jsonl`;
      const append = (body: string) => appendFileSync(history, `${JSON.stringify({
        id: randomUUID(), kind: 'progress', body: `# ${body}`, created_at: new Date().toISOString(),
      })}\n`);
      append(first);
      await expect.poll(() => page.evaluate(() => (window as any).__reportsRecovery.dropped?.version.revision),
        { timeout: 20_000 }).toBeGreaterThan(0);
      await expect(reports.getByRole('heading', { name: first, exact: true })).toHaveCount(0);
      append(second);
      await expect(reports.getByRole('heading', { name: first, exact: true })).toBeVisible({ timeout: 20_000 });
      await expect(reports.getByRole('heading', { name: second, exact: true })).toBeVisible();
      const recovery = await page.evaluate(id => {
        const state = (window as any).__reportsRecovery;
        const received = (window as any).__gwtPlaywrightMessages.map((entry: any) => entry.payload)
          .filter((event: any) => event.kind === 'pm_reports' && event.id === id);
        return { dropped: state.dropped, requests: state.requests, received };
      }, id);
      const gap = recovery.received.find((event: any) => !event.snapshot
        && event.version.revision > recovery.dropped.version.revision);
      expect(gap, 'ordinary response exposes a missing revision').toBeTruthy();
      const reset = recovery.received.find((event: any) => event.snapshot && event.request_id > gap.request_id);
      expect(reset?.version, 'force response supplies the entire latest slice').toEqual(gap.version);
      expect(recovery.requests.find((request: any) => request.request_id === reset.request_id)?.version).toBeNull();
      await expect(pane.locator('.pm-chat')).toHaveAttribute('data-state-version',
        await pane.locator('.pm-conversation-log').getAttribute('data-state-version') as string);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const screenshot = info.outputPath(`reports-recovery-${theme}.png`);
      await pane.screenshot({ path: screenshot });
      await info.attach('Reports recovery', { path: screenshot, contentType: 'image/png' });
      await page.reload();
      await gotoPmReports(page);
      await expect(reports.getByRole('heading', { name: second, exact: true })).toBeVisible();
      expect(errors).toEqual([]);
    });
  });

  test('unchanged polls are silent, inactive Reports receive live deltas and reconnect snapshots', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoPmReports(page);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const reports = pane.getByRole('log', { name: 'PM reports' });
      await expect(reports.getByRole('heading', { name: 'Persisted before restart', exact: true })).toBeVisible({ timeout: 30_000 });
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const id = await pane.getAttribute('data-id');
      expect(id).toBeTruthy();
      const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      const version = await page.evaluate(id => (window as any).__gwtPlaywrightMessages
        .findLast((entry: any) => entry.payload.kind === 'pm_reports' && entry.payload.id === id).payload.version, id);
      for (let index = 0; index < 3; index++) await sendLiveGwtEvent(page, {
        kind: 'load_pm_reports', id, request_id: 0, version,
      });
      // Observe an entire real frontend polling interval: silence is the contract.
      await page.waitForTimeout(6_000);
      const unchanged = await page.evaluate(({ id, cursor }) => (window as any).__gwtPlaywrightMessages.filter((entry: any) =>
        entry.sequence > cursor && entry.payload.kind === 'pm_reports' && entry.payload.id === id), { id, cursor });
      expect(unchanged).toHaveLength(0);

      let lastMarker = '';
      for (const snapshot of [false, true]) {
        const phase = snapshot ? 'snapshot' : 'delta';
        const marker = `${phase} report ${info.project.name} ${randomUUID()}`;
        const fixture = JSON.parse(readFileSync(`${HOME}/issue-5023-isolated.json`, 'utf8'));
        await page.locator('#op-pm-entry').focus();
        await expect(page.locator('#op-pm-entry')).toBeFocused();
        await page.evaluate(({ id, marker, snapshot }) => {
          const pane = document.querySelector(`.workspace-window[data-id="${id}"]`)!;
          const report = pane.querySelector<HTMLElement>('.pm-chat')!;
          const log = pane.querySelector<HTMLElement>('.pm-conversation-log')!;
          const versions = () => [report.dataset.stateVersion, log.dataset.stateVersion];
          const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
            candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
          if (!socket) throw new Error('Missing live project socket');
          const originalSocket = window.WebSocket;
          const observedSockets = new Set([socket]);
          const interceptedSockets = new Set<WebSocket>();
          const heldHydration: Array<() => void> = [];
          let holdHydration = snapshot;
          const originalFrame = window.requestAnimationFrame;
          const originalCancel = window.cancelAnimationFrame;
          const captured = new Map<number, FrameRequestCallback>();
          let requestedFrames = 0;
          let focusEvents = 0;
          let receivedAt: number | null = null;
          let before: (string | undefined)[] = [];
          const state: any = { result: null, snapshot, receivedSnapshot: null, replacementSocket: false,
            heldRequests: 0, forceHeld: false };
          const check = () => {
            if (receivedAt === null || state.result) return;
            const after = versions();
            const elapsed_ms = performance.now() - receivedAt;
            const updated = report.textContent!.includes(marker) && after[0] === after[1]
              && Number(after[0]) > Number(before[0]);
            if (updated || elapsed_ms >= 1_000) state.result = {
              before, after, elapsed_ms, updated, requestedFrames, focusEvents,
              snapshot: state.receivedSnapshot, replacementSocket: state.replacementSocket,
              heldRequests: state.heldRequests, forceHeld: state.forceHeld,
              nodesConnected: pane.isConnected && report.isConnected && log.isConnected,
              documentHidden: document.hidden, windowFocused: pane.contains(document.activeElement),
            };
          };
          const onMessage = (event: MessageEvent) => {
            const payload = JSON.parse(String(event.data));
            // Time only wire receipt -> joint DOM, excluding poll/reconnect wait.
            // Both paths use the real backend without focus/visibility triggers.
            if (receivedAt === null && payload.kind === 'pm_reports' && payload.id === id && payload.snapshot === snapshot
              && payload.reports.some((entry: any) => entry.body === `# ${marker}`)) {
              before = versions();
              receivedAt = performance.now();
              state.receivedSnapshot = payload.snapshot;
              state.replacementSocket = event.currentTarget !== socket;
            }
          };
          const onFocus = () => { focusEvents += 1; };
          const observer = new MutationObserver(check);
          observer.observe(report, { attributes: true, childList: true, subtree: true });
          socket.addEventListener('message', onMessage);
          if (snapshot) window.WebSocket = new Proxy(originalSocket, {
            construct(Target, args) {
              const replacement = new Target(...args as ConstructorParameters<typeof WebSocket>);
              if (new URL(replacement.url).searchParams.has('repo_hash')) {
                observedSockets.add(replacement);
                replacement.addEventListener('message', onMessage);
                const originalSend = replacement.send;
                interceptedSockets.add(replacement);
                replacement.send = data => {
                  let request: any;
                  try { request = JSON.parse(String(data)); } catch { /* forward non-JSON unchanged */ }
                  if (holdHydration && request?.kind === 'load_pm_reports' && request.id === id) {
                    heldHydration.push(() => originalSend.call(replacement, data));
                    state.heldRequests += 1;
                    if (request.version === null) state.forceHeld = true;
                    return;
                  }
                  originalSend.call(replacement, data);
                };
              }
              return replacement;
            },
          });
          state.disconnect = () => new Promise<void>(resolve => {
            socket.addEventListener('close', () => resolve(), { once: true });
            socket.close();
          });
          state.releaseHydration = () => {
            holdHydration = false;
            for (const send of heldHydration.splice(0)) send();
          };
          pane.addEventListener('focusin', onFocus);
          window.requestAnimationFrame = callback => {
            const handle = -(++requestedFrames);
            captured.set(handle, callback);
            return handle;
          };
          window.cancelAnimationFrame = handle => {
            if (!captured.delete(handle)) originalCancel.call(window, handle);
          };
          const timer = setInterval(check, 100);
          state.restore = () => {
            clearInterval(timer);
            observer.disconnect();
            heldHydration.length = 0;
            for (const candidate of interceptedSockets) delete (candidate as any).send;
            for (const candidate of observedSockets) candidate.removeEventListener('message', onMessage);
            window.WebSocket = originalSocket;
            pane.removeEventListener('focusin', onFocus);
            window.requestAnimationFrame = originalFrame;
            window.cancelAnimationFrame = originalCancel;
            for (const callback of captured.values()) originalFrame.call(window, callback);
          };
          (window as any).__inactiveReportsPoll = state;
        }, { id, marker, snapshot });
        let evidence: any;
        try {
          if (snapshot) {
            await page.evaluate(() => (window as any).__inactiveReportsPoll.disconnect());
            // Observe the app's forced hydration before writing, then release
            // its unchanged request only after the marker is durable.
            await page.waitForFunction(() => (window as any).__inactiveReportsPoll.forceHeld,
              undefined, { polling: 100, timeout: 20_000 });
          }
          appendFileSync(`${HOME}/.gwt/projects/${fixture.repo_hash}/project-state/pm-reports.jsonl`, `${JSON.stringify({
            id: randomUUID(), kind: 'progress', body: `# ${marker}`, created_at: new Date().toISOString(),
          })}\n`);
          if (snapshot) await page.evaluate(() => (window as any).__inactiveReportsPoll.releaseHydration());
          await page.waitForFunction(() => (window as any).__inactiveReportsPoll.result,
            undefined, { polling: 100, timeout: 20_000 });
          evidence = await page.evaluate(() => (window as any).__inactiveReportsPoll.result);
        } finally {
          await page.evaluate(() => { (window as any).__inactiveReportsPoll.restore(); delete (window as any).__inactiveReportsPoll; });
        }
        const evidenceFile = info.outputPath(`inactive-reports-${phase}-${theme}.json`);
        writeFileSync(evidenceFile, JSON.stringify(evidence, null, 2));
        await info.attach(`Real backend ${phase} to joint DOM (rAF withheld)`, { path: evidenceFile, contentType: 'application/json' });
        expect(evidence.updated, `backend ${phase} updates joint DOM within one second without focus or frames`).toBe(true);
        expect(evidence.snapshot).toBe(snapshot);
        expect(evidence.replacementSocket).toBe(snapshot);
        expect(evidence.nodesConnected).toBe(true);
        expect(evidence.forceHeld).toBe(snapshot);
        if (snapshot) expect(evidence.heldRequests).toBeGreaterThan(0);
        expect(evidence.elapsed_ms).toBeLessThanOrEqual(1_000);
        expect(evidence.before[0]).toMatch(/^\d+$/);
        expect(evidence.before[1]).toBe(evidence.before[0]);
        expect(evidence.after[1]).toBe(evidence.after[0]);
        expect(evidence.requestedFrames).toBeGreaterThan(0);
        expect(evidence.documentHidden).toBe(false);
        expect(evidence.windowFocused).toBe(false);
        expect(evidence.focusEvents).toBe(0);
        await expect(reports.getByRole('heading', { name: marker, exact: true })).toBeVisible();
        const screenshot = info.outputPath(`inactive-reports-${phase}-${theme}.png`);
        await pane.screenshot({ path: screenshot });
        await info.attach(`Inactive Reports after ${phase}`, { path: screenshot, contentType: 'image/png' });
        lastMarker = marker;
      }
      await reloadPmReports(page);
      await expect(reports.getByRole('heading', { name: lastMarker, exact: true })).toBeVisible();
      expect(errors).toEqual([]);
    });
  });

  test('peer PM sent and received reports persist without duplicates', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoPmReports(page);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const reports = pane.getByRole('log', { name: 'PM reports' });
      await expect(reports).toBeVisible({ timeout: 30_000 });
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const id = await pane.getAttribute('data-id');
      expect(id).toBeTruthy();

      // Use only the stopped PM registration in this owned fixture. No live
      // recipient or ambient agent session is claimed by the UI regression.
      const childEnv: NodeJS.ProcessEnv = { ...process.env, HOME, USERPROFILE: HOME };
      for (const key of Object.keys(childEnv)) {
        if (key.startsWith('GWT_SESSION_') || key.startsWith('GWT_HOOK_FORWARD_') || key === 'GWT_PANE_WS_URL') delete childEnv[key];
      }
      childEnv.GWT_SESSION_ID = SESSION;
      const peer = `${HOME}/issue-3431-missing-peer-${randomUUID()}`;
      expect(existsSync(peer)).toBe(false);
      const outgoingText = `Checkpoint requested ${info.project.name}\nPlease reply when available.`;
      const result = spawnSync(`${ROOT}/target/debug/gwtd`, [], {
        cwd: ROOT, env: childEnv, encoding: 'utf8',
        input: JSON.stringify({ schema_version: 1, operation: 'pm.peer.send', params: {
          project: peer, kind: 'request', text: outgoingText, timeout_seconds: 60,
        } }),
      });
      expect(result.status, result.stderr).toBe(0);
      const envelope = JSON.parse(result.stdout);
      expect(envelope.ok).toBe(true);
      const outgoing = JSON.parse(envelope.output);
      expect(outgoing.status).toBe('unavailable');
      expect(outgoing.message.unavailable).toBeTruthy();

      // Real two-project delivery/reply is covered by Rust integration tests;
      // this owned store entry exercises the received Reports projection only.
      const storePath = `${HOME}/.gwt/pm-peer-messages.json`;
      const store = JSON.parse(readFileSync(storePath, 'utf8'));
      expect(store.schema_version).toBe(1);
      const incoming = {
        id: randomUUID(), from: outgoing.message.to, to: outgoing.message.from,
        kind: 'notice', text: `Peer checkpoint received ${info.project.name}\nSaved successfully.`,
        reply_to: null, created_at: new Date().toISOString(),
        expires_at: new Date(Date.now() + 60_000).toISOString(), answered_by: null, unavailable: null,
      };
      store.messages.push(incoming);
      const seededStorePath = `${storePath}.${incoming.id}.tmp`;
      writeFileSync(seededStorePath, JSON.stringify(store));
      renameSync(seededStorePath, storePath);
      await refreshPmReports(page);
      const outgoingReport = reports.locator('article').filter({ hasText: `PM peer ${outgoing.message.id} →` });
      const incomingReport = reports.locator('article').filter({ hasText: `PM peer ${incoming.id} ←` });
      const outgoingBody = `PM peer ${outgoing.message.id} → Request ${peer}: ${outgoingText.replace(/\n/g, ' ')} — Waiting for the peer project (PM absent, stopped, or liveness unconfirmed)`;
      const incomingBody = `PM peer ${incoming.id} ← Notice ${peer}: ${incoming.text.replace(/\n/g, ' ')}`;
      for (const [report, body] of [[outgoingReport, outgoingBody], [incomingReport, incomingBody]] as const) {
        await expect(report).toHaveCount(1);
        await expect(report.locator('.pm-chat__text')).toHaveText(body);
        expect(await report.locator('.pm-chat__text').textContent()).toBe(body);
      }
      await refreshPmReports(page);
      await expect(outgoingReport).toHaveCount(1);
      await expect(incomingReport).toHaveCount(1);
      await reloadPmReports(page);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(outgoingReport).toHaveCount(1);
      await expect(incomingReport).toHaveCount(1);
      await expect(outgoingReport.locator('.pm-chat__text')).toHaveText(outgoingBody);
      await expect(incomingReport.locator('.pm-chat__text')).toHaveText(incomingBody);
      await incomingReport.scrollIntoViewIfNeeded();
      const screenshot = info.outputPath(`pm-peer-reports-${info.project.name}.png`);
      await pane.screenshot({ path: screenshot });
      await info.attach('PM peer reports', { path: screenshot, contentType: 'image/png' });
      const validPeerStore = readFileSync(storePath);
      try {
        writeFileSync(storePath, '{');
        const nativeBody = `Native report during peer outage ${randomUUID()}`;
        const nativeEnv = { ...childEnv };
        delete nativeEnv.GWT_SESSION_ID;
        const nativeResult = spawnSync(`${ROOT}/target/debug/gwtd`, [], {
          cwd: ROOT, env: nativeEnv, encoding: 'utf8',
          input: JSON.stringify({ schema_version: 1, operation: 'pm.report.post', params: {
            kind: 'progress', body: nativeBody,
          } }),
        });
        expect(nativeResult.status, nativeResult.stderr).toBe(0);
        expect(JSON.parse(nativeResult.stdout).ok).toBe(true);
        await refreshPmReports(page);
        await expect(reports.getByRole('heading', { name: 'Persisted before restart', exact: true })).toBeVisible();
        await expect(reports.locator('article').filter({ hasText: nativeBody })).toHaveCount(1);
        await expect(outgoingReport).toHaveCount(0);
        await expect(incomingReport).toHaveCount(0);
      } finally {
        writeFileSync(seededStorePath, validPeerStore);
        renameSync(seededStorePath, storePath);
      }
      await refreshPmReports(page);
      await expect(outgoingReport).toHaveCount(1);
      await expect(incomingReport).toHaveCount(1);
      expect(errors).toEqual([]);
    });
  });

  test('automatic prompts add no Q&A rows while actions and human answers remain visible', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoPmReports(page);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const reports = pane.getByRole('log', { name: 'PM reports' });
      await expect(reports.getByRole('heading', { name: 'Persisted before restart', exact: true })).toBeVisible({ timeout: 30_000 });
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const id = await pane.getAttribute('data-id');
      expect(id).toBeTruthy();
      const originalRows = await reports.locator('article').count();
      const childEnv: NodeJS.ProcessEnv = { ...process.env, HOME, USERPROFILE: HOME };
      for (const key of Object.keys(childEnv)) {
        if (key.startsWith('GWT_SESSION_') || key.startsWith('GWT_HOOK_FORWARD_') || key === 'GWT_PANE_WS_URL') delete childEnv[key];
      }
      const post = (params: Record<string, unknown>) => spawnSync(`${ROOT}/target/debug/gwtd`, [], {
        cwd: ROOT, env: childEnv, encoding: 'utf8', timeout: 30_000,
        input: JSON.stringify({ schema_version: 1, operation: 'pm.report.post', params }),
      });
      for (const question of ['$gwt-pm', '$gwt-pm\n$gwt-pm']) {
        const refused = post({ kind: 'qa', question, body: 'No change from the automatic prompt' });
        expect(refused.error).toBeUndefined();
        expect(refused.status).toBe(1);
        expect(refused.stdout + refused.stderr).toContain('Automatic PM prompts');
      }
      await refreshPmReports(page);
      await expect(reports.locator('article')).toHaveCount(originalRows);
      await expect(reports.getByText('No change from the automatic prompt')).toHaveCount(0);

      const action = `Issue #5541 action delivered ${info.project.name}`;
      const progress = post({ kind: 'progress', body: action });
      expect(progress.status, progress.stderr).toBe(0);
      const question = '[gwt] Scheduled supervision tick について説明して';
      const answer = `Human answer preserved ${info.project.name}`;
      const human = post({ kind: 'qa', question, body: answer });
      expect(human.status, human.stderr).toBe(0);
      await refreshPmReports(page);
      await expect(reports.locator('article').filter({ hasText: action })).toHaveCount(1);
      await expect(reports.locator('article').filter({ hasText: answer })).toHaveCount(1);
      await expect(reports.getByText(question, { exact: true })).toBeVisible();
      await reloadPmReports(page);
      await expect(reports.locator('article')).toHaveCount(originalRows + 2);
      await expect(reports.getByText('No change from the automatic prompt')).toHaveCount(0);
      const screenshot = info.outputPath(`automatic-prompt-reports-${info.project.name}.png`);
      await pane.screenshot({ path: screenshot });
      await info.attach('Automatic prompt reporting', { path: screenshot, contentType: 'image/png' });
      expect(errors).toEqual([]);
    });
  });

  test('durable Markdown and Q&A reports, kind filter, copy, reload and unchanged execution log', async ({ page, context }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: new URL(BASE).origin });
      await gotoPmReports(page);
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const reports = pane.getByRole('log', { name: 'PM reports' });
      // Startup restores the PM registration after the canvas-ready round trip.
      // Allow that real backend hydration on a shared, loaded verification host.
      await expect(reports).toBeVisible({ timeout: 30_000 });
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(reports.getByRole('heading', { name: 'Persisted before restart', exact: true })).toBeVisible();
      const id = await pane.getAttribute('data-id');
      expect(id).toBeTruthy();
      const marker = `PM report ${info.project.name}`;
      const body = `# ${marker}\n\n**Verified** and \`cargo test\`\n\n- First item\n- Second item\n\n\`\`\`sh\ncargo test -p gwt-core\n\`\`\``;
      // This isolated fixture is a direct CLI caller, with no agent authority
      // inherited from the session running Playwright.
      const childEnv = { ...process.env, HOME, USERPROFILE: HOME };
      for (const key of Object.keys(childEnv)) {
        if (key.startsWith('GWT_SESSION_') || key.startsWith('GWT_HOOK_FORWARD_')) delete childEnv[key];
      }
      const pmOperation = (operation: string, params: Record<string, unknown> = {}) => {
        const result = spawnSync(`${ROOT}/target/debug/gwtd`, [], {
          cwd: ROOT, env: childEnv, encoding: 'utf8',
          input: JSON.stringify({ schema_version: 1, operation, params }),
        });
        expect(result.status, result.stderr).toBe(0);
        const envelope = JSON.parse(result.stdout);
        expect(envelope.ok).toBe(true);
        const output = JSON.parse(envelope.output);
        if ('ok' in output) expect(output.ok).toBe(true);
        return output;
      };
      const currentCard = pane.getByRole('region', { name: 'PM status', exact: true });
      await expect(currentCard).toBeVisible();
      await currentCard.locator('summary').click();
      // Seed only this owned isolated fixture's history to observe the real
      // backend's default fifteen-minute threshold without a wall-clock wait.
      const fixture = JSON.parse(readFileSync(`${HOME}/issue-5023-isolated.json`, 'utf8'));
      const oldStatus = {
        items: [{ kind: 'waiting', text: 'リリースの公開待ち', started_at: new Date(Date.now() - 20 * 60_000).toISOString() }],
        next_trigger: 'v9.117.2 の公開', user_action: null, stale_after_minutes: 15,
        updated_at: new Date(Date.now() - 16 * 60_000).toISOString(), monitor_fingerprint: null,
      };
      writeFileSync(`${HOME}/.gwt/projects/${fixture.repo_hash}/project-state/pm-status.jsonl`, `${JSON.stringify(oldStatus)}\n`);
      await refreshPmReports(page);
      await expect(currentCard).toContainText('PM has not updated for 16 minutes');
      await expect(currentCard).toContainText('Next: v9.117.2 の公開');
      await expect(currentCard).toContainText('User action: None');
      await expect(currentCard.getByRole('region', { name: 'Live facts' })).toContainText('Release v9.117.2');
      await expect(currentCard.getByText('Release: #5352 Release v9.117.2', { exact: true })).toBeInViewport();
      await expect(currentCard).toContainText('Active windows: 0');
      const staleScreenshot = info.outputPath(`pm-current-stale-${info.project.name}.png`);
      await pane.screenshot({ path: staleScreenshot });
      await info.attach('PM current stale status', { path: staleScreenshot, contentType: 'image/png' });
      pmOperation('pm.status.set', {
        items: [{ kind: 'working', text: `公開結果を確認 ${info.project.name}`, started_at: new Date().toISOString() }],
        next_trigger: '公開確認の完了', user_action: null,
      });
      const statusReadback = pmOperation('pm.status');
      expect(statusReadback.status_history).toHaveLength(2);
      expect(statusReadback.status_history[0].items[0].text).toBe(oldStatus.items[0].text);
      expect(statusReadback.current_status.items[0].text).toContain('公開結果を確認');
      await refreshPmReports(page);
      await expect(currentCard).toContainText(`公開結果を確認 ${info.project.name}`);
      await expect(currentCard).not.toContainText('has not updated for 16 minutes');
      await expect(currentCard).not.toContainText('リリースの公開待ち');
      await expect(reports).not.toContainText('公開結果を確認');
      const saved = pmOperation('pm.report.post', { kind: 'progress', body });
      expect(saved.report.body).toBe(body);
      expect(saved.report.id).toBeTruthy();
      await refreshPmReports(page);
      const report = reports.locator('article').filter({ has: page.getByRole('heading', { name: marker, exact: true }) });
      await expect(report.locator('strong')).toHaveText('Verified');
      await expect(report.locator('li')).toHaveCount(2);
      await expect(report.locator('pre code')).toHaveText('cargo test -p gwt-core\n');
      await expect(report.locator('time')).toHaveAttribute('datetime', /T/);
      // Reports stay read-only; the Chat composer exists but is hidden here.
      await expect(reports.locator('input, textarea, form, [contenteditable], button[type="submit"]')).toHaveCount(0);
      await expect(pane.getByRole('form', { name: 'Message PM', exact: true })).toBeHidden();
      const attachments = await page.evaluate(() => {
        const calls: string[] = [];
        (window as any).__gwtAttachmentUploader = async () => { calls.push('upload'); return { upload_id: 'blocked' }; };
        (window as any).__pmReportAttachmentCalls = calls;
        for (const socket of (window as any).__gwtPlaywrightSockets) {
          const original = socket.send.bind(socket);
          socket.send = (raw: string) => { if (JSON.parse(raw).kind === 'attach_files') calls.push('send'); original(raw); };
        }
        const transfer = new DataTransfer();
        transfer.items.add(new File(['blocked'], 'blocked.txt'));
        document.querySelector('.pm-chat')!.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer: transfer }));
        return calls;
      });
      expect(attachments).toEqual([]);
      await report.getByRole('button', { name: 'Copy report' }).click();
      // Windows' native text clipboard round-trips line endings as CRLF.
      await expect.poll(() => page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, '\n'))).toBe(body);

      const question = 'Why did PM recommend waiting?\nWhat should I do next?';
      const answerMarker = `PM answer ${info.project.name}`;
      const answerBody = `# ${answerMarker}\n\n**Confirmed**: use \`pm.report.post\`.\n\n- Keep the question\n- Read the answer`;
      const original = `${question}\nPlease keep the full original request.`;
      const qaSaved = pmOperation('pm.report.post', {
        kind: 'qa', question, body: answerBody,
        instruction: { original, kind: 'question', actions: ['Explained the next steps'], issue_numbers: [5342] },
      });
      expect(qaSaved.report).toMatchObject({ kind: 'qa', question, body: answerBody });
      expect(qaSaved.instruction).toMatchObject({ original, summary: question, answer: answerBody, kind: 'question' });
      expect(qaSaved.instruction.id).toBe(qaSaved.report.id);
      const listed = pmOperation('pm.report.list');
      expect(listed.reports.find((entry: { id: string }) => entry.id === qaSaved.report.id))
        .toMatchObject({ kind: 'qa', question, body: answerBody });
      await refreshPmReports(page);
      const qaReport = reports.locator('article').filter({ has: page.getByRole('heading', { name: answerMarker, exact: true }) });
      await expect(qaReport.getByRole('heading', { name: 'Q', exact: true })).toBeVisible();
      await expect(qaReport.getByRole('region', { name: 'Question', exact: true }).locator('.pm-chat__question')).toHaveText(question);
      await expect(qaReport.getByRole('heading', { name: 'A', exact: true })).toBeVisible();
      await expect(qaReport.getByRole('region', { name: 'Answer', exact: true }).locator('strong')).toHaveText('Confirmed');
      await qaReport.getByRole('button', { name: 'Copy report' }).click();
      await expect.poll(() => page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, '\n'))).toBe(`Q: ${question}\n\nA:\n${answerBody}`);
      const kindFilter = pane.getByRole('combobox', { name: 'Report kind', exact: true });
      await kindFilter.selectOption('qa');
      await expect(qaReport).toBeVisible();
      await expect(report).not.toBeVisible();
      await refreshPmReports(page);
      await expect(kindFilter).toHaveValue('qa');
      await kindFilter.selectOption('progress');
      await expect(report).toBeVisible();
      await expect(qaReport).not.toBeVisible();
      await kindFilter.selectOption('blocker');
      await expect(reports.locator('article')).toHaveCount(0);
      await expect(pane.getByRole('status')).toHaveText('No Blocker reports yet.');
      await kindFilter.selectOption('all');

      const terminal = await pane.locator('.terminal-root').elementHandle();
      await page.evaluate(({ id, session }) => {
        const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) =>
          candidate.readyState === WebSocket.OPEN && new URL(candidate.url).searchParams.has('repo_hash'));
        for (const event of [
          { kind: 'terminal_snapshot', id, data_base64: btoa('RAW snapshot\r\n') },
          { kind: 'terminal_output', id, data_base64: btoa('RAW appended\r\n') },
          { kind: 'pm_conversation', id, session_id: session, snapshot: { messages: [{ text: 'Native transcript must not appear' }] } },
          { kind: 'pm_reports', id, session_id: 'stale-session', reports: [] },
        ]) socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(event) }));
      }, { id, session: SESSION });
      await expect(reports).not.toContainText('RAW');
      await expect(reports).not.toContainText('Native transcript');
      await pane.getByRole('button', { name: 'Execution log', exact: true }).click();
      await expect(kindFilter).not.toBeVisible();
      await expect(pane.locator('.xterm')).toContainText('RAW snapshot');
      await expect(pane.locator('.xterm')).toContainText('RAW appended');
      expect(await pane.locator('.terminal-root').evaluate((node, original) => node === original, terminal)).toBe(true);
      await pane.getByRole('button', { name: 'Reports', exact: true }).click();
      await expect(report).toBeVisible();
      expect(await page.evaluate(() => (window as any).__pmReportAttachmentCalls)).toEqual([]);
      await reloadPmReports(page);
      await expect(currentCard).toContainText(`公開結果を確認 ${info.project.name}`);
      await expect(report).toBeVisible();
      await expect(qaReport.locator('.pm-chat__question')).toHaveText(question);
      await expect(qaReport.locator('strong')).toHaveText('Confirmed');
      await kindFilter.selectOption('qa');
      await qaReport.scrollIntoViewIfNeeded();
      const screenshot = info.outputPath(`pm-reports-${info.project.name}.png`);
      await page.screenshot({ path: screenshot });
      await info.attach('PM reports', { path: screenshot, contentType: 'image/png' });
      expect(errors).toEqual([]);
    });
  });
});

test.describe('RPT-F Reports (isolated checkout)', () => {
  test.skip(process.env.GWT_PLAYWRIGHT_RPT !== '1', 'requires checkout binaries and isolated managed-source fixtures');
  test.skip(process.platform === 'win32', 'the existing managed-source provider callback uses POSIX hooks');
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(240_000);
  let managedSource: Awaited<ReturnType<typeof startManagedBrowserSource>> | undefined;
  test.beforeAll(async ({ browser }, info) => {
    test.setTimeout(240_000);
    managedSource = await startManagedBrowserSource(browser, info);
  });
  test.afterAll(async () => {
    test.setTimeout(240_000);
    const failures: unknown[] = [];
    try { await managedSource?.postAudit(); } catch (error) { failures.push(error); }
    try { await managedSource?.stop(); } catch (error) { failures.push(error); }
    if (failures.length) throw new AggregateError(failures, 'managed source post/cleanup failed:\n' + failures.map(String).join('\n').slice(0, 16384));
  });

  test('RPT-F structured Reports enforce durable decisions and complete details across two projects', async ({ page, context }, info) => {
    const CHECKOUT = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
    const suffix = process.platform === 'win32' ? '.exe' : '';
    const GWTD = join(CHECKOUT, `target/debug/gwtd${suffix}`);
    let ownedState = '';
    let targetEnv: NodeJS.ProcessEnv = {};
    const target = await startExactRelaunchFixture(info, {
      async beforeStart({ home, project, env, preferences }) {
        ownedState = preferences;
        targetEnv = { ...env };
        const at = new Date().toISOString();
        writeFileSync(join(preferences, 'pm.json'), JSON.stringify({ settings: { auto_start: false }, registration: {
          session_id: SESSION, agent_id: 'codex', worktree_path: project, created_at: at,
        } }));
        writeFileSync(join(preferences, '../workspace.json'), JSON.stringify({ viewport: { x: 0, y: 0, zoom: 1 },
          windows: [{ id: 'pm-reports', title: 'PM', preset: 'agent', geometry: { x: 30, y: 30, width: 1050, height: 720 },
            z_index: 1, status: 'stopped', session_id: SESSION, agent_id: 'codex', persist: true }], next_z_index: 2 }));
        mkdirSync(join(home, '.gwt/sessions'), { recursive: true });
        writeFileSync(join(home, '.gwt/sessions', `${SESSION}.toml`), `id = "${SESSION}"\nworktree_path = ${JSON.stringify(project)}\nbranch = "main"\nagent_id = { type = "Codex" }\nstatus = "Stopped"\ncreated_at = "${at}"\nupdated_at = "${at}"\nlast_activity_at = "${at}"\n`);
        writeFileSync(join(home, '.gwt/config.toml'), '[pr_inventory]\ncache_ttl_secs = 3600\nchecks_refresh_secs = 3600\n');
        writeFileSync(join(preferences, '../pr-inventory-cache.json'), JSON.stringify({ fetched_at: at, rows: [] }));
        writeFileSync(join(preferences, 'pm-reports.jsonl'), JSON.stringify({ id: 'persisted-before-restart', kind: 'decision',
          body: '# Persisted before restart\n\nLegacy details remain readable.', created_at: at }) + '\n');
      },
    });
    const HOME = target.home;
    const ROOT = target.project;
    const BASE = target.url;
    const primaryErrors: unknown[] = [];
    const cleanupErrors: unknown[] = [];
    try {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: new URL(BASE).origin });
      const choiceRequests: any[] = [], choiceResults: any[] = [];
      page.on('websocket', socket => {
        socket.on('framesent', ({ payload }) => {
          const event = JSON.parse(String(payload));
          if (event.kind === 'pane_send_input') choiceRequests.push(event);
        });
        socket.on('framereceived', ({ payload }) => {
          const event = JSON.parse(String(payload));
          if (event.kind === 'pane_send_result') choiceResults.push(event);
        });
      });
      await gotoPmReports(page, undefined, BASE);
      const primaryKey = new URL(page.url()).pathname.split('/').at(-1)!;
      const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
      const reports = pane.getByRole('log', { name: 'PM reports' });
      await expect(reports.getByRole('heading', { name: 'Persisted before restart', exact: true })).toBeVisible();
      const originalCount = await reports.locator('article').count();
      const state = ownedState;
      const fixture = { repo_hash: basename(join(state, '..')) };
      const history = join(state, 'pm-reports.jsonl');
      const childEnv: NodeJS.ProcessEnv = { ...targetEnv, HOME, USERPROFILE: HOME };
      for (const key of Object.keys(childEnv)) {
        if (key.startsWith('GWT_SESSION_') || key.startsWith('GWT_HOOK_FORWARD_')
          || key === 'GWT_PANE_WS_URL' || key === 'GWT_BIN_PATH') delete childEnv[key];
      }
      const calls: unknown[] = [];
      const journals = [history];
      const saveEvidence = async (name: string, body: Buffer | string, contentType: string) => {
        const extension = contentType === 'image/png' ? 'png' : contentType === 'application/x-ndjson' ? 'jsonl' : 'json';
        const path = info.outputPath(`rpt-f-${theme}-${name}.${extension}`);
        mkdirSync(info.outputDir, { recursive: true });
        writeFileSync(path, body);
        await info.attach(`RPT-F ${name} ${theme}`, { path, contentType });
      };
      const invoke = (operation: string, params: Record<string, unknown>, session: string | null = SESSION,
        projectRoot = ROOT, succeeds = true): any => {
        const env = { ...childEnv, GWT_PROJECT_ROOT: projectRoot, ...(session ? { GWT_SESSION_ID: session } : {}) };
        const result = spawnSync(GWTD, [], {
          cwd: projectRoot, env, encoding: 'utf8', timeout: 30_000, maxBuffer: 8 * 1024 * 1024,
          input: JSON.stringify({ schema_version: 1, operation, params: { ...params, project_root: projectRoot } }),
        });
        calls.push({ operation, projectRoot, session, params, status: result.status, stdout: result.stdout, stderr: result.stderr });
        expect(result.error).toBeUndefined();
        expect(result.status, result.stderr || result.stdout).toBe(succeeds ? 0 : 1);
        if (!succeeds) return result.stdout + result.stderr;
        const envelope = JSON.parse(result.stdout);
        expect(envelope.ok).toBe(true);
        const output = JSON.parse(envelope.output);
        if ('ok' in output) expect(output.ok).toBe(true);
        return { envelope, output };
      };
      const post = (submission: object, session = SESSION, projectRoot = ROOT) =>
        invoke('pm.report.post', { submission }, session, projectRoot).output.receipt;
      const request = (name: string, events: object[], digest?: object) => ({
        schema_version: 1, request_id: `rpt-f-${theme}-${name}`, events, ...(digest ? { digest } : {}),
      });
      const episode = `rpt-f-${theme}`;
      const event = (eventId: string, result: string) => ({ episode_id: episode, event_id: eventId,
        expected_revision: 0, kind: 'progress', state: { status: 'active', severity: 'routine', impact: 'project' },
        summary: { result, impact: 'Project remains supervised', owner: 'PM', next_action: 'Check the next material change' },
        details: `Complete evidence for ${eventId}`, refs: ['https://example.com/issues/3431'] });
      try {
        const served = await page.request.get(new URL('/pm-chat.js', BASE).toString());
        expect(served.ok()).toBe(true);
        expect(await served.text(), 'served renderer is bound to this checkout source')
          .toBe(readFileSync(join(CHECKOUT, 'crates/gwt/web/pm-chat.js'), 'utf8'));
        const sha = (bytes: Buffer | string) => createHash('sha256').update(bytes).digest('hex');
        await saveEvidence('target-checkout-binding', JSON.stringify({
          checkout: CHECKOUT, project: ROOT, home: HOME, url: BASE, pid: target.ownedPid,
          gwt_path: join(CHECKOUT, `target/debug/gwt${suffix}`), gwtd_path: GWTD,
          gwt_sha256: sha(readFileSync(join(CHECKOUT, `target/debug/gwt${suffix}`))),
          gwtd_sha256: sha(readFileSync(GWTD)), served_pm_chat_sha256: sha(await served.body()),
        }), 'application/json');

        const qaQuestion = `RPT-F explicit detailed question ${theme}`;
        const qaAnswer = `All requested Q&A details remain available ${theme}.`;
        const qa = invoke('pm.report.post', { kind: 'qa', question: qaQuestion, body: qaAnswer,
          instruction: { original: qaQuestion, kind: 'question', actions: ['Explain every result'], issue_numbers: [3431] } }, null).output;
        expect(qa.instruction.id).toBe(qa.report.id);
        const initialEvent = event('supervision', `RPT-F compact overview <literal> ${theme}`);
        initialEvent.refs.push('Issue #3431 <literal>');
        const longText = '詳細'.repeat(9000);
        initialEvent.details = `**Full evidence**\n\n${longText}\n\nTAIL: item 48 preserved\n\n<script>window.__rptFUnsafe=true</script>`;
        const initial = request('initial', [initialEvent]);
        const first = post(initial);
        expect(first.reports).toHaveLength(1);
        expect(first.decisions[0]).toMatchObject({ reason: 'first', revision: 1 });
        expect(first.reports[0].body.split('\n')).toHaveLength(4);
        expect(first.reports[0].structured.details).toBe(initialEvent.details);
        const transactions = () => readFileSync(history, 'utf8').trim().split('\n')
          .map(line => JSON.parse(line)).filter(row => row.journal_schema === 2).map(row => JSON.parse(row.payload));
        expect(transactions().find(row => row.submission.request_id === initial.request_id).receipt).toEqual(first);
        const beforeRetry = readFileSync(history);
        expect(post(initial)).toEqual(first);
        expect(readFileSync(history), 'exact request replay appends no record').toEqual(beforeRetry);

        await refreshPmReports(page);
        const firstRow = reports.locator(`article[data-report-id="${first.reports[0].id}"]`);
        const details = firstRow.locator('details');
        await expect(firstRow.locator(':scope > .pm-chat__text')).toHaveText(first.reports[0].body);
        const overview = firstRow.locator(':scope > .pm-chat__text');
        expect(await overview.textContent()).toBe(first.reports[0].body);
        await expect(overview.locator('literal')).toHaveCount(0);
        const lineMetrics = await overview.evaluate(node => ({
          whitespace: getComputedStyle(node).whiteSpace,
          lines: node.getBoundingClientRect().height / parseFloat(getComputedStyle(node).lineHeight),
        }));
        expect(lineMetrics.whitespace).toBe('pre-wrap');
        expect(lineMetrics.lines).toBeGreaterThanOrEqual(4);
        expect(lineMetrics.lines).toBeLessThan(5);
        await expect(details).not.toHaveAttribute('open');
        await expect(details.locator('.knowledge-markdown-body')).toBeHidden();
        await details.locator('summary').click();
        await expect(details.locator('strong')).toHaveText('Full evidence');
        expect(await details.textContent()).toContain(longText);
        await expect(details).toContainText('TAIL: item 48 preserved');
        await expect(details).toContainText(initialEvent.refs[0]);
        await expect(details.locator('a')).toHaveAttribute('href', initialEvent.refs[0]);
        await expect(details).toContainText(initialEvent.refs[1]);
        await expect(details.locator('literal')).toHaveCount(0);
        await expect(firstRow.locator('script')).toHaveCount(0);
        expect(await page.evaluate(() => (window as any).__rptFUnsafe)).toBeUndefined();
        await firstRow.getByRole('button', { name: 'Copy report' }).click();
        const fullCopy = `${first.reports[0].body}\n\n${initialEvent.details}\n\n${initialEvent.refs.map(ref => `- ${ref}`).join('\n')}`;
        await expect.poll(() => page.evaluate(async () => (await navigator.clipboard.readText()).replace(/\r\n/g, '\n'))).toBe(fullCopy);

        const summaryOnly = post(request('summary-only', [{ ...event('summary-only', `RPT-F summary only ${theme}`), details: ' \n ', refs: [] }]));
        await refreshPmReports(page);
        const summaryOnlyRow = reports.locator(`article[data-report-id="${summaryOnly.reports[0].id}"]`);
        await expect(summaryOnlyRow.locator(':scope > .pm-chat__text')).toHaveText(summaryOnly.reports[0].body);
        await expect(summaryOnlyRow.locator('details')).toHaveCount(0);

        const unchanged = post(request('unchanged', [{ ...initialEvent, expected_revision: 1,
          observed_at: '2026-11-02T12:00:00Z', summary: { ...initialEvent.summary, result: 'Only wording changed' }, details: 'Only observation text changed' }]));
        expect(unchanged.reports).toEqual([]);
        expect(unchanged.decisions[0]).toMatchObject({ reason: 'unchanged', revision: 1, report_id: null });
        await refreshPmReports(page);
        await expect(reports.locator('article')).toHaveCount(originalCount + 3);
        expect(invoke('pm.report.post', { kind: 'progress', body: 'Body-only bypass' }, SESSION, ROOT, false)).toContain('submission');

        const fatal = { ...initialEvent, expected_revision: 1, kind: 'blocker',
          state: { status: 'waiting', severity: 'fatal', impact: 'service', action: { id: 'restore-service', kind: 'intervene' } },
          summary: { result: `RPT-F fatal ${theme}` }, details: 'New fatal event needs immediate intervention.' };
        const decision = { ...event('judgment', `RPT-F decision ${theme}`), kind: 'decision',
          state: { status: 'waiting', severity: 'routine', impact: 'project', action: { id: 'choose-recovery', kind: 'decide' } },
          summary: { result: `RPT-F decision ${theme}`, decision: { question: 'Which recovery?', recommendation: 'Retry',
            reason: 'The saved work is intact', options: ['Retry — repeat the check', 'Wait — keep the current state',
              'Investigate — collect more evidence', 'Cancel — stop this attempt'] } } };
        const mixed = post(request('mixed', [
          { ...event('routine-a', 'Routine item A'), state: { ...initialEvent.state, action: { id: 'monitor-a', kind: 'monitor' } } },
          fatal, decision,
          { ...event('routine-b', 'Routine item B'), state: { ...initialEvent.state, action: { id: 'retry-b', kind: 'retry' } } },
        ], { result: `RPT-F routine digest ${theme}`, owner: 'PM', next_action: 'Keep supervising both items' }));
        expect(mixed.reports.map((row: any) => row.structured.summary.result))
          .toEqual([fatal.summary.result, decision.summary.result, `RPT-F routine digest ${theme}`]);
        expect(mixed.decisions.filter((row: any) => row.reason === 'immediate')).toHaveLength(2);
        expect(mixed.decisions[0].report_id).toBe(mixed.decisions[3].report_id);
        expect(mixed.reports[2].structured.details).toContain('Routine item A');
        expect(mixed.reports[2].structured.details).toContain('Routine item B');
        const resolved = post(request('resolved', [{ ...initialEvent, expected_revision: 2,
          state: { status: 'resolved', severity: 'routine', impact: 'none' },
          summary: { result: `RPT-F resolved ${theme}` }, details: 'The author reports recovery; this does not confer approval.' }]));
        expect(resolved.decisions[0]).toMatchObject({ reason: 'resolved', revision: 3 });
        expect(resolved.reports).toHaveLength(1);
        const listed = invoke('pm.report.list', {}).output;
        expect(listed.receipts.find((row: any) => row.request_id === unchanged.request_id)).toEqual(unchanged);
        expect(listed.events.find((row: any) => row.event_id === initialEvent.event_id)).toMatchObject({ revision: 3, state: { status: 'resolved' } });
        await refreshPmReports(page);
        await expect(reports.locator('article')).toHaveCount(originalCount + 7);
        await expect(details).toHaveAttribute('open', '');
        for (const receipt of [mixed, resolved]) {
          for (const row of receipt.reports) await expect(reports.locator(`article[data-report-id="${row.id}"]`)).toHaveCount(1);
        }
        const decisionRow = reports.locator(`article[data-report-id="${mixed.reports[1].id}"]`);
        await expect(decisionRow).toContainText('1. Retry — repeat the check\n2. Wait — keep the current state\n3. Investigate — collect more evidence\n4. Cancel — stop this attempt');
        await expect(decisionRow).toContainText('Reply with the option number, or Other: your own answer.');
        await decisionRow.getByRole('button', { name: 'Copy report' }).click();
        await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toContain('4. Cancel — stop this attempt');
        await saveEvidence('four-choice-decision', await decisionRow.screenshot(), 'image/png');

        // T-ASK-SURFACE: actual Reports -> existing send path -> stopped-pane
        // refusal. This fixture proves the request and visible failure, never
        // a provider-accepted answer or a human Q&A capture.
        const choices = decisionRow.locator('.pm-chat__decision-option');
        await expect(choices).toHaveText(decision.summary.decision.options.map((option, index) => `${index + 1}. ${option}`));
        await choices.nth(1).focus();
        await page.keyboard.press('Enter');
        await expect.poll(() => choiceRequests.length).toBe(1);
        expect(choiceRequests[0]).toEqual({ kind: 'pane_send_input', session_id: SESSION,
          text: 'Answer to "Which recovery?": 2. Wait — keep the current state\r', request_id: expect.any(Number) });
        await expect.poll(() => choiceResults.find(result => result.request_id === choiceRequests[0].request_id))
          .toMatchObject({ ok: false, request_id: choiceRequests[0].request_id });
        await expect(decisionRow.getByRole('alert')).not.toBeEmpty();
        await expect(choices.nth(1)).toBeEnabled();
        await expect(choices.nth(1)).toBeFocused();
        const refusal = await decisionRow.getByRole('alert').innerText();
        await pane.getByRole('button', { name: 'Chat', exact: true }).click();
        const chatDecisions = pane.getByRole('region', { name: 'Decision reports', exact: true });
        const chatDecision = chatDecisions.locator(`[data-decision-id="${mixed.reports[1].id}"]`);
        await expect(chatDecision.getByRole('alert')).toHaveText(refusal);
        await expect(chatDecision.locator('.pm-chat__decision-option')).toHaveCount(4);
        const draft = pane.getByRole('textbox', { name: 'Message to PM' });
        await draft.fill('Keep this original reply');
        await chatDecision.getByRole('button', { name: 'Other: reply in chat', exact: true }).click();
        await expect(draft).toBeFocused();
        await expect(draft).toHaveValue('Keep this original reply');
        expect(choiceRequests).toHaveLength(1);
        await chatDecision.locator('.pm-chat__decision-option').nth(0).click();
        await expect.poll(() => choiceRequests.length).toBe(2);
        expect(choiceRequests[1].text).toBe('Answer to "Which recovery?": 1. Retry — repeat the check\r');
        expect(choiceRequests[1].request_id).not.toBe(choiceRequests[0].request_id);
        await expect.poll(() => choiceResults.find(result => result.request_id === choiceRequests[1].request_id))
          .toMatchObject({ ok: false });
        await expect(chatDecision.getByRole('alert')).not.toBeEmpty();
        await expect(draft).toHaveValue('Keep this original reply');
        await page.setViewportSize({ width: 404, height: 1000 });
        await expect.poll(async () => Math.round((await pane.boundingBox())!.width)).toBe(400);
        await expect.poll(() => chatDecisions.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
        await chatDecision.scrollIntoViewIfNeeded();
        await saveEvidence('chat-decision-narrow', await pane.screenshot(), 'image/png');
        await page.setViewportSize({ width: 1440, height: 1000 });
        const retryRefusal = choiceResults.find(result => result.request_id === choiceRequests[1].request_id).error;
        await pane.getByRole('button', { name: 'Decide', exact: true }).click();
        await expect(pane.locator(`.task-decisions__card[data-decision-id="${mixed.reports[1].id}"]`)
          .getByRole('alert')).toContainText(retryRefusal);
        await pane.getByRole('button', { name: 'Reports', exact: true }).click();
        await saveEvidence('decision-controls-requests', JSON.stringify({ choiceRequests, choiceResults }), 'application/json');
        const qaRow = reports.locator(`article[data-report-id="${qa.report.id}"]`);
        await expect(qaRow.locator('.pm-chat__question')).toHaveText(qaQuestion);
        await expect(qaRow.getByRole('region', { name: 'Answer', exact: true })).toContainText(qaAnswer);
        await expect(qaRow.locator('details')).toHaveCount(0);
        const projected = await page.evaluate(id => (window as any).__gwtPlaywrightMessages.map((entry: any) => entry.payload)
          .findLast((message: any) => message.kind === 'pm_reports' && message.reports?.some((row: any) => row.id === id)), first.reports[0].id);
        expect(projected.reports.find((row: any) => row.id === first.reports[0].id).details_html).toContain('TAIL: item 48 preserved');
        await saveEvidence('pm-report-view', JSON.stringify(projected), 'application/json');
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        await reports.locator(`article[data-report-id="${resolved.reports[0].id}"]`).scrollIntoViewIfNeeded();
        await saveEvidence('primary-reports', await pane.screenshot(), 'image/png');

        // Only the registration/workspace scaffolding is seeded; both projects'
        // typed cards go through the real operation and the normal Reports route.
        const secondRoot = join(HOME, `rpt-f-second-${theme}`);
        mkdirSync(secondRoot, { recursive: true });
        // Recent Projects normalizes a bare repository to its parent workspace
        // home. Isolate the one bare child under its own wrapper so neither the
        // normal route nor its identity can resolve to the shared fixture HOME.
        const secondBare = join(secondRoot, 'repo.git');
        const git = spawnSync('git', ['init', '--bare', '--quiet', secondBare], { cwd: secondRoot, env: childEnv, encoding: 'utf8' });
        expect(git.status, git.stderr).toBe(0);
        const secondHash = invoke('issue.monitor.status', {}, null, secondRoot).envelope.project_store.hash;
        expect(secondHash).not.toBe(fixture.repo_hash);
        await saveEvidence('second-project-layout', JSON.stringify({ wrapper: secondRoot, bare: secondBare,
          project_store_hash: secondHash, git: { args: ['init', '--bare', '--quiet', secondBare], status: git.status },
          source_contract: { detection: 'runtime_support::resolve_project_target: child Bare layout is migration-clear',
            recent: 'launch_runtime::normalize_recent_project_path: bare layout root normalizes to parent workspace home' },
          change_reason: 'A bare repository directly under fixture HOME normalized Recent to shared HOME; use an isolated wrapper instead.',
        }), 'application/json');
        const secondProject = join(HOME, '.gwt/projects', secondHash);
        const secondState = join(secondProject, 'project-state');
        mkdirSync(secondState, { recursive: true });
        const secondSession = 'pm-rpt-f-claude-fixture';
        writeFileSync(join(secondState, 'pm.json'), JSON.stringify({ settings: { auto_start: false }, registration: {
          session_id: secondSession, agent_id: 'claude', worktree_path: secondRoot, created_at: '2026-11-02T00:00:00Z',
        } }));
        writeFileSync(join(secondState, 'issue-monitor.json'), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] }));
        const workspace = JSON.parse(readFileSync(join(state, '../workspace.json'), 'utf8'));
        const pmWindow = workspace.windows.find((window: any) => window.session_id === SESSION);
        expect(pmWindow).toBeTruthy();
        writeFileSync(join(secondProject, 'workspace.json'), JSON.stringify({ ...workspace,
          windows: [{ ...pmWindow, id: 'pm-rpt-f-second', session_id: secondSession, agent_id: 'claude', status: 'stopped' }] }));
        writeFileSync(join(HOME, '.gwt/sessions', `${secondSession}.toml`),
          readFileSync(join(HOME, '.gwt/sessions', `${SESSION}.toml`), 'utf8')
            .replaceAll(SESSION, secondSession).replaceAll(ROOT, secondRoot).replaceAll('"Codex"', '"ClaudeCode"'));
        writeFileSync(join(secondProject, 'pr-inventory-cache.json'), readFileSync(join(state, '../pr-inventory-cache.json')));
        const secondHistory = join(secondState, 'pm-reports.jsonl');
        journals.push(secondHistory);
        expect(invoke('pm.report.post', { submission: initial }, SESSION, secondRoot, false)).toContain('registered PM');
        expect(existsSync(secondHistory), 'foreign PM cannot create the other project journal').toBe(false);
        const secondFirst = post(initial, secondSession, secondRoot);
        expect(secondFirst.reports).toHaveLength(1);
        expect(secondFirst.decisions[0]).toMatchObject({ reason: 'first', revision: 1 });
        const secondOnly = post(request('second-only', [event('second-only', `RPT-F second project only ${theme}`)]), secondSession, secondRoot);
        const catalog = await readLiveHubCatalog(page, BASE, { send: { kind: 'reopen_recent_project', path: secondRoot },
          until: (hub, root) => hub.recent_projects.some(entry => entry.path === root && entry.project_key
            && hub.projects.some(project => project.project_key === entry.project_key)), arg: secondRoot });
        const secondKey = catalog.recent_projects.find(entry => entry.path === secondRoot)!.project_key!;
        expect(secondKey, 'normal Hub opening retains the native project-store identity').toBe(secondHash);
        await saveEvidence('second-project-hub-route', JSON.stringify({ wrapper: secondRoot, project_store_hash: secondHash,
          recent: catalog.recent_projects.find(entry => entry.path === secondRoot),
          open: catalog.projects.find(project => project.project_key === secondKey) }), 'application/json');
        await gotoPmReports(page, secondKey, BASE);
        await expect(page).toHaveURL(new RegExp(`/p/${secondKey}$`));
        expect(JSON.parse(readFileSync(join(secondState, 'pm.json'), 'utf8')).start_block ?? null).toBeNull();
        await expect(reports.locator('article')).toHaveCount(2);
        await expect(reports.locator(`article[data-report-id="${secondFirst.reports[0].id}"]`)).toHaveCount(1);
        await expect(reports.locator(`article[data-report-id="${secondOnly.reports[0].id}"]`)).toContainText(`RPT-F second project only ${theme}`);
        await expect(reports).not.toContainText(fatal.summary.result);
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        await saveEvidence('second-project-reports', await pane.screenshot(), 'image/png');
        await gotoPmReports(page, primaryKey, BASE);
        await expect(reports.locator('article')).toHaveCount(originalCount + 7);
        await expect(reports).not.toContainText(`RPT-F second project only ${theme}`);
        expect(invoke('pm.report.list', {}).output.events.find((row: any) => row.event_id === 'supervision').revision).toBe(3);
        expect(invoke('pm.report.list', {}, secondSession, secondRoot).output.events.find((row: any) => row.event_id === 'supervision').revision).toBe(1);
        expect(errors).toEqual([]);
      } catch (error) {
        primaryErrors.push(error);
        throw error;
      } finally {
        try { await saveEvidence('real-cli-calls', JSON.stringify(calls), 'application/json'); }
        catch (error) { cleanupErrors.push(error); }
        for (const [index, path] of journals.entries()) {
          try {
            if (existsSync(path)) await saveEvidence(`journal-project-${index + 1}`, readFileSync(path), 'application/x-ndjson');
          } catch (error) { cleanupErrors.push(error); }
        }
      }
    } catch (error) {
      if (!primaryErrors.includes(error)) primaryErrors.push(error);
      throw error;
    } finally {
      try { await page.close(); } catch (error) { cleanupErrors.push(error); }
      try { await target.stop(); } catch (error) { cleanupErrors.push(error); }
      if (cleanupErrors.length) {
        const failures = [...primaryErrors, ...cleanupErrors];
        throw new AggregateError(failures, 'RPT-F target cleanup failed:\n' + failures.map(String).join('\n'),
          { cause: primaryErrors[0] });
      }
    }
  });

  test('T-TASK-KEY-REOPEN T-TASK-CANCEL T-TASK-DETAIL T-TASK-CHILDREN T-TASK-CRITERIA T-CHAT-TASK-LINK T-UTASK-OV and T-UTASK-USER-REOPEN/CANCEL/UPDATE/COMPLETE persist browser authority and show confirmed Tasks after reconnect', async ({ page }, info) => {
    const theme = info.project.name.includes('light') ? 'light' : 'dark';
    const windowId = 'pm-request-task-fixture';
    const taskId = 'synthetic-fixture-task';
    const original = 'Keep this exact fixture request.\n再開しても原文を保存する。';
    const fixturePm = { kind: 'pm', id: 'synthetic-fixture-pm' };
    const draft = { id: taskId, title: 'Synthetic completed task', purpose: 'Verify the browser reopen adapter',
      completion_conditions: ['The original fixture outcome is verified'], issue_numbers: [3431], goal_ids: [], assignee: null };
    const pendingTask = { draft, source_id: 'synthetic-fixture-source', parent_id: null, status: 'pending',
      condition_revision: 1, review_generation: 1, review_required: false, next_action: null, wait_reason: null, completion: null };
    const completedTask = { ...pendingTask, status: 'completed',
      completion: { condition_revision: 1, review_generation: 1, evidence: ['synthetic-fixture: verified outcome'] } };
    let statePath = '';
    let seed: any;
    const target = await startExactRelaunchFixture(info, {
      async beforeStart({ home, project, preferences }) {
        const projectKey = basename(join(preferences, '..'));
        const at = '2026-11-06T00:00:00Z';
        const source = { id: pendingTask.source_id, project_key: projectKey, original,
          authority: { session_id: 'synthetic-fixture-intake', turn_id: 'synthetic-fixture-turn',
            prompt_sha256: createHash('sha256').update(original).digest('hex') } };
        const receipt = (actor: { kind: string; id: string }, operation_id: string, expected_revision: number,
          action: Record<string, unknown>, tasks: unknown[]) => {
          const request = { operation_id, expected_revision, reason: 'Synthetic fixture seed only', action };
          return { operation_id, action: action.kind, request_digest: createHash('sha256')
            .update(JSON.stringify([actor, request])).digest('hex'), revision: expected_revision + 1,
          actor, at, reason: request.reason, source_id: source.id, tasks };
        };
        // This is receipt-consistent TaskStore scaffolding, not native-intake proof.
        seed = { schema_version: 1, project_key: projectKey, revision: 3, sources: { [source.id]: source }, pending: [],
          tasks: { [taskId]: completedTask }, receipts: [
            receipt({ kind: 'intake', id: 'synthetic-fixture-intake' }, 'synthetic-capture', 0,
              { kind: 'capture', source }, []),
            receipt(fixturePm, 'synthetic-decompose', 1,
              { kind: 'decompose', source_id: source.id, root: draft, children: [] }, [pendingTask]),
            receipt(fixturePm, 'synthetic-complete', 2,
              { kind: 'complete', task_id: taskId, condition_revision: 1, review_generation: 1,
                evidence: completedTask.completion.evidence }, [completedTask]),
          ] };
        const taskDirectory = join(preferences, 'request-tasks');
        mkdirSync(taskDirectory, { recursive: true, mode: 0o700 });
        statePath = join(taskDirectory, 'state.json');
        writeFileSync(join(taskDirectory, 'state.initialized'), projectKey, { mode: 0o600 });
        writeFileSync(statePath, JSON.stringify(seed), { mode: 0o600 });
        writeFileSync(join(preferences, 'pm.json'), JSON.stringify({ settings: { auto_start: false }, registration: {
          session_id: SESSION, agent_id: 'codex', worktree_path: project, created_at: at,
        } }));
        writeFileSync(join(preferences, '../workspace.json'), JSON.stringify({ viewport: { x: 0, y: 0, zoom: 1 },
          windows: [{ id: windowId, title: 'PM', preset: 'agent', geometry: { x: 30, y: 30, width: 1050, height: 720 },
            z_index: 1, status: 'stopped', session_id: SESSION, agent_id: 'codex', persist: true }], next_z_index: 2 }));
        mkdirSync(join(home, '.gwt/sessions'), { recursive: true });
        writeFileSync(join(home, '.gwt/sessions', `${SESSION}.toml`), `id = "${SESSION}"\nworktree_path = ${JSON.stringify(project)}\nbranch = "main"\nagent_id = { type = "Codex" }\nstatus = "Stopped"\ncreated_at = "${at}"\nupdated_at = "${at}"\nlast_activity_at = "${at}"\n`);
        writeFileSync(join(home, '.gwt/config.toml'), '[pr_inventory]\ncache_ttl_secs = 3600\nchecks_refresh_secs = 3600\n');
        writeFileSync(join(preferences, '../pr-inventory-cache.json'), JSON.stringify({ fetched_at: at, rows: [] }));
      },
    });
    const errors: string[] = [];
    const exchanges: unknown[] = [];
    const primaryErrors: unknown[] = [];
    const cleanupErrors: unknown[] = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    try {
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoPmReports(page, undefined, target.url);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      let requestId = 0;
      const exchange = async (kind: string, body: Record<string, unknown>) => {
        await page.waitForFunction(() => (window as any).__gwtPlaywrightSockets?.some((socket: WebSocket) =>
          socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has('repo_hash')));
        const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
        await expect(pane).toHaveCount(1);
        const id = await pane.getAttribute('data-id');
        expect(id).toBeTruthy();
        const event = { kind, id, request_id: ++requestId, ...body };
        const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
        await sendLiveGwtEvent(page, event);
        const reply = await page.waitForFunction(({ after, id, requestId }) =>
          (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.sequence > after
            && entry.payload.kind === 'pm_request_task' && entry.payload.id === id
            && entry.payload.request_id === requestId)?.payload,
        { after, id, requestId }).then(handle => handle.jsonValue());
        exchanges.push({ request: event, response: reply });
        return reply.result;
      };
      const loaded = await exchange('load_pm_request_task', { task_id: taskId });
      expect(loaded).toMatchObject({ state: 'ready', revision: 3, task: completedTask });
      expect(loaded.receipt ?? null).toBeNull();
      const completeRequest = { operation_id: `browser-complete-${theme}`, expected_revision: 3,
        reason: 'The browser user verified the original outcome.', task_id: taskId,
        condition_revision: 1, review_generation: 1, evidence: ['synthetic:browser-verified-outcome'] };
      const completed = await exchange('complete_pm_request_task', { request: completeRequest });
      expect(completed).toMatchObject({ state: 'ready', revision: 4,
        task: { ...completedTask, completion: { ...completedTask.completion, evidence: completeRequest.evidence } },
        receipt: { action: 'complete', revision: 4, reason: completeRequest.reason, actor: { kind: 'user' } } });
      expect(completed.receipt.actor.id).toMatch(/^browser:[0-9a-f-]{36}$/i);
      expect(await exchange('complete_pm_request_task', { request: completeRequest })).toEqual(completed);
      const completeNoop = await exchange('complete_pm_request_task', { request: { ...completeRequest,
        operation_id: `browser-complete-noop-${theme}`, expected_revision: 4 } });
      expect(completeNoop).toMatchObject({ state: 'ready', revision: 5, task: completed.task,
        receipt: { action: 'complete', revision: 5, tasks: [], actor: completed.receipt.actor } });
      const completeState = JSON.parse(readFileSync(statePath, 'utf8'));
      const completionReceipts = [...seed.receipts, completed.receipt, completeNoop.receipt];
      expect(completeState.receipts).toEqual(completionReceipts);
      expect(completeState.sources).toEqual(seed.sources);
      expect(completeState.tasks[taskId]).toEqual(completed.task);
      const request = { operation_id: `browser-reopen-${theme}`, expected_revision: 5,
        reason: 'The original outcome needs another review.\nPlease reopen this task.', task_id: '' };
      const taskKey = createHash('sha256').update(taskId).digest('hex');
      const beforeReopen = readFileSync(statePath, 'utf8');
      for (const [task_key, selectedRequest, code] of [
        ['', request, 'invalid_request'],
        [taskKey, { ...request, task_id: taskId }, 'invalid_request'],
        ['unknown', request, 'missing_task'],
      ] as const) {
        expect(await exchange('reopen_pm_request_task', { task_key, request: selectedRequest }))
          .toMatchObject({ state: 'error', code, recovery_operation: 'load_pm_request_task' });
        expect(readFileSync(statePath, 'utf8')).toBe(beforeReopen);
      }
      const reopened = await exchange('reopen_pm_request_task', { task_key: taskKey, request });
      expect(reopened).toMatchObject({ state: 'ready', revision: 6, task: {
        ...pendingTask, review_generation: 2, review_required: true,
      }, receipt: { operation_id: request.operation_id, action: 'reopen', revision: 6,
        reason: request.reason, actor: { kind: 'user' } } });
      expect(reopened.receipt.actor.id).toMatch(/^browser:[0-9a-f-]{36}$/i);
      expect(reopened.receipt.actor.id).not.toBe(fixturePm.id);
      const savedBytes = readFileSync(statePath, 'utf8');
      const saved = JSON.parse(savedBytes);
      expect(saved.revision).toBe(6);
      expect(saved.receipts).toEqual([...completionReceipts, reopened.receipt]);
      expect(saved.tasks[taskId]).toEqual(reopened.task);
      expect(saved.sources).toEqual(seed.sources);
      expect(saved.sources[pendingTask.source_id].original).toBe(original);
      expect(await exchange('reopen_pm_request_task', { task_key: taskKey, request })).toEqual(reopened);
      expect(readFileSync(statePath, 'utf8')).toBe(savedBytes);
      expect(await exchange('reopen_pm_request_task', { task_key: taskKey, request: { ...request,
        operation_id: `${request.operation_id}-stale` } })).toMatchObject({ state: 'error', code: 'conflict',
        current_revision: 6, recovery_operation: 'load_pm_request_task' });
      expect(readFileSync(statePath, 'utf8')).toBe(savedBytes);
      expect(await exchange('complete_pm_request_task', { request: completeRequest }))
        .toEqual({ ...reopened, receipt: completed.receipt });
      expect(await exchange('complete_pm_request_task', { request: { ...completeRequest,
        operation_id: `browser-complete-stale-${theme}`, expected_revision: 6 } }))
        .toMatchObject({ state: 'error', code: 'stale_review' });
      expect(await exchange('complete_pm_request_task', { request: { ...completeRequest,
        operation_id: `browser-complete-reassessment-${theme}`, expected_revision: 6, review_generation: 2 } }))
        .toMatchObject({ state: 'error', code: 'reassessment_required' });
      expect(readFileSync(statePath, 'utf8')).toBe(savedBytes);

      // Reload creates a fresh server-generated browser actor. Reconcile by read;
      // never supply the old actor or reinterpret its operation as a new mutation.
      await page.reload();
      await page.waitForFunction(() => (window as any).__gwtPlaywrightTestBridgeInstalled === true);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      expect(await exchange('reopen_pm_request_task', { task_key: taskKey, request })).toMatchObject({ state: 'error',
        code: 'operation_conflict', recovery_operation: 'load_pm_request_task' });
      expect(await exchange('load_pm_request_task', { task_id: taskId,
        receipt_operation_id: request.operation_id })).toEqual(reopened);
      expect(readFileSync(statePath, 'utf8')).toBe(savedBytes);
      expect(await exchange('complete_pm_request_task', { request: completeRequest })).toMatchObject({ state: 'error',
        code: 'operation_conflict', recovery_operation: 'load_pm_request_task' });
      expect(await exchange('load_pm_request_task', { task_id: taskId,
        receipt_operation_id: completeRequest.operation_id })).toEqual({ ...reopened, receipt: completed.receipt });
      const cancelRequest = { operation_id: `browser-cancel-${theme}`, expected_revision: 6,
        reason: 'Withdraw the original request without claiming it was achieved.', task_id: taskId };
      const cancelled = await exchange('cancel_pm_request_task', { request: cancelRequest });
      expect(cancelled).toMatchObject({ state: 'ready', revision: 7, task: {
        ...pendingTask, status: 'cancelled', review_generation: 3, review_required: true, completion: null,
      }, receipt: { operation_id: cancelRequest.operation_id, action: 'cancel', revision: 7,
        reason: cancelRequest.reason, actor: { kind: 'user' } } });
      expect(cancelled.receipt.actor.id).toMatch(/^browser:[0-9a-f-]{36}$/i);
      expect(cancelled.receipt.actor.id).not.toBe(reopened.receipt.actor.id);
      const cancelledBytes = readFileSync(statePath, 'utf8');
      const cancelledState = JSON.parse(cancelledBytes);
      expect(cancelledState.receipts).toEqual([...completionReceipts, reopened.receipt, cancelled.receipt]);
      expect(cancelledState.tasks[taskId]).toEqual(cancelled.task);
      expect(cancelledState.sources).toEqual(seed.sources);
      expect(await exchange('cancel_pm_request_task', { request: cancelRequest })).toEqual(cancelled);
      expect(await exchange('cancel_pm_request_task', { request: { ...cancelRequest,
        operation_id: `${cancelRequest.operation_id}-stale` } })).toMatchObject({ state: 'error', code: 'conflict',
        current_revision: 7, recovery_operation: 'load_pm_request_task' });
      await page.reload();
      await page.waitForFunction(() => (window as any).__gwtPlaywrightTestBridgeInstalled === true);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      expect(await exchange('cancel_pm_request_task', { request: cancelRequest })).toMatchObject({ state: 'error',
        code: 'operation_conflict', recovery_operation: 'load_pm_request_task' });
      expect(await exchange('load_pm_request_task', { task_id: taskId,
        receipt_operation_id: cancelRequest.operation_id })).toEqual(cancelled);
      const historical = await exchange('load_pm_request_task', { task_id: taskId,
        receipt_operation_id: request.operation_id });
      expect(historical).toEqual({ ...cancelled, receipt: reopened.receipt });
      expect(readFileSync(statePath, 'utf8')).toBe(cancelledBytes);

      const noopRequest = { operation_id: `browser-update-noop-${theme}`, expected_revision: 7,
        reason: 'Confirm this saved task without changing its draft.', task: draft };
      const noop = await exchange('update_pm_request_task', { request: noopRequest });
      expect(noop).toMatchObject({ state: 'ready', revision: 8, task: cancelled.task,
        receipt: { operation_id: noopRequest.operation_id, action: 'update', revision: 8,
          reason: noopRequest.reason, actor: { kind: 'user' }, tasks: [] } });
      expect(noop.receipt.actor.id).toMatch(/^browser:[0-9a-f-]{36}$/i);
      expect(noop.receipt.actor.id).not.toBe(cancelled.receipt.actor.id);
      const noopBytes = readFileSync(statePath, 'utf8');
      const noopState = JSON.parse(noopBytes);
      expect(noopState.revision).toBe(8);
      expect(noopState.receipts).toEqual([...cancelledState.receipts, noop.receipt]);
      expect(noopState.tasks[taskId]).toEqual(cancelled.task);
      expect(noopState.sources).toEqual(seed.sources);
      expect(await exchange('update_pm_request_task', { request: noopRequest })).toEqual(noop);
      expect(readFileSync(statePath, 'utf8')).toBe(noopBytes);

      const correctedDraft = { ...draft, purpose: 'Verify the corrected original fixture outcome',
        completion_conditions: ['Verify the corrected outcome before claiming completion'] };
      const updateRequest = { operation_id: `browser-update-${theme}`, expected_revision: 8,
        reason: 'Correct the purpose and completion conditions while retaining the original request.', task: correctedDraft };
      const updated = await exchange('update_pm_request_task', { request: updateRequest });
      expect(updated).toMatchObject({ state: 'ready', revision: 9, task: { ...cancelled.task,
        draft: correctedDraft, condition_revision: 2, review_generation: 4, review_required: true,
      }, receipt: { operation_id: updateRequest.operation_id, action: 'update', revision: 9,
        reason: updateRequest.reason, actor: noop.receipt.actor } });
      expect(updated.receipt.tasks).toEqual([updated.task]);
      const updatedBytes = readFileSync(statePath, 'utf8');
      const updatedState = JSON.parse(updatedBytes);
      expect(updatedState.revision).toBe(9);
      expect(updatedState.receipts).toEqual([...noopState.receipts, updated.receipt]);
      expect(updatedState.tasks[taskId]).toEqual(updated.task);
      expect(updatedState.sources).toEqual(seed.sources);
      expect(updatedState.sources[pendingTask.source_id].original).toBe(original);
      expect(await exchange('update_pm_request_task', { request: noopRequest }))
        .toEqual({ ...updated, receipt: noop.receipt });
      expect(readFileSync(statePath, 'utf8')).toBe(updatedBytes);

      await page.reload();
      await page.waitForFunction(() => (window as any).__gwtPlaywrightTestBridgeInstalled === true);
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      expect(await exchange('update_pm_request_task', { request: noopRequest })).toMatchObject({ state: 'error',
        code: 'operation_conflict', recovery_operation: 'load_pm_request_task' });
      expect(await exchange('load_pm_request_task', { task_id: taskId,
        receipt_operation_id: noopRequest.operation_id })).toEqual({ ...updated, receipt: noop.receipt });
      expect(readFileSync(statePath, 'utf8')).toBe(updatedBytes);

      // Extend this owned, receipt-consistent fixture for read-only overview
      // coverage. This scaffolding does not prove native intake or PM actions.
      const overviewState = JSON.parse(updatedBytes);
      const nativeConversation = `task-link-${theme}`;
      const sourceId = `native:${createHash('sha256').update(JSON.stringify(['codex', nativeConversation, 'task-human'])).digest('hex')}`;
      const overviewSource = { ...seed.sources[pendingTask.source_id], id: sourceId,
        authority: { ...seed.sources[pendingTask.source_id].authority, turn_id: 'synthetic-overview-turn' } };
      const criteriaPurpose = '目的😀'.repeat(90) + 'PURPOSE_TAIL_NOT_IN_PREVIEW';
      const criteriaCondition = '条件🧪'.repeat(90) + 'CONDITION_TAIL_NOT_IN_PREVIEW';
      const criteriaConditions = [criteriaCondition, 'Second criterion <literal> stays text',
        'Third criterion', 'Fourth criterion', 'Fifth criterion', 'SIXTH_CRITERION_NOT_IN_PREVIEW'];
      const rootTask = { ...pendingTask, source_id: overviewSource.id,
        draft: { ...draft, id: 'overview-root', title: 'Original request still pending', assignee: 'PM',
          purpose: criteriaPurpose, completion_conditions: criteriaConditions } };
      const childTask = { ...pendingTask, source_id: overviewSource.id, parent_id: rootTask.draft.id,
        draft: { ...draft, id: 'overview-child', title: 'Verify remaining outcome', assignee: 'Reviewer' } };
      const doneChild = { ...childTask, status: 'completed',
        draft: { ...draft, id: 'overview-done', title: 'Verified child outcome' },
        completion: { condition_revision: childTask.condition_revision, review_generation: childTask.review_generation,
          evidence: ['synthetic:verified-child-outcome'] } };
      const appendFixtureReceipt = (actor: { kind: string; id: string }, action: Record<string, unknown>, tasks: unknown[]) => {
        const operation_id = `overview-fixture-${overviewState.revision}`;
        const request = { operation_id, expected_revision: overviewState.revision, reason: 'Synthetic overview fixture only', action };
        overviewState.receipts.push({ operation_id, action: action.kind,
          request_digest: createHash('sha256').update(JSON.stringify([actor, request])).digest('hex'),
          revision: ++overviewState.revision, actor, at: '2026-11-06T00:00:00Z', reason: request.reason,
          source_id: overviewSource.id, tasks });
      };
      appendFixtureReceipt({ kind: 'intake', id: 'synthetic-fixture-intake' }, { kind: 'capture', source: overviewSource }, []);
      appendFixtureReceipt(fixturePm, { kind: 'decompose', source_id: overviewSource.id,
        root: rootTask.draft, children: [childTask.draft, doneChild.draft] },
      [rootTask, childTask, { ...doneChild, status: 'pending', completion: null }]);
      appendFixtureReceipt(fixturePm, { kind: 'complete', task_id: doneChild.draft.id,
        ...doneChild.completion }, [doneChild]);
      const waitingChild = { ...childTask, status: 'waiting', wait_reason: 'CI verdict', next_action: 'Review required checks' };
      appendFixtureReceipt(fixturePm, { kind: 'progress', task_id: childTask.draft.id, status: 'waiting',
        wait_reason: waitingChild.wait_reason, next_action: waitingChild.next_action }, [waitingChild]);
      overviewState.sources[overviewSource.id] = overviewSource;
      overviewState.tasks[rootTask.draft.id] = rootTask;
      overviewState.tasks[childTask.draft.id] = waitingChild;
      overviewState.tasks[doneChild.draft.id] = doneChild;
      let overviewBytes = JSON.stringify(overviewState);
      writeFileSync(statePath, overviewBytes);
      // Real native-reader projection plus synthetic TaskStore scaffolding:
      // this proves source navigation, not live provider intake/classification.
      const nativePath = join(target.home, '.codex/sessions/2026/11/12', `rollout-${nativeConversation}.jsonl`);
      mkdirSync(join(nativePath, '..'), { recursive: true });
      const nativeRecords = [
        { type: 'session_meta', payload: { id: nativeConversation, cwd: target.project } },
        { type: 'response_item', payload: { type: 'message', id: 'task-human', role: 'user', content: [{ type: 'input_text', text: original }] } },
        ...Array.from({ length: 20 }, (_, index) => ({ type: 'response_item', payload: {
          type: 'message', id: `followup-${index}`, role: 'user', content: [{ type: 'input_text', text: `Followup ${index}` }],
        } })),
      ];
      writeFileSync(nativePath, nativeRecords.map(record => JSON.stringify(record)).join('\n') + '\n');
      const sessionPath = join(target.home, '.gwt/sessions', `${SESSION}.toml`);
      appendFileSync(sessionPath, `\ndisplay_name = "PM"\nagent_session_id = "${nativeConversation}"\n`);
      await gotoPmReports(page, undefined, target.url, true);
      const chat = page.locator('.pm-chat');
      const tasksView = chat.getByRole('region', { name: 'Request tasks' });
      await chat.getByRole('button', { name: 'Tasks', exact: true }).click();
      await expect(chat.getByRole('button', { name: 'Tasks', exact: true })).toHaveAttribute('aria-pressed', 'true');
      await expect(tasksView).toBeVisible();
      await expect(tasksView.locator('.pm-chat__task')).toHaveCount(2);
      await expect(tasksView).toContainText('Part of: Original request still pending');
      await expect(tasksView).toContainText('Waiting: CI verdict');
      await expect(tasksView).toContainText('Next: Review required checks');
      const completedFilter = tasksView.getByRole('button', { name: 'Include completed', exact: true });
      await completedFilter.focus();
      await page.keyboard.press('Enter');
      await expect(tasksView.locator('.pm-chat__task')).toHaveCount(4);
      await expect(tasksView).toContainText('Cancelled');
      await expect(tasksView).toContainText('PM reassessment required');
      await expect(completedFilter).toBeFocused();
      await chat.getByRole('button', { name: 'Chat', exact: true }).click();
      const criteriaDraft = chat.locator('.pm-chat__draft');
      await criteriaDraft.fill('Keep this draft while reviewing the original request');
      await chat.getByRole('button', { name: 'Tasks', exact: true }).click();
      const rootKey = createHash('sha256').update(rootTask.draft.id).digest('hex');
      const rootRow = tasksView.locator(`[data-task-key="${rootKey}"]`);
      const detailButton = rootRow.getByRole('button', { name: 'View task details', exact: true });
      await detailButton.focus();
      await page.keyboard.press('Enter');
      const taskDetail = tasksView.getByRole('region', { name: 'Selected task details' });
      await expect(taskDetail).toBeVisible();
      await expect(taskDetail).toContainText(original);
      await expect(taskDetail).toContainText('Showing 6 of 6 completion criteria');
      await expect(taskDetail).toContainText('SIXTH_CRITERION_NOT_IN_PREVIEW');
      await expect(taskDetail).toContainText('Showing 1 of 1 recorded changes');
      await expect(taskDetail).toContainText('Synthetic overview fixture only');
      await expect(taskDetail.locator('literal')).toHaveCount(0);
      expect(readFileSync(statePath, 'utf8')).toBe(overviewBytes, 'detail reads never write the store');
      await taskDetail.getByRole('button', { name: 'Close details', exact: true }).click();
      await expect(detailButton).toBeFocused();
      const progress = rootRow.locator('.pm-chat__task-meta').filter({ hasText: 'Direct subtasks:' });
      await expect(progress).toHaveText('Direct subtasks: 1 completed · 0 cancelled · 1 unfinished (2 total)');
      await expect(rootRow.locator('.pm-chat__task-meta').first()).toContainText('Pending');
      await chat.getByRole('button', { name: 'Chat', exact: true }).click();
      await expect(progress).toBeVisible();
      await chat.getByRole('button', { name: 'Tasks', exact: true }).click();
      const criteria = rootRow.locator('details.pm-chat__task-criteria');
      const criteriaSummary = criteria.locator('summary');
      await expect(criteriaSummary).toHaveText('Purpose and completion criteria');
      await expect(criteria).not.toHaveAttribute('open');
      await expect(criteria).toContainText(`Purpose: ${Array.from(criteriaPurpose).slice(0, 240).join('')}…`);
      await expect(criteria.locator('ol li')).toHaveText([
        Array.from(criteriaCondition).slice(0, 240).join('') + '…', ...criteriaConditions.slice(1, 5),
      ]);
      await expect(criteria).toContainText('Showing 5 of 6 completion criteria. Text is limited to 240 characters per item.');
      await expect(criteria).not.toContainText('PURPOSE_TAIL_NOT_IN_PREVIEW');
      await expect(criteria).not.toContainText('CONDITION_TAIL_NOT_IN_PREVIEW');
      await expect(criteria).not.toContainText(criteriaConditions[5]);
      await expect(criteria.locator('literal')).toHaveCount(0);
      await criteriaSummary.focus();
      await page.keyboard.press('Enter');
      await expect(criteria).toHaveAttribute('open');
      await expect(criteriaSummary).toBeFocused();
      const criteriaCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      const scopedWindowId = await page.locator('.workspace-window').filter({ has: chat }).getAttribute('data-id');
      expect(scopedWindowId).toBeTruthy();
      await refreshPmReports(page);
      await page.waitForFunction(({ cursor, id }) => (window as any).__gwtPlaywrightMessages?.some((entry: any) =>
        entry.sequence > cursor && entry.payload.kind === 'pm_reports' && entry.payload.id === id),
      { cursor: criteriaCursor, id: scopedWindowId });
      await expect(criteria).toHaveAttribute('open');
      await expect(criteriaSummary).toBeFocused();
      const criteriaUpdate = await exchange('update_pm_request_task', { request: {
        operation_id: `criteria-overview-update-${theme}`, expected_revision: overviewState.revision,
        reason: 'Refresh the saved Task summary without replacing its open criteria.',
        task: { ...rootTask.draft, assignee: 'PM reviewer' },
      } });
      expect(criteriaUpdate).toMatchObject({ state: 'ready', task: { draft: { assignee: 'PM reviewer' } } });
      overviewBytes = readFileSync(statePath, 'utf8');
      await refreshPmReports(page);
      await expect(rootRow).toContainText('PM reviewer');
      await expect(criteria).toHaveAttribute('open');
      await expect(criteriaSummary).toBeFocused();
      await detailButton.click();
      await expect(taskDetail).toContainText('Showing 2 of 2 recorded changes');
      await expect(taskDetail).toContainText('Refresh the saved Task summary');
      await taskDetail.getByRole('button', { name: 'Reload details', exact: true }).focus();
      try {
        writeFileSync(statePath, 'broken task store');
        await refreshPmReports(page);
        await expect(tasksView.getByRole('status')).toContainText('last confirmed list');
        await expect(tasksView.locator('.pm-chat__task')).toHaveCount(4);
        await expect(progress).toHaveText('Direct subtasks: 1 completed · 0 cancelled · 1 unfinished (2 total)');
        await expect(criteria).toHaveAttribute('open');
        await expect(taskDetail.getByRole('button', { name: 'Reload details', exact: true })).toBeFocused();
        await expect(criteria.locator('ol li')).toHaveCount(5);
        await expect(criteriaDraft).toHaveValue('Keep this draft while reviewing the original request');
        const detailErrorCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
        await taskDetail.getByRole('button', { name: 'Reload details', exact: true }).click();
        const detailError = await page.waitForFunction(({ cursor, id }) =>
          (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.sequence > cursor
            && entry.payload.kind === 'pm_request_task' && entry.payload.id === id)?.payload,
        { cursor: detailErrorCursor, id: scopedWindowId }).then(handle => handle.jsonValue());
        expect(detailError.result).toMatchObject({ state: 'error', code: 'unreadable_store' });
        await expect(taskDetail).toContainText('Request task details could not be read.');
        await expect(taskDetail).not.toContainText('Loading task details');
        await expect(taskDetail.getByRole('button', { name: 'Reload details', exact: true })).toBeFocused();
        await expect(taskDetail).toContainText('last confirmed');
        await expect(taskDetail).toContainText(original);
        await expect(taskDetail).not.toContainText('broken task store');
      } finally {
        writeFileSync(statePath, overviewBytes);
      }
      await refreshPmReports(page);
      await expect(tasksView.getByRole('status')).toContainText('4 of 4 recorded tasks shown');
      const detailRecoveryCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await taskDetail.getByRole('button', { name: 'Reload details', exact: true }).click();
      const detailRecovery = await page.waitForFunction(({ cursor, id }) =>
        (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.sequence > cursor
          && entry.payload.kind === 'pm_request_task' && entry.payload.id === id)?.payload,
      { cursor: detailRecoveryCursor, id: scopedWindowId }).then(handle => handle.jsonValue());
      expect(detailRecovery.result).toMatchObject({ state: 'detail', revision: criteriaUpdate.revision, key: rootKey });
      await expect(taskDetail).toContainText('Selected snapshot revision');
      await expect(taskDetail).not.toContainText('last confirmed');
      await expect(taskDetail).not.toContainText('Loading task details');
      await expect(taskDetail.getByRole('button', { name: 'Reload details', exact: true })).toBeFocused();
      await taskDetail.getByRole('button', { name: 'Close details', exact: true }).click();
      await expect(detailButton).toBeFocused();
      const detailFocusUpdate = await exchange('update_pm_request_task', { request: {
        operation_id: `detail-focus-update-${theme}`, expected_revision: criteriaUpdate.revision,
        reason: 'Keep keyboard focus on the detail action during a Task update.',
        task: { ...rootTask.draft, assignee: 'PM keyboard reviewer' },
      } });
      expect(detailFocusUpdate).toMatchObject({ state: 'ready' });
      await refreshPmReports(page);
      await expect(rootRow).toContainText('PM keyboard reviewer');
      await expect(detailButton).toBeFocused();
      await expect(detailButton).toBeInViewport({ ratio: 1 });
      await page.keyboard.press('Enter');
      await expect(taskDetail).toContainText('Showing 3 of 3 recorded changes');
      await expect(taskDetail).toContainText('Keep keyboard focus on the detail action during a Task update.');
      await taskDetail.getByRole('button', { name: 'Close details', exact: true }).click();
      await chat.getByRole('button', { name: 'Chat', exact: true }).click();
      const conversation = chat.getByRole('region', { name: 'PM conversation' });
      await expect(conversation).toBeVisible();
      await expect(tasksView).toBeVisible();
      const sourceMessage = conversation.locator('[data-message-id="task-human"]');
      await expect(sourceMessage).toHaveText(original);
      const link = tasksView.locator('.pm-chat__task').filter({ hasText: 'Verify remaining outcome' })
        .getByRole('button', { name: 'Show in conversation', exact: true });
      await expect(link).toBeEnabled();
      const composerDraft = conversation.getByRole('textbox', { name: 'Message to PM' });
      await composerDraft.fill('Keep this draft while reviewing the original request');
      await link.focus();
      await page.keyboard.press('Enter');
      await expect(sourceMessage).toBeFocused();
      await expect(sourceMessage).toBeInViewport();
      await expect(composerDraft).toHaveValue('Keep this draft while reviewing the original request');
      await info.attach(`T-CHAT-TASK-LINK ${theme} wide`, { body: await chat.screenshot(), contentType: 'image/png' });
      // A rewritten native window cannot retain the prior source link, even
      // when another human message repeats the original request verbatim.
      nativeRecords[1].payload.id = 'different-human';
      writeFileSync(`${nativePath}.replacement`, nativeRecords.map(record => JSON.stringify(record)).join('\n') + '\n');
      renameSync(`${nativePath}.replacement`, nativePath);
      await sendLiveGwtEvent(page, { kind: 'load_pm_conversation', id: windowId });
      await expect(conversation.locator('[data-message-id="different-human"]')).toHaveText(original);
      await expect(link).toBeDisabled();
      await expect(tasksView).toContainText('The original message is not in the current conversation preview.');
      await expect(composerDraft).toHaveValue('Keep this draft while reviewing the original request');
      await page.setViewportSize({ width: 720, height: 1000 });
      await expect(conversation).toBeVisible();
      await expect(tasksView).toBeVisible();
      expect(await chat.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      const tasksBox = (await tasksView.boundingBox())!, conversationBox = (await conversation.boundingBox())!;
      expect(tasksBox.y + tasksBox.height).toBeLessThanOrEqual(conversationBox.y + 1);
      await info.attach(`T-CHAT-TASK-LINK ${theme} narrow`, { body: await chat.screenshot(), contentType: 'image/png' });
      await chat.getByRole('button', { name: 'Reports', exact: true }).click();
      await expect(chat.locator('.pm-chat__body')).toBeVisible();
      await chat.getByRole('button', { name: 'Execution log', exact: true }).click();
      await expect(page.locator('.pm-conversation-log')).toBeVisible();
      await chat.getByRole('button', { name: 'Tasks', exact: true }).click();
      await expect(completedFilter).toHaveAttribute('aria-pressed', 'true');
      await page.setViewportSize({ width: 720, height: 1000 });
      await expect(tasksView).toBeVisible();
      expect(await tasksView.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      await info.attach(`T-UTASK-OV ${theme} Tasks`, { body: await tasksView.screenshot(), contentType: 'image/png' });
      await page.setViewportSize({ width: 404, height: 1000 });
      const criteriaPane = page.locator('.workspace-window').filter({ has: chat });
      await expect.poll(async () => Math.round((await criteriaPane.boundingBox())!.width)).toBe(400);
      const doneKey = createHash('sha256').update(doneChild.draft.id).digest('hex');
      await tasksView.locator(`[data-task-key="${doneKey}"]`).getByRole('button', { name: 'View task details', exact: true }).focus();
      await page.keyboard.press('Enter');
      await expect(taskDetail).toContainText('synthetic:verified-child-outcome');
      await expect(taskDetail).toContainText('Showing 1 of 1 evidence references');
      await taskDetail.getByRole('button', { name: 'Reload details', exact: true }).focus();
      await expect(taskDetail.getByRole('button', { name: 'Reload details', exact: true })).toBeInViewport({ ratio: 1 });
      expect(await taskDetail.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      const detailScreenshot = info.outputPath(`task-detail-${theme}-400.png`);
      await chat.screenshot({ path: detailScreenshot });
      await info.attach(`T-TASK-DETAIL ${theme} 400px`, { path: detailScreenshot, contentType: 'image/png' });
      const evidenceText = taskDetail.getByText('synthetic:verified-child-outcome', { exact: true });
      await evidenceText.scrollIntoViewIfNeeded();
      const historyHeading = taskDetail.getByRole('heading', { name: 'Recent recorded changes', exact: true });
      await historyHeading.scrollIntoViewIfNeeded();
      await expect(evidenceText).toBeInViewport({ ratio: 1 });
      await expect(historyHeading).toBeInViewport({ ratio: 1 });
      const evidenceScreenshot = info.outputPath(`task-detail-evidence-${theme}-400.png`);
      await chat.screenshot({ path: evidenceScreenshot });
      await info.attach(`T-TASK-DETAIL ${theme} evidence 400px`, { path: evidenceScreenshot, contentType: 'image/png' });
      await taskDetail.getByRole('button', { name: 'Close details', exact: true }).click();
      await criteriaSummary.scrollIntoViewIfNeeded();
      await criteriaSummary.focus();
      await expect(criteriaSummary).toBeFocused();
      await expect(criteria).toHaveAttribute('open');
      expect(await criteria.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      expect(await tasksView.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      await expect(criteriaDraft).toHaveValue('Keep this draft while reviewing the original request');
      const childKey = createHash('sha256').update(childTask.draft.id).digest('hex');
      await expect(tasksView.locator('.pm-chat__task').first()).toHaveAttribute('data-task-key', childKey);
      const cancelChild = await exchange('cancel_pm_request_task', { request: {
        operation_id: `criteria-list-reorder-${theme}`,
        expected_revision: JSON.parse(readFileSync(statePath, 'utf8')).revision,
        reason: 'Withdraw the child while retaining focus on the root criteria.', task_id: childTask.draft.id,
      } });
      expect(cancelChild).toMatchObject({ state: 'ready', task: { status: 'cancelled' } });
      await refreshPmReports(page);
      await expect(tasksView.locator('.pm-chat__task').first()).toHaveAttribute('data-task-key', rootKey);
      await expect(progress).toHaveText('Direct subtasks: 1 completed · 1 cancelled · 0 unfinished (2 total)');
      await expect(rootRow.locator('.pm-chat__task-meta').first()).toContainText('Pending');
      expect(JSON.parse(readFileSync(statePath, 'utf8')).tasks[rootTask.draft.id].status).toBe('pending');
      await expect(criteria).toHaveAttribute('open');
      await expect(criteriaSummary).toBeFocused();
      await expect(criteriaSummary).toBeInViewport({ ratio: 1 });
      await expect(criteriaDraft).toHaveValue('Keep this draft while reviewing the original request');
      await info.attach(`T-TASK-CRITERIA ${theme} 400px`, { body: await criteriaPane.screenshot(), contentType: 'image/png' });
      await page.keyboard.press('Enter');
      await expect(criteria).not.toHaveAttribute('open');
      await progress.scrollIntoViewIfNeeded();
      await expect(progress).toBeInViewport({ ratio: 1 });
      await expect(criteriaSummary).toBeFocused();
      await expect(criteriaSummary).toBeInViewport({ ratio: 1 });
      await info.attach(`T-TASK-CHILDREN ${theme} 400px`, { body: await criteriaPane.screenshot(), contentType: 'image/png' });
      await detailButton.click();
      const cancelReason = taskDetail.getByRole('textbox', { name: 'Reason for cancelling this task', exact: true });
      const cancelButton = taskDetail.getByRole('button', { name: 'Cancel task', exact: true });
      await expect(cancelReason).toBeVisible();
      await expect(cancelButton).toHaveAttribute('aria-disabled', 'true');
      const beforeCancel = readFileSync(statePath, 'utf8');
      await cancelButton.focus();
      await page.keyboard.press('Enter');
      expect(readFileSync(statePath, 'utf8')).toBe(beforeCancel, 'blank reason never writes');
      const cancelReasonText = 'Withdraw this request from tracking.\nKeep the original <literal> request and audit.';
      await cancelReason.fill(cancelReasonText);
      await expect(cancelButton).toHaveAttribute('aria-disabled', 'false');
      await cancelButton.focus();
      await expect(cancelButton).toBeInViewport({ ratio: 1 });
      const cancelImage = info.outputPath(`task-cancel-${theme}-400.png`);
      await chat.screenshot({ path: cancelImage });
      await info.attach(`T-TASK-CANCEL ${theme} controls 400px`, { path: cancelImage, contentType: 'image/png' });
      // Hold only the real outgoing cancellation. Apply a competing update on
      // the same authenticated host, then release the original bytes unchanged.
      await page.evaluate(() => {
        const host = window as any;
        for (const socket of new Set<WebSocket>(host.__gwtPlaywrightSockets || [])) {
          if (socket.readyState !== WebSocket.OPEN || !new URL(socket.url).searchParams.has('repo_hash')) continue;
          const originalSend = socket.send;
          socket.send = function(data: string) {
            const event = JSON.parse(String(data));
            if (event.kind === 'cancel_pm_request_task') {
              socket.send = originalSend;
              host.__heldTaskCancellation = event;
              host.__releaseTaskCancellation = () => originalSend.call(socket, data);
              return;
            }
            return originalSend.call(socket, data);
          };
        }
      });
      await page.keyboard.press('Enter');
      const held = await page.waitForFunction(() => (window as any).__heldTaskCancellation).then(handle => handle.jsonValue());
      expect(held).toMatchObject({ kind: 'cancel_pm_request_task', task_key: rootKey,
        request: { task_id: '', reason: cancelReasonText } });
      await expect(cancelButton).toHaveAttribute('aria-disabled', 'true');
      await expect(cancelReason).toHaveAttribute('readonly', '');
      await page.keyboard.press('Enter');
      expect(readFileSync(statePath, 'utf8')).toBe(beforeCancel, 'pending cancellation is neither optimistic nor duplicated');
      const competingUpdate = await exchange('update_pm_request_task', { request: {
        operation_id: `cancel-competing-update-${theme}`, expected_revision: held.request.expected_revision,
        reason: 'A competing browser corrects the assignee before cancellation arrives.',
        task: { ...JSON.parse(beforeCancel).tasks[rootTask.draft.id].draft, assignee: 'Another reviewer' },
      } });
      expect(competingUpdate).toMatchObject({ state: 'ready', revision: held.request.expected_revision + 1 });
      const competingBytes = readFileSync(statePath, 'utf8');
      const releaseCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await page.evaluate(() => (window as any).__releaseTaskCancellation());
      const conflict = await page.waitForFunction(({ id, cursor }) => (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
        entry.sequence > cursor && entry.payload.kind === 'pm_request_task' && entry.payload.request_id === id)?.payload,
      { id: held.request_id, cursor: releaseCursor }, { timeout: 15_000 }).then(handle => handle.jsonValue());
      exchanges.push({ request: held, response: conflict });
      expect(conflict.result).toMatchObject({ state: 'error', code: 'conflict', current_revision: competingUpdate.revision });
      await expect(taskDetail.locator('.pm-chat__task-cancel-status')).toContainText(conflict.result.message);
      await expect(taskDetail).toContainText('last confirmed detail (stale)');
      await expect(taskDetail).toContainText(original);
      await expect(cancelReason).toHaveValue(cancelReasonText);
      await expect(cancelButton).toBeFocused();
      await expect(cancelButton).toHaveAttribute('aria-disabled', 'true');
      expect(readFileSync(statePath, 'utf8')).toBe(competingBytes);
      await taskDetail.getByRole('button', { name: 'Reload details', exact: true }).click();
      await expect(cancelButton).toHaveAttribute('aria-disabled', 'false');
      await expect(taskDetail).not.toContainText('last confirmed detail (stale)');
      await expect(taskDetail).toContainText(`Selected snapshot revision ${competingUpdate.revision}.`);
      await expect(cancelReason).toHaveValue(cancelReasonText);
      await cancelButton.focus();
      await page.keyboard.press('Enter');
      await expect(taskDetail.locator('.pm-chat__task-cancel-status')).toContainText('This task is cancelled.');
      await expect(cancelReason).not.toBeVisible();
      const afterCancel = JSON.parse(readFileSync(statePath, 'utf8'));
      const beforeConfirmedCancel = JSON.parse(competingBytes);
      expect(afterCancel.revision).toBe(competingUpdate.revision + 1);
      expect(afterCancel.tasks[rootTask.draft.id]).toMatchObject({ status: 'cancelled', completion: null });
      expect(afterCancel.sources).toEqual(beforeConfirmedCancel.sources);
      expect(afterCancel.tasks[childTask.draft.id]).toEqual(beforeConfirmedCancel.tasks[childTask.draft.id]);
      expect(afterCancel.receipts.slice(0, -1)).toEqual(beforeConfirmedCancel.receipts);
      expect(afterCancel.receipts.at(-1)).toMatchObject({ action: 'cancel', reason: cancelReasonText, actor: { kind: 'user' } });
      expect(afterCancel.receipts.at(-1).actor.id).toMatch(/^browser:[0-9a-f-]{36}$/i);
      await expect(taskDetail).toContainText(cancelReasonText);
      await expect(criteriaDraft).toHaveValue('Keep this draft while reviewing the original request');
      const reloadAfterCancel = taskDetail.getByRole('button', { name: 'Reload details', exact: true });
      await expect(reloadAfterCancel).toBeFocused();
      await expect(reloadAfterCancel).toBeInViewport({ ratio: 1 });
      expect(await taskDetail.evaluate(node => node.scrollWidth <= node.clientWidth + 1)).toBe(true);
      const cancelledImage = info.outputPath(`task-cancel-confirmed-${theme}-400.png`);
      await chat.screenshot({ path: cancelledImage });
      await info.attach(`T-TASK-CANCEL ${theme} confirmed 400px`, { path: cancelledImage, contentType: 'image/png' });
      expect(errors).toEqual([]);
    } catch (error) {
      primaryErrors.push(error);
      throw error;
    } finally {
      try {
        const incoming = await page.evaluate(() => (window as any).__gwtPlaywrightMessages);
        const incomingPath = info.outputPath(`task-cancel-incoming-${theme}.json`);
        writeFileSync(incomingPath, JSON.stringify(incoming));
        await info.attach(`T-TASK-CANCEL ${theme} incoming`, { path: incomingPath, contentType: 'application/json' });
        await info.attach(`T-UTASK-USER-REOPEN/CANCEL/UPDATE/COMPLETE ${theme} transport and persistence`, {
          body: JSON.stringify({ synthetic_fixture_seed: true, native_intake_proof: false,
            production_task_ui: true, read_only_overview: true, theme, seed, exchanges, console_page_errors: errors,
            persisted: JSON.parse(readFileSync(statePath, 'utf8')) }), contentType: 'application/json',
        });
        const screenshot = info.outputPath(`task-reopen-transport-${theme}.png`);
        await page.screenshot({ path: screenshot });
        await info.attach(`T-UTASK-USER-REOPEN/CANCEL/UPDATE/COMPLETE ${theme} browser`, { path: screenshot, contentType: 'image/png' });
      } catch (error) { cleanupErrors.push(error); }
      try { await page.close(); } catch (error) { cleanupErrors.push(error); }
      try { await target.stop(); } catch (error) { cleanupErrors.push(error); }
      try { expect(errors).toEqual([]); } catch (error) { cleanupErrors.push(error); }
      if (cleanupErrors.length) {
        const failures = [...primaryErrors, ...cleanupErrors];
        throw new AggregateError(failures, 'Task reopen target evidence/cleanup failed:\n' + failures.map(String).join('\n'),
          { cause: primaryErrors[0] });
      }
    }
  });
});

test('typed model content selects the renderer across surfaces', async ({ page }, info) => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, 'requires an isolated checkout server');
  test.setTimeout(60_000);
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  const theme = info.project.name.includes('light') ? 'light' : 'dark';
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
  await gotoLiveGwt(page, BASE, { suppressProjectSurfaces: true });
  const style = await page.addStyleTag({ content: `
    body { height: auto !important; overflow: auto !important; }
    body > :not(#typed-content-host):not(script) { display: none !important; }
    #typed-content-host { display: grid; gap: var(--space-3); padding: var(--space-3); background: var(--color-canvas); color: var(--color-text); }
    #typed-content-host > section { height: 820px; min-height: 0; border: 1px solid var(--color-border); background: var(--color-surface); overflow: auto; }
    #typed-content-host > section > h2 { margin: var(--space-2); }
    #typed-content-host [data-factory-root] { position: relative; contain: layout paint; height: 760px; }
    #typed-content-host .window-body { position: relative; inset: auto; height: 100%; }
    #typed-content-host .release-notes-window { position: relative; inset: auto; transform: none; width: 100%; height: 760px; max-width: none; max-height: none; }
  ` });
  try {
    // Controlled factory acceptance against the checkout's hosted modules.
    // No renderer is injected, and no natural backend traffic or load is claimed.
    const evidence = await page.evaluate(async () => {
      const load = (path: string) => import(new URL(path, location.href).href);
      const [pmModule, boardModule, knowledgeModule, releaseModule] = await Promise.all([
        load('/pm-chat.js'), load('/board-logs-surface.js'),
        load('/knowledge-kanban-surface.js'), load('/release-notes-window.js'),
      ]);
      const host = document.createElement('main');
      host.id = 'typed-content-host';
      document.body.append(host);
      const cleanup: Array<() => void> = [() => host.remove()];
      (window as any).__typedContentCleanup = () => cleanup.reverse().forEach(dispose => dispose());
      const createNode = (tag: string, className = '', text?: string) => {
        const node = document.createElement(tag);
        node.className = className;
        if (text != null) node.textContent = String(text);
        return node;
      };
      const mount = (surface: string, title: string) => {
        const section = createNode('section');
        section.dataset.surface = surface;
        const root = createNode('div');
        root.dataset.factoryRoot = '';
        section.append(createNode('h2', '', title), root);
        host.append(section);
        return root;
      };
      const literal = '<script>window.__typedContentExecuted = true</script>';
      const markdown = { body: '**Shared Markdown**', body_html: '<p><strong>Shared Markdown</strong></p>' };
      const fallback = { body: literal };
      const noop = () => {};

      const reportsRoot = mount('reports', 'Reports');
      const copied: string[] = [];
      const pm = pmModule.createPmWindowModel();
      const reports = pmModule.createPmChat({ document, root: reportsRoot, sessionId: 'typed-session',
        onLogVisibility: noop, copyText: async (text: string) => { copied.push(text); } });
      reports.showReports();
      cleanup.push(() => reports.dispose(), () => pm.removePmWindowState('typed-pm'));
      pm.bindPmWindowState('typed-pm', 'typed-session');
      cleanup.push(pm.subscribePmWindowState((state: any) => state.windows['typed-pm'], (state: any) => reports.update(state)));
      const reportRows = [markdown, fallback].map((body, index) => ({ ...body, id: `typed-report-${index}`,
        kind: 'progress', created_at: '2026-10-21T00:00:00Z' }));
      const request = pm.requestPmReports('typed-pm');
      pm.applyPmWindowReceiveEvent({ kind: 'pm_reports', id: 'typed-pm', session_id: 'typed-session', reports: reportRows,
        request_id: request.request_id, version: { epoch: 'typed', revision: 1 }, snapshot: true });
      const pmState = pm.readPmWindowState().windows['typed-pm'];
      pm.applyPmWindowReceiveEvent({ kind: 'pm_reports', id: 'typed-pm', session_id: 'stale-session', reports: [] });
      (reportsRoot.querySelector('[aria-label="Copy report"]') as HTMLButtonElement).click();
      await Promise.resolve();

      const boardRoot = mount('board', 'Board');
      const boardBody = createNode('div', 'window-body');
      boardRoot.append(boardBody);
      const boardWindow = { id: 'typed-board', preset: 'board' };
      const board = boardModule.createBoardLogsSurface({ send: noop, createNode,
        windowMap: new Map([[boardWindow.id, boardRoot]]), focusWindowLocally: noop, sendWindowFocus: noop,
        pushAlertToast: noop, focusOrSpawnPreset: noop, activeWorkspace: () => ({ windows: [boardWindow] }),
        activeProjectTab: () => null, projectWindowContextById: () => null,
        visibleBounds: () => ({ x: 0, y: 0, width: 1400, height: 420 }), getActiveWorkProjection: () => null });
      cleanup.push(() => { board.boardStateMap.clear(); board.logStateMap.clear(); });
      board.mountBoardWindow(boardWindow, boardBody);
      for (const [kind, prefix, date] of [['board_entries', 'current', '21'], ['board_history_page', 'history', '20']]) {
        board.applyBoardLogsReceiveEvent({ kind, id: boardWindow.id, has_more_before: false,
          entries: [markdown, fallback].map((body, index) => ({ ...body, id: `${prefix}-${index}`,
            author_kind: 'user', author: 'Fixture', created_at: `2026-10-${date}T00:00:00Z` })) });
      }

      const knowledgeRoot = mount('knowledge', 'Knowledge');
      const knowledgeWindow = { id: 'typed-knowledge', preset: 'issue' };
      const sent: any[] = [];
      const knowledge = knowledgeModule.createKnowledgeKanbanSurface({ send: (message: any) => sent.push(message), createNode,
        windowMap: new Map([[knowledgeWindow.id, knowledgeRoot]]),
        workspaceWindowById: (id: string) => id === knowledgeWindow.id ? knowledgeWindow : null,
        getWorkspaceWindows: () => [knowledgeWindow], pendingIndexOpenTargetsByPreset: new Map(),
        knowledgeKindForPreset: () => 'issue', focusWindowLocally: noop, sendWindowFocus: noop,
        focusOrSpawnPreset: noop, openIssueLaunchWizard: noop, launchPending: {},
        visibleBounds: () => ({ x: 0, y: 0, width: 1400, height: 420 }), getActiveWorkProjection: () => null });
      cleanup.push(() => knowledge.clearKnowledgeBridgeState(knowledgeWindow.id));
      knowledge.mountKnowledgeWindow(knowledgeWindow, knowledgeRoot);
      const loadRequest = sent.find(message => message.kind === 'load_knowledge_bridge');
      knowledge.applyKnowledgeReceiveEvent({ kind: 'knowledge_entries', id: knowledgeWindow.id,
        knowledge_kind: 'issue', request_id: loadRequest.request_id, selected_number: null, refresh_enabled: true,
        entries: [{ number: 5016, title: 'Typed model content', state: 'open', labels: [], related_work_refs: [] }] });
      knowledge.requestKnowledgeDetail(knowledgeWindow.id, 'issue', 5016);
      const detailRequest = sent.findLast(message => message.kind === 'select_knowledge_bridge_entry');
      if (!detailRequest) throw new Error('Knowledge did not request the selected detail');
      knowledge.applyKnowledgeReceiveEvent({ kind: 'knowledge_detail', id: knowledgeWindow.id,
        knowledge_kind: 'issue', request_id: detailRequest.request_id,
        detail: { number: 5016, title: 'Typed model content', state: 'open', labels: [], related_works: [],
          sections: [markdown, fallback].map((body, index) => ({ ...body, title: `Content ${index + 1}` })) } });

      const releaseRoot = mount('release', 'Release Notes');
      const release = releaseModule.createReleaseNotesWindow({ document, send: noop });
      cleanup.push(() => release.close());
      release.handlePayload({ entries: [{ version: '9.0.0', date: '2026-10-21', sections: [
        { ...markdown, heading: 'Markdown' }, { heading: 'Legacy items', items: [literal] },
      ] }] });
      releaseRoot.append(release._state.root);
      return { literal, copied, sessionFenced: pm.readPmWindowState().windows['typed-pm'] === pmState,
        types: [pmState.reports.map((row: any) => row.content.type),
          board.boardStateMap.get(boardWindow.id).entries.map((row: any) => row.content.type),
          knowledge.knowledgeBridgeStateMap.get(knowledgeWindow.id).detail.sections.map((section: any) => section.content.type)] };
    });
    expect(evidence.types).toEqual([['markdown', 'markdown'], Array(4).fill('markdown'), ['markdown', 'markdown']]);
    expect(evidence.sessionFenced).toBe(true);
    expect(evidence.copied).toEqual(['**Shared Markdown**']);
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    const host = page.locator('#typed-content-host');
    await host.locator('[data-surface="knowledge"] .knowledge-section').last().locator('summary').click();
    for (const surface of ['reports', 'board', 'knowledge', 'release']) {
      const panel = host.locator(`:scope > [data-surface="${surface}"]`);
      await expect(panel.locator('.knowledge-markdown-body strong')).toHaveCount(surface === 'board' ? 2 : 1);
      await expect(panel.locator('.knowledge-markdown-body strong').first()).toBeVisible();
      const plaintext = surface === 'release' ? panel.locator('.release-notes-section li') : panel.locator('.knowledge-markdown-body.is-plaintext');
      await expect(plaintext).toHaveCount(surface === 'board' ? 2 : 1);
      for (const body of await plaintext.all()) await expect(body).toHaveText(evidence.literal);
      await expect(plaintext.first()).toBeVisible();
      await expect(panel.locator('script')).toHaveCount(0);
    }
    expect(await page.evaluate(() => (window as any).__typedContentExecuted)).toBeUndefined();
    const screenshot = info.outputPath(`typed-content-surfaces-${theme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true, animations: 'disabled' });
    await info.attach('Typed content controlled factory acceptance', { path: screenshot, contentType: 'image/png' });
    expect(errors).toEqual([]);
  } finally {
    await page.evaluate(() => { (window as any).__typedContentCleanup?.(); delete (window as any).__typedContentCleanup; });
    await style.evaluate(node => node.remove());
  }
});
