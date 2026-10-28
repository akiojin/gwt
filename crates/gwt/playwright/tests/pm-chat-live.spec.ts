/* SPEC #5023: explicit durable reports -> backend -> read-only PM view. */
import { expect, test, type Page } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { appendFileSync, existsSync, readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from './_helpers/live-gwt';

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? 'http://127.0.0.1:0/';
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? '';
const ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? '';
const SESSION = 'pm-reports-fixture';

async function gotoPmReports(page: Page) {
  await gotoLiveGwt(page, BASE, { enableTestBridge: true });
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

  test('Reports and Execution log render the same window version for every receive without focus or animation frames', async ({ page }, info) => {
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
          const originalFrame = window.requestAnimationFrame;
          const originalCancel = window.cancelAnimationFrame;
          const captured = new Map<number, FrameRequestCallback>();
          let requestedFrames = 0;
          let focusEvents = 0;
          let receivedAt: number | null = null;
          let before: (string | undefined)[] = [];
          const state: any = { result: null, snapshot, receivedSnapshot: null, replacementSocket: false };
          const check = () => {
            if (receivedAt === null || state.result) return;
            const after = versions();
            const elapsed_ms = performance.now() - receivedAt;
            const updated = report.textContent!.includes(marker) && after[0] === after[1]
              && Number(after[0]) > Number(before[0]);
            if (updated || elapsed_ms >= 1_000) state.result = {
              before, after, elapsed_ms, updated, requestedFrames, focusEvents,
              snapshot: state.receivedSnapshot, replacementSocket: state.replacementSocket,
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
              }
              return replacement;
            },
          });
          state.disconnect = () => new Promise<void>(resolve => {
            socket.addEventListener('close', () => resolve(), { once: true });
            socket.close();
          });
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
          if (snapshot) await page.evaluate(() => (window as any).__inactiveReportsPoll.disconnect());
          appendFileSync(`${HOME}/.gwt/projects/${fixture.repo_hash}/project-state/pm-reports.jsonl`, `${JSON.stringify({
            id: randomUUID(), kind: 'progress', body: `# ${marker}`, created_at: new Date().toISOString(),
          })}\n`);
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
      await page.reload();
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
      const refused = post({ kind: 'qa', question: '$gwt-pm', body: 'No change from the automatic prompt' });
      expect(refused.error).toBeUndefined();
      expect(refused.status).toBe(1);
      expect(refused.stdout + refused.stderr).toContain('Automatic PM prompts');
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
      await page.reload();
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
