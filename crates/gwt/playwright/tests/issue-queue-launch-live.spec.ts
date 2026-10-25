import { expect, test } from "@playwright/test";
import { readFile, realpath } from "node:fs/promises";
import { join } from "node:path";
import { startIssueQueueLaunchFixture } from "./_helpers/issue-queue-launch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.describe("Issue queue real backend launch", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_QUEUE_LAUNCH !== "1", "requires checkout binaries and isolated provider fixture");
  test.skip(process.platform === "win32", "fixture requires POSIX executables");
  test.setTimeout(240_000);
  test.use({ viewport: { width: 1600, height: 1100 } });

  test("AC34 actual provider exit removes the local Active agent without closing its Issue", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "settings" });
      const issueWindow = page.locator('.workspace-window[data-preset="settings"]');
      await expect(issueWindow).toBeVisible();
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "issues" } })));
      await expect(issueWindow.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
      const issues = issueWindow.locator('[data-settings-panel="issues"]');
      const metric = issues.locator('[data-metric="active"]');
      await expect(metric).toHaveText("Active 0 · 0 issues · PM 0", { timeout: 60_000 });
      const row = issues.locator(`[data-issue-number="${fixture.issueNumber}"]`);
      await expect(row).toBeVisible({ timeout: 60_000 });
      await row.dragTo(issues.locator('[data-queue-column="queued"] h3'));
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const launch = (await fixture.launches())[0];
      const activity = () => page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter((item: any) => item.payload.projection?.local_agent_activity)
        .at(-1)?.payload.projection.local_agent_activity);
      await expect(metric).toHaveText("Active 1 · 1 issue · PM 0", { timeout: 60_000 });
      const live = await activity();
      const agent = live.agents.find((agent: any) => agent.session_id === launch.gwt_session_id);
      expect(agent).toMatchObject({ issue_number: fixture.issueNumber, is_pm: false });
      expect(agent.runtime_incarnation).toBeGreaterThan(0);
      expect(() => process.kill(launch.pid, 0)).not.toThrow();
      await info.attach(`local-live-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      await page.locator('.op-rail__surface[data-surface="works"]').click();
      if (await page.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await page.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
      const main = page.getByRole("region", { name: "Main pane", exact: true });
      await expect(main).toHaveAttribute("data-surface", "works");
      const workRow = main.locator(`.work-row[data-window-id="${agent.window_id}"]`);
      await expect(workRow.locator(".terminal-text-preview")).toBeVisible();
      await workRow.getByRole("button", { name: "Open terminal", exact: true }).click();
      await expect(main.locator(`.works-full .works-terminal[data-window-id="${agent.window_id}"] .xterm`)).toBeVisible();
      const currentSurface = info.outputPath(`current-agent-surface-${theme}.png`);
      await page.screenshot({ path: currentSurface });
      await info.attach(`current-agent-surface-${theme}`, { path: currentSurface, contentType: "image/png" });
      await page.locator('.op-rail__surface[data-surface="issues"]').click();
      await expect(issueWindow).toBeVisible();
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: agent.window_id, data: "GWT_FIXTURE_EXIT\r" });
      await expect.poll(() => {
        try { process.kill(launch.pid, 0); return false; } catch { return true; }
      }, { timeout: 60_000, message: "the exact provider child exited and was reaped" }).toBe(true);
      await expect(metric).toHaveText("Active 0 · 0 issues · PM 0", { timeout: 60_000 });
      expect((await activity()).agents).toEqual([]);
      await expect(issues.locator('[data-queue-column="active"] .knowledge-row')).toHaveCount(0);
      await expect(row).toBeVisible();
      await expect(row.locator('[data-action="focus-surface-window"]')).toHaveCount(0);
      await expect(row).not.toContainText("Shown in Agents");
      expect(errors, "zero console/page errors across the real child exit").toEqual([]);
      await info.attach("local-live-exit", { body: JSON.stringify({ before: live, after: await activity(), launch }, null, 2), contentType: "application/json" });
    } finally {
      try {
        const observations = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
          .filter(({ payload }: any) => payload.projection || payload.kind === "terminal_status")
          .map(({ sequence, payload }: any) => ({ sequence, kind: payload.kind,
            activity: payload.projection?.local_agent_activity, id: payload.id, status: payload.status })));
        await info.attach("local-live-wire", { body: JSON.stringify({ observations, errors }, null, 2), contentType: "application/json" });
      } finally {
        // Close the client before its server so cleanup cannot trigger reconnect errors.
        try { await page.close(); } finally { await fixture.stop(); }
      }
    }
    expect(errors, "zero console/page errors including fixture cleanup").toEqual([]);
  });

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
      const issueWindow = page.locator('.workspace-window[data-preset="settings"]');
      await expect(issueWindow).toBeVisible();
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
      // The stopped diagnostic remains in the catalog; Works only shows live census members.
      await expect(restored.locator(`.workspace-window[data-id="${windowId}"]`)).toHaveCount(1);
      await restored.locator('.op-rail__surface[data-surface="works"]').click();
      if (await restored.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await restored.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
      await expect(restored.locator(`.work-row[data-window-id="${windowId}"]`)).toHaveCount(0);
      await expect(restored.locator(".works-empty")).toHaveText("No local agents are active.");
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

  test("authenticated fresh launch repairs an Active orphan and retains staged work", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info);
    const errors: string[] = [];
    const capture = (target: typeof page) => {
      target.on("pageerror", error => errors.push(error.message));
      target.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    };
    capture(page);
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const agentWindow = async (target: typeof page, sessionId: string) => target.evaluate(sessionId => {
      const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
      return (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
        .find((window: any) => window.session_id === sessionId);
    }, sessionId);
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] });
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const first = (await fixture.launches())[0];
      await expect.poll(() => fixture.executionStatus(first.cwd),
        { timeout: 60_000, message: "first Session holds the activated generation" })
        .toMatchObject({ ecr_status: "active", holder_session_id: first.gwt_session_id });
      await expect.poll(async () => (await fixture.workItems()).work_items.some((work: any) =>
        work.events.some((event: any) => event.kind === "start" && event.agent_session_id === first.gwt_session_id)),
      { timeout: 60_000, message: "first Session has an accepted Work Start" }).toBe(true);
      fixture.auditHooks(first.cwd);
      const firstWindow = await agentWindow(page, first.gwt_session_id!);
      // Both mutations run inside the provider with its own Host capability.
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: firstWindow.id, data: "GWT_FIXTURE_DONE\r" });
      await expect.poll(async () => (await fixture.workItems()).work_items.find((work: any) =>
        work.events.some((event: any) => event.kind === "start" && event.agent_session_id === first.gwt_session_id))?.status_category,
      { timeout: 60_000, message: "authenticated Session-bound Done commits" }).toBe("done");
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: firstWindow.id, data: "GWT_FIXTURE_BLOCK\r" });
      await expect.poll(() => fixture.executionStatus(first.cwd).ecr_status, { timeout: 60_000 }).toBe("blocked");
      const blocked = fixture.executionStatus(first.cwd);
      expect(blocked.ecr_settled_by_host_reaper ?? false).toBe(false);
      expect(blocked.generation_id).toBeTruthy();
      const previousStartup = fixture.startupReport().startup_id;
      const context = page.context();
      await page.close();
      const seed = await fixture.seedActiveOrphan(first.cwd, first.gwt_session_id!);
      expect(seed.stagedDiff.length).toBeGreaterThan(0);
      await info.attach("active-orphan-seed", { body: JSON.stringify({
        predecessor_id: seed.predecessorId, successor_id: seed.successorId, orphan: seed.orphan,
        staged_diff: seed.stagedDiff.toString(), scope: "Isolated historical Work event seed; no Session/ECR authority is fabricated.",
      }, null, 2), contentType: "application/json" });
      await fixture.restart();
      const restored = await context.newPage();
      capture(restored);
      await restored.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(restored, fixture.url, { enableTestBridge: true });
      await expect(restored.locator("html")).toHaveAttribute("data-theme", theme);
      await expect.poll(() => fixture.startupReport().startup_id !== previousStartup
        && fixture.startupReport().phases.some((phase: any) => phase.phase === "restore_drain"),
      { timeout: 60_000 }).toBe(true);
      await expect.poll(() => fixture.restoreSummary(fixture.startupReport().process_started_at),
        { timeout: 60_000 }).toBeTruthy();
      expect(await fixture.launches(), "Blocked predecessor stays stopped before explicit fresh launch").toHaveLength(1);
      await expect.poll(async () => (await fixture.workItems()).work_items.find((work: any) =>
        work.id === seed.successorId)?.status_category,
      { timeout: 60_000, message: "cold intake materializes the Active orphan" }).toBe("active");
      const cold = (await fixture.workItems()).work_items;
      expect(cold.find((work: any) => work.id === seed.predecessorId)?.status_category,
        "authenticated Done survives cold shared-source intake").toBe("done");
      expect(cold.find((work: any) => work.id === seed.successorId)?.related_work_item_ids).toEqual([]);

      // The restored orphan must have a guarded close action even though its
      // Issue-linked branch is intentionally absent from Other.
      await restored.locator('[data-surface="issues"]').click();
      await restored.locator(`.knowledge-row[data-issue-number="${fixture.issueNumber}"]`).click();
      const orphanCard = restored.locator(`.knowledge-related-work[data-work-id="${seed.successorId}"]`);
      await expect(orphanCard).toBeVisible();
      await expect(orphanCard.locator('[data-action="close-work-done"]')).toBeEnabled();
      await expect(orphanCard.locator('[data-action="close-work-done"]')).toHaveAttribute("data-work-id", seed.successorId);

      // Use the normal manual launch coordinator, rather than restarting the
      // terminal generation or installing a binding from the fixture.
      const wizardEvent = async (event: unknown, requireOpen = false) => {
        const after = await restored.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
        await sendLiveGwtEvent(restored, event);
        return (await restored.waitForFunction(({ after, requireOpen }) => {
          const entry = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
            entry.sequence > after && entry.payload.kind === "launch_wizard_state"
            && (!requireOpen || entry.payload.wizard)
            && (!entry.payload.wizard || (!entry.payload.wizard.is_hydrating
              && !entry.payload.wizard.runtime_resolution_pending && !entry.payload.wizard.launch_materialization_pending)));
          return entry ? { wizard: entry.payload.wizard } : null;
        }, { after, requireOpen }, { timeout: 30_000 })).jsonValue();
      };
      await wizardEvent({ kind: "open_active_work_launch_wizard", branch_name: seed.branch,
        linked_issue_number: fixture.issueNumber }, true);
      const action = (action: unknown) => wizardEvent({ kind: "launch_wizard_action", action,
        bounds: { x: 80, y: 80, width: 700, height: 440 } });
      await action({ kind: "set_launch_path", path: "manual_setup" });
      let state = await action({ kind: "set_agent", agent_id: "codex" });
      for (let step = 0; step < 12 && state.wizard; step += 1) {
        expect(state.wizard.error).toBeFalsy();
        if (state.wizard.selected_runtime_target !== "host"
          && state.wizard.runtime_target_options?.some((option: any) => option.value === "host")) {
          state = await action({ kind: "set_runtime_target", target: "Host" });
        } else {
          expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
          state = await action({ kind: "submit" });
        }
      }
      expect(state.wizard).toBeNull();
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(2);
      const fresh = (await fixture.launches())[1];
      expect(fresh.gwt_session_id).toBeTruthy();
      expect(fresh.gwt_session_id).not.toBe(first.gwt_session_id);
      expect(fresh.resume_session_id).toBeNull();
      expect(await realpath(fresh.cwd)).toBe(seed.worktree);
      await expect.poll(() => fixture.executionStatus(seed.worktree).ecr_status,
        { timeout: 60_000, message: "fresh generation activates after provider readiness" }).toBe("active");
      const execution = fixture.executionStatus(seed.worktree);
      expect(execution.holder_session_id).toBe(fresh.gwt_session_id);
      expect(execution.generation_id).not.toBe(blocked.generation_id);
      // Current recovery audit-closes malformed occupancy and starts its
      // successor, preserving the orphan's historical Start unchanged.
      const freshWork = async () => (await fixture.workItems()).work_items.find((work: any) =>
        work.events.some((event: any) => event.kind === "start"
          && event.agent_session_id === fresh.gwt_session_id && event.related_work_item_id === seed.successorId));
      await expect.poll(async () => Boolean(await freshWork()),
        { timeout: 60_000, message: "fresh launch commits an audited successor lineage" }).toBe(true);
      const successor = await freshWork();
      const retired = (await fixture.workItems()).work_items.find((work: any) => work.id === seed.successorId);
      expect(retired.discarded).toBe(true);
      // Canonical branch-bound audits carry portable content identities.
      expect(retired.events.some((event: any) => new RegExp(`^superseded-portable-${seed.successorId}-[0-9a-f]{64}$`).test(event.id)
        && event.kind === "discard" && event.related_work_item_id === seed.predecessorId)).toBe(true);
      expect(retired.events).toContainEqual(seed.orphan);
      expect(successor.id).not.toBe(seed.successorId);
      const start = successor.events.find((event: any) => event.kind === "start" && event.agent_session_id === fresh.gwt_session_id);
      expect(JSON.parse(await readFile(fixture.eventShardPath(seed.worktree, start.id), "utf8"))).toEqual({
        ...start, execution_container: { ...start.execution_container, worktree_path: "." },
      });
      expect(start.owner).toBe(seed.orphan.owner);
      expect(start.execution_container.branch).toBe(seed.branch);
      expect(await realpath(start.execution_container.worktree_path)).toBe(seed.worktree);
      expect(successor.owner).toBe(`Issue #${fixture.issueNumber}`);
      expect(successor.related_work_item_ids).toContain(seed.successorId);
      expect(fixture.stagedDiff(seed.worktree), "fresh launch preserves the complete staged diff").toEqual(seed.stagedDiff);
      await expect.poll(async () => ["running", "idle"].includes((await agentWindow(restored, fresh.gwt_session_id!))?.status),
        { timeout: 60_000 }).toBe(true);
      const freshWindow = await agentWindow(restored, fresh.gwt_session_id!);
      await restored.locator('.op-rail__surface[data-surface="works"]').click();
      if (await restored.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await restored.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
      const tile = restored.locator(`.work-row[data-window-id="${freshWindow.id}"]`);
      await expect(tile).toBeVisible();
      await tile.getByRole("button", { name: "Open terminal", exact: true }).click();
      await expect(restored.locator(".works-full-meta")).toContainText("Interactive");
      await expect(restored.locator(`.works-full .works-terminal[data-window-id="${freshWindow.id}"] .xterm-helper-textarea`)).toBeVisible();
      expect(errors, "console/page errors including cold restart and fresh launch").toEqual([]);
      await info.attach(`active-orphan-recovered-${theme}`, { body: await restored.screenshot(), contentType: "image/png" });
      await info.attach("active-orphan-recovery", { body: JSON.stringify({ execution, start,
        predecessor_id: seed.predecessorId, successor_id: successor.id, staged_diff_unchanged: true,
      }, null, 2), contentType: "application/json" });
    } finally {
      try {
        await info.attach("active-orphan-provider-handoff", { body: JSON.stringify({
          launches: await fixture.launches(), requests: fixture.requests, comments: fixture.comments, hooks: fixture.hookAudits,
          scope: "Isolated authenticated launch and staged-work regression; not recovery evidence for the real Issue #3824.",
        }, null, 2), contentType: "application/json" });
      } finally {
        await fixture.stop();
      }
    }
  });
});
