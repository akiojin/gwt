import { copyFile, mkdir, readFile, readdir, realpath, writeFile } from "node:fs/promises";
import { createReadStream } from "node:fs";
import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
test.use({ headless: false, viewport: { width: 1440, height: 1000 } });
const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };
const json = async (path: string) => JSON.parse(await readFile(path, "utf8"));
const exec = promisify(execFile);
async function installedGuiPids(executable: string): Promise<number[]> {
  const { stdout } = await exec("pwsh", ["-NoProfile", "-Command",
    "ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process -Filter \"Name = 'gwt.exe'\" | Where-Object { $_.ExecutablePath -eq $env:GWT_HANDOFF_INSTALL_GWT } | ForEach-Object { $_.ProcessId })"],
  { env: { ...process.env, GWT_HANDOFF_INSTALL_GWT: executable } });
  return JSON.parse(stdout);
}
async function sha256(path: string): Promise<string> {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

async function resident() {
  const directory = join(HOME, ".gwt", "run");
  const files = (await readdir(directory)).filter(name => /^tray-.*\.lock$/.test(name));
  expect(files, "one isolated resident lock").toHaveLength(1);
  return json(join(directory, files[0]));
}
async function listedWindowIds(page: Page): Promise<string[]> {
  const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
  await sendLiveGwtEvent(page, { kind: "list_windows" });
  return page.waitForFunction(after => (window as any).__gwtPlaywrightMessages.find(
    (entry: any) => entry.sequence > after && entry.payload.kind === "window_list")?.payload.windows.map(
    (pane: any) => pane.id), cursor, { timeout: 30_000 }).then(handle => handle.jsonValue());
}

async function createShell(page: Page, index: number): Promise<string> {
  const before = await page.locator(".workspace-window").evaluateAll(nodes =>
    nodes.map(node => (node as HTMLElement).dataset.id));
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "shell",
    bounds: { x: 32 + index % 4 * 320, y: 32 + Math.floor(index / 4) * 370, width: 600, height: 350 } });
  const id = await page.waitForFunction(seen => [...document.querySelectorAll<HTMLElement>(
    '.workspace-window[data-preset="shell"]')].find(node => !seen.includes(node.dataset.id))?.dataset.id,
  before, { timeout: 30_000 }).then(handle => handle.jsonValue());
  await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi.bufferText(id).trim(), id),
    { timeout: 30_000 }).toMatch(/[^\n]+[>$#%]$/);
  await sendLiveGwtEvent(page, { kind: "terminal_input", id,
    data: `node "${join(HOME, "shell-load.cjs")}"\r` });
  await expect.poll(() => page.evaluate(id => window.__gwtTerminalTestApi.bufferText(id), id),
    { timeout: 30_000 }).toContain("GWT_HANDOFF_PTY_");
  return id;
}

test.describe("Issue #5510 resident Windows update handoff", () => {
  test.skip(process.platform !== "win32" || !process.env.GWT_PLAYWRIGHT_UPDATE_HANDOFF,
    "requires verify-issue-5510.ps1 and its isolated checkout/home");
  test.setTimeout(240_000);

  test("eight active PTYs remain observable, then explicit apply hands off once to a new resident", async ({ page, context, request }, info) => {
    const fixture = await json(join(HOME, "fixture.json"));
    expect(await realpath(HOME)).not.toBe(await realpath(homedir()));
    expect(await realpath(fixture.checkout)).toBe(await realpath(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!));
    // Updating checkout gwtd would rename a live canonical verifier to .old;
    // Windows then denies deleting it for the second theme's update. The
    // private install exercises identical source bytes without updating the
    // verifier's own artifact. Initial browser-check smoke uses checkout paths.
    expect(await realpath(fixture.source_gwt)).toBe(await realpath(join(fixture.checkout, "target/debug/gwt.exe")));
    expect(await realpath(fixture.source_gwtd)).toBe(await realpath(join(fixture.checkout, "target/debug/gwtd.exe")));
    expect(await realpath(fixture.install_root)).toBe(await realpath(join(HOME, "install")));
    for (const [binary, installed, source] of [["gwt.exe", fixture.gwt, fixture.source_gwt],
      ["gwtd.exe", fixture.gwtd, fixture.source_gwtd]]) {
      expect(await sha256(installed), `${binary} private install equals checkout bytes`).toBe(fixture.source_sha256[binary]);
      expect(await sha256(source), `${binary} checkout source remains unchanged`).toBe(fixture.source_sha256[binary]);
    }
    // The owner verifies smoke exit through its Process handle before disposal.
    // A later PID-only lookup can observe a different process after PID reuse.
    expect((await json(join(fixture.preferences, "issue-monitor.json"))).enabled).toBe(false);
    expect((await json(join(fixture.preferences, "pm.json"))).settings.auto_start).toBe(false);
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    const errors: string[] = [];
    const observe = (target: Page) => {
      target.on("pageerror", error => errors.push(error.message));
      target.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    };
    observe(page);
    // The drain controls are a frontend fixture, isolated from the real HTTP
    // apply below. Clicking Stop waiting must not mutate its staged manifest.
    await page.addInitScript(() => {
      (window as any).__handoffUiActions = [];
      const original = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data) {
        const event = typeof data === "string" ? JSON.parse(data) : null;
        if (["cancel_update_auto_apply", "apply_update_restart_now"].includes(event?.kind)) {
          (window as any).__handoffUiActions.push(event.kind);
          return;
        }
        return original.call(this, data);
      };
    });
    const old = await resident();
    expect(old.pid).toBe(fixture.pid);
    expect(await installedGuiPids(fixture.gwt), "one live GUI in the private install").toEqual([old.pid]);
    expect(old.control_token).toBeTruthy();
    await gotoLiveGwt(page, old.url, { enableTestBridge: true, suppressUpdateApplyStart: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    // A previous theme may have restored interrupted shells; own this fresh
    // fixture's windows and start precisely eight running PTYs for this apply.
    // Wait for a fresh backend reply and its DOM projection before using an
    // empty canvas as evidence: initial hydration can arrive after page load.
    const existingIds = await listedWindowIds(page);
    await expect(page.locator(".workspace-window")).toHaveCount(existingIds.length);
    for (const id of existingIds) {
      await sendLiveGwtEvent(page, { kind: "close_window", id });
    }
    await expect.poll(() => listedWindowIds(page), { timeout: 30_000 }).toEqual([]);
    await expect(page.locator(".workspace-window")).toHaveCount(0);
    const receiptsBefore = (await readFile(join(HOME, "shell-processes.jsonl"), "utf8")).split("\n").filter(Boolean).length;
    const windows: string[] = [];
    for (let index = 0; index < 8; index++) windows.push(await createShell(page, index));
    await expect(page.locator('.workspace-window[data-preset="shell"]')).toHaveCount(8);
    const receipts = (await readFile(join(HOME, "shell-processes.jsonl"), "utf8"))
      .split("\n").filter(Boolean).slice(receiptsBefore).map(line => JSON.parse(line));
    expect(receipts).toHaveLength(8);
    for (const receipt of receipts) expect(alive(receipt.pid), "real PTY workload is alive").toBe(true);

    const inject = (payload: Record<string, unknown>) => page.evaluate(detail =>
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail })), payload);
    await inject({ kind: "update_apply_pending_persisted", version: old.version });
    await inject({ kind: "issue_monitor_status", status: { enabled: true, state: "update_drain",
      update_drain: { version: old.version, since: new Date().toISOString(), reason: "auto",
        blocking: windows.map((window_id, index) => ({ kind: "active_pane", window_id,
          label: `handoff shell ${index + 1}`, state: "running" })) } } });
    const cta = page.locator("#update-cta");
    await expect(cta).toHaveAttribute("data-status", "draining");
    await expect(cta).toContainText("draining 8 agents");
    await cta.click();
    const modal = page.locator("#update-modal");
    await expect(modal).toHaveAttribute("data-variant", "anyway");
    for (let index = 1; index <= 8; index++) await expect(modal).toContainText(`handoff shell ${index} (running)`);
    await expect(modal.locator("[data-update-modal-restart-now]")).toHaveText("Apply now anyway");
    await expect(modal.locator("[data-update-modal-stop-waiting]")).toHaveText("Stop waiting");
    await page.screenshot({ path: info.outputPath(`${theme}-eight-pty-drain.png`) });
    await modal.locator("[data-update-modal-stop-waiting]").click();
    await expect.poll(() => page.evaluate(() => (window as any).__handoffUiActions)).toEqual(["cancel_update_auto_apply"]);
    expect(alive(old.pid)).toBe(true);
    expect(errors, "old GUI console and page errors").toEqual([]);

    // Use the same checkout binaries as a portable fixture payload. This tests
    // the actual helper replacement/restart, without GitHub downloads or MSI.
    const stage = join(HOME, "staged", theme);
    await mkdir(stage, { recursive: true });
    await copyFile(fixture.gwt, join(stage, "gwt.exe"));
    await copyFile(fixture.gwtd, join(stage, "gwtd.exe"));
    const pending = join(HOME, ".gwt", "pending-update");
    await mkdir(pending, { recursive: true });
    await writeFile(join(pending, "manifest.json"), JSON.stringify({ version: old.version,
      asset_url: "https://example.invalid/fixture-portable.zip", payload: { PortableBinary: { path: join(stage, "gwt.exe") } },
      downloaded_at: new Date().toISOString(), origin: "manual_later", auto_apply_deferred: true }));
    // Retire the old browser before its deliberate shutdown, so reconnect
    // errors from a dead server are not confused with frontend failures.
    await page.close();
    const applyRequestedAt = Date.now();
    const response = await request.post(new URL("/internal/update/apply", old.url).href, {
      headers: { Authorization: `Bearer ${old.control_token}` }, data: { version: old.version },
    });
    expect(response.status()).toBe(202); // queued is not proof of apply success
    await expect.poll(() => alive(old.pid), { timeout: 90_000 }).toBe(false);
    let next: Awaited<ReturnType<typeof resident>>;
    await expect(async () => {
      next = await resident();
      expect(next.pid).not.toBe(old.pid);
      expect(alive(next.pid)).toBe(true);
      expect((await request.head(next.url)).status()).toBe(200);
    }).toPass({ timeout: 90_000, intervals: [200, 500, 1000] });
    await writeFile(join(HOME, "fixture.json"), JSON.stringify({ ...fixture, pid: next!.pid, url: next!.url }));
    const guiPids = await installedGuiPids(fixture.gwt);
    expect(guiPids, "only the new resident GUI survives the update").toEqual([next!.pid]);
    let applied: Record<string, any> = {};
    await expect(async () => {
      applied = await json(join(HOME, ".gwt", "updates", "apply-result.json"));
      expect(applied.outcome).toBe("success");
      expect(applied.to_version).toBe(old.version);
      expect(applied.observed_version).toBe(old.version);
      expect(Date.parse(applied.recorded_at), "fresh result for this theme's apply").toBeGreaterThanOrEqual(applyRequestedAt);
    }).toPass({ timeout: 30_000, intervals: [200, 500, 1000] });
    const restarted = await context.newPage();
    observe(restarted);
    await gotoLiveGwt(restarted, next!.url, { enableTestBridge: true });
    await expect(restarted.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(restarted.locator("#close-project-button")).toBeVisible();
    await restarted.screenshot({ path: info.outputPath(`${theme}-restarted-gui.png`) });
    expect(errors, "both resident GUIs have zero console and page errors").toEqual([]);
    await writeFile(join(HOME, `handoff-${theme}.json`), JSON.stringify({ theme, queued_status: response.status(),
      old_pid: old.pid, new_pid: next!.pid, new_url: next!.url, real_ptys: receipts.length,
      private_install_gui_pids: guiPids,
      apply_result: applied, errors, payload: "same-checkout portable binary", restart_args_modified: false,
      release_discovery: fixture.release_discovery,
      install_root: fixture.install_root, source_sha256: fixture.source_sha256, artifact_boundary: fixture.artifact_boundary }));
  });
});
