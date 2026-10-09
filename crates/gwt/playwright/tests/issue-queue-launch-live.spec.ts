import { expect, test } from "@playwright/test";
import { startIssueQueueLaunchFixture } from "./_helpers/issue-queue-launch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.describe("Issue queue real backend launch", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_QUEUE_LAUNCH !== "1", "requires checkout binaries and isolated provider fixture");
  test.skip(process.platform === "win32", "fixture requires POSIX executables");
  test.setTimeout(240_000);
  test.use({ viewport: { width: 1600, height: 1100 } });

  test("Backlog drag launches one real PTY provider process", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue",
        bounds: { x: 20, y: 60, width: 1480, height: 920 } });
      // create_window bounds position a preset-sized window; set the intended
      // fixture geometry so the drag source and target are both in view.
      const issueWindow = page.locator('.workspace-window[data-preset="issue"]');
      await expect(issueWindow).toBeVisible();
      await sendLiveGwtEvent(page, { kind: "update_window_geometry",
        id: await issueWindow.getAttribute("data-id"),
        geometry: { x: 20, y: 60, width: 1480, height: 920 }, cols: 0, rows: 0 });
      await expect(issueWindow).toHaveCSS("width", "1480px");
      await expect(issueWindow).toHaveCSS("height", "920px");
      const backlog = page.locator('[data-queue-column="backlog"]');
      const queued = page.locator('[data-queue-column="queued"]');
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
      await expect.poll(async () => page.evaluate(sessionId => {
        const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
        return (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
          .some((window: any) => window.session_id === sessionId && ["running", "idle"].includes(window.status));
      }, launch.gwt_session_id), { timeout: 60_000 }).toBe(true);
      fixture.auditHooks(launch.cwd);
      expect(fixture.comments.some(comment => comment.body.includes("gwt-auto-improve-claim"))).toBe(true);
      expect(errors, "console/page errors").toEqual([]);
      await info.attach(`queue-launch-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      await info.attach("actual-provider-handoff", { body: JSON.stringify({
        launches: await fixture.launches(), requests: fixture.requests, comments: fixture.comments, hooks: fixture.hookAudits,
        scope: "Real backend claim, materialization and PTY provider handoff; provider execution is a local recorder.",
      }, null, 2), contentType: "application/json" });
      await fixture.stop();
    }
  });
});
