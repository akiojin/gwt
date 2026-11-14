import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
test.skip(!base, "requires an isolated browser-check checkout and fresh HOME");
test.use({ userAgent: "", viewport: { width: 1440, height: 900 } });

test("Codex partial-region history survives the real owner snapshot", async ({ page }, info) => {
  test.setTimeout(120_000);
  await withLiveGwtBackendLock(base, info, async () => {
    const errors: string[] = [];
    let id = "";
    const inputs: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("websocket", socket => socket.on("framesent", ({ payload }) => {
      const event = JSON.parse(String(payload));
      if (event.kind === "terminal_input" && event.id === id) inputs.push(event.data);
    }));
    const windows = () => page.evaluate(() => (window as any).__gwtPlaywrightMessages
      .findLast((entry: any) => entry.payload.kind === "workspace_state")?.payload.workspace.tabs
      .flatMap((tab: any) => tab.workspace.windows) ?? []);
    const metrics = () => page.evaluate(id => (window as any).__gwtTerminalTestApi.metrics(id), id);
    const text = () => page.evaluate(id => (window as any).__gwtTerminalTestApi.bufferText(id, 1800), id);
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    await gotoLiveGwt(page, base, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect.poll(async () => {
      await sendLiveGwtEvent(page, { kind: "get_project_execution_status" });
      return page.evaluate(() => (window as any).__gwtPlaywrightMessages
        .findLast((entry: any) => entry.payload.kind === "project_execution_status")?.payload.status.view_state);
    }).toBe("open");
    const before = new Set((await windows()).map((window: any) => window.id));
    try {
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell" });
      await expect.poll(async () => (await windows()).filter((window: any) => window.preset === "shell" && !before.has(window.id)).length).toBe(1);
      id = (await windows()).find((window: any) => window.preset === "shell" && !before.has(window.id)).id;
      await sendLiveGwtEvent(page, { kind: "focus_window", id });
      await expect.poll(async () => (await windows()).sort((a: any, b: any) => b.z_index - a.z_index)[0]?.id).toBe(id);
      await page.reload();
      await expect(page.locator(`.workspace-window[data-id="${id}"] .xterm-screen`)).toBeVisible();
      await expect.poll(text, { timeout: 30_000 }).toMatch(/[>$#%]\s*$/);
      const rows = (await metrics()).rows;
      // Actual Codex 0.167.2 uses DECSTBM above its composer, then CRLF.
      // Feed that sequence through a real PTY and the production owner parser.
      const program = `import sys; sys.stdout.write("\\x1b[${rows};1Hcomposer\\x1b[1;${rows - 4}r\\x1b[1;1H" + "".join("line-%04d\\r\\n" % n for n in range(1,1501)) + "\\x1b[r\\x1b[${rows};1H\\r\\n")`;
      await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: `python3 -c '${program}'\r` });
      await expect.poll(text).toContain("line-1500");
      await page.reload();
      await expect.poll(text).toContain("line-0001");
      const restored = await metrics();
      expect(restored.bufferLength - restored.rows).toBeGreaterThanOrEqual(1000);
      const screen = page.locator(`.workspace-window[data-id="${id}"] .xterm-screen`);
      await expect(screen).toBeVisible();
      const box = (await screen.boundingBox())!;
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      const sentBeforeWheel = inputs.length;
      for (let notch = 0; notch < 600; notch++) {
        await page.mouse.wheel(0, -960);
        if (notch % 50 === 0 && restored.viewportY - (await metrics()).viewportY >= 1000) break;
      }
      expect(restored.viewportY - (await metrics()).viewportY).toBeGreaterThanOrEqual(1000);
      expect(inputs.slice(sentBeforeWheel), "history wheel must stay local").toEqual([]);
      expect(errors, "zero console/page errors").toEqual([]);
      await info.attach("partial-region-history.json", { contentType: "application/json",
        body: JSON.stringify({ platform: process.platform, restored, scrolled: await metrics(), errors }) });
      await info.attach("partial-region-history.png", { contentType: "image/png", body: await page.screenshot() });
    } finally {
      if (id) {
        await page.locator('.op-rail__surface[data-surface="works"]').click();
        await sendLiveGwtEvent(page, { kind: "close_window", id });
        await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0);
      }
    }
  });
});
