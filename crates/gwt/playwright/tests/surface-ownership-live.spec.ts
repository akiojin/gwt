import { expect, test, type Locator, type Page } from "@playwright/test";

const controlUrl = process.env.GWT_AC11_CONTROL_URL;

type WindowData = { id: string; preset: string; agent_name?: string; session_id?: string; is_pm?: boolean; surface_owner?: string };

async function catalog(page: Page): Promise<WindowData[]> {
  return page.evaluate(() => {
    const workspace = (window as any).__ac11Workspace;
    return workspace?.tabs?.flatMap((tab: any) => tab.workspace?.windows ?? []) ?? [];
  });
}

async function typeInto(terminal: Locator, text: string) {
  const page = terminal.page();
  await page.evaluate(() => { (window as any).__ac11Messages = []; });
  const input = terminal.locator(".xterm-helper-textarea");
  // Use xterm's visible mouse path: its public focus() preserves outer scroll.
  await terminal.locator(".xterm-screen").click({ position: { x: 8, y: 8 } });
  await expect(input).toBeFocused();
  await input.pressSequentially(text);
  await input.press("Enter");
  await expect.poll(() => page.evaluate(() => (window as any).__ac11Messages
    .filter((message: any) => message.kind === "terminal_input").map((message: any) => message.data).join("")))
    .toContain(text + "\r");
  const after = await page.locator(".project-bar").boundingBox();
  expect(after?.y).toBeGreaterThanOrEqual(0);
}

async function expectLive(terminal: Locator, role: string) {
  const rows = terminal.locator(".xterm-rows");
  await expect(rows).toBeVisible();
  await expect(rows).toContainText(`AC11 TICK ${role}`, { timeout: 30_000 });
  const tick = async () => {
    const values = [...(await rows.innerText()).matchAll(new RegExp(`AC11 TICK ${role} (\\d+)`, "g"))];
    return values.length ? Number(values.at(-1)![1]) : 0;
  };
  const previous = await tick();
  await expect.poll(tick, { timeout: 10_000 }).toBeGreaterThan(previous);
}

test.describe("AC-11 real backend surface ownership", () => {
  test.use({ actionTimeout: 15_000 });
  test.skip(!controlUrl, "Run through target/velune-4777/live-ownership.py with an owned checkout GUI");

  test.afterEach(async ({ page }, testInfo) => {
    if (!controlUrl || page.isClosed()) return;
    await testInfo.attach("real-socket-observation", {
      body: JSON.stringify(await page.evaluate(() => ({
        workspace: (window as any).__ac11Workspace, messages: (window as any).__ac11Messages,
        focus: document.activeElement?.outerHTML,
      })), null, 2), contentType: "application/json",
    });
  });

  test("actual startup PTYs, reload and owned GUI restart preserve PM and Agents ownership", async ({ page, request }, testInfo) => {
    test.setTimeout(240_000);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(String(error)));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    // Observe the real socket without stubbing transport, assets, or backend events.
    await page.addInitScript(() => {
      const NativeWebSocket = window.WebSocket;
      (window as any).__ac11Messages = [];
      window.WebSocket = class extends NativeWebSocket {
        constructor(url: string | URL, protocols?: string | string[]) {
          super(url, protocols);
          this.addEventListener("message", event => {
            if (typeof event.data !== "string") return;
            try {
              const message = JSON.parse(event.data);
              if (message.kind === "workspace_state") (window as any).__ac11Workspace = message.workspace;
            } catch { /* Binary terminal transport remains owned by the app. */ }
          });
        }
        send(data: string | ArrayBufferLike | Blob | ArrayBufferView) {
          if (typeof data === "string") {
            try { (window as any).__ac11Messages.push(JSON.parse(data)); } catch { /* Native frame. */ }
          }
          super.send(data);
        }
      };
    });
    const control = async (route: string, data: unknown = {}) => {
      const response = await request.post(controlUrl + route, { data, timeout: 90_000 });
      expect(response.ok(), await response.text()).toBe(true);
      return response.json();
    };
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    const launched = await control("/reset", { theme });
    await page.goto(launched.url);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect.poll(async () => (await catalog(page)).filter(data => data.is_pm).length, { timeout: 45_000 }).toBe(1);
    await expect.poll(async () => (await catalog(page)).filter(data => data.session_id).length).toBe(3);

    const main = page.getByRole("region", { name: "Main pane", exact: true });
    const pmLauncher = page.getByRole("button", { name: "Project Manager", exact: true });
    const gear = page.getByRole("button", { name: "Project Manager settings", exact: true });
    const openPm = async () => {
      await pmLauncher.click();
      await expect(main).toHaveAttribute("data-surface", "pm");
      const pm = (await catalog(page)).find(data => data.is_pm)!;
      await expect(main.locator(".workspace-window")).toHaveCount(1);
      const terminal = main.locator(`.workspace-window[data-id='${pm.id}']`);
      await expect(terminal).toBeVisible();
      await terminal.getByRole("button", { name: "Execution log", exact: true }).click();
      await expect(terminal.locator(".xterm")).toBeVisible();
      await expect(main.locator(".agent-tile")).toHaveCount(0);
      await expect(page.getByRole("button", { name: "Split view", exact: true })).toBeDisabled();
      return terminal;
    };
    const assertRegistration = async () => {
      const state = await control("/state");
      const pm = (await catalog(page)).find(data => data.is_pm)!;
      expect(state.pm.settings.auto_start).toBe(false);
      expect(state.pm.registration.session_id).toBe(pm.session_id);
      expect(state.providers.filter((provider: any) => provider.role === "PM").at(-1)?.session_id).toBe(pm.session_id);
      expect((await catalog(page)).filter(data => data.is_pm)).toHaveLength(1);
      return state;
    };
    const pm = await openPm();
    await expectLive(pm, "PM");
    await typeInto(pm, "pm-before-settings");
    await expect(pm.locator(".xterm-rows")).toContainText("AC11 INPUT PM pm-before-settings");
    await pm.locator(".xterm").evaluate(node => { (window as any).__ac11PmTerminal = node; });
    const initial = await assertRegistration();
    expect(initial.providers.filter((provider: any) => provider.role === "PM")).toHaveLength(1);
    expect(initial.providers.map((provider: any) => provider.role).sort()).toEqual(["PM", "WORKER_A", "WORKER_B"]);
    for (const provider of initial.providers) expect(provider.args).toContain(provider.handle);

    await gear.click();
    await expect(main).toHaveAttribute("data-surface", "pm");
    const settings = main.locator(".workspace-window[data-preset='settings']");
    await expect(settings).toBeVisible();
    const settingsId = await settings.getAttribute("data-id");
    await expect.poll(async () => (await catalog(page)).find(data => data.id === settingsId)?.surface_owner).toBe("pm");
    await settings.getByRole("button", { name: "Close window", exact: true }).click();
    const closeDialog = page.getByRole("dialog", { name: "Close window", exact: true });
    await expect(closeDialog).toBeVisible();
    await closeDialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(settings).toBeVisible();
    expect(await settings.getAttribute("data-id")).toBe(settingsId);
    await openPm();
    expect(await pm.locator(".xterm").evaluate(node => node === (window as any).__ac11PmTerminal)).toBe(true);
    await gear.click();
    expect(await settings.getAttribute("data-id")).toBe(settingsId);
    await settings.getByRole("button", { name: "Close window", exact: true }).click();
    await closeDialog.getByRole("button", { name: "Close window", exact: true }).click();
    await expect(main.locator(".workspace-window[data-preset='agent']")).toBeVisible();
    expect(await pm.locator(".xterm").evaluate(node => node === (window as any).__ac11PmTerminal)).toBe(true);
    await gear.click();
    await expect(settings).toBeVisible();
    const reopenedSettingsId = await settings.getAttribute("data-id");
    await expect.poll(async () => (await control("/state")).workspace.windows.find((data: any) => data.id === reopenedSettingsId?.split("::").at(-1))?.surface_owner).toBe("pm");

    await page.locator(".op-rail__surface[data-surface='agents']").click();
    await expect(main).toHaveAttribute("data-surface", "agents");
    await expect(main.locator(".agent-tile")).toHaveCount(3);
    for (const role of ["WORKER_A", "WORKER_B"]) {
      const worker = (await catalog(page)).find(data => data.agent_name === `AC11 ${role}`)!;
      const tile = main.locator(`.agent-tile[data-agent-id='${worker.id}']`);
      await main.locator(`[id="agents-tab-${worker.id}"]`).click();
      await expectLive(tile, role);
      await typeInto(tile, "worker-input-" + role);
      await expect(tile.locator(".xterm-rows")).toContainText(`AC11 INPUT ${role} worker-input-${role}`);
    }
    await page.getByRole("tab", { name: "All agents", exact: true }).click();
    const shellWindow = (await catalog(page)).find(data => data.preset === "shell")!;
    const shell = main.locator(`.agent-tile[data-agent-id='${shellWindow.id}']`);
    await expect(shell.locator(".agent-tile__header span")).toHaveText("Interactive");
    await main.getByRole("tab", { name: "Start Work", exact: true }).click();
    await typeInto(shell, "printf 'AC11_SHELL_%s\\n' INPUT_OK");
    await expect(shell.locator(".xterm-rows")).toContainText("AC11_SHELL_INPUT_OK");
    await expect(main.locator(`.agent-tile[data-agent-id='${(await catalog(page)).find(data => data.is_pm)!.id}']`)).toHaveCount(0);
    await main.getByRole("tab", { name: "All agents", exact: true }).click();
    await expect(main.locator(".agent-tile:visible")).toHaveCount(3);
    await page.screenshot({ path: testInfo.outputPath("live-agents-before-reload.png") });

    await page.reload();
    await expect.poll(async () => (await catalog(page)).filter(data => data.is_pm).length).toBe(1);
    await gear.click();
    await expect(main).toHaveAttribute("data-surface", "pm");
    await expect(main.locator(".workspace-window[data-preset='settings']")).toHaveAttribute("data-id", reopenedSettingsId!);
    const reloadedPm = await openPm();
    await expectLive(reloadedPm, "PM");
    await typeInto(reloadedPm, "pm-after-reload");
    await expect(reloadedPm.locator(".xterm-rows")).toContainText("AC11 INPUT PM pm-after-reload");
    expect((await assertRegistration()).providers).toHaveLength(3);
    await page.screenshot({ path: testInfo.outputPath("live-pm-after-reload.png") });

    // Stop only the controller-owned GUI. Startup must resume fresh real PTYs
    // from the saved workspace, Session TOML and native conversation handles.
    await page.goto("about:blank");
    const restarted = await control("/restart");
    expect(restarted.pid).not.toBe(launched.pid);
    await page.goto(restarted.url);
    await expect.poll(async () => (await catalog(page)).filter(data => data.is_pm).length, { timeout: 45_000 }).toBe(1);
    await expect.poll(async () => (await catalog(page)).filter(data => data.session_id).length).toBe(3);
    const restoredPm = await openPm();
    await expectLive(restoredPm, "PM");
    await typeInto(restoredPm, "pm-after-gui-restart");
    await expect(restoredPm.locator(".xterm-rows")).toContainText("AC11 INPUT PM pm-after-gui-restart");
    const restored = await assertRegistration();
    expect(restored.providers).toHaveLength(6);
    for (const role of ["PM", "WORKER_A", "WORKER_B"]) {
      const launches = restored.providers.filter((provider: any) => provider.role === role);
      expect(launches).toHaveLength(2);
      expect(launches[1].handle).toBe(launches[0].handle);
      expect(launches[1].pid).not.toBe(launches[0].pid);
    }
    expect((await catalog(page)).some(data => data.preset === "shell")).toBe(false);
    await gear.click();
    await expect(main.locator(".workspace-window[data-preset='settings']")).toHaveAttribute("data-id", reopenedSettingsId!);
    await page.locator(".op-rail__surface[data-surface='settings']").click();
    await expect(main).toHaveAttribute("data-surface", "settings");
    await expect(main.locator(".workspace-window[data-preset='settings']")).toHaveAttribute("data-id", "ownership-live::settings-normal");
    await expect(page.getByRole("button", { name: "Split view", exact: true })).toBeEnabled();
    await page.locator(".op-rail__surface[data-surface='agents']").click();
    await expect(main.locator(".agent-tile")).toHaveCount(2);
    const worker = (await catalog(page)).find(data => data.agent_name === "AC11 WORKER_A")!;
    const tile = main.locator(`.agent-tile[data-agent-id='${worker.id}']`);
    await main.locator(`[id="agents-tab-${worker.id}"]`).click();
    await expectLive(tile, "WORKER_A");
    await typeInto(tile, "worker-after-restart");
    await expect(tile.locator(".xterm-rows")).toContainText("AC11 INPUT WORKER_A worker-after-restart");
    await main.getByRole("tab", { name: "All agents", exact: true }).click();
    await expect(main.locator(".agent-tile:visible")).toHaveCount(2);
    await page.screenshot({ path: testInfo.outputPath("live-agents-after-gui-restart.png") });
    expect(errors).toEqual([]);
    await testInfo.attach("real-backend-restore", { body: JSON.stringify({ initial, restored }, null, 2), contentType: "application/json" });
  });
});
