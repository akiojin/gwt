import { readFile, readdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_5119_CHECK_HOME ?? "";
const PREFS = process.env.GWT_5119_PREFS_PATH ?? "";
const LOGS = process.env.GWT_5119_LOG_DIR ?? "";
const COMPLETIONS = ["IssueMonitorScheduledScanComplete", "IssueMonitorScheduledScanPrepared"];

test.describe("scheduled Issue Monitor completion (isolated live backend)", () => {
  test.skip(!BASE || !CHECK_HOME, "requires the isolated checkout launcher");
  test.setTimeout(380_000);

  test("scheduled completion stays responsive and each GUI dispatch fits 30 ms", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      expect(resolve(PREFS).startsWith(resolve(CHECK_HOME) + "\\")
        || resolve(PREFS).startsWith(resolve(CHECK_HOME) + "/")).toBe(true);
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute(
        "data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark",
      );
      const before = await dispatchSamples();
      // Only isolated controls change. Automatic launch remains disabled and
      // the impossible label excludes all real Issues from admission.
      await writeFile(PREFS, JSON.stringify({
        enabled: true, launch_auto: false, max_active_agents: 1,
        allowed_labels: ["gwt-5119-isolated-no-candidates"],
        priority_order: [], auto_apply_updates: false,
      }));
      await expect.poll(async () => {
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        // This GUI-owned query always replies; monitor status can be omitted
        // by a live daemon and its synchronous IPC is a separate surface.
        await sendLiveGwtEvent(page, { kind: "list_windows" });
        await page.waitForFunction(cursor => (window as any).__gwtPlaywrightMessages.some(
          (entry: any) => entry.sequence > cursor && entry.payload.kind === "window_list",
        ), cursor, { timeout: 5_000 });
        const current = await dispatchSamples();
        return COMPLETIONS.every(name => current[name].length > before[name].length);
      }, { timeout: 345_000, intervals: [5_000] }).toBe(true);
      const after = await dispatchSamples();
      const measured = Object.fromEntries(COMPLETIONS.map(name => [name,
        after[name].slice(before[name].length),
      ]));
      for (const samples of Object.values(measured)) {
        expect(samples.length).toBeGreaterThan(0);
        expect(Math.max(...samples)).toBeLessThanOrEqual(30);
      }
      await testInfo.attach("all-completion-dispatch-samples", {
        body: JSON.stringify(measured, null, 2), contentType: "application/json",
      });
      const screenshot = testInfo.outputPath("scheduled-completion.png");
      await page.screenshot({ path: screenshot });
      await testInfo.attach("scheduled-completion", { path: screenshot, contentType: "image/png" });
      expect(errors).toEqual([]);
    });
  });
});

async function dispatchSamples(): Promise<Record<string, number[]>> {
  const samples: Record<string, number[]> = Object.fromEntries(COMPLETIONS.map(name => [name, []]));
  for (const name of (await readdir(LOGS)).filter(name => name.startsWith("gwt.log"))) {
    // A non-blocking log writer may be appending the last record right now.
    for (const line of (await readFile(join(LOGS, name), "utf8")).split("\n").slice(0, -1)) {
      if (!line.includes("gwt.frontend.timing")) continue;
      const record = JSON.parse(line);
      if (COMPLETIONS.includes(record.fields?.event)) {
        samples[record.fields.event].push(record.fields.elapsed_ms);
      }
    }
  }
  return samples;
}

// Run each theme against a fresh browser-check instance: the startup drain is
// process-wide, so a reload of an already-ready backend cannot test this gate.
test("Issue #5876 canvas readiness precedes materializer admission", async ({ page }, testInfo) => {
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
  test.skip(!BASE || !home, "requires an isolated checkout browser-check instance");
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const roles = async () => {
    const projects = join(home, ".gwt/projects");
    const roots = [join(home, ".gwt/logs"), ...(await readdir(projects).catch(() => []))
      .map(project => join(projects, project, "logs"))];
    const rows: any[] = [];
    for (const root of roots) for (const file of (await readdir(root).catch(() => [])).filter(name => name.startsWith("gwt.log"))) {
      for (const line of (await readFile(join(root, file), "utf8")).split("\n")) {
        try {
          const row = JSON.parse(line);
          if (row.fields?.message === "Issue Monitor subscriber role changed") rows.push(row.fields);
        } catch { /* retry partial log writes */ }
      }
    }
    return rows;
  };
  await page.addInitScript(() => {
    const send = WebSocket.prototype.send;
    const held: { socket: WebSocket; data: string }[] = [];
    (window as any).__gwtHeldCanvas = held;
    WebSocket.prototype.send = function (data) {
      if (typeof data === "string" && JSON.parse(data).kind === "startup_auto_resume_ready") {
        held.push({ socket: this, data });
        return;
      }
      return send.call(this, data);
    };
    (window as any).__gwtReleaseCanvas = () => {
      for (const { socket, data } of held) {
        if (socket.readyState === WebSocket.OPEN) send.call(socket, data);
      }
    };
  });
  await gotoLiveGwt(page, BASE, { enableTestBridge: true });
  await page.waitForFunction(() => (window as any).__gwtHeldCanvas.length > 0);
  await expect.poll(async () => (await roles()).some(row => row.materializer_ready === false)).toBe(true);
  expect((await roles()).filter(row => row.materializer_ready === true), "canvas preparation must not advertise launch readiness").toHaveLength(0);
  await page.evaluate(() => (window as any).__gwtReleaseCanvas());
  await expect.poll(async () => (await roles()).filter(row => row.materializer_ready === true).length,
    { timeout: 90_000 }).toBe(1);
  // Keep the page alive across reconnects, as it is during an update restart.
  // Each new backend connection needs its own post-hydration readiness event.
  for (let reconnect = 0; reconnect < 3; reconnect++) {
    const previous = await page.evaluate(() => {
      const held = (window as any).__gwtHeldCanvas;
      const count = held.length;
      held.at(-1).socket.close();
      return count;
    });
    await expect.poll(() => page.evaluate(() => (window as any).__gwtHeldCanvas.length)).toBe(previous + 1);
    await page.evaluate(() => (window as any).__gwtReleaseCanvas());
  }
  // A repeated readiness event must preserve the already-live subscription.
  await page.evaluate(() => (window as any).__gwtReleaseCanvas());
  await sendLiveGwtEvent(page, { kind: "list_windows" });
  await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages.some(
    (entry: any) => entry.payload.kind === "window_list"));
  expect((await roles()).filter(row => row.materializer_ready === true)).toHaveLength(1);
  await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
  const screenshot = testInfo.outputPath("canvas-ready.png");
  await page.screenshot({ path: screenshot, animations: "disabled" });
  await testInfo.attach(`canvas-readiness-${testInfo.project.name}`, {
    path: screenshot, contentType: "image/png",
  });
  expect(errors, "zero console/page errors").toEqual([]);
});
