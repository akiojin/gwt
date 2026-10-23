/** Issue #5537: real local List I/O must not hold up another GUI request. */
import { randomUUID } from "node:crypto";
import { readFile, realpath, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
type Fixture = { project: string; repo_hash: string; gate_root: string; pid: number; label: string };

test.describe("Issue Monitor list worker (live backend)", () => {
  test.skip(!BASE || !CHECK_HOME || process.platform !== "win32", "requires the isolated Windows browser-check fixture");
  test.setTimeout(90_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("another GUI request replies while a local List read is pending", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const fixture = await requireFixture();
      const theme = info.project.name.includes("light") ? "light" : "dark";
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const generation = randomUUID();
      let windowId: string | undefined;
      let verified = false;
      try {
        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
        await expect(page.locator("#close-project-button")).toBeVisible();
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await sendLiveGwtEvent(page, {
          kind: "create_window", preset: "settings",
          bounds: { x: 40, y: 40, width: 1160, height: 860 },
        });
        const settings = page.locator('.workspace-window[data-preset="settings"]');
        await expect(settings).toBeVisible();
        windowId = await settings.getAttribute("data-id") ?? undefined;
        // Complete startup's automatic List before arming the one-shot Git gate.
        await observeList(page);
        await waitForList(page);
        await rm(join(fixture.gate_root, "started"), { force: true });
        await writeFile(join(fixture.gate_root, "armed"), generation);
        // Submit both requests in one browser turn. With tracing enabled, a
        // separate evaluate after polling the gate can miss its I/O deadline.
        await observeList(page, true);
        await expect.poll(() => readFile(join(fixture.gate_root, "started"), "utf8").catch(() => ""), {
          timeout: 10_000, intervals: [100], message: "the real local reader must enter the armed Git gate",
        }).toBe(generation);
        await waitForList(page);
        await page.waitForFunction(() => (window as any).__gwtListWorkerObservation.frames
          .some((frame: any) => frame.kind === "window_list"), undefined, { timeout: 10_000 });
        const observed = await observation(page);
        const fast = observed.frames.find((frame: any) => frame.kind === "window_list");
        const status = observed.frames.find((frame: any) => frame.kind === "issue_monitor_status");
        const inbox = observed.frames.find((frame: any) => frame.kind === "issue_monitor_inbox");
        expect(fast.elapsedMs, "GUI reply must precede the delayed List inbox").toBeLessThan(inbox.elapsedMs);
        expect(status.payload.status.enabled).toBe(false);
        expect(inbox.payload.items).toEqual([]);
        await expect(settings).toBeVisible();
        const screenshot = info.outputPath(`settings-${theme}.png`);
        await settings.screenshot({ path: screenshot });
        await info.attach(`settings-${theme}`, { path: screenshot, contentType: "image/png" });
        expect(errors, "zero console and page errors").toEqual([]);
        verified = true;
      } finally {
        await rm(join(fixture.gate_root, "armed"), { force: true });
        const path = info.outputPath(`list-worker-${fixture.label}-${theme}.json`);
        await writeFile(path, JSON.stringify({
          issue: 5537, label: fixture.label, theme, ownedPid: fixture.pid, verified, errors,
          // The existing operation deadline bounds this live Git delay. Rust
          // regression evidence separately covers the six-second reader gate.
          gate: "local git origin read under existing operation deadline",
          observation: await observation(page).catch(() => null),
        }, null, 2));
        await info.attach(`list-worker-${theme}`, { path, contentType: "application/json" });
        if (windowId) await sendLiveGwtEvent(page, { kind: "close_window", id: windowId }).catch(() => undefined);
      }
    });
  });
});

async function requireFixture(): Promise<Fixture> {
  expect(await realpath(CHECK_HOME), "fresh isolated HOME").not.toBe(await realpath(homedir()));
  const fixture: Fixture = JSON.parse(await readFile(join(CHECK_HOME, "issue-5537-isolated.json"), "utf8"));
  expect(await realpath(fixture.project)).toBe(await realpath(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!));
  expect(fixture.pid).toBe(Number(process.env.GWT_PLAYWRIGHT_PID));
  expect(fixture.gate_root).toBe(join(CHECK_HOME, "git-gate"));
  const stateRoot = join(CHECK_HOME, ".gwt/projects", fixture.repo_hash, "project-state");
  expect(JSON.parse(await readFile(join(stateRoot, "pm.json"), "utf8")).settings.auto_start).toBe(false);
  expect(JSON.parse(await readFile(join(stateRoot, "issue-monitor.json"), "utf8")).enabled).toBe(false);
  const served = (await readFile(join(CHECK_HOME, "url.txt"), "utf8")).trim();
  expect(new URL(BASE).origin).toBe(new URL(served).origin);
  return fixture;
}

async function observeList(page: Page, requestOther = false): Promise<void> {
  await page.evaluate(requestOther => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
      .find(socket => socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("project WebSocket missing");
    const observed = { started: performance.now(), frames: [] as unknown[] };
    (window as any).__gwtListWorkerObservation = observed;
    socket.addEventListener("message", event => {
      const payload = JSON.parse(String(event.data));
      if (["window_list", "issue_monitor_status", "issue_monitor_inbox"].includes(payload.kind)) {
        observed.frames.push({ kind: payload.kind, elapsedMs: performance.now() - observed.started, payload });
      }
    });
    window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail: { kind: "list_issue_monitor" } }));
    if (requestOther) {
      (window as any).__gwtListWorkerObservation.otherRequestedMs = performance.now() - observed.started;
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail: { kind: "list_windows" } }));
    }
  }, requestOther);
}

async function waitForList(page: Page): Promise<void> {
  await page.waitForFunction(() => ["issue_monitor_status", "issue_monitor_inbox"].every(kind =>
    (window as any).__gwtListWorkerObservation.frames.some((frame: any) => frame.kind === kind)),
  undefined, { timeout: 20_000 });
}

async function observation(page: Page): Promise<any> {
  return page.evaluate(() => (window as any).__gwtListWorkerObservation);
}
