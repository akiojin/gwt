/* Issue #3431 T-91001..3: real Windows Codex, checkout backend, actual PM PTY.
 * Native before evidence separately reproduces a 907-byte wake left in the
 * composer with steer=true and an already queued follow-up. Its short-success
 * run used unspecified features; it does not isolate message length as cause.
 */
import { spawnSync } from 'node:child_process';
import { appendFile, readdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? '';
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? '';
const ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? '';
const STATE = process.env.GWT_PLAYWRIGHT_PROJECT_STATE ?? '';
const POLICY_SENTINEL = 'GWT_PM_TRANSPORT_POLICY_3431';

async function json(path: string, fallback: any = {}) {
  return JSON.parse(await readFile(path, 'utf8').catch(() => JSON.stringify(fallback)));
}
async function lines(path: string): Promise<any[]> {
  const contents = await readFile(path, 'utf8').catch(() => '');
  const records = contents.split('\n');
  // A provider/receipt writer can be appending its final row. Observe only
  // complete records, and still fail on malformed committed JSON lines.
  if (!contents.endsWith('\n')) records.pop();
  return records.filter(Boolean).map(line => JSON.parse(line));
}
async function nativeRows(directory = join(HOME, '.codex', 'sessions')): Promise<any[]> {
  const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
  return (await Promise.all(entries.map(entry => entry.isDirectory()
    ? nativeRows(join(directory, entry.name))
    : entry.name.endsWith('.jsonl') ? lines(join(directory, entry.name)) : []))).flat();
}
function userMessages(rows: any[]) {
  return rows.filter(row => row.type === 'response_item' && row.payload.type === 'message' && row.payload.role === 'user')
    .map(row => row.payload.content?.map((part: any) => part.text ?? '').join('\n') ?? '');
}
function userTurn(rows: any[], operation: string) {
  return rows.filter(row => row.type === 'response_item' && row.payload.type === 'message' && row.payload.role === 'user')
    .find(row => row.payload.content?.some((part: any) => part.text?.includes(`[gwt-delivery:${operation}:`)))
    ?.payload.internal_chat_message_metadata_passthrough?.turn_id;
}
function turnForText(rows: any[], token: string) {
  return rows.filter(row => row.type === 'response_item' && row.payload.type === 'message' && row.payload.role === 'user')
    .find(row => row.payload.content?.some((part: any) => part.text?.includes(token)))
    ?.payload.internal_chat_message_metadata_passthrough?.turn_id;
}
function diagnostic(id: string, instruction: string) {
  // The existing reserved envelope classifies automation without arming a
  // human Q&A obligation. It creates no delivery receipt or acknowledgement.
  return `<task-notification><task-id>${id}</task-id><status>diagnostic</status><summary>${instruction}</summary></task-notification>`;
}
function pastedBodyVisible(contents: string, canary: string) {
  // Codex may fold a large paste. This confirms setup display only; native
  // exact input, hook ACK, and turn start are asserted independently later.
  const currentComposer = contents.slice(contents.lastIndexOf('›'));
  return contents.includes(canary) || /\[Pasted Content [1-9]\d* chars\]/.test(currentComposer);
}
async function text(page: Page, id: string, maxLines = 150) {
  return page.evaluate(({ id, maxLines }) => (window as any).__gwtTerminalTestApi.bufferText(id, maxLines), { id, maxLines });
}
async function composer(page: Page, id: string) {
  const contents = await text(page, id);
  return contents.slice(contents.lastIndexOf('›'));
}
async function input(page: Page, id: string, data: string) {
  await sendLiveGwtEvent(page, { kind: 'terminal_input', id, data });
}
async function pause(page: Page, value: boolean) {
  await sendLiveGwtEvent(page, { kind: 'set_pm_paused', paused: value });
  await expect.poll(async () => (await json(join(STATE, 'pm.json'))).settings.paused).toBe(value);
}
async function inbox(page: Page, items: any[]) {
  const path = join(HOME, 'signal-inbox.json');
  await writeFile(path, JSON.stringify(items));
  const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
  const result = spawnSync(process.env.GWT_PLAYWRIGHT_SIGNAL_PUBLISHER!, [HOME, ROOT, path], { encoding: 'utf8', timeout: 20_000 });
  expect(result.status, result.stderr).toBe(0);
  await page.waitForFunction(({ after, count }) => ((window as any).__gwtPlaywrightMessages ?? []).some((row: any) =>
    row.sequence > after && row.payload.kind === 'issue_monitor_inbox' && row.payload.items.length === count),
  { after, count: items.length }, { timeout: 20_000 });
}
test.describe('Windows real PM wake submission', () => {
  test.skip(process.platform !== 'win32' || process.env.GWT_PLAYWRIGHT_REAL_PM_WAKE !== '1',
    'Requires the isolated Windows real-provider browser-check wrapper');
  test.use({ viewport: { width: 1600, height: 1000 } });
  test.setTimeout(720_000);

  test('periodic and signal wake wait for the busy turn and start exactly one acknowledged turn', async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on('pageerror', error => errors.push(error.message));
      page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
      const theme = info.project.name.endsWith('light') ? 'light' : 'dark';
      await page.addInitScript(value => localStorage.setItem('gwt:ui:theme', value), theme);
      let id = '';
      let session = '';
      let observing = false;
      let observation: Promise<void> | undefined;
      const evidence: any[] = [];
      const policyAudits: any[] = [];
      try {
        expect(spawnSync('git', ['-C', ROOT, 'remote'], { encoding: 'utf8' }).stdout.trim(), 'fixture has no remote').toBe('');
        await gotoLiveGwt(page, BASE, { enableTestBridge: true, suppressProjectSurfaces: true, suppressUpdateApplyStart: true });
        await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
        await page.evaluate(() => {
          (window as any).__gwtWakeStateHistory = [];
          for (const socket of (window as any).__gwtPlaywrightSockets ?? []) socket.addEventListener('message', (event: MessageEvent) => {
            try {
              const payload = JSON.parse(String(event.data));
              if (payload.kind === 'window_state') (window as any).__gwtWakeStateHistory.push({ observedAt: new Date().toISOString(), payload });
            } catch { /* Only complete JSON state frames are diagnostic evidence. */ }
          });
        });
        await pause(page, true);
        await sendLiveGwtEvent(page, { kind: 'open_pm_agent' });
        await expect.poll(async () => {
          session = (await json(join(STATE, 'pm.json'))).registration?.session_id ?? '';
          return session;
        }, { timeout: 90_000 }).not.toBe('');
        id = await page.waitForFunction(session => {
          const messages = (window as any).__gwtPlaywrightMessages ?? [];
          const workspace = messages.findLast((row: any) => row.payload.kind === 'workspace_state')?.payload.workspace;
          return workspace?.tabs.flatMap((tab: any) => tab.workspace.windows)
            .find((pane: any) => pane.session_id === session)?.id;
        }, session, { timeout: 60_000 }).then(handle => handle.jsonValue());
        // Sidecars are overwritten in place. Save changes and exact GUI state
        // frames so a later Stop continuation cannot masquerade as prior Idle.
        observing = true;
        observation = (async () => {
          let previous = '';
          const path = join(HOME, `wake-state-history-${theme}.jsonl`);
          while (observing) {
            const runtimePath = join(HOME, '.gwt', 'sessions', 'runtime', process.env.GWT_PLAYWRIGHT_PID!, session + '.json');
            const runtime = await readFile(runtimePath, 'utf8').catch(() => '');
            if (runtime && runtime !== previous) {
              try {
                const value = JSON.parse(runtime);
                await appendFile(path, JSON.stringify({ observedAt: new Date().toISOString(), kind: 'runtime', session, value }) + '\n');
                previous = runtime;
              } catch { /* Retry an in-progress sidecar write on the next sample. */ }
            }
            const frames = await page.evaluate(() => (window as any).__gwtWakeStateHistory.splice(0)).catch(() => []);
            for (const frame of frames) await appendFile(path, JSON.stringify({ kind: 'gui', ...frame }) + '\n');
            await new Promise(resolve => setTimeout(resolve, 200));
          }
        })();
        await page.locator('#op-pm-entry').click();
        await page.getByRole('button', { name: 'Execution log', exact: true }).click();
        await expect(page.locator('.pm-conversation-log')).toBeVisible();
        await expect.poll(() => composer(page, id), { timeout: 60_000 }).toMatch(/Ask Codex|Trust and continue/);
        // Trust only the fixture whose source and generated hooks this wrapper
        // created. This is a one-time provider setup screen, never a wake retry.
        if ((await composer(page, id)).includes('Trust and continue')) await input(page, id, '\r');
        await expect.poll(() => composer(page, id), { timeout: 45_000 }).toContain('Ask Codex');
        // Both themes share this owned backend. Reset the prior signal while
        // paused before starting this theme's independent periodic fixture.
        await inbox(page, []);
        const baselineNative = userMessages(await nativeRows()).length;

        for (const kind of ['periodic', 'signal']) {
          await pause(page, true);
          if (kind === 'periodic') {
            const concerns = await json(join(STATE, 'concerns.json'));
            for (const concern of concerns.concerns ?? []) {
              if (concern.summary === 'Owned terminal transport fixture') concern.state = 'open';
            }
            await writeFile(join(STATE, 'concerns.json'), JSON.stringify(concerns));
          }
          if (kind === 'signal') {
            // Submission is accepted when its exact turn starts. The model may
            // legitimately reconcile for a long time; stop/restart this owned
            // PM as fixture cleanup instead of treating task completion as gate.
            await sendLiveGwtEvent(page, { kind: 'stop_window', id });
            await page.waitForFunction(id => ((window as any).__gwtPlaywrightMessages ?? []).findLast((row: any) =>
              row.payload.kind === 'window_state' && row.payload.window_id === id)?.payload.state === 'stopped', id,
            { timeout: 60_000 });
            await sendLiveGwtEvent(page, { kind: 'restart_window', id });
            await page.waitForFunction(id => ((window as any).__gwtPlaywrightMessages ?? []).findLast((row: any) =>
              row.payload.kind === 'window_state' && row.payload.window_id === id)?.payload.state === 'idle', id,
            { timeout: 90_000 });
            session = (await json(join(STATE, 'pm.json'))).registration.session_id;
            await page.getByRole('button', { name: 'Execution log', exact: true }).click();
            await expect(page.locator('.pm-conversation-log')).toBeVisible();
            await expect.poll(() => composer(page, id), { timeout: 30_000 }).toContain('Ask Codex');
            // Only the periodic case needs standing work. The signal producer
            // supplies its own inbox, so prevent an unrelated periodic wake
            // from winning this case's quiet boundary.
            const concerns = await json(join(STATE, 'concerns.json'));
            for (const concern of concerns.concerns ?? []) concern.state = 'withdrawn';
            await writeFile(join(STATE, 'concerns.json'), JSON.stringify(concerns));
          }
          // Exercise the existing explicit PM policy opt-in, and inspect the
          // actual generated leaf before giving the provider diagnostic input.
          const pmPrefs = await json(join(STATE, 'pm.json'));
          // Match the materializer's canonical PM checkout source, including
          // Git's checkout line endings. Delivery hashes remain byte-exact.
          const sourcePath = join(pmPrefs.registration.worktree_path, 'PM-TRANSPORT.md');
          const policySource = await readFile(sourcePath, 'utf8');
          const skillRoot = join(STATE, '..', 'pm', 'runtime', '.codex', 'skills', 'gwt-pm');
          const router = await readFile(join(skillRoot, 'SKILL.md'), 'utf8');
          const copiedPolicy = await readFile(join(skillRoot, 'references', 'project-policy.md'), 'utf8');
          const policyAudit: any = { kind, session, observedAt: new Date().toISOString(), sourcePath, router, policySource, copiedPolicy };
          policyAudits.push(policyAudit);
          await writeFile(join(HOME, `wake-policy-audit-${theme}.json`), JSON.stringify(policyAudits, null, 2));
          expect(pmPrefs.settings.project_policy_files).toEqual(['PM-TRANSPORT.md']);
          expect(router).toContain('](references/project-policy.md)');
          expect(policySource).toContain(POLICY_SENTINEL);
          expect(copiedPolicy).toContain(policySource);
          const baselineReceipts = await lines(join(STATE, 'pm-delivery-receipts.jsonl'));
          const operationIds = new Set(baselineReceipts.map(receipt => receipt.operation_id));
          const token = `GWT_WAKE_${theme.toUpperCase()}_${kind.toUpperCase()}`;
          const signalItems = [{ issue: { number: 1, author_login: 'akiojin', title: 'Owned signal fixture',
            labels: [], state: 'open', readiness: 'not_applicable' }, state: 'needs_human',
            claim_id: null, blocked_by_owner: null, claim_expires_at: null, launched_window_id: null }];
          const initial = diagnostic(`3431-busy-${theme}-${kind}`, `Authorized automated terminal transport test, not a human PM request or supervision cycle. Read the explicitly selected .codex/skills/gwt-pm/references/project-policy.md first. Its diagnostic exception applies to this turn only. Then execute only this PowerShell command: Write-Output ('GWT_DIAG_BIN=' + $env:GWT_BIN_PATH); Write-Output ('GWT_DIAG_SESSION=' + $env:GWT_SESSION_ID); Write-Output ('GWT_DIAG_PROJECT=' + $env:GWT_PROJECT_ROOT); Write-Output ('GWT_DIAG_CWD=' + (Get-Location).Path); Write-Output '${token}_BUSY'; Start-Sleep -Seconds 90; Write-Output '${token}_END'. Poll only that command if needed, wait for it to finish, then reply only ${token}_DONE. During this diagnostic turn, do not run PM setup, read unrelated skills/files/environment, discover tools, plan, delegate, contact GitHub, or invoke gwt. Actual scheduled and Monitor wake turns retain the normal PM contract. Input receipt marker: ${token}_BODY_READY`);
          await input(page, id, `\x1b[200~${initial}\x1b[201~`);
          await expect.poll(async () => pastedBodyVisible(await text(page, id), `${token}_BODY_READY`),
            { timeout: 30_000 }).toBe(true);
          await input(page, id, '\r');
          await expect.poll(() => text(page, id), { timeout: 90_000 }).toMatch(/Working|Waiting for background terminal/);
          await expect.poll(async () => (await nativeRows()).some(row => row.type === 'response_item'
            && ['custom_tool_call', 'function_call'].includes(row.payload.type)
            && String(row.payload.input ?? row.payload.arguments).includes(`Write-Output '${token}_BUSY'`)),
          { timeout: 240_000, message: 'the provider issues the controlled busy command before queue/tick setup' }).toBe(true);
          await expect.poll(async () => (await nativeRows()).some(row => row.type === 'response_item'
            && ['custom_tool_call_output', 'function_call_output'].includes(row.payload.type)
            && JSON.stringify(row.payload.output).includes(`${token}_BUSY`)),
          { timeout: 60_000, message: 'native tool output proves the controlled sleep actually started' }).toBe(true);
          const policyRows = await nativeRows();
          expect(userMessages(policyRows).filter(message => message === initial), 'the real provider accepted the exact initial body once').toHaveLength(1);
          const diagnosticTurn = turnForText(policyRows, `${token}_BODY_READY`);
          const policyReads = policyRows.filter(row => row.type === 'response_item'
            && ['custom_tool_call_output', 'function_call_output'].includes(row.payload.type)
            && row.payload.internal_chat_message_metadata_passthrough?.turn_id === diagnosticTurn
            && JSON.stringify(row.payload.output).includes(POLICY_SENTINEL));
          expect(policyReads.length, 'the real provider read the selected fixture policy in this diagnostic turn').toBeGreaterThan(0);
          policyAudit.nativePolicyReads = policyReads.map(row => ({ at: row.timestamp, call_id: row.payload.call_id,
            turn_id: diagnosticTurn, sentinel: POLICY_SENTINEL }));
          await writeFile(join(HOME, `wake-policy-audit-${theme}.json`), JSON.stringify(policyAudits, null, 2));
          // Queue acceptance is distinct from UserPromptSubmit and the next
          // model turn. Match the user's before screenshot with one real queue.
          const queued = diagnostic(`3431-queued-${theme}-${kind}`, `Automated transport diagnostic. After the current command finishes, reply only ${token}_QUEUED. Do not use tools or post human Q&A.`);
          await input(page, id, `\x1b[200~${queued}\x1b[201~`);
          await expect.poll(async () => pastedBodyVisible(await text(page, id), `${token}_QUEUED`),
            { timeout: 30_000 }).toBe(true);
          await input(page, id, '\t');
          await expect.poll(() => text(page, id), { timeout: 30_000 }).toContain('Queued follow-up inputs');
          await expect.poll(() => composer(page, id), { timeout: 30_000 }).toContain('Ask Codex');
          // Ordinary WS Tab queues the provider input but conservatively leaves
          // gwt's user-input tracker set. Clear the observed empty composer once
          // as fixture setup, before any actual wake; never use it as recovery.
          await input(page, id, '\x15');
          const loop = await json(join(STATE, 'pm-loop.json'));
          await pause(page, false);
          const resumedBusyAt = Date.now();
          await inbox(page, []);
          const atBusy = await text(page, id);
          expect(atBusy).toContain('Queued follow-up inputs');
          // Observe a scheduled scan beyond the real quiet-time floor while
          // the 90-second command is running. A recent user clock cannot be
          // substituted for the busy gate this case intends to exercise.
          await expect.poll(async () => {
            const now = await json(join(STATE, 'issue-monitor.json'));
            return Date.parse(now.last_scan_at ?? '');
          }, { timeout: 60_000 }).toBeGreaterThan(Math.max(resumedBusyAt + 10_000,
            Date.parse(loop.last_user_prompt_at ?? '2020-01-01') + 10_000));
          if (kind === 'signal') await inbox(page, signalItems);
          const busyRows = await nativeRows();
          expect(busyRows.some(row => row.type === 'response_item' && ['custom_tool_call_output', 'function_call_output'].includes(row.payload.type)
            && JSON.stringify(row.payload.output).includes(`${token}_END`)), 'the controlled command has not completed at the busy observation').toBe(false);
          const busyReceipts = (await lines(join(STATE, 'pm-delivery-receipts.jsonl')))
            .filter(receipt => !operationIds.has(receipt.operation_id));
          expect(busyReceipts, 'no wake is prepared or submitted while the original command is busy').toHaveLength(0);
          const busyText = await text(page, id);
          expect(busyText.slice(busyText.lastIndexOf('›')), 'no wake is staged into the composer while busy').not.toMatch(/Scheduled supervision tick|Monitor activity/);
          // Pause releases the resident Stop chain through its normal public
          // control. It does not submit the wake or type into its composer.
          await pause(page, true);
          const initialTurn = turnForText(busyRows, `${token}_BODY_READY`);
          const turnEvents = async (type: string, turn: string) => (await nativeRows()).filter(row => row.type === 'event_msg'
            && row.payload.type === type && row.payload.turn_id === turn);
          expect(initialTurn).toBeTruthy();
          await expect.poll(() => turnEvents('task_complete', initialTurn),
            { timeout: 180_000, message: 'the original busy turn completes while autonomous Stop is paused' }).toHaveLength(1);
          await expect.poll(async () => userMessages(await nativeRows()).some(message => message === queued),
            { timeout: 60_000, message: 'the prior queued input starts its own provider turn' }).toBe(true);
          const queuedTurn = turnForText(await nativeRows(), `${token}_QUEUED`);
          expect(queuedTurn).not.toBe(initialTurn);
          await expect.poll(() => turnEvents('task_complete', queuedTurn),
            { timeout: 150_000, message: 'the prior queued diagnostic reaches its real safe boundary' }).toHaveLength(1);
          await page.waitForFunction(id => ((window as any).__gwtPlaywrightMessages ?? []).findLast((row: any) =>
            row.payload.kind === 'window_state' && row.payload.window_id === id)?.payload.state === 'idle', id,
          { timeout: 30_000 });
          const runtimePath = join(HOME, '.gwt', 'sessions', 'runtime', process.env.GWT_PLAYWRIGHT_PID!, session + '.json');
          await expect.poll(async () => (await json(runtimePath)).status, { timeout: 30_000 }).toBe('Idle');
          const safeBoundary = { observedAt: new Date().toISOString(), initialTurn, queuedTurn,
            initialCompleted: await turnEvents('task_complete', initialTurn), queuedCompleted: await turnEvents('task_complete', queuedTurn),
            runtime: await json(runtimePath) };
          await pause(page, false);
          // Seed only the existing scheduling clocks after native completion
          // and actual Idle. No Stop budget/fingerprint, receipt, or turn is
          // fabricated; the ordinary producer/worker/hook performs delivery.
          const resumedLoop = await json(join(STATE, 'pm-loop.json'));
          const quietAt = new Date(Date.now() - 60_000).toISOString();
          await writeFile(join(STATE, 'pm-loop.json'), JSON.stringify({ ...resumedLoop,
            last_user_prompt_at: quietAt, last_continued_at: quietAt, last_wake_at: quietAt }));
          if (kind === 'signal') await inbox(page, signalItems);
          await expect.poll(async () => (await lines(join(STATE, 'pm-delivery-receipts.jsonl')))
            .filter(receipt => !operationIds.has(receipt.operation_id) && receipt.status === 'verified'),
          { timeout: 160_000, message: `${kind} exact target hook ACK after the busy turn` }).toHaveLength(1);
          await pause(page, true);
          const receipts = (await lines(join(STATE, 'pm-delivery-receipts.jsonl'))).filter(receipt => !operationIds.has(receipt.operation_id));
          const verified = receipts.find(receipt => receipt.status === 'verified');
          expect(verified.target_session_id).toBe(session);
          expect(receipts.filter(receipt => receipt.status === 'prepared')).toHaveLength(1);
          const matching = () => nativeRows().then(userMessages)
            .then(messages => messages.filter(message => message.includes(`[gwt-delivery:${verified.operation_id}:`)));
          await expect.poll(matching, { timeout: 30_000 }).toHaveLength(1);
          expect((await matching())[0]).toContain(kind === 'periodic' ? 'Scheduled supervision tick' : 'Monitor activity');
          const providerTurn = userTurn(await nativeRows(), verified.operation_id);
          expect(providerTurn, 'the provider records the exact accepted prompt turn').toBeTruthy();
          expect(providerTurn).not.toBe(initialTurn);
          expect(providerTurn).not.toBe(queuedTurn);
          const providerEvents = (type: string) => nativeRows().then(rows => rows.filter(row => row.type === 'event_msg'
            && row.payload.type === type && row.payload.turn_id === providerTurn));
          const started = await providerEvents('task_started');
          expect(started, 'queue acceptance is not substituted for turn start').toHaveLength(1);
          const prepared = receipts.find(receipt => receipt.status === 'prepared');
          expect(Date.parse(started[0].timestamp), 'a wake must start a new turn after preparation, not steer the old turn')
            .toBeGreaterThanOrEqual(Date.parse(prepared.recorded_at));
          await expect.poll(() => composer(page, id), { timeout: 30_000 }).toContain('Ask Codex');
          const complete = await text(page, id);
          expect(complete.slice(complete.lastIndexOf('›')), 'composer is empty after one acknowledged wake').toContain('Ask Codex');
          evidence.push({ kind, session, operation_id: verified.operation_id, receipts, busyText, safeBoundary, complete,
            nativePromptMatches: await matching(), providerTurn, providerStarted: await providerEvents('task_started'),
            observedAt: new Date().toISOString() });
          await writeFile(join(HOME, `wake-evidence-${theme}.json`), JSON.stringify(evidence, null, 2));
          await expect(page.locator('.pm-conversation-log')).toBeVisible();
          await page.screenshot({ path: join(HOME, `wake-${kind}-${theme}.png`) });
          console.log(`Real ${kind} wake acknowledged once (${theme}, operation ${verified.operation_id})`);
          expect((await json(join(STATE, 'issue-monitor.json'))).autonomous_mode).toBe(false);
          expect(await page.locator('.workspace-window[data-preset="agent"]').count(), 'no implementation agent was launched').toBe(1);
        }
        expect(userMessages(await nativeRows()).length).toBeGreaterThan(baselineNative);
        expect(errors, 'console and page errors').toEqual([]);
        await info.attach(`real-pm-wake-${theme}`, { body: await page.screenshot(), contentType: 'image/png' });
      } finally {
        observing = false;
        await observation;
        try {
          const finalEvidence = JSON.stringify({ home: HOME, root: ROOT,
            theme, session, id, evidence, policyAudits, errors, terminalBuffer: id ? await text(page, id).catch(() => '') : '',
            nativeRows: await nativeRows() }, null, 2);
          // Keep non-secret evidence outside Playwright's output directory,
          // which the next canonical run or test discovery may replace.
          await writeFile(join(HOME, `wake-final-evidence-${theme}.json`), finalEvidence);
          await info.attach('real-provider-wake-evidence', { body: finalEvidence, contentType: 'application/json' });
          await info.attach(`real-pm-wake-final-${theme}`, {
            body: await page.screenshot({ path: join(HOME, `wake-final-${theme}.png`) }), contentType: 'image/png' });
        } finally {
          if (id) {
            await pause(page, true).catch(() => {});
            await sendLiveGwtEvent(page, { kind: 'close_window', id }).catch(() => {});
          }
        }
      }
    });
  });
});
