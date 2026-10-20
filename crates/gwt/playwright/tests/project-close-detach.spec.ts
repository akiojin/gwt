/* Run against the browser-check isolated checkout, never a resident server.
 * A custom shell Agent supplies a real runtime/PTY without contacting an
 * external provider. The detach test requires the /bin/sh -i fixture. */
import { execFileSync } from "node:child_process";
import { dirname } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import {
  gotoLiveGwt, liveGwtProjectUrl, openLiveLaunchWizardForBranch,
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
    const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return state?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((entry: any) => entry.agent_id === agentId && ["running", "idle"].includes(entry.status))?.id;
  }, agentId);
  if (existing) return existing as string;
  const after = await cursor(page);
  const fixture = await openLiveLaunchWizardForBranch(page);
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
  if (bounds) await fixture.cleanup();
  return (await page.waitForFunction((agentId) => {
    const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return state?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((entry: any) => entry.agent_id === agentId && ["running", "idle"].includes(entry.status))?.id;
  }, agentId, { timeout: 30_000 })).jsonValue() as Promise<string>;
}
async function shell(page: Page) {
  const before = new Set(await page.locator('.workspace-window').evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.id)));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell", bounds: { x: 80, y: 80, width: 700, height: 440 } });
  const created = async () => (await page.locator('.workspace-window[data-preset="shell"]').evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.id!))).filter((id) => !before.has(id));
  await expect.poll(created).toHaveLength(1);
  return (await created())[0];
}
async function terminalReady(page: Page, id: string) {
  await expect.poll(() => page.evaluate((id) => (window as any).__gwtTerminalTestApi?.bufferText(id)?.trim() ?? "", id), { timeout: 30_000 }).toMatch(/[^\n]+[>$#%]$/);
}
async function command(page: Page, id: string, command: string, expected: string) {
  await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: `${command}\r` });
  await expect.poll(() => page.evaluate((id) => (window as any).__gwtTerminalTestApi.bufferText(id), id), { timeout: 15_000 }).toContain(expected);
}

test("new Agent canvas right edge stays reachable without camera movement", async ({ page }, testInfo) => {
  test.setTimeout(180_000);
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const agentId = process.env.GWT_PLAYWRIGHT_CLOSE_AGENT_ID;
  test.skip(!base || !agentId, "requires isolated server and custom shell Agent fixture");
  await page.setViewportSize({ width: 1024, height: 1000 });
  await withLiveGwtBackendLock(base!, testInfo, async () => {
    const observations = capture(page);
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    const canvas = page.locator("#canvas");
    const stage = page.locator("#canvas-stage");
    await expect(canvas).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      testInfo.project.name.includes("light") ? "light" : "dark");
    const bounds = await page.evaluate(() => {
      const canvas = document.getElementById("canvas")!;
      const stage = document.getElementById("canvas-stage")!;
      const matrix = new DOMMatrix(getComputedStyle(stage).transform);
      return { x: -matrix.e / matrix.a, y: -matrix.f / matrix.a,
        width: canvas.clientWidth / matrix.a, height: canvas.clientHeight / matrix.a };
    });
    expect(bounds.width).toBeGreaterThanOrEqual(420);
    expect(bounds.width).toBeLessThan(1280);
    expect(bounds.height).toBeGreaterThanOrEqual(800);
    const cameraBefore = await stage.evaluate((element: HTMLElement) => element.style.transform);
    const existing = new Set(await page.locator(".workspace-window").evaluateAll((nodes) =>
      nodes.map((node) => (node as HTMLElement).dataset.id)));
    let id = "";
    try {
      id = await launchAgent(page, agentId!, bounds);
      expect(existing.has(id), "the test must exercise a newly placed Agent").toBe(false);
      const frame = page.locator(`.workspace-window[data-id="${id}"]`);
      await expect(frame).toBeVisible();
      const canvasBox = (await canvas.boundingBox())!;
      for (const target of [frame.locator('[data-action="close"]'), frame.locator(".resize-handle")]) {
        await expect(target).toBeVisible();
        const box = (await target.boundingBox())!;
        expect(box.x).toBeGreaterThanOrEqual(canvasBox.x);
        expect(box.y).toBeGreaterThanOrEqual(canvasBox.y);
        expect(box.x + box.width).toBeLessThanOrEqual(canvasBox.x + canvasBox.width);
        expect(box.y + box.height).toBeLessThanOrEqual(canvasBox.y + canvasBox.height);
        expect(await target.evaluate((element) => {
          const box = element.getBoundingClientRect();
          const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
          return hit === element || element.contains(hit);
        })).toBe(true);
      }
      expect(await stage.evaluate((element: HTMLElement) => element.style.transform)).toBe(cameraBefore);
      await expect(frame.locator('[data-action="maximize"], [data-action="snap"]')).toHaveCount(0);
      await page.screenshot({ path: testInfo.outputPath("agent-visible-right-edge.png") });
    } finally {
      if (id && !existing.has(id)) {
        await sendLiveGwtEvent(page, { kind: "close_window", id });
        await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
      }
    }
    expect(observations.errors).toEqual([]);
  });
});

test("live detach preserves the Agent; explicit close reaches all A tabs and leaves B running", async ({ page, context }, testInfo) => {
  test.setTimeout(180_000);
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const agentId = process.env.GWT_PLAYWRIGHT_CLOSE_AGENT_ID;
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  test.skip(!base || !agentId, "requires isolated server and /bin/sh -i custom Agent fixture");
  await withLiveGwtBackendLock(base!, testInfo, async () => {
    const catalog = await readLiveHubCatalog(page, base!);
    expect(catalog.projects.length).toBeGreaterThanOrEqual(2);
    const projectRoot = dirname(execFileSync("git", ["-C", process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!,
      "rev-parse", "--path-format=absolute", "--git-common-dir"], { encoding: "utf8" }).trim());
    const aKey = catalog.recent_projects.find((entry) => entry.path === projectRoot)?.project_key;
    const a = catalog.projects.find((entry) => entry.project_key === aKey);
    expect(a, "the launch checkout must be present in the isolated catalog").toBeTruthy();
    const b = catalog.projects.find((entry) => entry.project_key !== a!.project_key)!;
    const mirror = await context.newPage();
    const other = await context.newPage();
    const hub = await context.newPage();
    const observations = [page, mirror, other, hub].map(capture);
    let agent = "";
    let bShell = "";
    let detached = false;
    let reopened: Page | undefined;
    try {
      await gotoLiveGwt(page, base!, { projectKey: a!.project_key, enableTestBridge: true });
      await gotoLiveGwt(mirror, base!, { projectKey: a!.project_key, enableTestBridge: true });
      await gotoLiveGwt(other, base!, { projectKey: b.project_key, enableTestBridge: true });
      await gotoLiveGwt(hub, base!, { hub: true });
      for (const current of [page, mirror, other, hub]) {
        await expect(current.locator("html")).toHaveAttribute("data-theme", theme);
      }
      agent = await launchAgent(page, agentId!);
      await terminalReady(page, agent);
      await command(page, agent, "GWT_DETACH_4539=$$; printf 'GWT_%s\\n' 'AGENT_READY'", "GWT_AGENT_READY");
      await expect(mirror.locator(`.workspace-window[data-id="${agent}"]`)).toBeVisible();
      bShell = await shell(other);
      await terminalReady(other, bShell);
      await command(other, bShell, "GWT_B_4539=$$; printf 'GWT_%s\\n' 'B_READY'", "GWT_B_READY");
      await expect(page.locator('.project-tab-close')).toHaveCount(0);
      await expect(page.locator('#close-project-button')).toBeVisible();

      // The removed app shortcut cannot send a destructive action.
      const before = observations[0].sent.length;
      const modifier = process.platform === "darwin" ? "Meta" : "Control";
      for (const key of ["ArrowUp", "ArrowDown", "p"]) {
        await page.keyboard.press(`${modifier}+Shift+${key}`);
      }
      expect(observations[0].sent.slice(before).filter((event) => /close.*project|select_project_tab/.test(event.kind))).toEqual([]);
      await expect(page.locator('#close-project-modal')).toBeHidden();
      let dialogs = 0;
      page.on("dialog", async (dialog) => { dialogs += 1; await dialog.dismiss(); });
      await page.close({ runBeforeUnload: true });
      detached = true;
      expect(dialogs).toBe(0);
      await expect(mirror.locator(`.workspace-window[data-id="${agent}"]`)).toBeVisible();
      await command(mirror, agent, "test \"$GWT_DETACH_4539\" = \"$$\" && printf 'GWT_%s\\n' 'SAME_AGENT'", "GWT_SAME_AGENT");
      reopened = await context.newPage();
      observations.push(capture(reopened));
      await gotoLiveGwt(reopened, base!, { projectKey: a!.project_key, enableTestBridge: true });
      await expect(reopened.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(reopened.locator(`.workspace-window[data-id="${agent}"]`)).toBeVisible();
      await command(reopened, agent, "test \"$GWT_DETACH_4539\" = \"$$\" && printf 'GWT_%s\\n' 'REOPEN_SAME_AGENT'", "GWT_REOPEN_SAME_AGENT");

      const close = reopened.locator('#close-project-button');
      await close.click();
      const dialog = reopened.getByRole('dialog', { name: 'Close Project?' });
      await expect(dialog).toBeVisible();
      await expect(dialog).toHaveAttribute('aria-modal', 'true');
      await expect(dialog.locator('li')).toHaveCount(1);
      await expect(dialog.locator('li')).toContainText('Close detach fixture');
      const cancel = dialog.locator('[data-role="close-project-cancel"]');
      const confirm = dialog.locator('[data-role="close-project-confirm"]');
      await expect(cancel).toBeFocused();
      await reopened.keyboard.press('Shift+Tab');
      await expect(confirm).toBeFocused();
      await reopened.keyboard.press('Tab');
      await expect(cancel).toBeFocused();
      await reopened.keyboard.press('Escape');
      await expect(dialog).toBeHidden();
      await expect(close).toBeFocused();
      await command(reopened, agent, "printf 'GWT_%s\\n' 'AFTER_CANCEL'", "GWT_AFTER_CANCEL");
      await close.click();
      await expect(dialog).toBeVisible();
      await reopened.screenshot({ path: testInfo.outputPath('running-agent-warning.png') });
      await confirm.click();
      for (const current of [reopened, mirror]) {
        await expect(current.locator('[data-hub]')).toBeVisible();
        await expect(current).toHaveURL(new URL('/', base!).toString());
      }
      await expect(hub.locator(`[data-hub-list="open"] a[href="/p/${a!.project_key}"]`)).toHaveCount(0);
      await command(other, bShell, "test \"$GWT_B_4539\" = \"$$\" && printf 'GWT_%s\\n' 'B_UNCHANGED'", "GWT_B_UNCHANGED");
      expect(other.url()).toBe(liveGwtProjectUrl(base!, b.project_key));
      await reopened.close();

      // A has no running Agent after reopening: no confirmation dialog.
      await gotoLiveGwt(mirror, base!, { projectKey: a!.project_key, enableTestBridge: true });
      await mirror.waitForFunction((key) => (window as any).__gwtPlaywrightMessages?.some((entry: any) =>
        entry.payload.kind === "workspace_state"
        && entry.payload.workspace.tabs.some((tab: any) => tab.project_key === key)), a!.project_key);
      await mirror.locator('#close-project-button').click();
      await expect(mirror.locator('[data-hub]')).toBeVisible({ timeout: 15_000 });
      expect(observations[1].sent.filter((event) => event.kind === 'confirm_close_project').length).toBeGreaterThan(0);
      await expect(mirror.locator('#close-project-modal')).toBeHidden();
    } catch (error) {
      const diagnosticPage = page.isClosed() ? mirror : page;
      if (!diagnosticPage.isClosed()) {
        await testInfo.attach("live-backend-messages", {
          body: JSON.stringify(await diagnosticPage.evaluate(() => (window as any).__gwtPlaywrightMessages)),
          contentType: "application/json",
        });
      }
      throw error;
    } finally {
      if (bShell) await sendLiveGwtEvent(other, { kind: 'close_window', id: bShell }).catch(() => {});
      // Restore the catalog for the next theme/spec without relaunching agents.
      await gotoLiveGwt(mirror, base!, { projectKey: a!.project_key, enableTestBridge: true }).catch(() => {});
      if (agent) await sendLiveGwtEvent(mirror, { kind: 'close_window', id: agent }).catch(() => {});
      await Promise.all([mirror.close(), other.close(), hub.close(), reopened?.close()]);
      if (!detached) await page.close();
    }
    expect(observations.flatMap((entry) => entry.errors)).toEqual([]);
  });
});
