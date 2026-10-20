/** #5379 delivery slice: real PTY output and screen restore, not latency acceptance. */
import { readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { clearLiveLaunchWizard, gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL || "";
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME || "";
const AGENT = "runtime-output-5379-0";

test.use({ headless: false, viewport: { width: 1280, height: 900 } });
test("one Agent delivers fixed output and restores its screen", async ({ page }, info) => {
  test.skip(!BASE || !HOME || process.platform !== "win32", "requires issue-5379-browser-check.ps1");
  test.setTimeout(180_000);
  await withLiveGwtBackendLock(BASE, info, async () => {
    const fixture = JSON.parse(await readFile(join(HOME, "fixture.json"), "utf8"));
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    const barrier = join(HOME, "workload", "barrier.json");
    const runId = "functional-" + theme;
    let pane: { id: string; session_id: string } | undefined;
    try {
      await rm(barrier, { force: true });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true, projectKey: fixture.projects[0].project_key });
      await clearLiveLaunchWizard(page);
      const migration = page.locator("#migration-modal.open");
      if (await migration.isVisible()) {
        await migration.getByRole("button", { name: "Migrate", exact: true }).click();
        await expect(migration).toHaveCount(0);
      }
      pane = await launch(page, fixture.projects[0].branch_name);
      const id = pane.id;
      await expect.poll(async () => (await receipts()).some(receipt => receipt.session === pane!.session_id),
        { timeout: 60_000 }).toBe(true);
      await page.locator('.op-rail__surface[data-surface="agents"]').click();
      const tile = page.locator('.agents-surface:not(.agents-surface--preview) .agent-tile[data-agent-id="' + id + '"]');
      await expect(tile.locator(".xterm-rows")).toBeVisible();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect.poll(() => buffer(page, id), { timeout: 30_000 }).toContain("GWT5379_READY 0");
      await page.evaluate(id => {
        (window as any).__gwt5379FunctionalOutput = [];
        for (const socket of (window as any).__gwtPlaywrightSockets as WebSocket[]) {
          socket.addEventListener("message", event => {
            const payload = JSON.parse(String(event.data));
            if (payload.kind === "terminal_output" && payload.id === id)
              (window as any).__gwt5379FunctionalOutput.push(atob(payload.data_base64));
          });
        }
      }, id);
      await writeFile(barrier + ".tmp", JSON.stringify({ start_at: Date.now() + 1000, seconds: 10, run_id: runId }));
      await rename(barrier + ".tmp", barrier);
      await expect.poll(async () => (await receipts()).some(receipt => receipt.session === pane!.session_id
        && receipt.run_id === runId && receipt.complete && receipt.sequence === 10 && receipt.bytes === 81920),
      { timeout: 45_000, intervals: [1000] }).toBe(true);
      const output = () => page.evaluate(() => (window as any).__gwt5379FunctionalOutput.join("") as string);
      await expect.poll(output).toContain("GWT5379_SEQ_0_0010");
      const liveOutput = await output();
      const sequences = [...new Set([...liveOutput.matchAll(/GWT5379_SEQ_0_(\d{4})/g)].map(match => Number(match[1])))];
      expect(sequences).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
      expect(liveOutput.length, "actual PTY output contains the fixed payload").toBeGreaterThanOrEqual(81920);
      await expect.poll(() => buffer(page, id)).toContain("GWT5379_SEQ_0_0010");

      // A new WebSocket must receive a fresh authoritative PTY snapshot, not the old browser buffer.
      await page.reload();
      await page.locator('.op-rail__surface[data-surface="agents"]').click();
      await expect(tile.locator(".xterm-rows")).toBeVisible();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect.poll(() => page.evaluate(id => (window as any).__gwtPlaywrightMessages.some((entry: any) =>
        entry.payload.kind === "terminal_snapshot" && entry.payload.id === id
        && atob(entry.payload.data_base64).includes("GWT5379_SEQ_0_0010")), id), { timeout: 30_000 }).toBe(true);
      await expect.poll(() => buffer(page, id)).toContain("GWT5379_SEQ_0_0010");
      expect(errors, "zero console/page errors").toEqual([]);
      const evidence = { issue: 5379, scope: "functional output and reconnect only; no latency acceptance",
        fixture, theme, pane, sequences, output_bytes: liveOutput.length,
        producer_receipts: (await receipts()).filter(receipt => receipt.session === pane!.session_id),
        fresh_snapshot_contains_final_sequence: true, browser_errors: errors };
      await writeFile(join(HOME, "functional-" + theme + ".json"), JSON.stringify(evidence, null, 2) + "\n");
      await info.attach("functional-" + theme, { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      await rm(barrier, { force: true });
      if (pane) {
        await sendLiveGwtEvent(page, { kind: "close_window", id: pane.id });
        await expect(page.locator('.agents-surface:not(.agents-surface--preview) .agent-tile[data-agent-id="' + pane.id + '"]'))
          .toHaveCount(0, { timeout: 60_000 });
      }
    }
  });
});

async function buffer(page: Page, id: string): Promise<string> {
  return page.evaluate(id => String((window as any).__gwtTerminalTestApi?.bufferText?.(id) || ""), id);
}

async function receipts(): Promise<any[]> {
  const directory = join(HOME, "workload");
  return Promise.all((await readdir(directory)).filter(name => /^\d+\.json$/.test(name)).map(async name => {
    const identity = JSON.parse(await readFile(join(directory, name), "utf8"));
    const progress = await readFile(join(directory, name + ".progress"), "utf8").catch(error => {
      if (error.code === "ENOENT") return ""; throw error;
    });
    const last = progress.slice(0, progress.lastIndexOf("\n")).split("\n").at(-1);
    return last ? { ...identity, ...JSON.parse(last) } : identity;
  }));
}

async function launch(page: Page, branch: string): Promise<{ id: string; session_id: string }> {
  const request = async (detail: unknown, requireOpen = false) => {
    const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
    await sendLiveGwtEvent(page, detail);
    return (await page.waitForFunction(({ after, requireOpen }) => {
      const message = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.sequence > after
        && entry.payload.kind === "launch_wizard_state" && (!requireOpen || entry.payload.wizard)
        && (!entry.payload.wizard || (!entry.payload.wizard.is_hydrating
          && !entry.payload.wizard.runtime_resolution_pending && !entry.payload.wizard.launch_materialization_pending)));
      return message ? { wizard: message.payload.wizard } : null;
    }, { after, requireOpen }, { timeout: 90_000 })).jsonValue();
  };
  await request({ kind: "open_active_work_launch_wizard", branch_name: branch, linked_issue_number: null }, true);
  const action = (action: unknown) => request({ kind: "launch_wizard_action", action,
    bounds: { x: 20, y: 30, width: 900, height: 600 } });
  await action({ kind: "set_launch_path", path: "manual_setup" });
  let state = await action({ kind: "set_agent", agent_id: AGENT });
  for (let step = 0; step < 12 && state.wizard; step++) {
    expect(state.wizard.error).toBeFalsy();
    if (state.wizard.selected_runtime_target !== "host") state = await action({ kind: "set_runtime_target", target: "Host" });
    else {
      expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
      state = await action({ kind: "submit" });
    }
  }
  expect(state.wizard).toBeNull();
  return (await page.waitForFunction(agent => {
    const message = (window as any).__gwtPlaywrightMessages.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return message?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((pane: any) => pane.agent_id === agent && pane.session_id && !["stopped", "error"].includes(pane.status));
  }, AGENT, { timeout: 60_000 })).jsonValue();
}
