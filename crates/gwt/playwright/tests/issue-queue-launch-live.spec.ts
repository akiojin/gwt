import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { startIssueQueueLaunchFixture } from "./_helpers/issue-queue-launch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.describe("Issue queue real backend launch", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_QUEUE_LAUNCH !== "1", "requires checkout binaries and isolated provider fixture");
  test.skip(process.platform === "win32", "fixture requires POSIX executables");
  test.setTimeout(240_000);
  test.use({ viewport: { width: 1600, height: 1100 } });

  test("Backlog drag launches once and Blocked generation stays stopped after app restart", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "settings",
        bounds: { x: 20, y: 60, width: 1480, height: 920 } });
      // create_window bounds position a preset-sized window; set the intended
      // fixture geometry so the drag source and target are both in view.
      const issueWindow = page.locator('.workspace-window[data-preset="settings"]');
      await expect(issueWindow).toBeVisible();
      await sendLiveGwtEvent(page, { kind: "update_window_geometry",
        id: await issueWindow.getAttribute("data-id"),
        geometry: { x: 20, y: 60, width: 1480, height: 920 }, cols: 0, rows: 0 });
      await expect(issueWindow).toHaveCSS("width", "1480px");
      await expect(issueWindow).toHaveCSS("height", "920px");
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", {
        detail: { section: "issues" },
      })));
      await expect(issueWindow.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
      const issues = issueWindow.locator('[data-settings-panel="issues"]');
      const backlog = issues.locator('[data-queue-column="backlog"]');
      const queued = issues.locator('[data-queue-column="queued"]');
      const row = backlog.locator(`[data-issue-number="${fixture.issueNumber}"]`);
      await expect(row).toBeVisible({ timeout: 60_000 });
      expect(await fixture.launches(), "Backlog alone must not admit a launch").toEqual([]);
      await row.dragTo(queued.locator("h3"));
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 90_000, message: "real provider process launched by queue admission" }).toBe(1);
      const launch = (await fixture.launches())[0];
      expect(launch.pid).toBeGreaterThan(0);
      expect(() => process.kill(launch.pid, 0), "recorded provider process remains alive").not.toThrow();
      expect(launch.gwt_session_id).toBeTruthy();
      expect(launch.cwd).toContain(fixture.home);
      expect(launch.argv.join(" ")).toContain(`#${fixture.issueNumber}`);
      const agentWindow = async (target = page) => target.evaluate(sessionId => {
        const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
        return (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
          .find((window: any) => window.session_id === sessionId);
      }, launch.gwt_session_id);
      await expect.poll(async () => ["running", "idle"].includes((await agentWindow())?.status),
        { timeout: 60_000 }).toBe(true);
      fixture.auditHooks(launch.cwd);
      expect(fixture.comments.some(comment => comment.body.includes("gwt-auto-improve-claim"))).toBe(true);
      expect(errors, "console/page errors").toEqual([]);
      await info.attach(`queue-launch-${theme}`, { body: await page.screenshot(), contentType: "image/png" });

      // Prove this is a resumable linked-Issue Session before blocking it;
      // missing provider metadata must not make the no-restore check pass.
      await expect.poll(async () => (await readFile(join(fixture.home, ".gwt", "sessions",
        `${launch.gwt_session_id}.toml`), "utf8")).match(/^agent_session_id\s*=\s*"([^"]*)"/m)?.[1],
      { timeout: 60_000, message: "provider SessionStart supplies the exact resume handle" }).toBe(launch.provider_session_id);
      const windowId = (await agentWindow()).id;
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: windowId, data: "GWT_FIXTURE_BLOCK\r" });
      await expect.poll(() => fixture.executionStatus(launch.cwd).ecr_status,
        { timeout: 60_000, message: "provider settles its own execution as Blocked" }).toBe("blocked");
      expect(fixture.executionStatus(launch.cwd).ecr_settled_by_host_reaper ?? false).toBe(false);
      const previousStartup = fixture.startupReport().startup_id;
      const previousPid = fixture.ownedPid;
      // Closing the old page stops reconnects without running the theme
      // initialization script on an opaque about:blank origin.
      const context = page.context();
      await page.close();
      await fixture.restart();
      expect(fixture.ownedPid).not.toBe(previousPid);
      const restored = await context.newPage();
      restored.on("pageerror", error => errors.push(error.message));
      restored.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await restored.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(restored, fixture.url, { enableTestBridge: true });
      await expect(restored.locator("html")).toHaveAttribute("data-theme", theme);
      await expect.poll(() => {
        const startup = fixture.startupReport();
        return startup?.startup_id !== previousStartup
          && startup?.phases.some((phase: any) => phase.phase === "restore_drain");
      }, { timeout: 60_000, message: "new GUI completes its restore drain" }).toBe(true);
      // The summary waits for asynchronous restore outcomes, unlike the
      // drain timer. It closes the observation window without a fixed sleep.
      await expect.poll(() => fixture.restoreSummary(fixture.startupReport().process_started_at),
        { timeout: 60_000, message: "new GUI completes every startup restore outcome" }).toBeTruthy();
      const restoreSummary = await fixture.restoreSummary(fixture.startupReport().process_started_at);
      expect(restoreSummary.restored).toBe(0);
      expect(restoreSummary.reasons).toContain("execution_blocked");
      await expect.poll(async () => (await agentWindow(restored))?.status,
        { timeout: 60_000, message: "Blocked Session keeps its stopped diagnostic window" }).toBe("stopped");
      // The current Agents surface reparents terminals from hidden legacy
      // canvas nodes. Open the real surface to inspect the retained diagnostic.
      await expect(restored.locator(`.workspace-window[data-id="${windowId}"]`)).toHaveCount(1);
      await restored.locator('.op-rail__surface[data-surface="agents"]').click();
      await expect(restored.locator(`.agents-surface:not(.agents-surface--preview) .agent-tile[data-agent-id="${windowId}"]`)).toBeVisible();
      expect(await fixture.launches(), "Blocked generation must not launch another provider after restore completes").toHaveLength(1);
      expect(fixture.executionStatus(launch.cwd).ecr_status).toBe("blocked");
      expect(errors, "console/page errors including app restart").toEqual([]);
      await info.attach(`blocked-after-restart-${theme}`, { body: await restored.screenshot(), contentType: "image/png" });
      await info.attach("blocked-restore-completion", { body: JSON.stringify({
        startup: fixture.startupReport(), restore_summary: restoreSummary, execution: fixture.executionStatus(launch.cwd),
        diagnostic_window: await agentWindow(restored), provider_launch_count: (await fixture.launches()).length,
      }, null, 2), contentType: "application/json" });
    } finally {
      try {
        await info.attach("actual-provider-handoff", { body: JSON.stringify({
          launches: await fixture.launches(), requests: fixture.requests, comments: fixture.comments, hooks: fixture.hookAudits,
          scope: "Real backend claim, materialization, authenticated Blocked settlement and app restore; provider execution and GitHub are local recorders.",
        }, null, 2), contentType: "application/json" });
      } finally {
        await fixture.stop();
      }
    }
  });
});
