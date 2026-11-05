/* Run against the browser-check isolated checkout, never a resident server.
 * A custom shell Agent supplies a real runtime/PTY without contacting an
 * external provider. The detach test requires the /bin/sh -i fixture. */
import { expect, test, type Page } from "@playwright/test";
import {
  focusLiveWindow, gotoLiveGwt, liveGwtProjectUrl,
  readLiveHubCatalog, sendLiveGwtEvent, withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

function capture(page: Page) {
  const errors: string[] = [];
  const sent: any[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  page.on("websocket", (socket) => socket.on("framesent", ({ payload }) => {
    try { sent.push(JSON.parse(String(payload))); } catch { /* no JSON */ }
  }));
  return { errors, sent };
}

async function latestWizard(page: Page, cursor: number, requireOpen = false) {
  return (await page.waitForFunction(({ cursor, requireOpen }) => {
    const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
      entry.sequence > cursor && entry.payload.kind === "launch_wizard_state"
      && (!requireOpen || entry.payload.wizard)
      && (!entry.payload.wizard || (!entry.payload.wizard.is_hydrating
        && !entry.payload.wizard.runtime_resolution_pending
        && !entry.payload.wizard.launch_materialization_pending)));
    return message ? { wizard: message.payload.wizard } : null;
  }, { cursor, requireOpen }, { timeout: 30_000 })).jsonValue();
}
async function cursor(page: Page) {
  return page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
}
type CanvasBounds = { x: number; y: number; width: number; height: number };
async function wizardAction(page: Page, action: unknown, bounds?: CanvasBounds) {
  const after = await cursor(page);
  await sendLiveGwtEvent(page, { kind: "launch_wizard_action", action,
    bounds: bounds ?? { x: 80, y: 80, width: 700, height: 440 } });
  return latestWizard(page, after);
}
async function launchAgent(page: Page, agentId: string, bounds?: CanvasBounds) {
  const existing = await page.evaluate((agentId) => {
    const messages = (window as any).__gwtPlaywrightMessages ?? [];
    const state = messages.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return state?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((entry: any) => entry.agent_id === agentId && ["running", "idle"].includes(
        messages.findLast((message: any) => message.sequence > state.sequence
          && message.payload.kind === "terminal_status" && message.payload.id === entry.id)?.payload.status ?? entry.status))?.id;
  }, agentId);
  if (existing) return existing as string;
  const after = await cursor(page);
  await sendLiveGwtEvent(page, { kind: 'open_active_work_launch_wizard',
    branch_name: process.env.GWT_PLAYWRIGHT_BRANCH_NAME, linked_issue_number: null });
  await latestWizard(page, after, true);
  await wizardAction(page, { kind: "set_launch_path", path: "manual_setup" }, bounds);
  let state = await wizardAction(page, { kind: "set_agent", agent_id: agentId }, bounds);
  for (let step = 0; step < 12 && state.wizard; step += 1) {
    expect(state.wizard.error).toBeFalsy();
    if (state.wizard.selected_runtime_target !== "host"
      && state.wizard.runtime_target_options?.some((option: any) => option.value === "host")) {
      state = await wizardAction(page, { kind: "set_runtime_target", target: "Host" }, bounds);
    } else {
      expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
      state = await wizardAction(page, { kind: "submit" }, bounds);
    }
  }
  expect(state.wizard).toBeNull();
  return (await page.waitForFunction((agentId) => {
    const messages = (window as any).__gwtPlaywrightMessages ?? [];
    const state = messages.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return state?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((entry: any) => entry.agent_id === agentId && ["running", "idle"].includes(
        messages.findLast((message: any) => message.sequence > state.sequence
          && message.payload.kind === "terminal_status" && message.payload.id === entry.id)?.payload.status ?? entry.status))?.id;
  }, agentId, { timeout: 30_000 })).jsonValue() as Promise<string>;
}
async function shell(page: Page) {
  const before = new Set(await page.locator('.workspace-window').evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.id)));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell", bounds: { x: 80, y: 80, width: 700, height: 440 } });
  const created = async () => (await page.locator('.workspace-window[data-preset="shell"]').evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.id!))).filter((id) => !before.has(id));
  await expect.poll(created).toHaveLength(1);
  return (await created())[0];
}
async function revealTerminal(page: Page, id: string) {
  await page.bringToFront();
  if (await page.locator(`.workspace-window[data-id="${id}"]`).isVisible()
    || await page.locator(`.works-full .works-terminal[data-window-id="${id}"]`).isVisible()) return;
  await page.locator('.op-rail__surface[data-surface="works"]').click();
  if (await page.locator('.works-surface:not(.works-surface--preview) .works-full:not([hidden])').isVisible()) {
    await page.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
  }
  const preset = await page.waitForFunction(id => {
    const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === 'workspace_state');
    return state?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows).find((pane: any) => pane.id === id)?.preset;
  }, id).then(handle => handle.jsonValue());
  const work = page.locator(`.work-row[data-window-id="${id}"]`);
  if (preset === 'agent') {
    await expect(work).toBeVisible();
    await work.getByRole('button', { name: 'Open terminal', exact: true }).click();
    await expect(page.locator(`.works-full .works-terminal[data-window-id="${id}"] .xterm-helper-textarea`)).toBeVisible();
  } else {
    await focusLiveWindow(page, id);
  }
}
async function terminalReady(page: Page, id: string) {
  await revealTerminal(page, id);
  await expect.poll(() => page.evaluate((id) => (window as any).__gwtTerminalTestApi?.bufferText(id)?.trim() ?? "", id), { timeout: 30_000 }).toMatch(/[^\n]+[>$#%]$/);
}
async function command(page: Page, id: string, command: string, expected: string) {
  await revealTerminal(page, id);
  await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: `${command}\r` });
  await expect.poll(() => page.evaluate((id) => (window as any).__gwtTerminalTestApi.bufferText(id), id), { timeout: 15_000 }).toContain(expected);
}
async function actualViewReady(page: Page) {
  await expect.poll(() => page.evaluate(() => (window as any).__gwtPlaywrightMessages
    ?.filter((entry: any) => entry.payload.kind === 'project_execution_status').at(-1)?.payload.status.view_state)).toBe('open');
}

// PW1 acceptance uses the real isolated backend, real PTY, and real choices.
test("live Project view close, Continue, ACK reconciliation and Stop preserve the other project", async ({ page, context }, testInfo) => {
  test.setTimeout(180_000);
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const agentId = process.env.GWT_PLAYWRIGHT_CLOSE_AGENT_ID;
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  test.skip(!base || !agentId, "requires isolated server and /bin/sh -i custom Agent fixture");
  await withLiveGwtBackendLock(base!, testInfo, async () => {
    const catalog = await readLiveHubCatalog(page, base!);
    expect(catalog.projects.length).toBeGreaterThanOrEqual(2);
    const a = catalog.projects.find(entry => entry.project_key === process.env.GWT_PLAYWRIGHT_PROJECT_KEY)!;
    expect(a, "the launch checkout must be present in the isolated catalog").toBeTruthy();
    const b = catalog.projects.find(entry => entry.project_key !== a.project_key)!;
    const mirror = await context.newPage(), other = await context.newPage(), hub = await context.newPage();
    const observations = [page, mirror, other, hub].map(capture);
    let agent = "", bShell = "", reopened: Page | undefined;
    try {
      for (const [current, key] of [[page, a.project_key], [mirror, a.project_key], [other, b.project_key]] as const) {
        await current.bringToFront();
        await gotoLiveGwt(current, base!, { projectKey: key, enableTestBridge: true });
        await expect.poll(() => current.evaluate(() => {
          const observations = (window as any).__gwtPlaywrightMessages?.filter((entry: any) => entry.payload.kind === 'project_execution_status') || [];
          return observations.length >= 2 && observations.at(-1).payload.status.view_state === 'open'
            && observations.at(-1).payload.status.view_set_revision > observations[0].payload.status.view_set_revision;
        })).toBe(true);
      }
      await gotoLiveGwt(hub, base!, { hub: true });
      for (const current of [page, mirror, other, hub]) await expect(current.locator("html")).toHaveAttribute("data-theme", theme);
      await page.bringToFront();
      agent = await launchAgent(page, agentId!); await terminalReady(page, agent);
      await command(page, agent, "GWT_PW1=$$; printf 'GWT_%s\\n' 'AGENT_READY'", "GWT_AGENT_READY");
      await other.bringToFront();
      bShell = await shell(other); await terminalReady(other, bShell);
      await command(other, bShell, "GWT_PW1_B=$$; printf 'GWT_%s\\n' 'B_READY'", "GWT_B_READY");

      // A normal close with another actual view detaches only this client.
      await page.bringToFront();
      await page.locator('#close-project-button').click();
      await expect(page.locator('[data-hub]')).toBeVisible();
      await expect(page.locator('#close-project-modal')).toBeHidden();
      await command(mirror, agent, "test \"$GWT_PW1\" = \"$$\" && printf 'GWT_%s\\n' 'SAME_AGENT'", "GWT_SAME_AGENT");
      let dialogs = 0; mirror.on('dialog', async dialog => { dialogs += 1; await dialog.dismiss(); });
      await mirror.close({ runBeforeUnload: true }); expect(dialogs).toBe(0);
      await expect(hub.locator(`[data-project-execution="${a.project_key}"]`).first()).toContainText(/Selection pending|Unknown/);

      reopened = await context.newPage(); observations.push(capture(reopened));
      let dropAck = false, dropped = 0;
      const confirmations: any[] = [];
      await reopened.routeWebSocket(url => url.pathname === '/ws' && url.searchParams.get('repo_hash') === a.project_key, ws => {
        const server = ws.connectToServer();
        ws.onMessage(message => {
          try { const event = JSON.parse(String(message)); if (event.kind === 'confirm_close_project') confirmations.push(event); } catch { /* binary forwarding */ }
          server.send(message);
        });
        server.onMessage(message => {
          let event: any; try { event = JSON.parse(String(message)); } catch { /* binary forwarding */ }
          if (dropAck && ['project_view_closed', 'project_closed', 'project_execution_status'].includes(event?.kind)) { dropped += 1; return; }
          ws.send(message);
        });
      });
      await reopened.bringToFront();
      await gotoLiveGwt(reopened, base!, { projectKey: a.project_key, enableTestBridge: true });
      await actualViewReady(reopened);
      await command(reopened, agent, "test \"$GWT_PW1\" = \"$$\" && printf 'GWT_%s\\n' 'REOPEN_SAME_AGENT'", "GWT_REOPEN_SAME_AGENT");
      const dialog = reopened.getByRole('dialog', { name: 'Close Project?' });
      const close = reopened.locator('#close-project-button');
      await close.click(); await expect(dialog).toBeVisible();
      const cancel = dialog.getByRole('button', { name: 'Cancel', exact: true });
      const stop = dialog.getByRole('button', { name: 'Stop', exact: true });
      await expect(cancel).toBeFocused(); await reopened.keyboard.press('Shift+Tab'); await expect(stop).toBeFocused();
      await reopened.keyboard.press('Tab'); await expect(cancel).toBeFocused();
      await reopened.keyboard.press('Escape'); await expect(dialog).toBeHidden(); await expect(close).toBeFocused();
      await command(reopened, agent, "printf 'GWT_%s\\n' 'AFTER_CANCEL'", "GWT_AFTER_CANCEL");
      await reopened.setViewportSize({ width: 400, height: 820 });
      await close.click(); await expect(dialog).toBeVisible();
      expect(await dialog.evaluate(element => element.getBoundingClientRect().width)).toBeLessThanOrEqual(400);
      await reopened.screenshot({ path: testInfo.outputPath(`project-choice-400-${theme}.png`) });
      dropAck = true;
      await dialog.getByRole('button', { name: 'Continue', exact: true }).click();
      await expect.poll(() => dropped).toBeGreaterThan(0);
      // The server committed the exact intent. Reload drops the ACK and
      // restores only its read-only status reconciliation, never a new choice.
      dropAck = false; await reopened.reload();
      await expect(reopened.locator('[data-hub]')).toBeVisible({ timeout: 30_000 });
      expect(confirmations).toHaveLength(1); expect(confirmations[0].choice).toBe('continue');
      await expect(hub.locator(`[data-project-execution="${a.project_key}"]`).first()).toContainText(/Saved choice: Continue/);

      await reopened.bringToFront();
      await gotoLiveGwt(reopened, base!, { projectKey: a.project_key, enableTestBridge: true });
      await actualViewReady(reopened);
      await command(reopened, agent, "test \"$GWT_PW1\" = \"$$\" && printf 'GWT_%s\\n' 'AFTER_CONTINUE'", "GWT_AFTER_CONTINUE");
      await reopened.setViewportSize({ width: 1440, height: 900 });
      await reopened.locator('.op-rail__surface[data-surface="agents"]').click();
      await expect(reopened.locator('.agents-surface [data-project-execution-status]')).toContainText('Saved choice: Continue');
      await reopened.locator('.agents-surface').getByRole('button', { name: 'Project Manager settings', exact: true }).click();
      await expect(reopened.locator('[data-settings-panel="project-manager"] [data-project-execution-status]').first()).toContainText('Saved choice: Continue');
      await reopened.screenshot({ path: testInfo.outputPath(`project-execution-settings-${theme}.png`) });

      // Hub close is explicitly project-wide, even with an actual view left.
      await hub.bringToFront();
      await hub.locator(`[data-close-project="${a.project_key}"]`).click();
      const hubDialog = hub.getByRole('dialog', { name: 'Close Project?' });
      await expect(hubDialog).toBeVisible(); await hubDialog.getByRole('button', { name: 'Cancel', exact: true }).click();
      await command(reopened, agent, "printf 'GWT_%s\\n' 'AFTER_HUB_CANCEL'", "GWT_AFTER_HUB_CANCEL");
      await hub.bringToFront();
      await hub.locator(`[data-close-project="${a.project_key}"]`).click();
      await hubDialog.getByRole('button', { name: 'Stop', exact: true }).click();
      await expect(reopened.locator('[data-hub]')).toBeVisible({ timeout: 30_000 });
      await expect(hub.locator(`[data-project-execution="${a.project_key}"]`).first()).toContainText(/Stopped.*Saved choice: Stop/);
      const reconcileDialog = hub.getByRole('dialog', { name: 'Check Project Choice' });
      if (await reconcileDialog.isVisible()) await reconcileDialog.getByRole('button', { name: 'Check status', exact: true }).click();
      await expect(reconcileDialog).toBeHidden();
      await command(other, bShell, "test \"$GWT_PW1_B\" = \"$$\" && printf 'GWT_%s\\n' 'B_UNCHANGED'", "GWT_B_UNCHANGED");
      expect(other.url()).toBe(liveGwtProjectUrl(base!, b.project_key));
      await hub.screenshot({ path: testInfo.outputPath(`project-stopped-${theme}.png`) });
    } catch (error) {
      const diagnosticPage = reopened && !reopened.isClosed() ? reopened : !page.isClosed() ? page : other;
      console.log('Project close lifecycle observations', JSON.stringify(await diagnosticPage.evaluate(() => {
        const messages = (window as any).__gwtPlaywrightMessages ?? [];
        return {
          workspace: messages.findLast((entry: any) => entry.payload.kind === 'workspace_state')?.payload.workspace,
          terminal: messages.filter((entry: any) => entry.payload.kind === 'terminal_status').slice(-8),
        };
      })));
      await diagnosticPage.screenshot({ path: testInfo.outputPath('failure.png') }).catch(() => {});
      await testInfo.attach('live-backend-messages', { body: JSON.stringify(await diagnosticPage.evaluate(() => (window as any).__gwtPlaywrightMessages) ?? []), contentType: 'application/json' });
      throw error;
    } finally {
      if (bShell) await sendLiveGwtEvent(other, { kind: 'close_window', id: bShell }).catch(() => {});
      // Restore only this test's custom Agent and views for the next theme.
      if (agent) {
        const cleanup = reopened && !reopened.isClosed() ? reopened : page;
        await gotoLiveGwt(cleanup, base!, { projectKey: a.project_key, enableTestBridge: true }).catch(() => {});
        await sendLiveGwtEvent(cleanup, { kind: 'close_window', id: agent }).catch(() => {});
      }
      await Promise.all([mirror.close(), other.close(), hub.close(), reopened?.close()]);
    }
    expect(observations.flatMap(entry => entry.errors)).toEqual([]);
  });
});
