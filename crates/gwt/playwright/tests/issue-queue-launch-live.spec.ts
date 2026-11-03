import { expect, test } from "@playwright/test";
import { mkdir, readFile, realpath } from "node:fs/promises";
import { join } from "node:path";
import { startIssueQueueLaunchFixture } from "./_helpers/issue-queue-launch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.describe("Issue queue real backend launch", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_QUEUE_LAUNCH !== "1", "requires checkout binaries and isolated provider fixture");
  test.setTimeout(240_000);
  test.use({ viewport: { width: 1600, height: 1100 } });

  test("roster Manual same-value save updates machine policy before autonomous queue launch", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, { autonomousMode: true });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const prefs = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const machine = async () => JSON.parse(await readFile(join(fixture.home, ".gwt/machine-state/agent-settings.json"), "utf8"));
    const operation = (name: "issue.monitor.config.set" | "issue.monitor.status" | "issue.monitor.schedule.read",
      params: Record<string, unknown> = {}, root = fixture.project) => {
      const envelope = fixture.fixtureOperation(name, params, root);
      expect(envelope.ok, JSON.stringify(envelope)).toBe(true);
      return { envelope, output: JSON.parse(envelope.output) };
    };
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      operation("issue.monitor.config.set", { max_active: 10 });
      operation("issue.monitor.schedule.read");
      await expect.poll(async () => (await prefs()).agent_roster?.members.filter((member: any) => member.enabled).length).toBe(10);
      const roster = (await prefs()).agent_roster;
      // A separate, unregistered fixture scope uses the normal local ConfigSet
      // path to establish an older machine limit without editing trusted state.
      const seed = join(fixture.home, "machine-policy-seed");
      await mkdir(seed);
      operation("issue.monitor.config.set", { max_active: 4 }, seed);
      const olderMachine = await machine();
      expect(olderMachine).toMatchObject({ mode: "manual", manual_limit: 4 });
      expect(await prefs()).toMatchObject({ max_active_agents: 10, max_active_agents_mode: "manual", agent_roster: roster });
      const before = operation("issue.monitor.status");
      expect(before.envelope.project_store.identity_resolved).toBe(true);
      expect(before.output.autonomous_mode).toBe(true);
      operation("issue.monitor.config.set", { auto_close_merged_issues: false });
      expect(await machine(), "reads and unrelated settings preserve the newer machine policy").toEqual(olderMachine);
      operation("issue.monitor.config.set", { max_active: 10, expected_config_token: before.output.config_token });
      // This is the regression assertion: a roster and unchanged project value
      // must not suppress the explicit write to the shared machine authority.
      expect(await machine()).toMatchObject({ mode: "manual", manual_limit: 10,
        last_change: { source: "json_operation", max_active_agents_override: 10 } });
      expect((await prefs()).agent_roster).toEqual(roster);
      const saved = operation("issue.monitor.status").output;
      const capacity = saved.agent_capacity;
      const available = Math.max(0, 10 - Math.max(0, capacity.other_live_agents - capacity.other_pm_agents)
        - capacity.own_untracked_workers);
      expect(saved.max_active).toBe(Math.min(10, available));
      expect(saved.max_active).toBeGreaterThan(0);
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] });
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 90_000, message: "normal autonomous claim, materialization and provider launch" }).toBe(1);
      const launch = (await fixture.launches())[0];
      expect(launch.pid).toBeGreaterThan(0);
      expect(() => process.kill(launch.pid, 0)).not.toThrow();
      expect(launch.gwt_session_id).toBeTruthy();
      const window = () => page.evaluate(sessionId => {
        const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
        return (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
          .find((window: any) => window.session_id === sessionId);
      }, launch.gwt_session_id);
      await expect.poll(async () => ["running", "idle"].includes((await window())?.status), { timeout: 60_000 }).toBe(true);
      const actualWindow = await window();
      await expect.poll(async () => (await prefs()).launch_confirmations?.[fixture.issueNumber]?.window_id,
        { timeout: 60_000 }).toBe(actualWindow.id);
      const confirmed = (await prefs()).launch_confirmations[fixture.issueNumber];
      expect(confirmed.claim_id).toBeTruthy();
      expect(confirmed.delivery_id).toBeTruthy();
      expect((await prefs()).launch_bindings[actualWindow.id]).toBe(fixture.issueNumber);
      expect(fixture.comments.some(comment => comment.body.includes("gwt-auto-improve-claim")
        && comment.body.includes(confirmed.claim_id))).toBe(true);
      const record = () => readFile(join(fixture.home, ".gwt/sessions", `${launch.gwt_session_id}.toml`), "utf8");
      await expect.poll(async () => (await record()).match(/^agent_session_id\s*=\s*"([^"]*)"/m)?.[1],
        { timeout: 60_000 }).toBe(launch.provider_session_id);
      expect(await record()).toContain(`linked_issue_number = ${fixture.issueNumber}`);
      const running = operation("issue.monitor.status").output;
      expect(running.active_launch_details).toContainEqual(expect.objectContaining({ issue_number: fixture.issueNumber,
        session_id: launch.gwt_session_id, window_id: actualWindow.id }));
      expect(running.occupied_slot_count).toBeLessThanOrEqual(running.max_active);
      await info.attach("machine-manual-native-launch", { body: JSON.stringify({ before: before.output, saved, running,
        machine: await machine(), launch, confirmation: confirmed,
        scope: "Real native ConfigSet, machine authority and autonomous claim/Session/window/provider handoff; actual inventory is recorded. Exact other-workers=4 arithmetic is covered by Rust acceptance." }, null, 2), contentType: "application/json" });
      await info.attach(`machine-manual-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      expect(errors).toEqual([]);
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned process cleanup").toEqual([]);
  });

  test("T20b pending switch preserves current and cancels without replay after restart", async ({ page }, info) => {
    test.skip(process.platform === "win32", "switch actor fixture requires POSIX executables");
    const fixture = await startIssueQueueLaunchFixture(info, { targetIssueNumber: 44, switching: true });
    const errors: string[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    let current = page;
    const capture = (target: typeof page) => {
      target.on("pageerror", error => errors.push(error.message));
      target.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    };
    const request = async (event: unknown, kind: string): Promise<any> => {
      const after = await current.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(current, event);
      return (await current.waitForFunction(({ after, kind }) => {
        const found = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.sequence > after && [kind, `${kind}_error`].includes(entry.payload.kind));
        return found?.payload || null;
      }, { after, kind }, { timeout: 30_000 })).jsonValue();
    };
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const intent = async (operation: string) => (await preferences()).agent_switch_intents?.[operation];
    const openPlans = async (memberId: string) => {
      await current.locator('.op-rail__surface[data-surface="agents"]').click();
      const agents = current.getByRole("region", { name: "Main pane", exact: true })
        .locator(".agents-surface:not(.agents-surface--preview)");
      await agents.locator(`.roster-row[data-member-id="${memberId}"]`).click();
      return agents.locator(".roster-plans");
    };
    capture(current);
    try {
      expect(fixture.targetIssueNumber).toBe(44);
      await current.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(current, fixture.url, { enableTestBridge: true });
      await expect(current.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(current, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 });
      await expect.poll(async () => (await preferences()).max_active_agents_mode, { timeout: 30_000 }).toBe("manual");
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] }, "issue_monitor_queue_result");
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const source = (await fixture.launches())[0];
      await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
        entry.stage === "idle" && entry.gwt_session_id === source.gwt_session_id), { timeout: 60_000 }).toBe(true);
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.targetIssueNumber] }, "issue_monitor_queue_result");
      const roster = (await request({ kind: "agent_roster", action: { kind: "read" } }, "agent_roster")).roster;
      const member = roster.members.find((member: any) => member.assignment?.session_id === source.gwt_session_id);
      expect(member).toBeTruthy();
      const sourceAssignment = member.assignment;
      let plans = await openPlans(member.agent_member_id);
      await plans.getByRole("button", { name: "Refresh plans", exact: true }).click();
      const change = plans.locator('[data-action="switch-next"]');
      await expect(change).toBeEnabled();
      await change.click();
      await expect.poll(async () => Object.keys((await preferences()).agent_switch_intents || {}).length).toBe(1);
      const operation = Object.keys((await preferences()).agent_switch_intents)[0];
      await expect.poll(async () => (await intent(operation))?.reason, { timeout: 45_000 }).toMatch(/Waiting.*(?:background|complete activity)/);
      const saved = await intent(operation);
      expect(saved.stage).toBe("requested");
      const duplicate = await request({ kind: "agent_switch", action: { kind: "request", ...saved.request } }, "agent_switch");
      expect(duplicate.snapshot.intents.filter((value: any) => value.operation_id === operation)).toHaveLength(1);
      expect((await preferences()).agent_roster.members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment).toEqual(sourceAssignment);
      expect((await preferences()).monitor_runtime_stop_requests || []).toEqual([]);
      expect(fixture.processAlive(source.pid)).toBe(true);
      expect(await fixture.launches()).toHaveLength(1);
      await info.attach(`agent-switch-pending-${theme}`, { body: await current.screenshot(), contentType: "image/png" });
      await plans.getByRole("button", { name: "Cancel switch", exact: true }).click();
      await expect.poll(async () => (await intent(operation))?.stage).toBe("cancelled");
      expect(fixture.processAlive(source.pid)).toBe(true);
      expect((await preferences()).agent_roster.members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment).toEqual(sourceAssignment);
      await expect(change).toBeEnabled();
      await change.click();
      await expect.poll(async () => Object.keys((await preferences()).agent_switch_intents).length).toBe(2);
      const pending = Object.keys((await preferences()).agent_switch_intents).find(value => value !== operation)!;
      const unfinishedWork = (await fixture.workItems()).work_items.find((work: any) =>
        work.events.some((event: any) => event.agent_session_id === source.gwt_session_id));
      expect(unfinishedWork).toBeTruthy();
      expect(unfinishedWork.status_category).not.toMatch(/done|blocked/);
      expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
      await current.close();
      await fixture.restart();
      current = await page.context().newPage();
      capture(current);
      await current.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(current, fixture.url, { enableTestBridge: true });
      plans = await openPlans(member.agent_member_id);
      await plans.getByRole("button", { name: "Refresh plans", exact: true }).click();
      await expect.poll(async () => (await intent(pending))?.stage, { timeout: 45_000 }).toBe("cancelled");
      expect((await preferences()).monitor_runtime_stop_requests || []).toEqual([]);
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 45_000 }).toBe(2);
      const restored = (await fixture.launches())[1];
      expect(restored.provider_session_id).toBe(source.provider_session_id);
      expect(restored.resume_session_id).toBe(source.provider_session_id);
      await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
        entry.stage === "idle" && entry.pid === restored.pid && entry.gwt_session_id === restored.gwt_session_id), { timeout: 60_000 }).toBe(true);
      expect(fixture.processAlive(restored.pid)).toBe(true);
      const restoredSession = await readFile(join(fixture.home, ".gwt", "sessions", `${restored.gwt_session_id}.toml`), "utf8");
      expect(restoredSession).toContain(`agent_member_id = "${member.agent_member_id}"`);
      expect(restoredSession).toContain(`agent_name = "${member.name}"`);
      expect((await intent(pending)).source.assignment.session_id).toBe(source.gwt_session_id);
      expect((await fixture.launchIssues()).every((number: number) => number === fixture.issueNumber)).toBe(true);
      expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
      const retainedWork = (await fixture.workItems()).work_items.find((work: any) => work.id === unfinishedWork.id);
      expect(retainedWork).toBeTruthy();
      expect(retainedWork.status_category).not.toMatch(/done|blocked/);
      await expect(plans.locator(".roster-switch-status")).toContainText(/cancelled|Cancelled/);
      await info.attach(`agent-switch-restarted-${theme}`, { body: await current.screenshot(), contentType: "image/png" });
      expect(errors).toEqual([]);
    } finally {
      try {
        await info.attach("agent-switch-actor-events", { body: JSON.stringify(await fixture.actorEvents()), contentType: "application/json" });
      } finally {
        try { await current.close(); } finally { await fixture.stop(); }
      }
    }
    expect(errors).toEqual([]);
  });

  test("T20b saved checkpoint switches through the real Host and Monitor", async ({ page }, info) => {
    test.skip(process.platform !== "linux" || process.env.GWT_PLAYWRIGHT_SWITCH_POSITIVE !== "1",
      "positive proof requires the isolated Linux root process namespace");
    const fixture = await startIssueQueueLaunchFixture(info, {
      targetIssueNumber: 44, switching: true, positive: true, autonomousMode: true,
    });
    const errors: string[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const actor = async (stage: string) => (await fixture.actorEvents()).find((entry: any) => entry.stage === stage);
    const session = async (id: string) => readFile(join(fixture.home, ".gwt", "sessions", `${id}.toml`), "utf8");
    const field = (record: string, key: string) => record.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, "m"))?.[1];
    const request = async (event: unknown, kind: string): Promise<any> => {
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(page, event);
      return (await page.waitForFunction(({ after, kind }) => {
        const found = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.sequence > after && [kind, `${kind}_error`].includes(entry.payload.kind));
        return found?.payload || null;
      }, { after, kind }, { timeout: 30_000 })).jsonValue();
    };
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      expect((await preferences()).autonomous_mode).toBe(true);
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 });
      await expect.poll(async () => (await preferences()).max_active_agents_mode, { timeout: 30_000 }).toBe("manual");
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] }, "issue_monitor_queue_result");
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const source = (await fixture.launches())[0];
      await expect.poll(async () => !!await actor("idle"), { timeout: 60_000 }).toBe(true);
      const idle = await actor("idle");
      expect(idle.owner_number).toBe(fixture.issueNumber);
      expect(idle.cwd, "the source waits in its ordinary worktree before checkpoint delivery").toBe(source.cwd);
      expect(idle.tree_child_pid).toBeGreaterThan(0);
      expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
      expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.targetIssueNumber] }, "issue_monitor_queue_result");
      const roster = (await request({ kind: "agent_roster", action: { kind: "read" } }, "agent_roster")).roster;
      expect(roster.members.filter((member: any) => member.enabled)).toHaveLength(1);
      const member = roster.members.find((member: any) => member.assignment?.session_id === source.gwt_session_id);
      expect(member).toBeTruthy();
      const before = await session(source.gwt_session_id!);
      await page.locator('.op-rail__surface[data-surface="agents"]').click();
      const agents = page.getByRole("region", { name: "Main pane", exact: true })
        .locator(".agents-surface:not(.agents-surface--preview)");
      await agents.locator(`.roster-row[data-member-id="${member.agent_member_id}"]`).click();
      const plans = agents.locator(".roster-plans");
      await plans.getByRole("button", { name: "Refresh plans", exact: true }).click();
      await expect(plans.locator('[data-action="switch-next"]')).toBeEnabled();
      await plans.locator('[data-action="switch-next"]').click();
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 120_000, message: "actual source checkpoint ACK, physical exit and ordinary target Monitor launch" }).toBe(2);
      const launches = await fixture.launches();
      const target = launches[1];
      await expect.poll(async () => Object.values((await preferences()).agent_switch_intents || {})
        .map((intent: any) => intent.stage), { timeout: 30_000 }).toEqual(["completed"]);
      const saved = Object.values((await preferences()).agent_switch_intents)[0] as any;
      expect(saved.runtime_exit_confirmed).toBe(true);
      expect(saved.source.assignment.session_id).toBe(source.gwt_session_id);
      expect(saved.request.member_id).toBe(member.agent_member_id);
      expect(saved.request.target_issue_number).toBe(44);
      const ack = await actor("checkpoint_acked");
      expect(ack).toMatchObject({ operation: saved.request.operation_id, head: saved.checkpoint.verified_head,
        work_id: saved.checkpoint.work_id, board_entry_id: saved.checkpoint.board_entry_id });
      expect(await actor("checkpoint_idle")).toMatchObject({ operation: saved.request.operation_id, cwd: source.cwd });
      expect((await preferences()).autonomous_mode).toBe(true);
      expect(fixture.processAlive(source.pid)).toBe(false);
      expect(fixture.processAlive(idle.tree_child_pid)).toBe(false);
      expect(fixture.processAlive(target.pid)).toBe(true);
      expect(target.gwt_session_id).not.toBe(source.gwt_session_id);
      expect(target.provider_session_id).not.toBe(source.provider_session_id);
      expect(await fixture.launchIssues()).toEqual([43, 44]);
      await expect.poll(async () => field(await session(target.gwt_session_id!), "agent_session_id"),
        { timeout: 45_000 }).toBe(target.provider_session_id);
      const after = await session(target.gwt_session_id!);
      expect(field(after, "agent_member_id")).toBe(member.agent_member_id);
      expect(field(after, "agent_name")).toBe(field(before, "agent_name"));
      expect(field(after, "agent_name")).toBe(member.name);
      expect((await preferences()).agent_roster.members.find((value: any) => value.agent_member_id === member.agent_member_id)
        .assignment.session_id).toBe(target.gwt_session_id);
      expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
      const work = (await fixture.workItems()).work_items.find((value: any) => value.id === ack.work_id);
      expect(work.status_category).toBe("idle");
      expect(work.owner).toBe("Issue #43");
      expect(work.board_refs).toContain(ack.board_entry_id);
      const checkpoint = fixture.savedSource(source.cwd);
      expect(checkpoint.head).toBe(ack.head);
      expect(checkpoint.remoteHead).toBe(ack.head);
      expect(checkpoint.porcelain).toBe("");
      await expect(plans.locator(".roster-switch-status")).toContainText("Switch completed");
      await info.attach(`agent-switch-completed-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      await info.attach("agent-switch-native-proof", { body: JSON.stringify({ source, target, idle,
        checkpoint, operation: ack.operation, work_id: ack.work_id, board_entry_id: ack.board_entry_id }), contentType: "application/json" });
      expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "actor_failed")).toEqual([]);
      expect(errors).toEqual([]);
    } finally {
      try {
        if (!page.isClosed()) {
          const admission = await page.evaluate(() => {
            const status = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
              entry.payload.kind === "issue_monitor_status")?.payload.status;
            return status ? { active_count: status.active_count, max_active_agents: status.max_active_agents,
              occupied_slot_count: status.occupied_slot_count, pending_claim_issues: status.pending_claim_issues,
              last_error: status.last_error, capacity_reason: status.agent_capacity?.reason } : null;
          });
          await info.attach("agent-switch-monitor-admission", { body: JSON.stringify(admission), contentType: "application/json" });
        }
        await info.attach("agent-switch-actor-events", { body: JSON.stringify(await fixture.actorEvents()), contentType: "application/json" });
      } finally {
        try { await page.close(); } finally { await fixture.stop(); }
      }
    }
    expect(errors).toEqual([]);
  });

  test("T20a UI reassigned schedule survives GUI restart and force requeue with live Busy and Works", async ({ page }, info) => {
    // Setup, GUI restart and two bounded launches reached proof collection at
    // 242s on Windows. Budget their aggregate plus cleanup; retain phase limits.
    test.setTimeout(420_000);
    const fixture = await startIssueQueueLaunchFixture(info);
    const errors: string[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const context = page.context();
    let current = page;
    const capture = (target: typeof page) => {
      target.on("pageerror", error => errors.push(error.message));
      target.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    };
    const request = async (event: unknown, kind: string) => {
      const after = await current.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(current, event);
      return (await current.waitForFunction(({ after, kind }) => {
        const entry = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.sequence > after && [kind, `${kind}_error`].includes(entry.payload.kind));
        return entry?.payload || null;
      }, { after, kind }, { timeout: 30_000 })).jsonValue();
    };
    const schedule = () => {
      const reply = fixture.scheduleOperation("issue.monitor.schedule.read");
      expect(reply.ok, JSON.stringify(reply)).toBe(true);
      return JSON.parse(reply.output);
    };
    const owner = (snapshot: any) => snapshot.members.find((member: any) =>
      member.issues.some((issue: any) => issue.issue_number === fixture.issueNumber));
    const openMemberPlans = async (memberId: string) => {
      await current.locator('.op-rail__surface[data-surface="agents"]').click();
      const main = current.getByRole("region", { name: "Main pane", exact: true });
      await expect(main).toHaveAttribute("data-surface", "agents");
      const agents = main.locator(".agents-surface:not(.agents-surface--preview)");
      await agents.locator(`.roster-row[data-member-id="${memberId}"]`).click();
      return agents.locator(".roster-plans");
    };
    capture(current);
    try {
      await current.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(current, fixture.url, { enableTestBridge: true });
      await expect(current.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(current, { kind: "set_issue_monitor_enabled", enabled: false });
      await expect.poll(async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString()).enabled,
        { timeout: 30_000 }).toBe(false);
      const initial = (await request({ kind: "agent_roster", action: { kind: "read" } }, "agent_roster")).roster;
      expect(initial.error).toBeFalsy();
      const roster = (await request({ kind: "agent_roster", action: { kind: "add" },
        expected_revision: initial.revision }, "agent_roster")).roster;
      expect(roster.error).toBeFalsy();
      expect(roster.members.filter((member: any) => member.enabled)).toHaveLength(2);
      // Registration and launch eligibility are separate: explicitly select
      // both fixed-tool people while retaining the legacy profile's tuning.
      const preferences = JSON.parse((await fixture.monitorPreferencesBytes()).toString());
      const baseProfile = preferences.launch_profiles[0];
      expect(baseProfile.agent_id).toBe("codex");
      expect(baseProfile.agent_member_id).toBe(initial.members[0].agent_member_id);
      const selectedProfiles = roster.members.filter((member: any) => member.enabled).map((member: any) => {
        expect(member.agent_id).toBe("codex");
        return { ...baseProfile, agent_member_id: member.agent_member_id, agent_id: member.agent_id };
      });
      const beforeProfiles = await current.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(current, { kind: "issue_monitor_profiles_set", request_id: 477727, profiles: selectedProfiles });
      const profileReply = await current.waitForFunction(after => ((window as any).__gwtPlaywrightMessages || []).find((entry: any) =>
        entry.sequence > after && entry.payload.request_id === 477727
        && ["issue_monitor_profiles", "issue_monitor_profiles_write_failed"].includes(entry.payload.kind))?.payload,
      beforeProfiles).then(handle => handle.jsonValue());
      expect(profileReply.kind, profileReply.message).toBe("issue_monitor_profiles");
      expect(profileReply.profiles).toEqual(selectedProfiles);
      expect(JSON.parse((await fixture.monitorPreferencesBytes()).toString()).launch_profiles).toEqual(selectedProfiles);
      const queued = await request({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] },
        "issue_monitor_queue_result");
      expect(queued.error).toBeNull();
      expect(await fixture.launches()).toEqual([]);
      const observed = schedule();
      const source = owner(observed);
      expect(source).toBeTruthy();
      const target = roster.members.find((member: any) => member.enabled
        && member.agent_member_id !== source.agent_member_id);
      expect(target).toBeTruthy();
      const reassignment = {
        issue_number: fixture.issueNumber,
        source_member_id: source.agent_member_id,
        target_member_id: target.agent_member_id,
        expected_roster_revision: observed.roster_revision,
        expected_queue_revision: observed.queue_revision,
        expected_schedule_revision: observed.schedule_revision,
      };
      const sourcePlans = await openMemberPlans(source.agent_member_id);
      const queuedPlan = sourcePlans.locator(`.roster-plan[data-issue-number="${fixture.issueNumber}"]`);
      await expect(queuedPlan).toBeVisible();
      await queuedPlan.getByRole("combobox", { name: `Assign issue #${fixture.issueNumber} to member`, exact: true })
        .selectOption(target.agent_member_id);
      await queuedPlan.getByRole("button", { name: `Reassign issue #${fixture.issueNumber}`, exact: true }).click();
      const targetPlans = await openMemberPlans(target.agent_member_id);
      await expect(targetPlans.locator(`.roster-plan[data-issue-number="${fixture.issueNumber}"] .roster-plan-reason`))
        .toHaveText("Assigned by you");
      const saved = schedule();
      expect(owner(saved)).toMatchObject({ agent_member_id: target.agent_member_id,
        issues: [{ issue_number: fixture.issueNumber, reason: "assigned_by_you" }] });
      await info.attach(`agent-schedule-ui-${theme}`, { body: await current.screenshot(), contentType: "image/png" });
      const beforeStale = await fixture.monitorPreferencesBytes();
      const stale = fixture.scheduleOperation("issue.monitor.schedule.reassign", reassignment);
      expect(stale.ok).toBe(false);
      const afterStale = await fixture.monitorPreferencesBytes();
      await info.attach("agent-schedule-stale-prefs", { body: JSON.stringify({
        before: beforeStale.toString(), after: afterStale.toString(),
      }, null, 2), contentType: "application/json" });
      const plan = (bytes: Buffer) => {
        const prefs = JSON.parse(bytes.toString());
        return { agent_issue_schedule: prefs.agent_issue_schedule, agent_roster: prefs.agent_roster,
          terminal_queues: Object.fromEntries(Object.entries(prefs.terminal_queues)
            .map(([host, queue]) => [host, (queue as any).entries])) };
      };
      expect(plan(afterStale), "rejected stale move preserves canonical member plans and queue order").toEqual(plan(beforeStale));
      expect(schedule()).toEqual(saved);
      const gui = await request({ kind: "agent_schedule", action: { kind: "read" } }, "agent_schedule");
      expect(gui.schedule, "GUI and JSON use the same canonical schedule").toEqual(saved);

      const previousPid = fixture.ownedPid;
      await current.close();
      await fixture.restart();
      expect(fixture.ownedPid).not.toBe(previousPid);
      current = await context.newPage();
      capture(current);
      await current.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(current, fixture.url, { enableTestBridge: true });
      await expect(current.locator("html")).toHaveAttribute("data-theme", theme);
      expect(owner(schedule()), "fresh GUI retains the reassigned member").toEqual(owner(saved));
      const restoredPlans = await openMemberPlans(target.agent_member_id);
      await expect(restoredPlans.locator(`.roster-plan[data-issue-number="${fixture.issueNumber}"] .roster-plan-reason`))
        .toHaveText("Assigned by you");
      await expect(current.getByRole("region", { name: "Main pane", exact: true }).locator(".roster-detail h3"))
        .toHaveText(target.name);
      expect(await fixture.launches(), "disabled Monitor stays idle across restart").toEqual([]);
      await info.attach(`agent-schedule-ui-restored-${theme}`, { body: await current.screenshot(), contentType: "image/png" });
      await sendLiveGwtEvent(current, { kind: "set_issue_monitor_enabled", enabled: true });
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 90_000, message: "normal Monitor launches the reassigned queued Issue" }).toBe(1);
      const launch = (await fixture.launches())[0];
      expect(launch.gwt_session_id).toBeTruthy();
      expect(() => process.kill(launch.pid, 0)).not.toThrow();
      const session = () => readFile(join(fixture.home, ".gwt", "sessions", `${launch.gwt_session_id}.toml`), "utf8");
      const field = (record: string, key: string) => record.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, "m"))?.[1];
      const receivedActivity = (after = 0) => current.evaluate(after => {
        let latest: any;
        for (const entry of (window as any).__gwtPlaywrightMessages ?? []) {
          if (entry.sequence <= after) continue;
          // Only the independent observer's standalone event proves this
          // scheduler path; a Work projection could mask an empty observer.
          const next = entry.payload.kind === "local_agent_activity" ? entry.payload.activity : undefined;
          if (next && (!latest || Date.parse(next.observed_at) >= Date.parse(latest.observed_at))) latest = next;
        }
        return latest;
      }, after);
      const assertObservedWork = async (sessionId: string, memberId: string, after = 0, previousIncarnation?: number) => {
        await expect.poll(async () => (await receivedActivity(after))?.agents?.some((agent: any) =>
          agent.session_id === sessionId && !agent.is_pm && agent.runtime_incarnation !== previousIncarnation),
        { timeout: 60_000, message: "the real independent observer sees the launched child" }).toBe(true);
        const activity = await receivedActivity(after);
        const agent = activity.agents.find((entry: any) => entry.session_id === sessionId);
        expect(agent).toMatchObject({ issue_number: fixture.issueNumber, is_pm: false });
        expect(agent.runtime_incarnation).toBeGreaterThan(0);
        expect(agent.runtime_incarnation).not.toBe(previousIncarnation);
        const actual = (await request({ kind: "agent_roster", action: { kind: "read" } }, "agent_roster")).roster;
        expect(actual.error).toBeFalsy();
        expect(actual.members.find((member: any) => member.agent_member_id === memberId)?.assignment)
          .toMatchObject({ session_id: sessionId, issue_number: fixture.issueNumber });
        await openMemberPlans(memberId);
        const main = current.getByRole("region", { name: "Main pane", exact: true });
        await expect(main.locator(".roster-capacity-note")).toContainText("Busy 1 / Team");
        await expect(main.locator(`.roster-row[data-member-id="${memberId}"] .roster-current`)).toContainText("In Works");
        await current.locator('.op-rail__surface[data-surface="works"]').click();
        await expect(main).toHaveAttribute("data-surface", "works");
        if (await main.locator(".works-full:not([hidden])").isVisible())
          await main.locator('.works-full [data-action="back"]').click();
        await expect(main.locator(`.work-row[data-window-id="${agent.window_id}"]`)).toBeVisible();
        return { activity, agent, memberId };
      };
      await expect.poll(async () => field(await session(), "agent_session_id"),
        { timeout: 60_000, message: "authentic Session receives the provider SessionStart identity" }).toBe(launch.provider_session_id);
      const record = await session();
      expect(field(record, "id")).toBe(launch.gwt_session_id);
      expect(field(record, "agent_member_id")).toBe(target.agent_member_id);
      expect(field(record, "agent_name")).toBe(target.name);
      expect(record).toMatch(new RegExp(`^linked_issue_number\\s*=\\s*${fixture.issueNumber}$`, "m"));
      const firstObservation = await assertObservedWork(launch.gwt_session_id!, target.agent_member_id);
      fixture.auditHooks(launch.cwd);

      // Stop only the owned provider, then recover through the real authenticated
      // Monitor path. No incoming census or roster frame is manufactured here.
      await sendLiveGwtEvent(current, { kind: "set_issue_monitor_enabled", enabled: false });
      await expect.poll(async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString()).enabled).toBe(false);
      await sendLiveGwtEvent(current, { kind: "terminal_input", id: firstObservation.agent.window_id, data: "GWT_FIXTURE_EXIT\r" });
      await expect.poll(() => {
        try { process.kill(launch.pid, 0); return false; } catch { return true; }
      }, { timeout: 60_000, message: "the predecessor provider really exited" }).toBe(true);
      // Provider exit precedes the Host's PTY exit event and durable failure.
      // Observe the guard's state before the single force-requeue write.
      await expect.poll(async () => {
        const prefs = JSON.parse((await fixture.monitorPreferencesBytes()).toString());
        const ownsIssue = (entry: any) => entry.issue_number === fixture.issueNumber;
        return !prefs.launched_issues?.some(ownsIssue)
          && !prefs.pending_launch_deliveries?.some(ownsIssue)
          && prefs.failed_issues?.some((entry: any) => ownsIssue(entry)
            && entry.window_id === firstObservation.agent.window_id);
      }, { timeout: 60_000, message: "the Host records the exact predecessor exit and releases its launch" }).toBe(true);
      const after = await current.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      const requeued = fixture.fixtureOperation("issue.monitor.requeue", {
        number: fixture.issueNumber, reason: "Issue #6362 real observer force-requeue regression", force: true,
      });
      expect(requeued.ok, JSON.stringify(requeued)).toBe(true);
      await sendLiveGwtEvent(current, { kind: "set_issue_monitor_enabled", enabled: true });
      await expect.poll(async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString()).enabled,
        { timeout: 30_000, message: "Monitor enable is durable before the separate launch_now operation" }).toBe(true);
      const immediate = fixture.fixtureOperation("issue.monitor.launch_now", { number: fixture.issueNumber });
      expect(immediate.ok, JSON.stringify(immediate)).toBe(true);
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(2);
      const successor = (await fixture.launches())[1];
      expect(successor.gwt_session_id).toBeTruthy();
      expect(() => process.kill(successor.pid, 0)).not.toThrow();
      const successorSession = () => readFile(join(fixture.home, ".gwt", "sessions", `${successor.gwt_session_id}.toml`), "utf8");
      await expect.poll(async () => field(await successorSession(), "agent_session_id"), { timeout: 60_000 })
        .toBe(successor.provider_session_id);
      const successorMember = field(await successorSession(), "agent_member_id");
      expect(successorMember).toBeTruthy();
      const successorObservation = await assertObservedWork(successor.gwt_session_id!, successorMember!, after,
        firstObservation.agent.runtime_incarnation);
      fixture.auditHooks(successor.cwd);
      await info.attach(`agent-schedule-${theme}`, { body: await current.screenshot(), contentType: "image/png" });
      await info.attach("agent-schedule-launch", { body: JSON.stringify({ observed, saved, stale,
        target: { agent_member_id: target.agent_member_id, name: target.name }, launch, firstObservation,
        requeued, immediate, successor, successorObservation }, null, 2), contentType: "application/json" });
      expect(errors, "zero console/page errors across UI reassignment, GUI restart and real launch").toEqual([]);
    } finally {
      try { await current.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned fixture cleanup").toEqual([]);
  });

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
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue" });
      const issueWindow = page.locator('.workspace-window[data-preset="issue"]');
      await expect(issueWindow).toBeVisible();
      const issues = issueWindow.locator(".issue-bridge-root");
      const metric = issues.locator('[data-metric="active"]');
      await expect(metric).toHaveText("Active 0 · 0 issues · PM 0", { timeout: 60_000 });
      const row = issues.locator(`[data-issue-number="${fixture.issueNumber}"]`);
      await expect(row).toBeVisible({ timeout: 60_000 });
      await row.dragTo(issues.locator('[data-queue-column="queued"] h3'));
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const launch = (await fixture.launches())[0];
      const activity = () => page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .map(({ payload }: any) => payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity)
        .filter((snapshot: any) => snapshot?.project_key === location.pathname.split("/")[2] && snapshot.host_id
          && Number.isFinite(Date.parse(snapshot.observed_at)))
        .sort((left: any, right: any) => Date.parse(left.observed_at) - Date.parse(right.observed_at)).at(-1));
      await expect(metric).toHaveText("Active 1 · 1 issue · PM 0", { timeout: 60_000 });
      await expect.poll(async () => (await activity())?.agents?.some((agent: any) => agent.session_id === launch.gwt_session_id)).toBe(true);
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
          .filter(({ payload }: any) => payload.projection || ["terminal_status", "local_agent_activity"].includes(payload.kind))
          .map(({ sequence, payload }: any) => ({ sequence, kind: payload.kind,
            activity: payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity, id: payload.id, status: payload.status })));
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
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue",
        bounds: { x: 20, y: 60, width: 1480, height: 920 } });
      const issueWindow = page.locator('.workspace-window[data-preset="issue"]');
      await expect(issueWindow).toBeVisible();
      const issues = issueWindow.locator(".issue-bridge-root");
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
