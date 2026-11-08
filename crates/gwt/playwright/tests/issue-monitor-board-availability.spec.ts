import { spawn } from "node:child_process";
import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test("Issue #5287 real backend journals Issue load and render causes in errors.list", async ({ page }, info) => {
  test.setTimeout(120_000);
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "requires an isolated checkout gwt instance");
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
  expect(base, "requires the isolated checkout harness").not.toBe("");
  expect(await realpath(home)).not.toBe(await realpath(homedir()));
  const marker = JSON.parse(await readFile(join(home, "issue-5287-isolated.json"), "utf8"));
  expect(new URL(base).origin).toBe(new URL(marker.url).origin);
  const state = join(home, ".gwt/projects", marker.repo_hash, "project-state");
  expect(JSON.parse(await readFile(join(state, "pm.json"), "utf8")).settings.auto_start).toBe(false);
  expect(JSON.parse(await readFile(join(state, "issue-monitor.json"), "utf8")).enabled).toBe(false);
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  await page.addInitScript(theme => {
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      if (typeof data === "string") {
        const event = JSON.parse(data);
        // Fault this RPC before a completion can race the injected failure.
        // Diagnostic events still reach the real backend and persistent ledger.
        if (event.kind === "load_knowledge_bridge" && event.knowledge_kind === "issue") {
          queueMicrotask(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
            ...event, kind: "knowledge_error", message: `Issue #5287 ${theme} load acceptance cause`,
          } })));
          return;
        }
        if (event.kind === "report_knowledge_error") ((window as any).__availabilityReports ??= []).push(event);
      }
      return send.call(this, data);
    };
  }, theme);
  await gotoLiveGwt(page, base, { enableTestBridge: true });
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue", bounds: { x: 40, y: 40, width: 1200, height: 900 } });
  const window = page.locator('.workspace-window[data-preset="issue"]').last();
  await expect(window).toBeVisible();
  const id = await window.getAttribute("data-id");
  await sendLiveGwtEvent(page, { kind: "focus_window", id });
  await expect(window).toHaveClass(/focused/);
  const loadBadge = window.getByRole("button", { name: "Issue load failed · Details", exact: true }).first();
  await loadBadge.focus();
  await loadBadge.press("Enter");
  const loadNotice = page.locator(`[data-error-key="issue-window:${id}:load"]`);
  await expect(loadNotice).toBeFocused();
  await expect(loadNotice).toContainText(`Issue #5287 ${theme} load acceptance cause`);
  await page.getByRole("button", { name: "Close notifications", exact: true }).click();
  await page.evaluate(theme => {
    const original = Element.prototype.appendChild;
    Element.prototype.appendChild = function <T extends Node>(child: T): T {
      if (child instanceof Element && child.classList.contains("knowledge-row")) {
        Element.prototype.appendChild = original;
        throw new Error(`Issue #5287 ${theme} render acceptance cause`);
      }
      return original.call(this, child) as T;
    };
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: { kind: "issue_monitor_inbox",
      items: [{ issue: { number: 528701, title: "Diagnostic fixture", state: "open", labels: [] }, state: "queued" }],
    } }));
  }, theme);
  // The live Monitor may refresh immediately after injection; the fixture E2E
  // holds that response to assert visible failure and recovery. Here assert the
  // emitted diagnostic and actual backend ledger independently of refresh timing.
  await expect.poll(() => page.evaluate(() => ((window as any).__availabilityReports ?? [])
    .map((event: any) => event.stage))).toContain("render");
  const ledger = () => new Promise<any[]>((resolve, reject) => {
    const child = spawn(join(marker.project, "target/debug", process.platform === "win32" ? "gwtd.exe" : "gwtd"), [], {
      cwd: marker.project, windowsHide: true,
      env: { ...process.env, HOME: home, USERPROFILE: home, GWT_SESSION_ID: "", GWT_BIN_PATH: "" },
    });
    let output = "", stderr = "";
    child.stdout.on("data", chunk => { output += chunk; });
    child.stderr.on("data", chunk => { stderr += chunk; });
    child.once("error", reject);
    child.once("close", code => {
      if (code) return reject(new Error(stderr));
      try { resolve(JSON.parse(JSON.parse(output).output).errors); } catch (error) { reject(error); }
    });
    child.stdin.end(JSON.stringify({ schema_version: 1, operation: "errors.list", params: {} }));
  });
  await expect.poll(async () => (await ledger()).filter(row => row.target.window_id === id &&
    row.message.includes(`Issue #5287 ${theme}`)).map(row => row.message).sort(), { timeout: 20_000 }).toEqual([
      `Issue window load failed: Issue #5287 ${theme} load acceptance cause`,
      `Issue window render failed: Issue #5287 ${theme} render acceptance cause`,
    ].sort());
  await sendLiveGwtEvent(page, { kind: "close_window", id });
  expect(errors, "zero console/page errors").toEqual([]);
});
