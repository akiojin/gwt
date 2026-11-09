import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { startExactRelaunchFixture } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.skip(process.env.GWT_PLAYWRIGHT_REMOTE_COMMAND_RECOVERY !== "1",
  "requires checkout binaries and the isolated real owner/provider fixture");
test.use({ headless: false, viewport: { width: 1440, height: 1000 } });

type WindowView = { id: string; preset: string; session_id?: string; status: string };

test("Issue #6929: ten consecutive Start Work launches and remote inputs succeed", async ({ page }, info) => {
  test.setTimeout(900_000);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const errors: string[] = [];
  const refusals: string[] = [];
  const samples: Array<{ iteration: number; elapsed_ms: number; session_id: string; window_id: string; pid: number }> = [];
  let windows: WindowView[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("websocket", socket => socket.on("framereceived", ({ payload }) => {
    let event: any;
    try { event = JSON.parse(String(payload)); } catch { return; }
    if (String(payload).includes("Refresh the project connection and try again.")) {
      refusals.push(String(payload));
    }
    if (event.kind === "workspace_state") {
      windows = (event.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace?.windows ?? []);
    } else if (event.kind === "terminal_status") {
      const window = windows.find(window => window.id === event.id);
      if (window) window.status = event.status;
    }
  }));
  const fixture = await startExactRelaunchFixture(info, {
    async prepare({ home, project, env }) {
      // Start Work materializes from origin/develop. Keep every Git operation
      // real and local to this throwaway project and its private bare origin.
      const origin = join(home, "origin.git");
      for (const args of [
        ["init", "--bare", "--initial-branch=main", origin],
        ["remote", "add", "origin", origin],
        ["push", "origin", "HEAD:refs/heads/main", "HEAD:refs/heads/develop"],
      ]) {
        execFileSync("git", args, { cwd: project, env, encoding: "utf8", timeout: 30_000, windowsHide: true });
      }
    },
  });
  let passed = false;
  try {
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "agent_kanban" });
    await expect.poll(() => windows.find(window => window.preset === "agent_kanban")?.id,
      { timeout: 30_000 }).toBeTruthy();
    const boardId = windows.find(window => window.preset === "agent_kanban")!.id;

    for (let iteration = 1; iteration <= 10; iteration++) {
      const started = Date.now();
      const opened = await request(page, {
        kind: "open_start_work_in_agent_kanban", board_id: boardId, lane_id: "active",
      }, "launch_wizard_state", true);
      expect(opened.wizard.mode, "exercise Start Work, including branch materialization").toBe("start_work");
      await wizardAction(page, { kind: "set_launch_path", path: "manual_setup" });
      let state = await wizardAction(page, { kind: "set_agent", agent_id: "codex" });
      for (let step = 0; step < 12 && state.wizard; step++) {
        expect(state.wizard.error).toBeFalsy();
        if (state.wizard.selected_runtime_target !== "host"
          && state.wizard.runtime_target_options?.some((option: any) => option.value === "host")) {
          state = await wizardAction(page, { kind: "set_runtime_target", target: "Host" });
        } else {
          expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
          state = await wizardAction(page, { kind: "submit" });
        }
      }
      expect(state.wizard).toBeNull();
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 90_000, message: `native provider launch ${iteration}` }).toBe(iteration);
      const launch = (await fixture.launches())[iteration - 1];
      expect(launch.gwt_session_id).toBeTruthy();
      expect(samples.map(sample => sample.session_id)).not.toContain(launch.gwt_session_id);
      await expect.poll(() => windows.find(window => window.session_id === launch.gwt_session_id
        && ["running", "idle"].includes(window.status))?.id,
      { timeout: 60_000, message: "real PTY and provider hook bind the launched Session" }).toBeTruthy();
      const windowId = windows.find(window => window.session_id === launch.gwt_session_id)!.id;
      expect(() => process.kill(launch.pid, 0), "native provider PID remains alive").not.toThrow();
      const reply = await request(page, {
        kind: "pane_send_input", session_id: launch.gwt_session_id, text: `recovery-probe-${iteration}\n`,
      }, "pane_send_result");
      expect(reply, "the owner acknowledges the real remote input").toMatchObject({ ok: true, window_id: windowId });
      samples.push({ iteration, elapsed_ms: Date.now() - started,
        session_id: launch.gwt_session_id!, window_id: windowId, pid: launch.pid });
      expect(refusals, "no manual-refresh refusal during consecutive Start Work").toEqual([]);
      await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      await expect.poll(() => windows.some(window => window.id === windowId), { timeout: 30_000 }).toBe(false);
    }
    expect(samples).toHaveLength(10);
    expect(await fixture.launches()).toHaveLength(10);
    expect(errors, "zero console/page errors").toEqual([]);
    await info.attach(`consecutive-start-work-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    passed = true;
  } finally {
    try {
      await info.attach(`start-work-evidence-${theme}`, {
        body: JSON.stringify({ theme, isolated_home: fixture.home, project: fixture.project,
          passed, launches: samples, manual_refresh_refusals: refusals, browser_errors: errors,
          "User Verification Result": "n/a (autonomous)", "Agent Visual Check": passed ? "pass" : "fail",
          scope: "Ten real Start Work materializations, native provider PTYs and remote input acknowledgements. Owner transition and recovery deadlines are covered by Rust regression fixtures.",
        }, null, 2), contentType: "application/json",
      });
    } finally { await fixture.stop(); }
  }
});

async function wizardAction(page: Page, action: unknown): Promise<any> {
  return request(page, { kind: "launch_wizard_action", action }, "launch_wizard_state");
}

// Install the listener before sending. Native output can evict a wizard reply
// from the test bridge history while Playwright records DOM snapshots.
async function request(page: Page, event: unknown, replyKind: string, requireOpen = false): Promise<any> {
  return page.evaluate(({ event, replyKind, requireOpen }) => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])]
      .reverse().find(socket => socket.readyState === WebSocket.OPEN
        && new URL(socket.url).pathname === "/ws" && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("no project socket");
    return new Promise((resolve, reject) => {
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener("message", reply); };
      const reply = (event: MessageEvent) => {
        let payload;
        try { payload = JSON.parse(String(event.data)); } catch { return; }
        if (payload.kind !== replyKind || (requireOpen && !payload.wizard)
          || (payload.wizard && (payload.wizard.is_hydrating || payload.wizard.runtime_resolution_pending
            || payload.wizard.launch_materialization_pending))) return;
        cleanup(); resolve(payload);
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error(`${replyKind} reply timed out`)); }, 90_000);
      socket.addEventListener("message", reply);
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail: event }));
    });
  }, { event, replyKind, requireOpen });
}
