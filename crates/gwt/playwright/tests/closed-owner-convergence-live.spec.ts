import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { expect, test, type Page } from "@playwright/test";
import { startIssueQueueLaunchFixture } from "./_helpers/issue-queue-launch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.use({ headless: false, viewport: { width: 1600, height: 1100 } });

test.describe("Issue #3590 closed owner terminal convergence", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_CLOSED_OWNER !== "1",
    "requires checkout binaries and isolated provider/loopback GitHub fixtures");
  test.setTimeout(240_000);

  test("closes only the idle completed owner with a current bookkeeping warning", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, {
      targetIssueNumber: 44, terminalConvergence: true,
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    const request = async (payload: unknown, kind: string) => {
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(page, payload);
      return page.waitForFunction(({ after, kind }) =>
        ((window as any).__gwtPlaywrightMessages ?? []).findLast((entry: any) =>
          entry.sequence > after && entry.payload.kind === kind)?.payload,
      { after, kind }, { timeout: 60_000 }).then(handle => handle.jsonValue());
    };
    const windows = (target: Page = page) => target.evaluate(() => {
      const latest = ((window as any).__gwtPlaywrightMessages ?? [])
        .findLast((entry: any) => entry.payload.kind === "workspace_state");
      return (latest?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows);
    });
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 2 });
      await expect.poll(async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString())
        .max_active_agents).toBe(2);
      // The legacy profile initially selects only the first roster member.
      // Select both fixed-tool members through the normal configuration path.
      const roster = (await request({ kind: "agent_roster", action: { kind: "read" } }, "agent_roster")).roster;
      const preferences = JSON.parse((await fixture.monitorPreferencesBytes()).toString());
      const profiles = roster.members.filter((member: any) => member.enabled).map((member: any) => ({
        ...preferences.launch_profiles[0], agent_id: member.agent_id, agent_member_id: member.agent_member_id,
      }));
      expect(profiles).toHaveLength(2);
      const selected = await request({ kind: "issue_monitor_profiles_set", request_id: 359050, profiles },
        "issue_monitor_profiles");
      expect(selected.profiles).toEqual(profiles);
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [43] }, "issue_monitor_queue_result");
      // Confirm the first ordinary admission before requesting its companion;
      // both must be alive before this case closes only one of them.
      await expect.poll(async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString())
        .launch_confirmations?.["43"]?.window_id,
      { timeout: 90_000, message: "the first actual native launch is confirmed" }).toBeTruthy();
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [44] }, "issue_monitor_queue_result");
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 90_000, message: "both real provider children materialize" }).toBe(2);
      const launches = await fixture.launches();
      const owners = await fixture.launchIssues();
      const source = launches[owners.indexOf(43)];
      const companion = launches[owners.indexOf(44)];
      expect(source?.gwt_session_id).toBeTruthy();
      expect(companion?.gwt_session_id).toBeTruthy();
      await expect.poll(async () => (await windows()).filter((pane: any) =>
        [source.gwt_session_id, companion.gwt_session_id].includes(pane.session_id)).length).toBe(2);
      const sourceId = (await windows()).find((pane: any) => pane.session_id === source.gwt_session_id).id;
      const companionId = (await windows()).find((pane: any) => pane.session_id === companion.gwt_session_id).id;
      // The scheduled Monitor close path must not make this pass. Its runtime
      // convergence observer remains active while automatic admission is off.
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_enabled", enabled: false });
      await expect.poll(async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString()).enabled).toBe(false);
      let before: any;
      await expect.poll(async () => {
        before = await request({ kind: "list_issue_monitor" }, "issue_monitor_status");
        return before.status.active_count;
      }, { timeout: 60_000, message: "the fresh physical census includes both provider children" }).toBe(2);
      expect(fixture.terminalExecutionStatus(companion.cwd)).toMatchObject({ ecr_status: "active", owner_number: 44 });

      const actorEvidence = async (name: string) => {
        const until = Date.now() + 90_000;
        while (Date.now() < until) {
          const failure = await readFile(join(fixture.home,
            "terminal-warning-failed-" + source.gwt_session_id + ".json"), "utf8").catch(() => null);
          if (failure) throw new Error("Terminal provider fixture failed: " + JSON.parse(failure).error);
          const contents = await readFile(join(fixture.home,
            name + source.gwt_session_id + ".json"), "utf8").catch(() => null);
          if (contents) return JSON.parse(contents);
          await delay(250);
        }
        throw new Error("Terminal provider fixture did not produce " + name + " evidence");
      };
      // The provider holds only its own execution authority. Closing the
      // loopback Issue belongs to this fixture operator's GitHub environment.
      await sendLiveGwtEvent(page, {
        kind: "terminal_input", id: sourceId, data: "GWT_FIXTURE_PREPARE_TERMINAL_WARNING\r",
      });
      await actorEvidence("terminal-warning-prepared-");
      const closed = await fixture.closeIssue();
      expect(closed.ok, JSON.stringify(closed)).toBe(true);
      await sendLiveGwtEvent(page, {
        kind: "terminal_input", id: sourceId, data: "GWT_FIXTURE_TERMINAL_WARNING\r",
      });
      const evidence = await actorEvidence("terminal-warning-");
      expect(evidence.diagnosis).toMatchObject({
        ecr_status: "completed", binding_state: "terminal", owner_number: 43,
        work_event_receipt_matches_current_generation: true,
        settlement_severity: "warning", settlement_obligation_open: true, open_obligations: [],
      });
      expect(Object.keys(evidence.hashes).some(path => /[/\\]events[/\\].*\.jsonl$/.test(path))).toBe(true);
      await expect.poll(async () => (await windows()).some((pane: any) => pane.id === sourceId),
        { timeout: 90_000, message: "the real terminal observer removes the completed idle pane" }).toBe(false);
      await expect(page.locator('.workspace-window[data-id="' + sourceId + '"]')).toHaveCount(0);
      expect((await windows()).some((pane: any) => pane.id === companionId)).toBe(true);
      expect(() => process.kill(companion.pid, 0)).not.toThrow();
      expect(fixture.terminalExecutionStatus(companion.cwd)).toMatchObject({ ecr_status: "active", owner_number: 44 });
      let after: any;
      await expect.poll(async () => {
        after = await request({ kind: "list_issue_monitor" }, "issue_monitor_status");
        return after.status.active_count;
      }, { timeout: 60_000, message: "the fresh census releases only the completed provider" }).toBe(1);
      // Pane removal precedes its detached process proof and durable finalizer.
      await expect.poll(async () => {
        const session = await readFile(join(fixture.home, ".gwt/sessions", source.gwt_session_id + ".toml"), "utf8");
        return {
          stopped: /^status\s*=\s*"Stopped"/m.test(session),
          restoreDisabled: /^restore_window_on_startup\s*=\s*false/m.test(session),
        };
      }, { timeout: 30_000, message: "the exact completed Session is stopped and cannot restore a window" })
        .toEqual({ stopped: true, restoreDisabled: true });
      for (const [path, digest] of Object.entries(evidence.hashes)) {
        expect(createHash("sha256").update(await readFile(path)).digest("hex"), path).toBe(digest);
      }
      const remaining = fixture.terminalExecutionStatus(source.cwd);
      expect(remaining).toMatchObject({
        ecr_status: "completed", settlement_severity: "warning", settlement_obligation_open: true,
        work_event_receipt_matches_current_generation: true,
      });
      // Works membership arrives separately from the window catalog and Monitor
      // census. Prove its real activity projection converged before capture.
      const currentWorks = page.getByRole("list", { name: "Current agent works" });
      await expect(currentWorks.locator(".work-row")).toHaveCount(1, { timeout: 30_000 });
      await expect(currentWorks.locator('.work-row[data-window-id="' + sourceId + '"]')).toHaveCount(0);
      await expect(currentWorks.locator('.work-row[data-window-id="' + companionId + '"]')).toBeVisible();
      await expect(page.locator(".works-active")).toHaveText("Active 1");
      await info.attach("closed-owner-convergence-" + theme, {
        body: await page.screenshot(), contentType: "image/png",
      });
      await info.attach("canonical-convergence-evidence-" + theme, {
        body: JSON.stringify({ before: before.status.active_count, after: after.status.active_count,
          sourceSession: source.gwt_session_id, companionSession: companion.gwt_session_id,
          diagnosis: evidence.diagnosis, preservedHashes: evidence.hashes, errors }, null, 2),
        contentType: "application/json",
      });
      expect(errors, "console/page errors across the actual observer close").toEqual([]);
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "console/page errors including owned fixture cleanup").toEqual([]);
  });
});
