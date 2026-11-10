import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
test.skip(!base || !home, "requires browser-check checkout and isolated HOME");
test.use({ headless: false, viewport: { width: 1440, height: 900 } });

test("Issue #6806: ASCII, IME and paste reach the same real PTY once through three minutes of output", async ({ page }, info) => {
  test.setTimeout(300_000);
  await withLiveGwtBackendLock(base, info, async () => {
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    const errors: string[] = [];
    const created: string[] = [];
    const expected = new Map<string, string[]>();
    const wire = new Map<string, string[]>();
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("websocket", socket => socket.on("framesent", ({ payload }) => {
      const event = JSON.parse(String(payload));
      if (event.kind === "terminal_input" && created.includes(event.id)) {
        const chunks = wire.get(event.id) ?? [];
        chunks.push(event.data);
        wire.set(event.id, chunks);
      }
    }));
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    const fixture = join(home, `input-${theme}.cjs`);
    // The receipt is written by the child after reading stdin. Its ACK is an
    // application response, so terminal echo cannot satisfy these assertions.
    await writeFile(fixture, `
      const fs = require('node:fs'), path = require('node:path');
      const id = process.argv[2], file = path.join(__dirname, id + '.jsonl');
      fs.writeFileSync(file, '');
      process.stdin.setRawMode(true); process.stdin.setEncoding('utf8');
      let pending = '', n = 0;
      process.stdin.on('data', data => {
        pending += data;
        while (/[\\r\\n]/.test(pending)) {
          const end = pending.search(/[\\r\\n]/), value = pending.slice(0, end);
          pending = pending.slice(end + 1);
          if (!value) continue;
          fs.appendFileSync(file, JSON.stringify(value) + '\\n');
          process.stdout.write('ACK:' + value + '\\r\\n');
        }
      });
      process.stdin.resume(); process.stdout.write('INPUT_FIXTURE_READY\\r\\n');
      if (id.includes('stream')) setInterval(() => process.stdout.write('OUTPUT:' + n++ + '\\r\\n'), 100);
    `);
    const started = Date.now();
    try {
      await gotoLiveGwt(page, base, { enableTestBridge: true });
      await expect.poll(async () => {
        await sendLiveGwtEvent(page, { kind: "get_project_execution_status" });
        return page.evaluate(() => (window as any).__gwtPlaywrightMessages
          .findLast((entry: any) => entry.payload.kind === "project_execution_status")?.payload.status);
      }, { timeout: 60_000, intervals: [1000] }).toMatchObject({ view_state: "open", degraded: false });
      for (const label of ["control", "stream"]) {
        const before = (await windows(page)).map(window => window.id);
        await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell" });
        await expect.poll(async () => (await windows(page)).filter(window => window.preset === "shell" && !before.includes(window.id)))
          .toHaveLength(1);
        const id = (await windows(page)).find(window => window.preset === "shell" && !before.includes(window.id))!.id;
        created.push(id);
        expected.set(id, []);
        await expect.poll(() => page.evaluate(id => (window as any).__gwtPlaywrightMessages
          .findLast((entry: any) => entry.payload.kind === "terminal_status" && entry.payload.id === id)?.payload.status, id),
        { timeout: 60_000, intervals: [1000] }).toBe("running");
        await show(page, id);
        await expect.poll(() => text(page, id), { timeout: 60_000 }).toMatch(/[>$#%]\s*$/);
        const receiptId = `6806-${theme}-${label}`;
        await sendLiveGwtEvent(page, { kind: "terminal_input", id, data: `node "${fixture}" ${receiptId}\r` });
        await expect.poll(() => text(page, id), { timeout: 30_000 }).toContain("INPUT_FIXTURE_READY");
        await checkInputs(page, id, receiptId, label + "-start", expected.get(id)!, wire);
      }
      const stream = created[1];
      const observationStarted = Date.now();
      for (let round = 1; round <= 3; round++) {
        await page.waitForTimeout(Math.max(100, observationStarted + round * 60_000 - Date.now()));
        // A real layout change and fresh connection must retain input readiness.
        await page.setViewportSize({ width: 1440 + round * 20, height: 900 });
        if (round === 2) await show(page, stream);
        await checkInputs(page, stream, `6806-${theme}-stream`, `stream-${round}`, expected.get(stream)!, wire);
      }
      await show(page, created[0]);
      await checkInputs(page, created[0], `6806-${theme}-control`, "control-end", expected.get(created[0])!, wire);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      expect(errors, "zero console/page errors").toEqual([]);
      await info.attach("input-continuity", { body: Buffer.from(JSON.stringify({
        theme, os: process.platform, elapsed_ms: Date.now() - started,
        observation_ms: Date.now() - observationStarted,
        panes: created.map(id => ({ id, accepted_once: expected.get(id)!.length })),
        wire_sequence_verified: true,
        console_page_errors: errors, scope: "real Shell PTYs; provider TUIs and other OSes unmeasured",
      }, null, 2)), contentType: "application/json" });
      await info.attach("input-continuity-screen", { body: await page.screenshot(), contentType: "image/png" });
    } catch (error) {
      await info.attach("synthetic-input-wire", { body: Buffer.from(JSON.stringify(Object.fromEntries(wire), null, 2)), contentType: "application/json" });
      await info.attach("startup-diagnostic", { body: Buffer.from(JSON.stringify(await page.evaluate(() => ({
        route: location.pathname,
        status: (window as any).__gwtPlaywrightMessages.filter((entry: any) =>
          ["terminal_status", "workspace_state", "project_execution_status", "error"].includes(entry.payload.kind)).slice(-5),
      })), null, 2)), contentType: "application/json" });
      await info.attach("failed-input-screen", { body: await page.screenshot(), contentType: "image/png" });
      throw error;
    } finally {
      await page.locator('.op-rail__surface[data-surface="works"]').click();
      for (const id of created.reverse()) {
        await sendLiveGwtEvent(page, { kind: "close_window", id });
        await expect(page.locator(`.workspace-window[data-id="${id}"]`)).toHaveCount(0, { timeout: 30_000 });
      }
    }
  });
});

async function checkInputs(page: Page, id: string, receipt: string, label: string, expected: string[], wire: Map<string, string[]>) {
  const sentBefore = wire.get(id)?.length ?? 0;
  const input = page.locator(`.split-pane .workspace-window[data-id="${id}"] .xterm-helper-textarea`);
  await expect(input).toBeAttached();
  await input.focus();
  await page.keyboard.type(`ASCII-${label}`);
  await page.keyboard.press("Enter");
  const cdp = await page.context().newCDPSession(page);
  try {
    await cdp.send("Input.imeSetComposition", { text: `日本語-${label}`, selectionStart: 3, selectionEnd: 3 });
    await cdp.send("Input.insertText", { text: `日本語-${label}` });
  } finally { await cdp.detach(); }
  await page.keyboard.press("Enter");
  await input.evaluate((element, value) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", value + "\n");
    element.dispatchEvent(new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }));
  }, `PASTE-${label}`);
  await expect.poll(() => wire.get(id)?.slice(sentBefore).join(""))
    .toBe(`ASCII-${label}\r日本語-${label}\rPASTE-${label}\r`);
  expected.push(`ASCII-${label}`, `日本語-${label}`, `PASTE-${label}`);
  await expect.poll(async () => (await readFile(join(home, receipt + ".jsonl"), "utf8"))
    .trim().split("\n").filter(Boolean).map(line => JSON.parse(line))).toEqual(expected);
  await expect.poll(() => text(page, id)).toContain(`ACK:PASTE-${label}`);
}

async function windows(page: Page): Promise<Array<{ id: string; preset: string; z_index: number }>> {
  return page.evaluate(() => (window as any).__gwtPlaywrightMessages
    .findLast((entry: any) => entry.payload.kind === "workspace_state")?.payload.workspace.tabs
    .flatMap((tab: any) => tab.workspace.windows) ?? []);
}
async function show(page: Page, id: string) {
  await sendLiveGwtEvent(page, { kind: "focus_window", id });
  await expect.poll(async () => (await windows(page)).sort((a, b) => b.z_index - a.z_index)[0]?.id).toBe(id);
  await page.reload();
  await expect(page.locator(`.split-pane .workspace-window[data-id="${id}"]`)).toBeVisible({ timeout: 30_000 });
}
async function text(page: Page, id: string): Promise<string> {
  return page.evaluate(id => (window as any).__gwtTerminalTestApi.bufferText(id), id);
}
