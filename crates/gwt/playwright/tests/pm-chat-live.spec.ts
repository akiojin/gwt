/* SPEC #5023: explicit durable reports -> backend -> read-only PM view. */
import { expect, test } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? 'http://127.0.0.1:0/';
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? '';
const ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? '';
const SESSION = 'pm-reports-fixture';

test.describe('PM reports', () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL || !HOME, 'requires isolated checkout fixture');
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(90_000);

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
      await expect(reports).toBeVisible();
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
        expect(output.ok).toBe(true);
        return output;
      };
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
