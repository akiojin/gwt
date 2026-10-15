/* SPEC #5023: explicit durable reports -> backend -> read-only PM view. */
import { expect, test } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { existsSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? 'http://127.0.0.1:0/';
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? '';
const ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? '';
const SESSION = 'pm-reports-fixture';

test.describe('PM reports', () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL || !HOME, 'requires isolated checkout fixture');
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(90_000);

  test('peer PM sent and received reports persist without duplicates', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
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
      await sendLiveGwtEvent(page, { kind: 'load_pm_reports', id });
      const outgoingReport = reports.locator('article').filter({ hasText: `PM peer ${outgoing.message.id} →` });
      const incomingReport = reports.locator('article').filter({ hasText: `PM peer ${incoming.id} ←` });
      const outgoingBody = `PM peer ${outgoing.message.id} → Request ${peer}: ${outgoingText.replace(/\n/g, ' ')} — 相手プロジェクトの対応待ち（PM 不在・停止または生存未確認）`;
      const incomingBody = `PM peer ${incoming.id} ← Notice ${peer}: ${incoming.text.replace(/\n/g, ' ')}`;
      for (const [report, body] of [[outgoingReport, outgoingBody], [incomingReport, incomingBody]] as const) {
        await expect(report).toHaveCount(1);
        await expect(report.locator('.pm-chat__text')).toHaveText(body);
        expect(await report.locator('.pm-chat__text').textContent()).toBe(body);
      }
      await sendLiveGwtEvent(page, { kind: 'load_pm_reports', id });
      await expect(outgoingReport).toHaveCount(1);
      await expect(incomingReport).toHaveCount(1);
      await page.reload();
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      await expect(outgoingReport).toHaveCount(1);
      await expect(incomingReport).toHaveCount(1);
      await expect(outgoingReport.locator('.pm-chat__text')).toHaveText(outgoingBody);
      await expect(incomingReport.locator('.pm-chat__text')).toHaveText(incomingBody);
      await incomingReport.scrollIntoViewIfNeeded();
      const screenshot = info.outputPath(`pm-peer-reports-${info.project.name}.png`);
      await pane.screenshot({ path: screenshot });
      await info.attach('PM peer reports', { path: screenshot, contentType: 'image/png' });
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
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
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
      await sendLiveGwtEvent(page, { kind: 'load_pm_reports', id });
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
      await sendLiveGwtEvent(page, { kind: 'load_pm_reports', id });
      await expect(currentCard).toContainText(`公開結果を確認 ${info.project.name}`);
      await expect(currentCard).not.toContainText('has not updated for 16 minutes');
      await expect(currentCard).not.toContainText('リリースの公開待ち');
      await expect(reports).not.toContainText('公開結果を確認');
      const saved = pmOperation('pm.report.post', { kind: 'progress', body });
      expect(saved.report.body).toBe(body);
      expect(saved.report.id).toBeTruthy();
      await sendLiveGwtEvent(page, { kind: 'load_pm_reports', id });
      const report = reports.locator('article').filter({ has: page.getByRole('heading', { name: marker, exact: true }) });
      await expect(report.locator('strong')).toHaveText('Verified');
      await expect(report.locator('li')).toHaveCount(2);
      await expect(report.locator('pre code')).toHaveText('cargo test -p gwt-core\n');
      await expect(report.locator('time')).toHaveAttribute('datetime', /T/);
      await expect(pane.locator('.pm-chat').locator('input, textarea, form, [contenteditable], button[type="submit"]')).toHaveCount(0);
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
      await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(body);

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
      await sendLiveGwtEvent(page, { kind: 'load_pm_reports', id });
      const qaReport = reports.locator('article').filter({ has: page.getByRole('heading', { name: answerMarker, exact: true }) });
      await expect(qaReport.getByRole('heading', { name: 'Q', exact: true })).toBeVisible();
      await expect(qaReport.getByRole('region', { name: 'Question', exact: true }).locator('.pm-chat__question')).toHaveText(question);
      await expect(qaReport.getByRole('heading', { name: 'A', exact: true })).toBeVisible();
      await expect(qaReport.getByRole('region', { name: 'Answer', exact: true }).locator('strong')).toHaveText('Confirmed');
      await qaReport.getByRole('button', { name: 'Copy report' }).click();
      await expect.poll(() => page.evaluate(() => navigator.clipboard.readText())).toBe(`Q: ${question}\n\nA:\n${answerBody}`);
      const kindFilter = pane.getByRole('combobox', { name: 'Report kind', exact: true });
      await kindFilter.selectOption('qa');
      await expect(qaReport).toBeVisible();
      await expect(report).not.toBeVisible();
      await sendLiveGwtEvent(page, { kind: 'load_pm_reports', id });
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
      await page.reload();
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
