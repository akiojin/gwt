import { expect, test, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "node:child_process";
import { mkdir, readFile, readdir, realpath, rename, rmdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { startIssueQueueLaunchFixture } from "./_helpers/issue-queue-launch";
import { gotoLiveGwt, liveGwtProjectUrl, sendLiveGwtEvent } from "./_helpers/live-gwt";
import { operationOutput, readOperationOutput } from "./_helpers/operation-envelope.mjs";
import { openWorks } from "./_helpers/works-entry";

let rosterReadSequence = 0;
async function readCanonicalRoster(page: Page): Promise<any> {
  const requestId = `t814-roster-${++rosterReadSequence}`;
  const projectKey = await page.evaluate(() => location.pathname.split("/")[2]);
  expect(projectKey).toBeTruthy();
  const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
  await sendLiveGwtEvent(page, { kind: "agent_roster", project_key: projectKey, request_id: requestId, action: { kind: "read" } });
  const reply = await page.waitForFunction(({ after, requestId }) => ((window as any).__gwtPlaywrightMessages || []).find((entry: any) =>
    entry.sequence > after && entry.payload.request_id === requestId
    && ["agent_roster", "agent_roster_error"].includes(entry.payload.kind))?.payload,
  { after, requestId }, { timeout: 30_000 }).then(handle => handle.jsonValue());
  expect(reply.kind, reply.message).toBe("agent_roster");
  expect(reply.roster.project_key).toBe(projectKey);
  expect(reply.roster.error).toBeFalsy();
  expect(Array.isArray(reply.roster.members)).toBe(true);
  return reply.roster;
}
function durableRoster(roster: any) {
  const fields = ["agent_member_id", "agent_id", "name", "pm_eligible", "pm_project_key", "created_at",
    "origin_session_id", "last_issue_number", "last_spec_number", "last_owner_project_key", "assignment"];
  return { revision: roster.revision, members: roster.members.map((member: any) =>
    Object.fromEntries(fields.map(field => [field, member[field]]))) };
}

test.describe("Issue queue real backend launch", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_QUEUE_LAUNCH !== "1", "requires checkout binaries and isolated provider fixture");
  test.setTimeout(240_000);
  test.use({ viewport: { width: 1600, height: 1100 } });

  test("Issue #6996 five submissions immediately after GUI restart do not report finalizer waits as errors", async ({ page }, info) => {
    test.setTimeout(360_000);
    const issueNumbers = [43, 44, 45, 46, 47];
    const fixture = await startIssueQueueLaunchFixture(info, {
      additionalIssueNumbers: issueNumbers.slice(1),
      terminalConvergence: true,
      monitorPreferences: { enabled: false, max_active_agents: 5 },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const ledgerRows = async () => {
      const directory = join(fixture.home, ".gwt/logs/errors");
      const files = (await readdir(directory).catch(() => [])).filter(file => /^errors\..*\.jsonl$/.test(file));
      return (await Promise.all(files.map(file => readFile(join(directory, file), "utf8"))))
        .flatMap(bytes => bytes.split("\n").filter(Boolean).map(line => JSON.parse(line)));
    };
    try {
      expect(fixture.fixtureOperation("issue.monitor.config.set", { max_active: 5 }).ok).toBe(true);
      const previousPid = fixture.ownedPid;
      await fixture.restart();
      expect(fixture.ownedPid).not.toBe(previousPid);
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await page.locator('[data-surface="issues"]').click();
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: issueNumbers });
      await page.getByRole("group", { name: "Issues controls", exact: true })
        .getByRole("button", { name: "Start", exact: true }).click();
      await expect.poll(async () => (await fixture.launchIssues()).sort((a, b) => a - b),
        { timeout: 180_000, message: "all five real provider handoffs after restart" }).toEqual(issueNumbers);
      await expect.poll(async () => {
        const prefs = JSON.parse((await fixture.monitorPreferencesBytes()).toString());
        return issueNumbers.filter(number => prefs.launch_confirmations?.[number]?.window_id);
      }, { timeout: 60_000 }).toEqual(issueNumbers);
      const failures = (await ledgerRows()).filter(row => row.kind === "launch_failure"
        && /workspace operation .*is in flight|workspace finalizer .*timed out/.test(row.message));
      expect(failures, "no finalizer waiting messages in the real error ledger").toEqual([]);
      const waitToasts = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_toast" && payload.level === "error"
          && /workspace operation .*is in flight/.test(payload.message)));
      expect(waitToasts).toEqual([]);
      const evidence = info.outputPath("issue-6996-post-restart.json");
      await writeFile(evidence, JSON.stringify({ theme, previousPid, pid: fixture.ownedPid, issueNumbers,
        launches: await fixture.launches(), finalizerFailures: failures, errorToasts: waitToasts }, null, 2));
      await info.attach("issue-6996-post-restart", { path: evidence, contentType: "application/json" });
      const screenshot = info.outputPath(`issue-6996-${theme}.png`);
      await page.screenshot({ path: screenshot });
      await info.attach(`issue-6996-${theme}`, { path: screenshot, contentType: "image/png" });
      expect(errors).toEqual([]);
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including shutdown").toEqual([]);
  });

  for (const savedLimit of [0, 2]) test(`Issue #6922 Start refills the queue on its first scan with saved limit ${savedLimit}`, async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, {
      additionalIssueNumbers: [45, 46, 47],
      monitorPreferences: { enabled:false, terminal_queue_auto_refill:false, terminal_queue_auto_refill_limit:savedLimit },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const prefs = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const expectedLimit = savedLimit || 3;
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge:true });
      await page.locator('[data-surface="issues"]').click();
      const controls = page.getByRole("group", { name:"Issues controls", exact:true });
      const refill = controls.getByRole("switch", { name:"Auto-refill", exact:true });
      await expect(refill).toHaveAttribute("aria-checked", "false");
      await controls.getByRole("button", { name:"Issues settings", exact:true }).click();
      const settings = page.locator('[data-settings-panel="issues"]');
      await expect(settings.getByLabel("Refill limit", { exact:true })).toHaveValue(String(savedLimit));
      await page.locator('[data-surface="issues"]').click();
      await refill.click();
      await expect.poll(async () => (await prefs()).terminal_queue_auto_refill_limit).toBe(expectedLimit);
      await expect(refill).toHaveAttribute("aria-checked", "true");
      await expect(() => {
        const envelope = fixture.fixtureOperation("issue.monitor.status");
        expect(envelope.ok, JSON.stringify(envelope)).toBe(true);
        expect(JSON.parse(envelope.output).saved_config).toMatchObject({
          terminal_queue_auto_refill:true, terminal_queue_auto_refill_limit:expectedLimit,
        });
      }).toPass({ timeout:10_000 });
      await controls.getByRole("button", { name:"Issues settings", exact:true }).click();
      await expect(settings.getByLabel("Refill limit", { exact:true })).toHaveValue(String(expectedLimit));
      await page.locator('[data-surface="issues"]').click();
      const beforeStart = await prefs();
      expect(beforeStart.enabled).toBe(false);
      expect(Object.values(beforeStart.terminal_queues ?? {}).flatMap((queue: any) => queue.entries)).toEqual([]);
      await expect(page.locator('[data-role="monitor-refill-guidance"]')).toBeVisible();
      await expect(page.locator('[data-queue-column="queued"] .knowledge-empty')).toContainText("Start the project to begin Auto-refill.");
      // Issue #7728: Issues has no own Start; the project stop/start in Settings starts it.
      await expect(controls.getByRole("button", { name:"Start", exact:true })).toHaveCount(0);
      await controls.getByRole("button", { name:"Issues settings", exact:true }).click();
      const projectStart = settings.locator('[data-settings-launch-action="project-autonomy"] [data-project-autonomy-action="start"]');
      await expect(projectStart).toBeVisible();
      const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      await projectStart.click();
      await page.locator('[data-surface="issues"]').click();
      const firstScan = await page.waitForFunction(({ cursor, previousScan }) => {
        return ((window as any).__gwtPlaywrightMessages ?? []).find(({ sequence, payload }: any) =>
          sequence > cursor && payload.kind === "issue_monitor_status" && payload.status.enabled
          && payload.status.last_scan_at && payload.status.last_scan_at !== previousScan)?.payload.status;
      }, { cursor, previousScan:beforeStart.last_scan_at }, { timeout:90_000 }).then(handle => handle.jsonValue());
      expect(firstScan.terminal_queue).toHaveLength(expectedLimit);
      expect(firstScan.terminal_queue.every((entry: any) => entry.queued_by === "auto-refill")).toBe(true);
      await expect(page.locator('[data-queue-column="queued"] .knowledge-row')).toHaveCount(expectedLimit);
      await expect(page.locator('[data-role="monitor-refill-guidance"]')).toBeHidden();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await info.attach(`refill-first-scan-${savedLimit}-${theme}`, {
        body:JSON.stringify({ beforeStart, firstScan, saved:await prefs() }, null, 2), contentType:"application/json",
      });
      await info.attach(`refill-${savedLimit}-${theme}`, { body:await page.screenshot(), contentType:"image/png" });
      expect(errors).toEqual([]);
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned process cleanup").toEqual([]);
  });

  test("roster Manual same-value save updates machine policy before autonomous queue launch", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, { page, autonomousMode: true });
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
      await expect.poll(async () => (await readCanonicalRoster(page)).members.length).toBe(10);
      const canonical = await readCanonicalRoster(page);
      expect(canonical.members).toHaveLength(10);
      const originalMembers = durableRoster(canonical).members;
      // Lower the machine policy while preserving the ten registered members.
      const seed = join(fixture.home, "machine-policy-seed");
      await mkdir(seed);
      operation("issue.monitor.config.set", { max_active: 4 }, seed);
      const olderMachine = await machine();
      expect(olderMachine).toMatchObject({ mode: "manual", manual_limit: 4 });
      const reduced = await readCanonicalRoster(page);
      const resizeRequestId = `t814-resize-${++rosterReadSequence}`;
      const beforeResize = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(page, { kind: "agent_roster", project_key: reduced.project_key, request_id: resizeRequestId,
        expected_revision: reduced.revision, action: { kind: "resize", size: 10 } });
      const resizedReply = await page.waitForFunction(({ after, requestId }) => ((window as any).__gwtPlaywrightMessages || []).find((entry: any) =>
        entry.sequence > after && entry.payload.request_id === requestId
        && ["agent_roster", "agent_roster_error"].includes(entry.payload.kind))?.payload,
      { after: beforeResize, requestId: resizeRequestId }, { timeout: 30_000 }).then(handle => handle.jsonValue());
      expect(resizedReply.kind, resizedReply.message).toBe("agent_roster");
      expect(resizedReply.roster.project_key).toBe(reduced.project_key);
      expect(resizedReply.roster.error).toBeFalsy();
      expect(resizedReply.roster.revision).toBe(reduced.revision);
      expect(resizedReply.roster.members).toHaveLength(10);
      expect(durableRoster(resizedReply.roster).members).toEqual(originalMembers);
      expect(await machine()).toEqual(olderMachine);
      const roster = durableRoster(await readCanonicalRoster(page));
      expect(roster).toEqual(durableRoster(resizedReply.roster));
      expect(await prefs()).toMatchObject({ max_active_agents: 10, max_active_agents_mode: "manual" });
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
      expect(durableRoster(await readCanonicalRoster(page))).toEqual(roster);
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
        const matches = (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
          .filter((window: any) => window.session_id === sessionId);
        return matches.length === 1 ? matches[0] : undefined;
      }, launch.gwt_session_id);
      await expect.poll(async () => ["running", "idle"].includes((await window())?.status), { timeout: 60_000 }).toBe(true);
      const actualWindow = await window();
      expect(actualWindow.session_id).toBe(launch.gwt_session_id);
      // GUI layout aliases and native owner pane IDs share the exact Session.
      // The canonical status reader validates the native pane's Issue binding.
      await expect.poll(async () => {
        const id = (await prefs()).launch_confirmations?.[fixture.issueNumber]?.window_id;
        if (typeof id !== "string" || !id) return false;
        const matches = operation("issue.monitor.status").output.active_launch_details
          .filter((detail: any) => detail.issue_number === fixture.issueNumber && detail.session_id === launch.gwt_session_id);
        return matches.length === 1 && matches[0].window_id === id;
      }, { timeout: 60_000 }).toBe(true);
      const confirmed = (await prefs()).launch_confirmations[fixture.issueNumber];
      expect(confirmed.claim_id).toBeTruthy();
      expect(confirmed.delivery_id).toBeTruthy();
      expect((await prefs()).launch_bindings[confirmed.window_id]).toBe(fixture.issueNumber);
      expect(fixture.comments.some(comment => comment.body.includes("gwt-auto-improve-claim")
        && comment.body.includes(confirmed.claim_id))).toBe(true);
      const record = () => readFile(join(fixture.home, ".gwt/sessions", `${launch.gwt_session_id}.toml`), "utf8");
      await expect.poll(async () => (await record()).match(/^agent_session_id\s*=\s*"([^"]*)"/m)?.[1],
        { timeout: 60_000 }).toBe(launch.provider_session_id);
      expect(await record()).toContain(`linked_issue_number = ${fixture.issueNumber}`);
      const running = operation("issue.monitor.status").output;
      expect(running.active_launch_details).toContainEqual(expect.objectContaining({ issue_number: fixture.issueNumber,
        session_id: launch.gwt_session_id, window_id: confirmed.window_id }));
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
    const fixture = await startIssueQueueLaunchFixture(info, { page, targetIssueNumber: 44, switching: true });
    const errors: string[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    let current = page;
    let blocker: ChildProcess | undefined;
    let blockerClosed: Promise<void> | undefined;
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
      await expect.poll(async () => JSON.parse(await readFile(
        join(fixture.home, ".gwt/machine-state/agent-settings.json"), "utf8")), { timeout: 30_000 })
        .toMatchObject({ mode: "manual", manual_limit: 1,
          last_change: { source: "gui", max_active_agents_override: 1 } });
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] }, "issue_monitor_queue_result");
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const source = (await fixture.launches())[0];
      await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
        entry.stage === "idle" && entry.gwt_session_id === source.gwt_session_id), { timeout: 60_000 }).toBe(true);
      // Own an actual worktree process across the GUI/runtime restart. The
      // negative must not depend on incidental processes in the host namespace.
      blocker = spawn(process.execPath, ["-e",
        "process.stdout.write(JSON.stringify({pid:process.pid,cwd:process.cwd()})+'\\n');setInterval(()=>{},1000)"], {
        cwd: source.cwd, env: { HOME: fixture.home, PATH: process.env.PATH ?? "" }, stdio: ["ignore", "pipe", "ignore"],
      });
      blockerClosed = new Promise(resolve => blocker!.once("close", () => resolve()));
      const ready = await new Promise<{ pid: number; cwd: string }>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error("Owned census blocker did not start")), 5_000);
        blocker!.once("error", error => { clearTimeout(timer); reject(error); });
        blocker!.stdout!.once("data", bytes => {
          clearTimeout(timer);
          try { resolve(JSON.parse(bytes.toString())); } catch (error) { reject(error); }
        });
      });
      expect(ready).toEqual({ pid: blocker.pid, cwd: source.cwd });
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
      expect(saved.reason).toMatch(/Observed worktree PIDs: \[[\d, ]*\] \(\d+ total\); unknown cwd PIDs: \[[\d, ]*\] \(\d+ total\); census complete: (?:true|false)\./);
      const observedPids = /Observed worktree PIDs: \[([\d, ]*)\]/.exec(saved.reason)![1]
        .split(",").map((value: string) => Number(value.trim()));
      expect(observedPids).toContain(ready.pid);
      await expect(plans.locator(".roster-switch-status")).toContainText("Observed worktree PIDs:");
      const duplicate = await request({ kind: "agent_switch", action: { kind: "request", ...saved.request } }, "agent_switch");
      expect(duplicate.snapshot.intents.filter((value: any) => value.operation_id === operation)).toHaveLength(1);
      expect((await readCanonicalRoster(current)).members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment).toEqual(sourceAssignment);
      expect((await preferences()).monitor_runtime_stop_requests || []).toEqual([]);
      expect(fixture.processAlive(source.pid)).toBe(true);
      expect(await fixture.launches()).toHaveLength(1);
      await info.attach(`agent-switch-pending-${theme}`, { body: await current.screenshot(), contentType: "image/png" });
      await plans.getByRole("button", { name: "Cancel switch", exact: true }).click();
      await expect.poll(async () => (await intent(operation))?.stage).toBe("cancelled");
      expect(fixture.processAlive(source.pid)).toBe(true);
      expect((await readCanonicalRoster(current)).members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment).toEqual(sourceAssignment);
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
      await fixture.restartRuntime(saved.runtime_stop.host_pid);
      expect(Number.isSafeInteger(sourceAssignment.host_pid) && sourceAssignment.host_pid > 0).toBe(true);
      expect(fixture.processAlive(sourceAssignment.host_pid)).toBe(false);
      expect(fixture.processAlive(ready.pid)).toBe(true);
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
        try { await current.close(); } finally {
          try { await fixture.stop(); } finally {
            if (blocker && blockerClosed) {
              if (blocker.exitCode === null && blocker.signalCode === null) blocker.kill("SIGTERM");
              const escalation = setTimeout(() => blocker!.kill("SIGKILL"), 2_500);
              let timer: ReturnType<typeof setTimeout> | undefined;
              try {
                await Promise.race([blockerClosed, new Promise<never>((_, reject) => {
                  timer = setTimeout(() => reject(new Error("Owned census blocker did not close")), 5_000);
                })]);
              } finally { clearTimeout(escalation); clearTimeout(timer); }
              await info.attach("owned-census-blocker-cleanup", {
                body: JSON.stringify({ pid: blocker.pid, closed: true, signal: blocker.signalCode }), contentType: "application/json",
              });
            }
          }
        }
      }
    }
    expect(errors).toEqual([]);
  });

  for (const afterAck of ["claim replaced", "stop claim replaced", "before body cancel", "source replaced", "reserved submit", "body submit", "body cancel", "body input", "queued body", "unchanged", "refresh cancel", "refresh draft", "refresh submit", "stop refresh draft", "stop refresh cancel", "initial draft", "input", "draft", "source edit", "cancel", "target removed", "worktree job", "cancel pending job", "unknown cwd", "restart"] as const) test(afterAck === "claim replaced"
    ? "T94442 claim-bearing replacement before checkpoint body preserves the successor"
    : afterAck === "stop claim replaced"
    ? "T94442 claim-bearing replacement after saved ACK preserves the successor"
    : afterAck === "before body cancel"
    ? "T94440 cancellation after reservation rejects the checkpoint body CAS"
    : afterAck === "source replaced"
    ? "T94440 actual source replacement after reservation rejects the old checkpoint"
    : afterAck === "reserved submit"
    ? "T94440 submitted input before the checkpoint Session lease rejects the old body"
    : afterAck === "body submit"
    ? "T94440 submitted input after the checkpoint Session lease follows its single CR"
    : afterAck === "body cancel"
    ? "T94439 cancellation after checkpoint body finishes one submit without saving"
    : afterAck === "body input"
    ? "T94439 ordinary input after checkpoint body waits for its submit"
    : afterAck === "queued body"
    ? "T94436 queued input after reservation prevents checkpoint body delivery"
    : afterAck === "refresh cancel"
    ? "T94431 cancellation during target refresh prevents checkpoint delivery and permits a new switch"
    : afterAck === "refresh draft"
    ? "T94432 draft during target refresh preserves input and resumes the same switch"
    : afterAck === "refresh submit"
    ? "T94433 submitted input during target refresh resumes with the new revision"
    : afterAck === "stop refresh draft"
    ? "T94434 draft during stop target refresh preserves saved source and input"
    : afterAck === "stop refresh cancel"
    ? "T94435 cancellation during stop target refresh preserves the saved source"
    : afterAck === "initial draft"
    ? "T94430 unsent draft before the first Switch request waits and resumes naturally"
    : afterAck === "input"
    ? "T94421 input after checkpoint ACK preserves the source without switching"
    : afterAck === "draft"
      ? "T94429 unsent draft after checkpoint ACK survives the delayed Normal Stop"
      : afterAck === "source edit"
      ? "T94422 source edit after checkpoint ACK preserves unsaved work without switching"
      : afterAck === "cancel"
        ? "T94423 cancellation after checkpoint ACK survives the delayed Normal Stop"
        : afterAck === "target removed"
          ? "T94424 target removal after checkpoint ACK preserves the source after delayed Stop"
          : afterAck === "worktree job"
            ? "T94425 worktree job after checkpoint ACK holds the source until ordinary job exit"
            : afterAck === "cancel pending job"
              ? "T94426 cancellation while a post-ACK job is pending survives ordinary job exit"
              : afterAck === "unknown cwd"
                ? "T94427 unknown job cwd after checkpoint ACK holds the source until ordinary job exit"
                : afterAck === "restart"
                  ? "T94428 owner restart after checkpoint ACK preserves saved work without replay"
                  : "T20b saved checkpoint switches through the real Host and Issues", async ({ page }, info) => {
    const macOsSwitch = afterAck === "unchanged" && process.platform === "darwin" && (process.geteuid?.() ?? 0) !== 0;
    test.skip((process.platform !== "linux" && !macOsSwitch) || process.env.GWT_PLAYWRIGHT_SWITCH_POSITIVE !== "1",
      "positive proof requires the isolated Linux root process namespace or a non-root macOS Host for the normal switch");
    const postBody = afterAck === "body cancel" || afterAck === "body input" || afterAck === "body submit";
    const preBody = afterAck === "before body cancel" || afterAck === "source replaced" || afterAck === "claim replaced" || afterAck === "reserved submit";
    const gatedInput = postBody || preBody;
    let bodyGate = "";
    const fixture = await startIssueQueueLaunchFixture(info, {
      page,
      ...(macOsSwitch || gatedInput || afterAck === "stop claim replaced" ? { prepare: async ({ home, bin, env }) => {
        const provider = join(bin, "codex");
        const source = await readFile(provider, "utf8");
        const lineHandler = "readline.createInterface({input:process.stdin}).on('line', line => {";
        if (macOsSwitch) {
          env.RUST_LOG = "info,gwt::issue_monitor::switch_checkpoint::macos=debug";
          // The real TUI reads raw input. Darwin's canonical PTY buffer drops the full checkpoint prompt.
          await writeFile(provider, source.replace(lineHandler, "process.stdin.setRawMode(true);\n" + lineHandler));
          return;
        }
        bodyGate = join(home, "switch-body-gate");
        env.GWT_TEST_SWITCH_BODY_GATE = bodyGate;
        expect(source.split(lineHandler)).toHaveLength(2);
        // Capture before checkpointed Set deduplication, with the actual PTY
        // byte stream as the independent proof of the body and its one CR.
        const cancelledStop = "    nativeHook('Stop');\n    actorEvent('checkpoint_cancelled',{operation});";
        expect(source.split(cancelledStop)).toHaveLength(2);
        // Manual Continue is not an Issues launch and cannot declare an Issues
        // wait. Leave its genuine UserPromptSubmit active, as ongoing work.
        const continuedSource = afterAck === "source replaced"
          ? source.replace("  sourceOperation('issue.monitor.wait',{number:owner,", "  if (resumed) { actorEvent('successor_ready', observedRuntime()); return; }\n  sourceOperation('issue.monitor.wait',{number:owner,")
          : afterAck === "claim replaced" || afterAck === "stop claim replaced"
          ? source.replace("  nativeHook('UserPromptSubmit',args.at(-1));", String.raw`  nativeHook('UserPromptSubmit',args.at(-1));
  const firstLaunch = JSON.parse(fs.readFileSync(path.join(process.env.HOME,'provider-argv.jsonl'),'utf8').trim().split('\n')[0]);
  if (owner === switching.sourceIssueNumber && firstLaunch.gwt_session_id !== process.env.GWT_SESSION_ID) {
    // This unfinished successor keeps working. G4 protects its actual current
    // hook observation, including a pending outcome; it does not certify Stop.
    actorEvent('successor_ready',{first_session_id:firstLaunch.gwt_session_id,...observedRuntime()});
    return;
  }`)
          : source;
        const orderedSource = afterAck === "body submit"
          ? continuedSource.replace(lineHandler, "let checkpointInFlight = Promise.resolve();\n" + lineHandler)
            .replace("rawMonitorProbe().catch(", "checkpointInFlight.then(() => rawMonitorProbe()).catch(")
            .replace("checkpoint(line).catch(", "(checkpointInFlight = checkpoint(line)).catch(") : continuedSource;
        await writeFile(provider, orderedSource.replace(lineHandler,
          "process.stdin.setRawMode(true);\nprocess.stdin.on('data', bytes => actorEvent('stdin_bytes',{bytes:Array.from(bytes)}));\n"
          + lineHandler + "\n  actorEvent('stdin_line',{line});")
          // Cancellation keeps the unfinished provider running. Calling Stop
          // after clearing its wait would correctly trip the completion gate.
          .replace(cancelledStop, "    actorEvent('checkpoint_cancelled',{operation});"));
      } } : {}),
      targetIssueNumber: 44, switching: true, positive: true, autonomousMode: true,
      targetRefreshGate: gatedInput || afterAck === "queued body" || afterAck === "refresh cancel" || afterAck === "refresh draft" || afterAck === "refresh submit" || afterAck === "stop refresh draft" || afterAck === "stop refresh cancel",
    });
    const errors: string[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    let job: ChildProcess | undefined;
    let jobClosed: Promise<void> | undefined;
    let coordinator: ChildProcess | undefined;
    let coordinatorClosed: Promise<number | null> | undefined;
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const actor = async (stage: string) => (await fixture.actorEvents()).find((entry: any) => entry.stage === stage);
    const releaseNormalStop = async (operation: string) => {
      const path = join(fixture.home, `normal-n2-${operation}.json`);
      await writeFile(`${path}.tmp`, JSON.stringify({ operation }));
      await rename(`${path}.tmp`, path);
    };
    const session = async (id: string) => readFile(join(fixture.home, ".gwt", "sessions", `${id}.toml`), "utf8");
    const field = (record: string, key: string) => record.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, "m"))?.[1];
    const request = async (event: unknown, kind: string, timeout = 30_000): Promise<any> => {
      const requestId = (event as { action?: { request_id?: string } }).action?.request_id;
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(page, event);
      return (await page.waitForFunction(({ after, kind, requestId }) => {
        const found = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.sequence > after && [kind, `${kind}_error`].includes(entry.payload.kind)
            && (!requestId || entry.payload.snapshot?.request_id === requestId));
        return found?.payload || null;
      }, { after, kind, requestId }, { timeout })).jsonValue();
    };
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      expect((await preferences()).autonomous_mode).toBe(true);
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 });
      await expect.poll(async () => JSON.parse(await readFile(
        join(fixture.home, ".gwt/machine-state/agent-settings.json"), "utf8")), { timeout: 30_000 })
        .toMatchObject({ mode: "manual", manual_limit: 1,
          last_change: { source: "gui", max_active_agents_override: 1 } });
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] }, "issue_monitor_queue_result");
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const source = (await fixture.launches())[0];
      await expect.poll(async () => !!await actor("idle"), { timeout: 60_000 }).toBe(true);
      const idle = await actor("idle");
      const initialNormal = await actor("normal_n1");
      expect(initialNormal.current_hook_event).toMatchObject({ event: "Stop", outcome: "normal" });
      expect(initialNormal.current_hook_event.correlation_id).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
      expect(idle.owner_number).toBe(fixture.issueNumber);
      expect(idle.cwd, "the source waits in its ordinary worktree before checkpoint delivery").toBe(source.cwd);
      expect(idle.tree_child_pid).toBeGreaterThan(0);
      expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
      expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
      const sourceWindow = () => page.evaluate(sessionId => {
        const state = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
        return (state?.payload.workspace.tabs ?? []).flatMap((tab: any) => tab.workspace.windows)
          .find((window: any) => window.session_id === sessionId);
      }, source.gwt_session_id);
      await expect.poll(async () => ["running", "idle"].includes((await sourceWindow())?.status),
        { timeout: 60_000 }).toBe(true);
      const projectedSource = await sourceWindow();
      expect(projectedSource.session_id).toBe(source.gwt_session_id);
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: "GWT_RAW_MONITOR_PROBE\r" });
      await expect.poll(async () => !!await actor("raw_successor"),
        { timeout: 60_000, message: "real raw input reaches the source before its next genuine Stop" }).toBe(true);
      const rawAccepted = await actor("raw_accepted");
      expect(rawAccepted).toMatchObject({ pid: source.pid, gwt_session_id: source.gwt_session_id });
      expect(rawAccepted.current_hook_event, "raw queue acceptance invalidates the previous Stop").toBeNull();
      expect(rawAccepted.runtime_without_current_sha256, "raw acceptance preserves history and native identity")
        .toBe(initialNormal.runtime_without_current_sha256);
      const n1 = await actor("raw_successor");
      expect(n1).toMatchObject({ pid: source.pid, gwt_session_id: source.gwt_session_id,
        current_hook_event: { event: "Stop", outcome: "normal" } });
      expect(n1.current_hook_event.correlation_id).not.toBe(initialNormal.current_hook_event.correlation_id);
      await request({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.targetIssueNumber] }, "issue_monitor_queue_result");
      const roster = (await request({ kind: "agent_roster", action: { kind: "read" } }, "agent_roster")).roster;
      expect(roster.members).toHaveLength(1);
      const member = roster.members.find((member: any) => member.assignment?.session_id === source.gwt_session_id);
      expect(member).toBeTruthy();
      const before = await session(source.gwt_session_id!);
      const claims = () => fixture.commentsForIssue(43).flatMap(comment => {
        const json = /```json\s*([\s\S]*?)```/.exec(comment.body)?.[1];
        return comment.body.startsWith("<!-- gwt-auto-improve-claim v1 -->") && json ? [JSON.parse(json)] : [];
      });
      const nativeIdentity = (runtime: any) => ({ incarnation: runtime.runtime_incarnation,
        host_started_at: runtime.host_started_at, child: runtime.child_pid, child_started_at: runtime.child_started_at,
        identity: runtime.execution_identity, status: runtime.status, current_hook_event: runtime.current_hook_event });
      const replaceClaimedSource = async (intent: any) => {
        // Failover is the real claim/queue lifecycle. OFF and Continue would
        // remove the old claim without obtaining a claim for the successor.
        const response = fixture.fixtureOperation("issue.monitor.failover", { number: 43,
          reason: "T94442 replace the exact source while its old Switch is in flight",
          claim_id: intent.source.claim_id, window_id: intent.source.window_id,
          delivery_id: intent.source.assignment.delivery_id });
        expect(response.ok, JSON.stringify(response)).toBe(true);
        expect(JSON.parse(response.output).status).toBe("restarting");
        await sendLiveGwtEvent(page, { kind: "stop_window", id: projectedSource.id });
        await expect.poll(() => fixture.processAlive(source.pid), { timeout: 30_000 }).toBe(false);
        await expect.poll(() => fixture.processAlive(idle.tree_child_pid), { timeout: 30_000 }).toBe(false);
        await expect.poll(async () => (await fixture.launches()).length, { timeout: 30_000 }).toBe(2);
        const successor = (await fixture.launches())[1];
        expect(successor.gwt_session_id).not.toBe(source.gwt_session_id);
        expect(successor.provider_session_id).not.toBe(source.provider_session_id);
        expect(successor.pid).not.toBe(source.pid);
        await expect.poll(async () => (await fixture.actorEvents()).some((event: any) => event.stage === "successor_ready"
          && event.pid === successor.pid && event.gwt_session_id === successor.gwt_session_id),
        { timeout: 30_000 }).toBe(true);
        const ready = (await fixture.actorEvents()).find((event: any) => event.stage === "successor_ready"
          && event.pid === successor.pid && event.gwt_session_id === successor.gwt_session_id);
        expect(ready).toMatchObject({ first_session_id: source.gwt_session_id, runtime_status: "Running",
          current_hook_event: { event: "UserPromptSubmit" } });
        await expect.poll(async () => {
          const prefs = await preferences();
          return prefs.launched_claims?.[43] && prefs.launched_claims[43] !== intent.source.claim_id
            && prefs.launch_confirmations?.[43]?.claim_id === prefs.launched_claims[43];
        }, { timeout: 30_000 }).toBe(true);
        const prefs = await preferences();
        const claim = prefs.launched_claims[43];
        await expect.poll(() => claims().find(value => value.claim_id === intent.source.claim_id)?.status,
          { timeout: 30_000 }).toBe("released");
        expect(claims().find(value => value.claim_id === claim)?.status).toBe("active");
        const assignment = (await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment;
        expect(assignment).toMatchObject({ issue_number: 43, session_id: successor.gwt_session_id,
          host_pid: intent.runtime_stop.host_pid, host_started_at: intent.runtime_stop.host_started_at });
        expect(assignment.reservation_id).not.toBe(intent.source.assignment.reservation_id);
        const runtimePath = join(fixture.home, ".gwt/sessions/runtime", String(intent.runtime_stop.host_pid), `${successor.gwt_session_id}.json`);
        const native = JSON.parse(await readFile(runtimePath, "utf8"));
        expect(native.runtime_incarnation).not.toBe(intent.runtime_stop.runtime_incarnation);
        expect(native.child_pid).toBe(successor.pid);
        expect(native.execution_identity).not.toEqual(intent.runtime_stop.execution_identity);
        expect(native.status).toBe("Running");
        expect(native.current_hook_event).toMatchObject({ event: "UserPromptSubmit" });
        expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
        expect(await fixture.launchIssues()).toEqual([43, 43]);
        const successorWork = async () => (await fixture.workItems()).work_items.find((value: any) =>
          value.events.some((event: any) => event.agent_session_id === successor.gwt_session_id));
        await expect.poll(async () => !!await successorWork(), { timeout: 30_000 }).toBe(true);
        return { successor, claim, assignment, native: nativeIdentity(native), runtimePath,
          confirmation: prefs.launch_confirmations[43], session: await session(successor.gwt_session_id!),
          source: fixture.savedSource(source.cwd), work: await successorWork() };
      };
      const assertClaimedSuccessor = async (intent: any, replacement: Awaited<ReturnType<typeof replaceClaimedSource>>) => {
        const prefs = await preferences();
        expect(prefs.enabled).toBe(true);
        expect(prefs.launched_claims[43]).toBe(replacement.claim);
        expect(prefs.launch_confirmations[43]).toEqual(replacement.confirmation);
        expect(prefs.launch_bindings[replacement.confirmation.window_id]).toBe(43);
        expect(prefs.launched_issues).toEqual([{ issue_number: 43, window_id: replacement.confirmation.window_id }]);
        expect(claims().find(value => value.claim_id === replacement.claim)?.status).toBe("active");
        expect(claims().find(value => value.claim_id === intent.source.claim_id)?.status).toBe("released");
        expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment)
          .toEqual(replacement.assignment);
        expect(nativeIdentity(JSON.parse(await readFile(replacement.runtimePath, "utf8")))).toEqual(replacement.native);
        expect(await session(replacement.successor.gwt_session_id!)).toBe(replacement.session);
        expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
        expect(fixture.processAlive(replacement.successor.pid)).toBe(true);
        expect((await fixture.actorEvents()).filter((event: any) => event.pid === replacement.successor.pid
          && ["checkpoint_before_prompt", "checkpoint_received", "checkpoint_acked"].includes(event.stage))).toEqual([]);
        expect(await fixture.launchIssues()).toEqual([43, 43]);
        expect(fixture.savedSource(source.cwd)).toEqual(replacement.source);
        expect(replacement.work).toBeTruthy();
        expect((await fixture.workItems()).work_items.find((value: any) => value.id === replacement.work.id))
          .toEqual(replacement.work);
        expect(fixture.requests.filter(request => request.method === "POST"
          && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
        const status = fixture.fixtureOperation("issue.monitor.status");
        expect(status.ok, JSON.stringify(status)).toBe(true);
        const observation = JSON.parse(status.output);
        expect(observation.active_launches).toEqual([43]);
        expect(observation.occupied_slot_count).toBe(1);
        expect(observation.inbox.find((row: any) => row.issue_number === 43)).toMatchObject({
          claim_id: replacement.claim, launched_window_id: replacement.confirmation.window_id });
      };
      await page.locator('.op-rail__surface[data-surface="agents"]').click();
      const agents = page.getByRole("region", { name: "Main pane", exact: true })
        .locator(".agents-surface:not(.agents-surface--preview)");
      await agents.locator(`.roster-row[data-member-id="${member.agent_member_id}"]`).click();
      const plans = agents.locator(".roster-plans");
      await plans.getByRole("button", { name: "Refresh plans", exact: true }).click();
      await expect(plans.locator('[data-action="switch-next"]')).toBeEnabled();
      const refreshDraft = afterAck === "refresh draft";
      const refreshSubmit = afterAck === "refresh submit" || afterAck === "reserved submit";
      const initialDraft = afterAck === "initial draft" || refreshDraft;
      let pendingDraftOperation: string | undefined;
      if (initialDraft) {
        const echoedProbes = () => page.evaluate(id =>
          ((window as any).__gwtTerminalTestApi?.bufferText(id) ?? "").split("GWT_RAW_MONITOR_PROBE").length - 1,
        projectedSource.id);
        await expect.poll(echoedProbes, { timeout: 30_000, message: "the initial submitted probe has reached the terminal" }).toBe(1);
        if (!refreshDraft) {
          await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: "GWT_RAW_MONITOR_PROBE" });
          await expect.poll(echoedProbes, { timeout: 30_000, message: "the real source PTY echoes the unsubmitted draft" }).toBe(2);
        }
      }
      const handoffDeadline = Date.now() + 120_000;
      if (gatedInput || afterAck === "queued body" || afterAck === "refresh cancel" || refreshDraft || refreshSubmit) await fixture.targetRefreshGate.arm();
      await plans.locator('[data-action="switch-next"]').click();
      if (afterAck === "queued body") {
        await expect.poll(() => fixture.targetRefreshGate.waiting(), { timeout: 30_000 }).not.toBeNull();
        const held = await fixture.targetRefreshGate.waiting();
        const pending = Object.values((await preferences()).agent_switch_intents)[0] as any;
        expect(pending.stage).toBe("requested");
        expect(held.parent).toBe(pending.runtime_stop.host_pid);
        expect(held.argv.slice(0, 3)).toEqual(["issue", "view", "44"]);
        const saved = fixture.savedSource(source.cwd);
        const work = (await fixture.workItems()).work_items.find((value: any) =>
          value.events.some((event: any) => event.agent_session_id === source.gwt_session_id));
        expect(work).toBeTruthy();
        // Keep browser scheduling outside the two-second runtime lease interval.
        // This takes the existing lock only; it never edits runtime or authority records.
        const gate = spawn("python3", ["-c", String.raw`import fcntl, glob, json, pathlib, sys, time, urllib.request, urllib.parse
home, project, operation, host, session, child = sys.argv[1:]
home, host, child = pathlib.Path(home), int(host), int(child)
endpoints = [json.loads(pathlib.Path(p).read_text()) for p in glob.glob(str(home / '.gwt/projects/*/runtime/daemon/*.json'))]
endpoints = [e for e in endpoints if e['pid'] == host and e['scope']['project_root'] == project]
assert len(endpoints) == 1, 'exact fixture Host endpoint'
endpoint = endpoints[0]
url = endpoint['project_transport_url'] + '/private/project/command'
assert urllib.parse.urlparse(url).hostname == '127.0.0.1', 'fixture loopback transport'
http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
def command(value):
    request = urllib.request.Request(url, data=json.dumps({'project_key': endpoint['scope']['repo_hash'], 'command': value}).encode(),
        headers={'Authorization': 'Bearer ' + endpoint['auth_token'], 'Content-Type': 'application/json'})
    with http.open(request, timeout=1.5) as response:
        return json.load(response)
health = command({'type': 'health'})
assert health['pid'] == host and health['project_key'] == endpoint['scope']['repo_hash'] and not health['draining']
runtime_path = home / '.gwt/sessions/runtime' / str(host) / (session + '.json')
runtime = json.loads(runtime_path.read_text())
assert runtime['child_pid'] == child and runtime['host_started_at'] == health['host_started_at']
incarnation = runtime['runtime_incarnation']
source = [p for p in command({'type': 'list_panes'})['panes'] if p.get('session_id') == session]
assert len(source) == 1 and source[0]['incarnation'] == incarnation and source[0]['child_pid'] == child
source = source[0]
assert source['input_observation']['has_unsent_user_input'] is False
assert runtime_path.exists() and runtime['runtime_incarnation'] == incarnation
prefs_path = home / '.gwt/projects' / endpoint['scope']['repo_hash'] / 'project-state/issue-monitor.json'
lock_path = home / '.gwt/sessions' / ('.runtime-writer-' + str(host) + '-' + session + '.json.lock')
with lock_path.open('r+') as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    try:
        # The existing target read remains before the reservation; only release it after holding the runtime writer.
        (home / 'target-refresh.release').write_text('')
        until = time.monotonic() + 30
        while True:
            intent = json.loads(prefs_path.read_text())['agent_switch_intents'][operation]
            if intent['stage'] == 'checkpointing':
                break
            assert intent['stage'] == 'requested' and time.monotonic() < until, 'reservation did not reach Checkpointing'
            time.sleep(0.01)
        assert intent['runtime_stop']['session_id'] == session
        assert intent['source_input_revision'] == source['input_observation']['revision']
        ack = command({'type': 'input', 'id': source['id'], 'incarnation': incarnation,
            'data': list(b'GWT_RAW_MONITOR_PROBE')})
        assert ack['ok'] and ack['input_observation']['has_unsent_user_input']
        assert ack['input_observation']['revision'] == intent['source_input_revision'] + 1
        print(json.dumps({'checkpointing': intent, 'input_ack': ack, 'host_pid': host,
            'host_started_at': health['host_started_at'], 'source_id': source['id'], 'incarnation': incarnation}), flush=True)
    finally:
        fcntl.flock(lock, fcntl.LOCK_UN)`, fixture.home, fixture.project,
          pending.request.operation_id, String(held.parent), source.gwt_session_id!, String(source.pid)],
        { stdio: ["ignore", "pipe", "pipe"] });
        let stdout = "", stderr = "";
        gate.stdout.on("data", data => { stdout += data; });
        gate.stderr.on("data", data => { stderr += data; });
        const result = new Promise<number | null>((resolve, reject) => {
          gate.once("error", reject);
          gate.once("close", resolve);
        });
        const watchdog = setTimeout(() => gate.kill("SIGTERM"), 35_000);
        let code;
        try { code = await result; } finally { clearTimeout(watchdog); }
        expect(code, `queued input barrier failed: ${stderr}`).toBe(0);
        const proof = JSON.parse(stdout);
        expect(proof.checkpointing).toMatchObject({ stage: "checkpointing", source: pending.source,
          runtime_stop: pending.runtime_stop, source_input_revision: expect.any(Number) });
        expect(proof.input_ack.input_observation).toMatchObject({ has_unsent_user_input: true,
          revision: proof.checkpointing.source_input_revision + 1 });
        const reason = "PTY I/O error: PTY I/O error: ordinary input changed before protected body commit";
        await expect.poll(async () => (await preferences()).agent_switch_intents[pending.request.operation_id],
          { timeout: 30_000 }).toMatchObject({ stage: "failed", reason });
        const assertRetained = async () => {
          const prefs = await preferences();
          const failed = prefs.agent_switch_intents[pending.request.operation_id];
          expect(failed).toMatchObject({ stage: "failed", reason, source: pending.source, runtime_stop: pending.runtime_stop });
          expect(failed.checkpoint).toBeUndefined();
          expect(failed.runtime_exit_confirmed).not.toBe(true);
          expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
          expect(prefs.launched_claims[43]).toBe(pending.source.claim_id);
          expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
            .assignment).toEqual(pending.source.assignment);
          expect(fixture.processAlive(source.pid)).toBe(true);
          expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
          expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
          expect(await fixture.launchIssues()).toEqual([43]);
          expect(fixture.requests.filter(request => request.method === "POST"
            && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
          const current = fixture.savedSource(source.cwd);
          expect(current).toMatchObject({ head: saved.head, remoteHead: saved.remoteHead, branch: saved.branch });
          expect(current.porcelain).not.toContain("checkpoint-source.txt");
          const retained = (await fixture.workItems()).work_items.find((value: any) => value.id === work.id);
          expect(retained.events).toEqual(expect.arrayContaining(work.events));
          expect(retained.status_category).not.toMatch(/done|blocked/);
          expect((await fixture.actorEvents()).filter((entry: any) =>
            ["checkpoint_before_prompt", "checkpoint_received", "checkpoint_acked", "actor_failed"].includes(entry.stage))).toEqual([]);
        };
        await assertRetained();
        expect(await session(source.gwt_session_id!)).toBe(before);
        await expect.poll(() => page.evaluate(id =>
          ((window as any).__gwtTerminalTestApi?.bufferText(id) ?? "").split("GWT_RAW_MONITOR_PROBE").length - 1,
        projectedSource.id), { timeout: 30_000 }).toBe(2);
        expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor")).toHaveLength(1);
        // Submit the retained queued draft once; no body is rewritten to make this pass.
        await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: "\r" });
        await expect.poll(async () => (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").length,
          { timeout: 30_000 }).toBe(2);
        await assertRetained();
        await expect(plans.locator(".roster-switch-status")).toContainText(reason);
        await info.attach("reserved-body-input-proof", { body: JSON.stringify({ held, pending, proof, saved,
          failed: (await preferences()).agent_switch_intents[pending.request.operation_id] }), contentType: "application/json" });
        await info.attach(`reserved-body-input-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
        expect(errors).toEqual([]);
        return;
      }
      if (gatedInput) {
        await expect.poll(() => fixture.targetRefreshGate.waiting(), { timeout: 30_000 }).not.toBeNull();
        const held = await fixture.targetRefreshGate.waiting();
        const pending = Object.values((await preferences()).agent_switch_intents)[0] as any;
        expect(pending.stage).toBe("requested");
        expect(held.parent).toBe(pending.runtime_stop.host_pid);
        const saved = fixture.savedSource(source.cwd);
        const work = (await fixture.workItems()).work_items.find((value: any) =>
          value.events.some((event: any) => event.agent_session_id === source.gwt_session_id));
        expect(work).toBeTruthy();
        await writeFile(`${bodyGate}.armed.tmp`, JSON.stringify({ root: fixture.project, operation: pending.request.operation_id,
          phase: afterAck === "before body cancel" ? "before_body" : preBody ? "after_reservation" : "after_body",
          session_id: source.gwt_session_id, incarnation: pending.runtime_stop.runtime_incarnation,
          observe_raw: afterAck === "body submit" }));
        await rename(`${bodyGate}.armed.tmp`, `${bodyGate}.armed`);
        await fixture.targetRefreshGate.release();
        await expect.poll(() => readFile(`${bodyGate}.reached`, "utf8").then(JSON.parse).catch(() => null),
          { timeout: 30_000 }).toMatchObject({ operation: pending.request.operation_id, host_pid: held.parent,
            incarnation: pending.runtime_stop.runtime_incarnation,
            phase: afterAck === "before body cancel" ? "before_body" : preBody ? "after_reservation" : "after_body" });
        const reached = JSON.parse(await readFile(`${bodyGate}.reached`, "utf8"));
        if (preBody) {
          let successorState: any;
          const sourceContents = await readFile(join(source.cwd, "source-in-progress.txt"), "utf8");
          expect(reached.body_bytes).toBe(0);
          expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "checkpoint_before_prompt")).toEqual([]);
          if (afterAck === "before body cancel") {
            expect((await preferences()).agent_switch_intents[pending.request.operation_id].stage).toBe("checkpointing");
            await plans.getByRole("button", { name: "Cancel switch", exact: true }).click();
            await expect.poll(async () => (await preferences()).agent_switch_intents[pending.request.operation_id].stage).toBe("cancelled");
          } else if (afterAck === "claim replaced") {
            successorState = await replaceClaimedSource(pending);
          } else if (afterAck === "source replaced") {
            // Explicit stop protects the target queue while this test performs a real source replacement.
            await sendLiveGwtEvent(page, { kind: "set_issue_monitor_enabled", enabled: false });
            await expect.poll(async () => (await preferences()).enabled).toBe(false);
            await sendLiveGwtEvent(page, { kind: "stop_window", id: projectedSource.id });
            await expect.poll(() => fixture.processAlive(source.pid), { timeout: 30_000 }).toBe(false);
            await expect.poll(() => fixture.processAlive(idle.tree_child_pid), { timeout: 30_000 }).toBe(false);
            await expect.poll(() => sourceWindow(), { timeout: 30_000 }).toMatchObject({ status: "stopped" });
            const continued = await request({ kind: "continue_work", operation_id: (await import("node:crypto")).randomUUID(), work_id: work.id }, "continue_work_outcome");
            expect(continued.error_code, JSON.stringify(continued)).toBeFalsy();
            expect(continued.outcome).toBe("continued_conversation");
            await expect.poll(async () => (await fixture.launches()).length, { timeout: 30_000 }).toBe(2);
            const successor = (await fixture.launches())[1];
            expect(successor.pid).not.toBe(source.pid);
            expect(successor.gwt_session_id).not.toBe(source.gwt_session_id);
            expect(successor.provider_session_id).toBe(source.provider_session_id);
            expect(fixture.processAlive(successor.pid)).toBe(true);
            await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) => entry.stage === "successor_ready" && entry.pid === successor.pid && entry.gwt_session_id === successor.gwt_session_id),
              { timeout: 30_000 }).toBe(true);
            await expect.poll(async () => (await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)?.assignment,
              { timeout: 30_000 }).toMatchObject({ session_id: successor.gwt_session_id });
            const native = JSON.parse(await readFile(join(fixture.home, ".gwt/sessions/runtime", String(held.parent), `${successor.gwt_session_id}.json`), "utf8"));
            expect(native.runtime_incarnation).not.toBe(reached.incarnation);
            expect(native.child_pid).toBe(successor.pid);
            expect(native.current_hook_event).toMatchObject({ event: "UserPromptSubmit", outcome: "normal" });
            expect(native.status).toBe("Running");
            successorState = {
              native: { incarnation: native.runtime_incarnation, child: native.child_pid, identity: native.execution_identity },
              assignment: (await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment,
              claim: (await preferences()).launched_claims?.[43] ?? null,
            };
          } else {
            pendingDraftOperation = pending.request.operation_id;
            await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: "GWT_RAW_MONITOR_PROBE\r" });
            // This read is passive: acceptance must finish while the existing input reservation is held.
            const paths: string[] = [];
            for (const hash of await readdir(join(fixture.home, ".gwt/projects"))) {
              const dir = join(fixture.home, ".gwt/projects", hash, "runtime/daemon");
              for (const file of await readdir(dir).catch(() => [])) if (file.endsWith(".json")) paths.push(join(dir, file));
            }
            const endpoints = await Promise.all(paths.map(async path => JSON.parse(await readFile(path, "utf8"))));
            const endpoint = endpoints.find(value => value.pid === held.parent && value.scope.project_root === fixture.project);
            expect(endpoint).toBeTruthy();
            await expect.poll(async () => {
              const response = await fetch(`${endpoint.project_transport_url}/private/project/command`, {
                method: "POST", headers: { Authorization: `Bearer ${endpoint.auth_token}`, "Content-Type": "application/json" },
                body: JSON.stringify({ project_key: endpoint.scope.repo_hash, command: { type: "list_panes" } }),
              });
              expect(response.status).toBe(200);
              const result = await response.json();
              return result.panes.find((pane: any) => pane.session_id === source.gwt_session_id)?.input_observation;
            }, { timeout: 30_000 }).toMatchObject({ revision: reached.source_input_revision + 1, has_unsent_user_input: false });
            expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor")).toHaveLength(1);
          }
          await writeFile(`${bodyGate}.release`, pending.request.operation_id);
          await expect.poll(() => readFile(`${bodyGate}.finished`, "utf8").then(JSON.parse).catch(() => null),
            { timeout: 30_000 }).toMatchObject({ operation: pending.request.operation_id, error: expect.any(String) });
          const finished = JSON.parse(await readFile(`${bodyGate}.finished`, "utf8"));
          if (afterAck === "reserved submit") {
            expect(finished.error).toContain("input to settle");
            await expect.poll(async () => (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").length,
              { timeout: 30_000 }).toBe(2);
            const bytes = Buffer.from((await fixture.actorEvents()).filter((entry: any) => entry.stage === "stdin_bytes" && entry.pid === source.pid).flatMap((entry: any) => entry.bytes));
            expect(bytes.subarray(0, "GWT_RAW_MONITOR_PROBE\r".length * 2).toString()).toBe("GWT_RAW_MONITOR_PROBE\rGWT_RAW_MONITOR_PROBE\r");
            await info.attach("reserved-cr-refusal", { body: JSON.stringify({ reached, finished }), contentType: "application/json" });
          } else {
            let current = (await preferences()).agent_switch_intents[pending.request.operation_id];
            if (afterAck === "before body cancel") {
              expect(finished.error).toBe("Switch cancelled or source changed before checkpoint delivery");
              expect(current.stage).toBe("cancelled");
              expect(fixture.processAlive(source.pid)).toBe(true);
              expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
              expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
              expect((await preferences()).launched_claims[43]).toBe(pending.source.claim_id);
              expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment).toEqual(pending.source.assignment);
            } else {
              await expect.poll(async () => (await preferences()).agent_switch_intents[pending.request.operation_id].stage,
                { timeout: 30_000 }).toMatch(/failed|cancelled/);
              current = (await preferences()).agent_switch_intents[pending.request.operation_id];
              if (afterAck === "claim replaced") {
                await assertClaimedSuccessor(pending, successorState);
              } else {
                const successor = (await fixture.launches())[1];
                expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
                const assignment = (await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment;
                expect(assignment.session_id).toBe(successor.gwt_session_id);
                expect(assignment.reservation_id).not.toBe(pending.source.assignment.reservation_id);
                expect(assignment).toEqual(successorState.assignment);
                expect((await preferences()).launched_claims?.[43] ?? null).toBe(successorState.claim);
                const native = JSON.parse(await readFile(join(fixture.home, ".gwt/sessions/runtime", String(held.parent), `${successor.gwt_session_id}.json`), "utf8"));
                expect({ incarnation: native.runtime_incarnation, child: native.child_pid, identity: native.execution_identity }).toEqual(successorState.native);
                expect(fixture.processAlive(successor.pid)).toBe(true);
                expect(await fixture.launchIssues()).toEqual([43, 43]);
              }
            }
            const events = await fixture.actorEvents();
            const originalBytes = Buffer.from(events.filter((entry: any) => entry.stage === "stdin_bytes" && entry.pid === source.pid).flatMap((entry: any) => entry.bytes));
            expect(originalBytes.toString()).toBe("GWT_RAW_MONITOR_PROBE\r");
            if (afterAck === "source replaced" || afterAck === "claim replaced") {
              const successor = (await fixture.launches())[1];
              expect(events.filter((entry: any) => entry.stage === "stdin_bytes" && entry.pid === successor.pid)).toEqual([]);
            }
            expect(events.filter((entry: any) => ["checkpoint_before_prompt", "checkpoint_received", "checkpoint_acked"].includes(entry.stage))).toEqual([]);
            expect(events.filter((entry: any) => ["actor_failed", "operation_refused"].includes(entry.stage))).toEqual([]);
            expect(current.checkpoint).toBeUndefined();
            expect(current.runtime_exit_confirmed).not.toBe(true);
            expect((await preferences()).monitor_runtime_stop_requests || []).toEqual([]);
            const finalSource = fixture.savedSource(source.cwd);
            expect(await readFile(join(source.cwd, "source-in-progress.txt"), "utf8")).toBe(sourceContents);
            if (afterAck === "claim replaced") {
              expect(finalSource).toMatchObject({ branch: saved.branch, head: saved.head, remoteHead: saved.remoteHead });
            } else if (afterAck === "source replaced") {
              expect({ ...finalSource, porcelain: saved.porcelain }).toEqual(saved);
              expect(finalSource.porcelain.split("\n")).toEqual(expect.arrayContaining(saved.porcelain.split("\n")));
              const added = finalSource.porcelain.split("\n").filter((line: string) => !saved.porcelain.split("\n").includes(line));
              expect(added.length).toBeGreaterThan(0);
              for (const line of added) {
                expect(line).toMatch(/^\?\? \.gwt\/work\/events\/[0-9a-f]{2}\/[0-9a-f]{64}\.jsonl$/);
                const events = (await readFile(join(source.cwd, line.slice(3)), "utf8")).trim().split("\n").map(line => JSON.parse(line));
                expect(events.every((event: any) => event.agent_session_id === successorState.assignment.session_id)).toBe(true);
              }
            } else expect(finalSource).toEqual(saved);
            const retained = (await fixture.workItems()).work_items.find((value: any) => value.id === work.id);
            expect(retained.events).toEqual(expect.arrayContaining(work.events));
            expect(retained.status_category).not.toMatch(/done|blocked/);
            expect(fixture.requests.filter(request => request.method === "POST" && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
            await info.attach("pre-body-refusal", { body: JSON.stringify({ reached, finished, current,
              successor: successorState && { ...successorState, session: undefined } }), contentType: "application/json" });
            await info.attach(`pre-body-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
            expect(errors).toEqual([]);
            return;
          }
        } else {
        const eventsAtBody = await fixture.actorEvents();
        expect(eventsAtBody.filter((entry: any) => entry.stage === "stdin_line"
          && entry.line.includes("GWT checkpoint request"))).toEqual([]);
        const sourceBytes = (events: any[]) => Buffer.from(events.filter(entry => entry.stage === "stdin_bytes"
          && entry.pid === source.pid).flatMap(entry => entry.bytes));
        await expect.poll(async () => {
          const bytes = sourceBytes(await fixture.actorEvents());
          const start = bytes.indexOf("GWT checkpoint request");
          return start < 0 ? 0 : bytes.length - start;
        }, { timeout: 30_000 }).toBe(reached.body_bytes);
        const bodyBytes = sourceBytes(await fixture.actorEvents());
        expect(bodyBytes.subarray(bodyBytes.indexOf("GWT checkpoint request")).includes(13)).toBe(false);
        expect(bodyBytes.subarray(bodyBytes.indexOf("GWT checkpoint request")).includes(10)).toBe(false);
        expect((await preferences()).agent_switch_intents[pending.request.operation_id].stage).toBe("checkpointing");
        const endpointDirs = await readdir(join(fixture.home, ".gwt/projects"));
        const endpoints: any[] = [];
        for (const hash of endpointDirs) {
          const directory = join(fixture.home, ".gwt/projects", hash, "runtime/daemon");
          for (const file of await readdir(directory).catch(() => [])) {
            if (file.endsWith(".json")) endpoints.push(JSON.parse(await readFile(join(directory, file), "utf8")));
          }
        }
        const endpoint = endpoints.find(value => value.pid === held.parent && value.scope.project_root === fixture.project);
        expect(endpoint).toBeTruthy();
        expect(new URL(endpoint.project_transport_url).hostname).toBe("127.0.0.1");
        const sourceInput = async () => {
          const response = await fetch(`${endpoint.project_transport_url}/private/project/command`, {
            method: "POST", headers: { Authorization: `Bearer ${endpoint.auth_token}`, "Content-Type": "application/json" },
            body: JSON.stringify({ project_key: endpoint.scope.repo_hash, command: { type: "list_panes" } }),
          });
          expect(response.status).toBe(200);
          const result = await response.json();
          return result.panes.find((pane: any) => pane.session_id === source.gwt_session_id)?.input_observation;
        };
        if (afterAck === "body cancel") {
          await plans.getByRole("button", { name: "Cancel switch", exact: true }).click();
          await expect.poll(async () => (await preferences()).agent_switch_intents[pending.request.operation_id].stage,
            { timeout: 30_000 }).toBe("cancelled");
        } else if (afterAck === "body submit") {
          const endpointPath = join(fixture.home, ".gwt/projects", endpoint.scope.repo_hash, "runtime/daemon");
          coordinator = spawn(process.execPath, ["-e", String.raw`
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const [home, gate, endpointDir, host, session, incarnation, revision] = process.argv.slice(1);
const endpoint = fs.readdirSync(endpointDir).filter(f => f.endsWith('.json'))
  .map(f => JSON.parse(fs.readFileSync(path.join(endpointDir, f)))).find(e => e.pid === Number(host));
assert(endpoint && new URL(endpoint.project_transport_url).hostname === '127.0.0.1');
const armed = JSON.parse(fs.readFileSync(gate + '.claimed'));
const runtimePath = path.join(home, '.gwt/sessions/runtime', host, session + '.json');
const before = JSON.parse(fs.readFileSync(runtimePath));
const actorBytes = () => fs.readFileSync(path.join(home, 'switch-actor.jsonl'), 'utf8').trim().split('\n')
  .map(line => JSON.parse(line)).filter(e => e.stage === 'stdin_bytes' && e.gwt_session_id === session).flatMap(e => e.bytes);
const bytesBefore = actorBytes();
const command = async value => {
  const response = await fetch(endpoint.project_transport_url + '/private/project/command', {
    signal: AbortSignal.timeout(30000), method:'POST',
    headers:{Authorization:'Bearer ' + endpoint.auth_token, 'Content-Type':'application/json'},
    body:JSON.stringify({project_key:endpoint.scope.repo_hash,command:value})});
  const result = await response.json();
  assert.equal(response.status,200,JSON.stringify(result));
  return result;
};
(async () => {
const nativePane = (await command({type:'list_panes'})).panes.find(p => p.session_id === session);
assert.equal(nativePane.incarnation,Number(incarnation));
const request = fetch(endpoint.project_transport_url + '/private/project/command', {
  signal: AbortSignal.timeout(30000), method: 'POST', headers: {Authorization:'Bearer ' + endpoint.auth_token, 'Content-Type':'application/json'},
  body: JSON.stringify({project_key:endpoint.scope.repo_hash, command:{type:'input',id:nativePane.id,
    incarnation:Number(incarnation),data:Array.from(Buffer.from('GWT_RAW_MONITOR_PROBE\r'))}})
}).then(async response => { const result = await response.json(); assert.equal(response.status,200,JSON.stringify(result)); return result; });
request.catch(() => {});
  const deadline = Date.now() + 30000;
  let entered;
  try {
    while (!fs.existsSync(gate + '.raw-entered')) {
      assert(Date.now() < deadline, 'raw confirming handler not entered');
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    entered = JSON.parse(fs.readFileSync(gate + '.raw-entered'));
    assert.equal(entered.operation, armed.operation);
    assert.equal(entered.input_revision, Number(revision));
    assert(!fs.existsSync(gate + '.raw-accepted'), 'input accepted while checkpoint owns Session');
    assert.deepEqual(JSON.parse(fs.readFileSync(runtimePath)).current_hook_event, before.current_hook_event);
    assert.deepEqual(actorBytes(), bytesBefore, 'ordinary bytes interleaved into checkpoint body');
  } finally {
    // No browser round trip consumes the two-second Session lease wait.
    fs.writeFileSync(gate + '.release', armed.operation);
  }
  const response = await request;
  assert(response.ok, response.error);
  const accepted = JSON.parse(fs.readFileSync(gate + '.raw-accepted'));
  assert.equal(accepted.operation, armed.operation);
  assert.equal(accepted.input_revision, Number(revision) + 1);
  console.log(JSON.stringify({entered, accepted, input:response.input_observation, bytes_before:bytesBefore.length}));
})().catch(error => { console.error(error.stack); process.exitCode=1; });
`, fixture.home, bodyGate, endpointPath, String(held.parent), source.gwt_session_id!,
            String(reached.incarnation), String(reached.source_input_revision)], { stdio: ["ignore", "pipe", "pipe"] });
          let output = "", error = "";
          coordinator.stdout.on("data", data => { output += data; });
          coordinator.stderr.on("data", data => { error += data; });
          coordinatorClosed = new Promise<number | null>((resolve, reject) => {
            coordinator!.once("error", reject); coordinator!.once("close", resolve);
          });
          const code = await coordinatorClosed;
          expect(code, error).toBe(0);
          const receipt = JSON.parse(output);
          expect(receipt.input).toMatchObject({ revision: reached.source_input_revision + 1, has_unsent_user_input: false });
          await info.attach("checkpoint-first-cr", { body: output, contentType: "application/json" });
        } else {
          // Non-CR input may enqueue while the checkpoint holds its Session
          // lease. It must not reach the physical writer until its CR completes.
          await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: "GWT_RAW_MONITOR_PROBE" });
          await expect.poll(sourceInput, { timeout: 30_000 }).toMatchObject({
            revision: reached.source_input_revision + 1, has_unsent_user_input: true,
          });
        }
        if (afterAck !== "body submit") {
          const whileHeld = sourceBytes(await fixture.actorEvents());
          expect(whileHeld).toEqual(bodyBytes);
          expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "checkpoint_before_prompt")).toEqual([]);
          await writeFile(`${bodyGate}.release`, pending.request.operation_id);
        }
        if (afterAck === "body cancel") {
          await expect.poll(async () => !!await actor("checkpoint_cancelled"), { timeout: 60_000 }).toBe(true);
        } else {
          await expect.poll(async () => !!await actor("checkpoint_acked"), { timeout: 60_000 }).toBe(true);
          const ack = await actor("checkpoint_acked");
          expect(ack.operation).toBe(pending.request.operation_id);
          await releaseNormalStop(ack.operation);
          await expect.poll(async () => !!await actor("normal_n2_hook_returned"), { timeout: 60_000 }).toBe(true);
          await expect.poll(async () => (await preferences()).agent_switch_intents[pending.request.operation_id],
            { timeout: 30_000 }).toMatchObject({ stage: "failed",
              reason: "New input invalidated the saved checkpoint. Current work is kept; request another switch." });
          // The queued draft is retained after the checkpoint's CR, and only
          // this later explicit CR submits it as a separate ordinary line.
          if (afterAck !== "body submit") await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: "\r" });
          await expect.poll(async () => (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").length,
            { timeout: 60_000 }).toBe(2);
        }
        const events = await fixture.actorEvents();
        const lines = events.filter((entry: any) => entry.stage === "stdin_line" && entry.pid === source.pid).map((entry: any) => entry.line);
        const checkpointLines = lines.filter((line: string) => line.includes("GWT checkpoint request"));
        expect(checkpointLines).toHaveLength(1);
        expect(checkpointLines[0]).toContain(pending.request.operation_id);
        expect(checkpointLines[0]).not.toContain("GWT_RAW_MONITOR_PROBE");
        const bytes = sourceBytes(events);
        const checkpointBytes = bytes.subarray(bytes.indexOf("GWT checkpoint request"));
        expect(checkpointBytes.subarray(0, reached.body_bytes).toString()).toBe(checkpointLines[0]);
        expect(checkpointBytes[reached.body_bytes]).toBe(13);
        expect(checkpointBytes.subarray(reached.body_bytes).toString()).toBe(afterAck === "body cancel"
          ? "\r" : "\rGWT_RAW_MONITOR_PROBE\r");
        expect(lines.filter((line: string) => line === "GWT_RAW_MONITOR_PROBE")).toHaveLength(afterAck === "body cancel" ? 1 : 2);
        const current = (await preferences()).agent_switch_intents[pending.request.operation_id];
        expect(current.stage).toBe(afterAck === "body cancel" ? "cancelled" : "failed");
        expect(current.source).toEqual(pending.source);
        expect(current.runtime_exit_confirmed).not.toBe(true);
        expect((await preferences()).monitor_runtime_stop_requests || []).toEqual([]);
        expect((await preferences()).launched_claims[43]).toBe(pending.source.claim_id);
        expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
          .assignment).toEqual(pending.source.assignment);
        expect(fixture.processAlive(source.pid)).toBe(true);
        expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
        expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
        expect(await fixture.launchIssues()).toEqual([43]);
        expect(fixture.requests.filter(request => request.method === "POST"
          && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
        const retained = (await fixture.workItems()).work_items.find((value: any) => value.id === work.id);
        expect(retained.events).toEqual(expect.arrayContaining(work.events));
        expect(retained.status_category).not.toMatch(/done|blocked/);
        if (afterAck === "body cancel") {
          expect(current.checkpoint).toBeUndefined();
          expect(fixture.savedSource(source.cwd)).toEqual(saved);
          expect(events.filter((entry: any) => ["checkpoint_received", "checkpoint_acked"].includes(entry.stage))).toEqual([]);
        }
        expect(events.filter((entry: any) => entry.stage === "actor_failed")).toEqual([]);
        await info.attach("post-body-proof", { body: JSON.stringify({ reached, pending, current, lines }), contentType: "application/json" });
        await info.attach(`post-body-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
        expect(errors).toEqual([]);
        return;
        }
      }
      let cancelledDuringRefresh: any;
      if (refreshDraft || afterAck === "refresh submit") {
        await expect.poll(() => fixture.targetRefreshGate.waiting(), { timeout: 30_000 }).not.toBeNull();
        const held = await fixture.targetRefreshGate.waiting();
        const pending = Object.values((await preferences()).agent_switch_intents)[0] as any;
        expect(pending.stage).toBe("requested");
        expect(pending.reason ?? "").not.toMatch(/^Waiting for the current turn/);
        expect(held.argv.slice(0, 3)).toEqual(["issue", "view", "44"]);
        expect(held.parent, "the captured Switch Host owns the blocked target read").toBe(pending.runtime_stop.host_pid);
        expect(fixture.processAlive(held.pid)).toBe(true);
        pendingDraftOperation = pending.request.operation_id;
        await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: refreshSubmit ? "GWT_RAW_MONITOR_PROBE\r" : "GWT_RAW_MONITOR_PROBE" });
        if (refreshSubmit) {
          await expect.poll(async () => (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").length,
            { timeout: 30_000 }).toBe(2);
          const events = await fixture.actorEvents();
          const accepted = events.filter((entry: any) => entry.stage === "raw_accepted").at(-1);
          const successor = events.filter((entry: any) => entry.stage === "raw_successor").at(-1);
          expect(accepted).toMatchObject({ pid: source.pid, gwt_session_id: source.gwt_session_id, current_hook_event: null });
          expect(successor).toMatchObject({ pid: source.pid, gwt_session_id: source.gwt_session_id,
            current_hook_event: { event: "Stop", outcome: "normal" } });
          expect(successor.current_hook_event.correlation_id).not.toBe(n1.current_hook_event.correlation_id);
          const prefs = await preferences();
          expect(prefs.agent_switch_intents[pendingDraftOperation!]).toEqual(pending);
          expect(pending.checkpoint).toBeUndefined();
          expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
          expect(prefs.launched_claims[43]).toBe(pending.source.claim_id);
          expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
            .assignment).toEqual(pending.source.assignment);
          // The genuine Stop advances activity timestamps; all other Session fields stay exact.
          const withoutActivityTimes = (record: string) => record.replace(
            /^(?:updated_at|last_activity_at|last_hook_event_at|last_completed_stop_at) = .*\n/gm, "");
          expect(withoutActivityTimes(await session(source.gwt_session_id!))).toBe(withoutActivityTimes(before));
          expect(fixture.processAlive(source.pid)).toBe(true);
          expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
          expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
          expect(await fixture.launchIssues()).toEqual([43]);
          expect(fixture.requests.filter(request => request.method === "POST"
            && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
          expect(events.filter((entry: any) => ["checkpoint_before_prompt", "checkpoint_received", "checkpoint_acked"].includes(entry.stage))).toEqual([]);
          const work = (await fixture.workItems()).work_items.find((value: any) =>
            value.events.some((event: any) => event.agent_session_id === source.gwt_session_id));
          expect(work).toBeTruthy();
          expect(work.status_category).not.toMatch(/done|blocked/);
          expect(fixture.savedSource(source.cwd).porcelain).not.toContain("checkpoint-source.txt");
          await info.attach("submitted-during-target-refresh", { body: JSON.stringify({ held, pending, accepted, successor, work }),
            contentType: "application/json" });
          await info.attach(`submitted-during-target-refresh-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
        }
        await expect.poll(() => page.evaluate(id =>
          ((window as any).__gwtTerminalTestApi?.bufferText(id) ?? "").split("GWT_RAW_MONITOR_PROBE").length - 1,
        projectedSource.id), { timeout: 30_000, message: "the blocked target read permits real terminal input" }).toBe(2);
        await fixture.targetRefreshGate.release();
        await expect.poll(() => fixture.targetRefreshGate.returned(), { timeout: 30_000 }).toBe(true);
        // A new reason proves the driver resumed beyond the held read and rejected its old snapshot.
        if (refreshDraft) await expect.poll(async () => (await preferences()).agent_switch_intents[pendingDraftOperation!],
          { timeout: 30_000 }).toMatchObject({ stage: "requested", reason: expect.stringMatching(/^Waiting for the current turn (?:and input to settle|to finish)\./) });
        await info.attach("draft-during-target-refresh", { body: JSON.stringify({ held, pending }),
          contentType: "application/json" });
      }
      if (afterAck === "refresh cancel") {
        await expect.poll(() => fixture.targetRefreshGate.waiting(), { timeout: 30_000 }).not.toBeNull();
        const held = await fixture.targetRefreshGate.waiting();
        const prefs = await preferences();
        const pending = Object.values(prefs.agent_switch_intents)[0] as any;
        expect(pending.stage).toBe("requested");
        expect(held.argv.slice(0, 3)).toEqual(["issue", "view", "44"]);
        expect(held.parent, "the captured Switch Host owns the blocked target read").toBe(pending.runtime_stop.host_pid);
        expect(fixture.processAlive(held.pid)).toBe(true);
        await plans.getByRole("button", { name: "Cancel switch", exact: true }).click();
        await expect.poll(async () => (await preferences()).agent_switch_intents[pending.request.operation_id]?.stage)
          .toBe("cancelled");
        cancelledDuringRefresh = (await preferences()).agent_switch_intents[pending.request.operation_id];
        await fixture.targetRefreshGate.release();
        await expect.poll(() => fixture.targetRefreshGate.returned(), { timeout: 30_000 }).toBe(true);
        const retained = await preferences();
        expect(retained.agent_switch_intents[pending.request.operation_id]).toEqual(cancelledDuringRefresh);
        expect(cancelledDuringRefresh.checkpoint).toBeUndefined();
        expect(cancelledDuringRefresh.runtime_exit_confirmed).not.toBe(true);
        expect(retained.monitor_runtime_stop_requests || []).toEqual([]);
        expect(retained.launched_claims[43]).toBe(pending.source.claim_id);
        expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
          .assignment).toEqual(pending.source.assignment);
        expect(await session(source.gwt_session_id!)).toBe(before);
        expect(fixture.processAlive(source.pid)).toBe(true);
        expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
        expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
        const work = (await fixture.workItems()).work_items.find((value: any) =>
          value.events.some((event: any) => event.agent_session_id === source.gwt_session_id));
        expect(work).toBeTruthy();
        expect(work.status_category).not.toMatch(/done|blocked/);
        expect(fixture.savedSource(source.cwd).porcelain).not.toContain("checkpoint-source.txt");
        expect(await fixture.launchIssues()).toEqual([43]);
        expect(fixture.requests.filter(request => request.method === "POST"
          && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
        expect((await fixture.actorEvents()).filter((entry: any) =>
          ["checkpoint_before_prompt", "checkpoint_received", "checkpoint_acked"].includes(entry.stage))).toEqual([]);
        await info.attach("cancelled-target-refresh", { body: JSON.stringify({ held, cancelledDuringRefresh }),
          contentType: "application/json" });
        await info.attach(`cancelled-target-refresh-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
        // A fresh request must work after the cancelled worker finishes its read.
        await expect(plans.locator('[data-action="switch-next"]')).toBeEnabled();
        await plans.locator('[data-action="switch-next"]').click();
      }
      if (initialDraft) {
        await expect.poll(async () => Object.values((await preferences()).agent_switch_intents || {}),
          { timeout: 30_000 }).toEqual([expect.objectContaining({ stage: "requested",
            reason: refreshDraft ? expect.stringMatching(/^Waiting for the current turn (?:and input to settle|to finish)\./)
              : "Waiting for the current turn to finish. Current work is kept." })]);
        const prefs = await preferences();
        const pending = Object.values(prefs.agent_switch_intents)[0] as any;
        if (refreshDraft) expect(pending.request.operation_id).toBe(pendingDraftOperation);
        pendingDraftOperation = pending.request.operation_id;
        expect(pending.checkpoint).toBeUndefined();
        expect(pending.runtime_exit_confirmed).not.toBe(true);
        expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
        expect(prefs.launched_claims[43]).toBe(pending.source.claim_id);
        expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
          .assignment).toEqual(pending.source.assignment);
        expect(await session(source.gwt_session_id!)).toBe(before);
        expect(fixture.processAlive(source.pid)).toBe(true);
        expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
        expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
        expect(await fixture.launchIssues()).toEqual([43]);
        expect(fixture.requests.filter(request => request.method === "POST"
          && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
        const events = await fixture.actorEvents();
        expect(events.filter((entry: any) => entry.stage === "raw_successor")).toHaveLength(1);
        expect(events.filter((entry: any) => ["checkpoint_before_prompt", "checkpoint_received", "checkpoint_acked"].includes(entry.stage))).toEqual([]);
        const work = (await fixture.workItems()).work_items.find((value: any) =>
          value.events.some((event: any) => event.agent_session_id === source.gwt_session_id));
        expect(work).toBeTruthy();
        expect(work.status_category).not.toMatch(/done|blocked/);
        const unsaved = fixture.savedSource(source.cwd);
        expect(unsaved.porcelain).not.toContain("checkpoint-source.txt");
        await expect(plans.locator(".roster-switch-status")).toContainText(refreshDraft
          ? /Waiting for the current turn (?:and input to settle|to finish)/ : "Waiting for the current turn to finish");
        await info.attach("pre-request-draft-held", { body: JSON.stringify({ pending, work, unsaved }), contentType: "application/json" });
        await info.attach(`pre-request-draft-held-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
        // Submit only the existing composer body; no new Switch or refresh drives recovery.
        await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: "\r" });
        await expect.poll(async () => (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").length,
          { timeout: Math.max(1, handoffDeadline - Date.now()) }).toBe(2);
        const submitted = (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").at(-1);
        expect(submitted).toMatchObject({ pid: source.pid, gwt_session_id: source.gwt_session_id,
          current_hook_event: { event: "Stop", outcome: "normal" } });
        expect(submitted.current_hook_event.correlation_id).not.toBe(n1.current_hook_event.correlation_id);
      }
      await expect.poll(async () => !!await actor("awaiting_normal_n2"),
        { timeout: 60_000, message: "actual checkpoint ACK before the next genuine Normal Stop" }).toBe(true);
      const waiting = await actor("awaiting_normal_n2");
      if (cancelledDuringRefresh) {
        expect(waiting.operation).not.toBe(cancelledDuringRefresh.request.operation_id);
        expect((await preferences()).agent_switch_intents[cancelledDuringRefresh.request.operation_id])
          .toEqual(cancelledDuringRefresh);
        expect((await fixture.actorEvents()).filter((entry: any) =>
          entry.operation === cancelledDuringRefresh.request.operation_id)).toEqual([]);
      }
      if (initialDraft || refreshSubmit) expect(waiting.operation).toBe(pendingDraftOperation);
      const beforePrompt = await actor("checkpoint_before_prompt");
      const activity = await actor("checkpoint_activity");
      expect(beforePrompt.current_hook_event, "explicit checkpoint acceptance invalidates N1 before the next provider hook").toBeNull();
      expect(beforePrompt.runtime_sha256).not.toBe(n1.runtime_sha256);
      const previousNormal = initialDraft || refreshSubmit
        ? (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").at(-1) : n1;
      expect(beforePrompt.runtime_without_current_sha256, "acceptance preserves all history, activity and native source fields").toBe(previousNormal.runtime_without_current_sha256);
      expect(activity).toMatchObject({ pid: source.pid, gwt_session_id: source.gwt_session_id,
        runtime_status: "Running", current_hook_event: { event: "UserPromptSubmit", outcome: "normal" } });
      expect(activity.current_hook_event.correlation_id).not.toBe(n1.current_hook_event.correlation_id);
      expect(waiting.current_hook_event).toEqual(activity.current_hook_event);
      await expect.poll(async () => (await preferences()).agent_switch_intents[waiting.operation]?.reason,
        { timeout: 30_000 }).toMatch(/Waiting for the current turn to finish/);
      expect((await preferences()).monitor_runtime_stop_requests || []).toEqual([]);
      expect(fixture.processAlive(source.pid)).toBe(true);
      expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
      expect(await fixture.launches()).toHaveLength(1);
      if (afterAck === "stop claim replaced") {
        const acknowledged = (await preferences()).agent_switch_intents[waiting.operation];
        expect(acknowledged.stage).toBe("checkpoint_verified");
        const checkpoint = fixture.savedSource(source.cwd);
        expect(checkpoint.porcelain).toBe("");
        const savedWork = (await fixture.workItems()).work_items.find((value: any) => value.id === acknowledged.checkpoint.work_id);
        expect(savedWork.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
        // Holding gh issue view also holds the candidate readback used by
        // the replacement launch. Pause only after that read has completed,
        // with the genuine N2 and protected input reservation already captured.
        await writeFile(`${bodyGate}.armed.tmp`, JSON.stringify({ root: fixture.project, operation: waiting.operation,
          phase: "before_stop", session_id: source.gwt_session_id,
          incarnation: acknowledged.runtime_stop.runtime_incarnation }));
        await rename(`${bodyGate}.armed.tmp`, `${bodyGate}.armed`);
        await releaseNormalStop(waiting.operation);
        await expect.poll(() => readFile(`${bodyGate}.reached`, "utf8").then(JSON.parse).catch(() => null),
          { timeout: 30_000 }).toMatchObject({ operation: waiting.operation, phase: "before_stop", body_bytes: 0,
            host_pid: acknowledged.runtime_stop.host_pid, incarnation: acknowledged.runtime_stop.runtime_incarnation });
        const held = JSON.parse(await readFile(`${bodyGate}.reached`, "utf8"));
        const claimed = JSON.parse(await readFile(`${bodyGate}.claimed`, "utf8"));
        expect(claimed).toMatchObject({ root: fixture.project, operation: waiting.operation,
          phase: "before_stop", session_id: source.gwt_session_id, incarnation: acknowledged.runtime_stop.runtime_incarnation });
        await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
          entry.stage === "normal_n2_hook_returned" && entry.operation === waiting.operation),
        { timeout: 30_000 }).toBe(true);
        const replacement = await replaceClaimedSource(acknowledged);
        expect((await preferences()).agent_switch_intents[waiting.operation].stage).toBe("checkpoint_verified");
        await writeFile(`${bodyGate}.release`, waiting.operation);
        await expect.poll(async () => (await preferences()).agent_switch_intents[waiting.operation].stage,
          { timeout: 30_000 }).toBe("failed");
        const rejected = (await preferences()).agent_switch_intents[waiting.operation];
        expect(rejected).toMatchObject({ source: acknowledged.source, checkpoint: acknowledged.checkpoint,
          runtime_stop: acknowledged.runtime_stop,
          reason: "Source execution authority changed; no stop was committed" });
        expect(rejected.runtime_exit_confirmed).not.toBe(true);
        expect((await preferences()).monitor_runtime_stop_requests || []).toEqual([]);
        await assertClaimedSuccessor(acknowledged, replacement);
        expect(replacement.source).toMatchObject({ branch: checkpoint.branch, head: checkpoint.head, remoteHead: checkpoint.remoteHead });
        const retained = (await fixture.workItems()).work_items.find((value: any) => value.id === savedWork.id);
        expect(retained.events).toEqual(expect.arrayContaining(savedWork.events));
        expect(retained.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
        expect((await fixture.actorEvents()).filter((entry: any) => ["actor_failed", "operation_refused"].includes(entry.stage))).toEqual([]);
        await info.attach("claim-bearing-post-ack-replacement", { body: JSON.stringify({ acknowledged, held,
          rejected, replacement: { ...replacement, session: undefined } }), contentType: "application/json" });
        await info.attach(`claim-bearing-post-ack-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      } else if (afterAck === "restart") {
        const acknowledged = (await preferences()).agent_switch_intents[waiting.operation];
        expect(acknowledged.stage).toBe("checkpoint_verified");
        const checkpoint = fixture.savedSource(source.cwd);
        expect(checkpoint.porcelain).toBe("");
        const savedWork = (await fixture.workItems()).work_items.find((value: any) => value.id === acknowledged.checkpoint.work_id);
        expect(savedWork).toMatchObject({ owner: "Issue #43", status_category: "idle" });
        expect(savedWork.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
        const context = page.context();
        await page.close();
        // The fixture replaces its owned Host and cleans up its old providers;
        // their exit here is not a successful Switch stop.
        await fixture.restartRuntime(acknowledged.runtime_stop.host_pid);
        page = await context.newPage();
        page.on("pageerror", error => errors.push(error.message));
        page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
        await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
        await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await expect.poll(async () => (await preferences()).agent_switch_intents[waiting.operation]?.stage,
          { timeout: 45_000 }).toBe("cancelled");
        await expect.poll(async () => (await fixture.launches()).length, { timeout: 45_000 }).toBe(2);
        const restored = (await fixture.launches())[1];
        expect(restored.provider_session_id).toBe(source.provider_session_id);
        expect(restored.resume_session_id).toBe(source.provider_session_id);
        expect(restored.gwt_session_id).toBeTruthy();
        expect(restored.pid).not.toBe(source.pid);
        await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
          entry.stage === "idle" && entry.pid === restored.pid && entry.gwt_session_id === restored.gwt_session_id),
        { timeout: 60_000 }).toBe(true);
        const prefs = await preferences();
        const cancelled = prefs.agent_switch_intents[waiting.operation];
        expect(cancelled).toMatchObject({ stage: "cancelled", source: acknowledged.source,
          checkpoint: acknowledged.checkpoint, runtime_stop: acknowledged.runtime_stop });
        expect(cancelled.runtime_exit_confirmed).not.toBe(true);
        expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
        const assignment = (await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id).assignment;
        expect(assignment).toMatchObject({ issue_number: 43, session_id: restored.gwt_session_id });
        expect(assignment.host_pid).toBeGreaterThan(0);
        expect(assignment.host_pid).not.toBe(acknowledged.runtime_stop.host_pid);
        expect(fixture.processAlive(acknowledged.runtime_stop.host_pid)).toBe(false);
        expect(fixture.processAlive(source.pid)).toBe(false);
        expect(fixture.processAlive(idle.tree_child_pid)).toBe(false);
        expect(fixture.processAlive(restored.pid)).toBe(true);
        expect(await fixture.launchIssues()).toEqual([43, 43]);
        expect(fixture.requests.filter(request => request.method === "POST"
          && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
        expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
        const saved = fixture.savedSource(source.cwd);
        expect(saved).toMatchObject({ branch: checkpoint.branch, head: checkpoint.head, remoteHead: checkpoint.remoteHead });
        for (const line of saved.porcelain.split("\n").filter(Boolean))
          expect(line).toMatch(/^\?\? \.gwt\/work\/events\/[0-9a-f]{2}\/[0-9a-f]{64}\.jsonl$/);
        const work = (await fixture.workItems()).work_items.find((value: any) => value.id === savedWork.id);
        expect(work).toMatchObject({ owner: "Issue #43" });
        expect(["idle", "active"]).toContain(work.status_category);
        expect(work.events).toEqual(expect.arrayContaining(savedWork.events));
        expect(work.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
        const resumedNormal = (await fixture.actorEvents()).find((entry: any) => entry.stage === "normal_n1"
          && entry.pid === restored.pid && entry.gwt_session_id === restored.gwt_session_id);
        expect(resumedNormal.current_hook_event).toMatchObject({ event: "Stop", outcome: "normal" });
        expect((await fixture.actorEvents()).some((entry: any) => entry.stage === "normal_n2_started"
          && entry.operation === waiting.operation)).toBe(false);
        await page.locator('.op-rail__surface[data-surface="agents"]').click();
        const resumedAgents = page.getByRole("region", { name: "Main pane", exact: true })
          .locator(".agents-surface:not(.agents-surface--preview)");
        await resumedAgents.locator(`.roster-row[data-member-id="${member.agent_member_id}"]`).click();
        await expect(resumedAgents.locator(".roster-switch-status")).toContainText(/cancelled|Cancelled/);
        expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "actor_failed")).toEqual([]);
        await info.attach("post-ack-restart-preserved", { body: JSON.stringify({ acknowledged, cancelled, restored,
          assignment, checkpoint, saved, resumedNormal }), contentType: "application/json" });
        await info.attach(`post-ack-restart-preserved-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      } else if (afterAck === "input" || afterAck === "draft" || afterAck === "stop refresh draft") {
        const acknowledged = (await preferences()).agent_switch_intents[waiting.operation];
        expect(acknowledged.stage).toBe("checkpoint_verified");
        const checkpoint = fixture.savedSource(source.cwd);
        const rawCount = (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").length;
        const lateDraft = afterAck === "stop refresh draft";
        const draft = afterAck === "draft" || lateDraft;
        const echoedProbes = () => page.evaluate(id =>
          ((window as any).__gwtTerminalTestApi?.bufferText(id) ?? "").split("GWT_RAW_MONITOR_PROBE").length - 1,
        projectedSource.id);
        if (lateDraft) {
          await expect.poll(echoedProbes, { timeout: 30_000 }).toBe(rawCount);
          await fixture.targetRefreshGate.arm();
          await releaseNormalStop(waiting.operation);
          await expect.poll(() => fixture.targetRefreshGate.waiting(), { timeout: 30_000 }).not.toBeNull();
          const held = await fixture.targetRefreshGate.waiting();
          expect(held.argv.slice(0, 3)).toEqual(["issue", "view", "44"]);
          expect(held.parent).toBe(acknowledged.runtime_stop.host_pid);
          expect(fixture.processAlive(held.pid)).toBe(true);
          await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
            entry.stage === "normal_n2_hook_returned" && entry.operation === waiting.operation),
          { timeout: 30_000 }).toBe(true);
          expect((await preferences()).agent_switch_intents[waiting.operation]).toMatchObject({
            stage: "checkpoint_verified", source: acknowledged.source, checkpoint: acknowledged.checkpoint });
          expect(fixture.savedSource(source.cwd).porcelain).toBe("");
          await info.attach("stop-target-refresh-held", { body: JSON.stringify({ held, acknowledged }),
            contentType: "application/json" });
        }
        await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id,
          data: draft ? "GWT_RAW_MONITOR_PROBE" : "GWT_RAW_MONITOR_PROBE\r" });
        let accepted: any;
        let subsequentStop: any;
        const observeSubmittedProbe = async () => {
          await expect.poll(async () => (await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor").length,
            { timeout: 60_000 }).toBe(rawCount + 1);
          const events = await fixture.actorEvents();
          accepted = events.filter((entry: any) => entry.stage === "raw_accepted").at(-1);
          subsequentStop = events.filter((entry: any) => entry.stage === "raw_successor").at(-1);
          expect(accepted).toMatchObject({ pid: source.pid, gwt_session_id: source.gwt_session_id, current_hook_event: null });
          expect(subsequentStop).toMatchObject({ pid: source.pid, gwt_session_id: source.gwt_session_id,
            current_hook_event: { event: "Stop", outcome: "normal" } });
          expect(subsequentStop.current_hook_event.correlation_id).not.toBe(n1.current_hook_event.correlation_id);
        };
        if (!draft) await observeSubmittedProbe();
        if (lateDraft) {
          await expect.poll(echoedProbes, { timeout: 30_000 }).toBe(rawCount + 1);
          expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor")).toHaveLength(rawCount);
          expect((await preferences()).agent_switch_intents[waiting.operation].stage).toBe("checkpoint_verified");
          expect(fixture.savedSource(source.cwd).porcelain).toBe("");
          await fixture.targetRefreshGate.release();
          await expect.poll(() => fixture.targetRefreshGate.returned(), { timeout: 30_000 }).toBe(true);
        }
        const reason = lateDraft ? "PTY I/O error: ordinary input changed before protected body commit"
          : "New input invalidated the saved checkpoint. Current work is kept; request another switch.";
        await expect.poll(async () => (await preferences()).agent_switch_intents[waiting.operation],
          { timeout: Math.max(1, handoffDeadline - Date.now()) }).toMatchObject({ stage: "failed", reason });
        const assertSourceKept = async () => {
          const prefs = await preferences();
          const failed = prefs.agent_switch_intents[waiting.operation];
          expect(failed).toMatchObject({ stage: "failed",
            source: acknowledged.source, checkpoint: acknowledged.checkpoint });
          expect(failed.runtime_exit_confirmed).not.toBe(true);
          expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
          expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
            .assignment).toEqual(acknowledged.source.assignment);
          expect(prefs.launched_claims[43]).toBe(acknowledged.source.claim_id);
          expect(fixture.processAlive(source.pid)).toBe(true);
          expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
          expect(await fixture.launchIssues()).toEqual([43]);
          expect(fixture.requests.filter(request => request.method === "POST"
            && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
          expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
          const saved = fixture.savedSource(source.cwd);
          expect(saved).toMatchObject({ branch: checkpoint.branch, head: checkpoint.head, remoteHead: checkpoint.remoteHead });
          // A later normal Stop may append immutable Work history, but cannot change source files.
          for (const line of saved.porcelain.split("\n").filter(Boolean))
            expect(line).toMatch(/^\?\? \.gwt\/work\/events\/[0-9a-f]{2}\/[0-9a-f]{64}\.jsonl$/);
          const work = (await fixture.workItems()).work_items.find((value: any) => value.id === acknowledged.checkpoint.work_id);
          expect(work).toMatchObject({ owner: "Issue #43" });
          expect(["idle", "active"]).toContain(work.status_category);
          expect(work.events).toEqual(expect.arrayContaining([expect.objectContaining({
            summary: `Saved source checkpoint ${waiting.operation}`, status_category: "idle",
          })]));
          expect(work.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
        };
        await assertSourceKept();
        if (draft && !lateDraft) {
          expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor")).toHaveLength(rawCount);
          expect((await fixture.actorEvents()).some((entry: any) => entry.stage === "normal_n2_started"
            && entry.operation === waiting.operation)).toBe(false);
        }
        if (!lateDraft) await releaseNormalStop(waiting.operation);
        await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
          entry.stage === "normal_n2_hook_returned" && entry.operation === waiting.operation
            && entry.pid === source.pid && entry.gwt_session_id === source.gwt_session_id),
        { timeout: Math.max(1, handoffDeadline - Date.now()) }).toBe(true);
        await assertSourceKept();
        if (draft) {
          expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "raw_successor")).toHaveLength(rawCount);
          // Submit only the retained draft; no body is rewritten after the late Stop.
          await sendLiveGwtEvent(page, { kind: "terminal_input", id: projectedSource.id, data: "\r" });
          await observeSubmittedProbe();
          await assertSourceKept();
        }
        await expect(plans.locator(".roster-switch-status")).toContainText(reason);
        expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "actor_failed")).toEqual([]);
        await info.attach(`post-ack-${lateDraft ? "stop-refresh-draft" : draft ? "draft" : "input"}-preserved-source`, { body: JSON.stringify({ acknowledged,
          accepted, subsequentStop, failed: (await preferences()).agent_switch_intents[waiting.operation], checkpoint }),
        contentType: "application/json" });
        await info.attach(`post-ack-${lateDraft ? "stop-refresh-draft" : draft ? "draft" : "input"}-preserved-source-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      } else if (afterAck === "source edit") {
        const acknowledged = (await preferences()).agent_switch_intents[waiting.operation];
        expect(acknowledged.stage).toBe("checkpoint_verified");
        const checkpoint = fixture.savedSource(source.cwd);
        expect(checkpoint.porcelain).toBe("");
        const editedPath = join(source.cwd, "checkpoint-source.txt");
        const editedBytes = "Unsaved work after checkpoint ACK must survive the switch request.\n";
        await writeFile(editedPath, editedBytes);
        expect(fixture.savedSource(source.cwd).porcelain).toMatch(/^M checkpoint-source\.txt$/);
        await releaseNormalStop(waiting.operation);
        await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
          entry.stage === "normal_n2_hook_returned" && entry.operation === waiting.operation
            && entry.pid === source.pid && entry.gwt_session_id === source.gwt_session_id),
        { timeout: Math.max(1, handoffDeadline - Date.now()) }).toBe(true);
        const reason = "checkpoint requires a clean worktree, including untracked files";
        await expect.poll(async () => (await preferences()).agent_switch_intents[waiting.operation],
          { timeout: Math.max(1, handoffDeadline - Date.now()) }).toMatchObject({ stage: "failed", reason });
        const prefs = await preferences();
        const failed = prefs.agent_switch_intents[waiting.operation];
        expect(failed).toMatchObject({ source: acknowledged.source, checkpoint: acknowledged.checkpoint });
        expect(failed.runtime_exit_confirmed).not.toBe(true);
        expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
        expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
          .assignment).toEqual(acknowledged.source.assignment);
        expect(prefs.launched_claims[43]).toBe(acknowledged.source.claim_id);
        expect(fixture.processAlive(source.pid)).toBe(true);
        expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
        expect(await fixture.launchIssues()).toEqual([43]);
        expect(fixture.requests.filter(request => request.method === "POST"
          && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
        expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
        const saved = fixture.savedSource(source.cwd);
        expect(saved).toMatchObject({ branch: checkpoint.branch, head: checkpoint.head, remoteHead: checkpoint.remoteHead });
        expect(saved.porcelain).toMatch(/^\s*M checkpoint-source\.txt$/m);
        expect(await readFile(editedPath, "utf8")).toBe(editedBytes);
        const work = (await fixture.workItems()).work_items.find((value: any) => value.id === acknowledged.checkpoint.work_id);
        expect(work).toMatchObject({ owner: "Issue #43" });
        expect(work.events).toEqual(expect.arrayContaining([expect.objectContaining({
          summary: `Saved source checkpoint ${waiting.operation}`, status_category: "idle",
        })]));
        expect(work.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
        await expect(plans.locator(".roster-switch-status")).toContainText(reason);
        expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "actor_failed")).toEqual([]);
        await info.attach("post-ack-source-edit-preserved", { body: JSON.stringify({ acknowledged, failed, saved }),
          contentType: "application/json" });
        await info.attach(`post-ack-source-edit-preserved-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      } else if (afterAck === "cancel" || afterAck === "target removed" || afterAck === "stop refresh cancel") {
        const acknowledged = (await preferences()).agent_switch_intents[waiting.operation];
        expect(acknowledged.stage).toBe("checkpoint_verified");
        const checkpoint = fixture.savedSource(source.cwd);
        expect(checkpoint.porcelain).toBe("");
        const targetRemoved = afterAck === "target removed";
        const lateCancel = afterAck === "stop refresh cancel";
        const stage = targetRemoved ? "failed" : "cancelled";
        const reason = targetRemoved
          ? "The current assignment or next plan changed. Current work is kept; refresh plans."
          : "Cancelled before stopping the current Session";
        const visibleReason = targetRemoved ? reason : "Switch cancelled";
        if (lateCancel) {
          await fixture.targetRefreshGate.arm();
          await releaseNormalStop(waiting.operation);
          await expect.poll(() => fixture.targetRefreshGate.waiting(), { timeout: 30_000 }).not.toBeNull();
          const held = await fixture.targetRefreshGate.waiting();
          expect(held.argv.slice(0, 3)).toEqual(["issue", "view", "44"]);
          expect(held.parent).toBe(acknowledged.runtime_stop.host_pid);
          expect(fixture.processAlive(held.pid)).toBe(true);
          await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
            entry.stage === "normal_n2_hook_returned" && entry.operation === waiting.operation
              && entry.pid === source.pid && entry.gwt_session_id === source.gwt_session_id),
          { timeout: 30_000 }).toBe(true);
          expect((await preferences()).agent_switch_intents[waiting.operation]).toMatchObject({
            stage: "checkpoint_verified", source: acknowledged.source, checkpoint: acknowledged.checkpoint });
          expect(fixture.savedSource(source.cwd).porcelain).toBe("");
          await info.attach("cancel-stop-target-refresh-held", { body: JSON.stringify({ held, acknowledged }),
            contentType: "application/json" });
        }
        if (targetRemoved) {
          await request({ kind: "issue_monitor_queue_remove", issue_numbers: [fixture.targetIssueNumber] },
            "issue_monitor_queue_result");
          await expect.poll(async () => {
            const prefs = await preferences();
            const host = acknowledged.source.assignment.host_id;
            return { queued: prefs.terminal_queues[host].entries.some((entry: any) => entry.number === 44),
              excluded: prefs.terminal_queue_exclusions[host].includes(44) };
          }).toEqual({ queued: false, excluded: true });
        } else {
          await plans.getByRole("button", { name: "Cancel switch", exact: true }).click();
        }
        await expect.poll(async () => (await preferences()).agent_switch_intents[waiting.operation])
          .toMatchObject({ stage, reason });
        await expect(plans.locator(".roster-switch-status")).toContainText(visibleReason);
        const cancelledBeforeRelease = (await preferences()).agent_switch_intents[waiting.operation];
        if (lateCancel) {
          await fixture.targetRefreshGate.release();
          await expect.poll(() => fixture.targetRefreshGate.returned(), { timeout: 30_000 }).toBe(true);
        } else {
          await releaseNormalStop(waiting.operation);
        }
        await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
          entry.stage === "normal_n2_hook_returned" && entry.operation === waiting.operation
            && entry.pid === source.pid && entry.gwt_session_id === source.gwt_session_id),
        { timeout: Math.max(1, handoffDeadline - Date.now()) }).toBe(true);
        const prefs = await preferences();
        const cancelled = prefs.agent_switch_intents[waiting.operation];
        expect(cancelled).toMatchObject({ stage, source: acknowledged.source,
          checkpoint: acknowledged.checkpoint, reason });
        expect(cancelled.runtime_exit_confirmed).not.toBe(true);
        if (targetRemoved) {
          const host = acknowledged.source.assignment.host_id;
          expect(prefs.terminal_queues[host].entries.some((entry: any) => entry.number === 44)).toBe(false);
          expect(prefs.terminal_queue_exclusions[host]).toContain(44);
        }
        expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
        expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
          .assignment).toEqual(acknowledged.source.assignment);
        expect(prefs.launched_claims[43]).toBe(acknowledged.source.claim_id);
        expect(fixture.processAlive(source.pid)).toBe(true);
        expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
        expect(await fixture.launchIssues()).toEqual([43]);
        expect(fixture.requests.filter(request => request.method === "POST"
          && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
        expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
        const saved = fixture.savedSource(source.cwd);
        expect(saved).toMatchObject({ branch: checkpoint.branch, head: checkpoint.head, remoteHead: checkpoint.remoteHead });
        for (const line of saved.porcelain.split("\n").filter(Boolean))
          expect(line).toMatch(/^\?\? \.gwt\/work\/events\/[0-9a-f]{2}\/[0-9a-f]{64}\.jsonl$/);
        const work = (await fixture.workItems()).work_items.find((value: any) => value.id === acknowledged.checkpoint.work_id);
        expect(work).toMatchObject({ owner: "Issue #43" });
        expect(["idle", "active"]).toContain(work.status_category);
        expect(work.events).toEqual(expect.arrayContaining([expect.objectContaining({
          summary: `Saved source checkpoint ${waiting.operation}`, status_category: "idle",
        })]));
        expect(work.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
        await expect(plans.locator(".roster-switch-status")).toContainText(visibleReason);
        if (lateCancel) expect((await preferences()).agent_switch_intents[waiting.operation]).toEqual(cancelledBeforeRelease);
        expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "actor_failed")).toEqual([]);
        await info.attach(targetRemoved ? "post-ack-target-removal-preserved-source" : lateCancel ? "stop-refresh-cancellation-preserved-source" : "post-ack-cancellation-preserved-source", { body: JSON.stringify({ acknowledged, cancelled, saved }),
          contentType: "application/json" });
        await info.attach(`post-ack-${targetRemoved ? "target-removal" : lateCancel ? "stop-refresh-cancellation" : "cancellation"}-preserved-source-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      } else {
        if (afterAck === "worktree job" || afterAck === "cancel pending job" || afterAck === "unknown cwd") {
          const acknowledged = (await preferences()).agent_switch_intents[waiting.operation];
          expect(acknowledged.stage).toBe("checkpoint_verified");
          const checkpoint = fixture.savedSource(source.cwd);
          const unknownCwd = afterAck === "unknown cwd";
          const jobCwd = unknownCwd ? join(source.cwd, "pending-job-cwd") : source.cwd;
          if (unknownCwd) await mkdir(jobCwd);
          job = spawn(process.execPath, ["-e",
            "process.stdout.write(JSON.stringify({pid:process.pid,cwd:process.cwd()})+'\\n');process.stdin.resume();process.stdin.on('end',()=>process.exit(0))"], {
            cwd: jobCwd, env: { HOME: fixture.home, PATH: process.env.PATH ?? "" }, stdio: ["pipe", "pipe", "ignore"],
          });
          jobClosed = new Promise(resolve => job!.once("close", () => resolve()));
          const ready = await new Promise<{ pid: number; cwd: string }>((resolve, reject) => {
            const timer = setTimeout(() => reject(new Error("Owned worktree job did not start")), 5_000);
            job!.once("error", error => { clearTimeout(timer); reject(error); });
            job!.stdout!.once("data", bytes => {
              clearTimeout(timer);
              try { resolve(JSON.parse(bytes.toString())); } catch (error) { reject(error); }
            });
          });
          expect(ready).toEqual({ pid: job.pid, cwd: jobCwd });
          if (unknownCwd) {
            await rmdir(jobCwd);
            await expect(realpath(`/proc/${ready.pid}/cwd`)).rejects.toMatchObject({ code: "ENOENT" });
            expect(fixture.savedSource(source.cwd)).toEqual(checkpoint);
          }
          await releaseNormalStop(waiting.operation);
          await expect.poll(async () => (await fixture.actorEvents()).some((entry: any) =>
            entry.stage === "normal_n2_hook_returned" && entry.operation === waiting.operation
              && entry.pid === source.pid && entry.gwt_session_id === source.gwt_session_id),
          { timeout: Math.max(1, handoffDeadline - Date.now()) }).toBe(true);
          await expect.poll(async () => {
            const reason = (await preferences()).agent_switch_intents[waiting.operation]?.reason || "";
            const pids = /Observed worktree PIDs: \[([\d, ]*)\]/.exec(reason)?.[1]
              .split(",").map((value: string) => Number(value.trim()));
            const unknownPids = /unknown cwd PIDs: \[([\d, ]*)\]/.exec(reason)?.[1]
              .split(",").map((value: string) => Number(value.trim()));
            return unknownCwd
              ? unknownPids?.includes(ready.pid) && !pids?.includes(ready.pid) && reason.includes("census complete: true.")
              : pids?.includes(ready.pid) && reason.includes("unknown cwd PIDs: [] (0 total); census complete: true.");
          }, { timeout: Math.max(1, handoffDeadline - Date.now()), message: "complete post-ACK census identifies the real unfinished job and its cwd status" }).toBe(true);
          const prefs = await preferences();
          const held = prefs.agent_switch_intents[waiting.operation];
          expect(held).toMatchObject({ stage: "checkpoint_verified", source: acknowledged.source, checkpoint: acknowledged.checkpoint });
          expect(held.runtime_exit_confirmed).not.toBe(true);
          expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
          expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
            .assignment).toEqual(acknowledged.source.assignment);
          expect(prefs.launched_claims[43]).toBe(acknowledged.source.claim_id);
          expect(fixture.processAlive(source.pid)).toBe(true);
          expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
          expect(fixture.processAlive(ready.pid)).toBe(true);
          expect(await fixture.launchIssues()).toEqual([43]);
          expect(fixture.requests.filter(request => request.method === "POST"
            && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
          expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
          expect(fixture.savedSource(source.cwd)).toEqual(checkpoint);
          const work = (await fixture.workItems()).work_items.find((value: any) => value.id === acknowledged.checkpoint.work_id);
          expect(work).toMatchObject({ owner: "Issue #43", status_category: "idle" });
          expect(work.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
          await expect(plans.locator(".roster-switch-status")).toContainText("Waiting for background work");
          await info.attach("post-ack-worktree-job-held", { body: JSON.stringify({ ready, held, checkpoint }), contentType: "application/json" });
          await info.attach(`post-ack-worktree-job-held-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
          if (afterAck === "cancel pending job") {
            await plans.getByRole("button", { name: "Cancel switch", exact: true }).click();
            await expect.poll(async () => (await preferences()).agent_switch_intents[waiting.operation])
              .toMatchObject({ stage: "cancelled", reason: "Cancelled before stopping the current Session" });
            await expect(plans.locator(".roster-switch-status")).toContainText("Switch cancelled");
          }
          job.stdin!.end();
          await expect.poll(() => job!.exitCode, { timeout: 5_000 }).toBe(0);
          await jobClosed;
          expect(job.signalCode).toBeNull();
          expect(fixture.processAlive(ready.pid)).toBe(false);
          if (afterAck === "cancel pending job") {
            const previousScan = (await preferences()).last_scan_at;
            await expect.poll(async () => {
              const prefs = await preferences();
              return !!prefs.last_scan_at && prefs.last_scan_at !== previousScan && !prefs.last_scan_error;
            }, { timeout: Math.max(1, handoffDeadline - Date.now()),
              message: "a normal successful scan after the cancelled job exits" }).toBe(true);
            const prefs = await preferences();
            const cancelled = prefs.agent_switch_intents[waiting.operation];
            expect(cancelled).toMatchObject({ stage: "cancelled", source: acknowledged.source,
              checkpoint: acknowledged.checkpoint, reason: "Cancelled before stopping the current Session" });
            expect(cancelled.runtime_exit_confirmed).not.toBe(true);
            expect(prefs.monitor_runtime_stop_requests || []).toEqual([]);
            expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
              .assignment).toEqual(acknowledged.source.assignment);
            expect(prefs.launched_claims[43]).toBe(acknowledged.source.claim_id);
            expect(prefs.enabled).toBe(true);
            expect(prefs.autonomous_mode).toBe(true);
            expect(fixture.processAlive(source.pid)).toBe(true);
            expect(fixture.processAlive(idle.tree_child_pid)).toBe(true);
            expect(await fixture.launchIssues()).toEqual([43]);
            expect(fixture.requests.filter(request => request.method === "POST"
              && request.url === "/repos/fixture/queue/issues/44/comments")).toEqual([]);
            expect(fixture.executionStatus(source.cwd).ecr_status).toBe("active");
            expect(fixture.savedSource(source.cwd)).toEqual(checkpoint);
            const work = (await fixture.workItems()).work_items.find((value: any) => value.id === acknowledged.checkpoint.work_id);
            expect(work).toMatchObject({ owner: "Issue #43", status_category: "idle" });
            expect(work.board_refs).toContain(acknowledged.checkpoint.board_entry_id);
            await expect(plans.locator(".roster-switch-status")).toContainText("Switch cancelled");
            expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "actor_failed")).toEqual([]);
            await info.attach("post-ack-job-cancellation-preserved", { body: JSON.stringify({ ready, cancelled, checkpoint,
              previousScan, lastScan: prefs.last_scan_at }), contentType: "application/json" });
            await info.attach(`post-ack-job-cancellation-preserved-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
          }
        }
        if (afterAck !== "cancel pending job") {
          await releaseNormalStop(waiting.operation);
          await expect.poll(async () => (await fixture.launches()).length === 2
            || (await fixture.actorEvents()).some((entry: any) => entry.stage === "normal_n2_hook_returned"
              && entry.operation === waiting.operation && entry.pid === source.pid
              && entry.gwt_session_id === source.gwt_session_id),
          { timeout: Math.max(1, handoffDeadline - Date.now()), message: "genuine N2 hook returned or ordinary target already launched" }).toBe(true);
          await expect.poll(async () => (await fixture.launches()).length,
            { timeout: Math.max(1, handoffDeadline - Date.now()), message: "actual source checkpoint ACK, physical exit and ordinary target Issues launch" }).toBe(2);
          const launches = await fixture.launches();
          const target = launches[1];
          const completionDeadline = Date.now() + 30_000;
          const targetConfirmation = async () => {
            const prefs = await preferences();
            const assignment = (await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)?.assignment;
            const confirmation = prefs.launch_confirmations?.[44];
            const intent = prefs.agent_switch_intents[waiting.operation];
            return {
              assignment_matches: !!target.gwt_session_id && target.gwt_session_id !== source.gwt_session_id
                && assignment?.issue_number === 44 && assignment.session_id === target.gwt_session_id,
              confirmation_matches: !!confirmation?.window_id,
              binding_matches: !!confirmation && prefs.launch_bindings?.[confirmation.window_id] === 44,
              launched_matches: !!confirmation && (prefs.launched_issues || []).some((value: any) =>
                value.issue_number === 44 && value.window_id === confirmation.window_id),
              stage: intent?.stage, reason: intent?.reason,
            };
          };
          await expect.poll(targetConfirmation, { timeout: Math.max(1, completionDeadline - Date.now()),
            message: "ordinary target assignment and launch confirmation before completion evaluation" })
            .toMatchObject({ assignment_matches: true, confirmation_matches: true, binding_matches: true, launched_matches: true });
          await info.attach("agent-switch-target-confirmation", {
            body: JSON.stringify(await targetConfirmation()), contentType: "application/json",
          });
          await expect.poll(async () => Object.values((await preferences()).agent_switch_intents || {})
            .map((intent: any) => intent.stage).sort(), { timeout: Math.max(1, completionDeadline - Date.now()) })
            .toEqual(cancelledDuringRefresh ? ["cancelled", "completed"] : ["completed"]);
          const finalPreferences = await preferences();
          const saved = finalPreferences.agent_switch_intents[waiting.operation];
          if (cancelledDuringRefresh) {
            expect(finalPreferences.agent_switch_intents[cancelledDuringRefresh.request.operation_id])
              .toEqual(cancelledDuringRefresh);
            expect(saved.runtime_stop.host_pid).toBe(cancelledDuringRefresh.runtime_stop.host_pid);
            expect((await fixture.actorEvents()).filter((entry: any) =>
              entry.operation === cancelledDuringRefresh.request.operation_id)).toEqual([]);
          }
          if (refreshSubmit) {
            const events = await fixture.actorEvents();
            expect(events.filter((entry: any) => entry.stage === "raw_successor")).toHaveLength(2);
            expect(events.filter((entry: any) => entry.stage === "checkpoint_acked").map((entry: any) => entry.operation))
              .toEqual([waiting.operation]);
          }
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
          expect((await readCanonicalRoster(page)).members.find((value: any) => value.agent_member_id === member.agent_member_id)
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
          const targetClaimId = finalPreferences.launch_confirmations?.[44]?.claim_id;
          if (macOsSwitch) {
            expect(targetClaimId).toEqual(expect.any(String));
            expect(targetClaimId).toBe(finalPreferences.launched_claims?.[44]);
            expect(targetClaimId).not.toBe(saved.source.claim_id);
            await expect.poll(async () => (await fixture.switchCwdCensus()).filter(fields =>
              fields.source_pid === source.pid && fields.complete === true && fields.matching === 0 &&
              fields.unknown === 0 && fields.excluded_by_uid !== undefined).length,
            { timeout: 10_000, message: "macOS Host recorded clear censuses for the source provider" }).toBeGreaterThanOrEqual(2);
          }
          await info.attach(`agent-switch-completed-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
          await info.attach("agent-switch-native-proof", { body: JSON.stringify({ source, target, idle,
            checkpoint, operation: ack.operation, work_id: ack.work_id, board_entry_id: ack.board_entry_id,
            target_claim_id: targetClaimId }), contentType: "application/json" });
          expect((await fixture.actorEvents()).filter((entry: any) => entry.stage === "actor_failed")).toEqual([]);
        }

      }
      expect(errors).toEqual([]);
    } finally {
      try {
        if (macOsSwitch) {
          const prefs = await preferences();
          await info.attach("agent-switch-cwd-census", {
            body: JSON.stringify({ uid: process.geteuid?.(), observations: await fixture.switchCwdCensus(),
              intents: prefs.agent_switch_intents }), contentType: "application/json",
          });
        }
        if ((gatedInput || afterAck === "stop claim replaced") && bodyGate) {
          const armed = await readFile(`${bodyGate}.claimed`, "utf8").then(JSON.parse).catch(() => null);
          if (armed) await writeFile(`${bodyGate}.release`, armed.operation);
        }
        if (coordinator && coordinatorClosed && coordinator.exitCode === null && coordinator.signalCode === null) {
          coordinator.kill("SIGTERM");
          await coordinatorClosed;
        }
        await info.attach("agent-switch-startup-stages", {
          body: await readFile(join(fixture.home, "switch-startup.jsonl"), "utf8").catch(() => ""),
          contentType: "application/x-ndjson",
        });
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
        try { await page.close(); } finally {
          try { await fixture.stop(); } finally {
            if (job && jobClosed) {
              if (job.exitCode === null && job.signalCode === null) job.kill("SIGTERM");
              const escalation = setTimeout(() => job!.kill("SIGKILL"), 2_500);
              let timer: ReturnType<typeof setTimeout> | undefined;
              try {
                await Promise.race([jobClosed, new Promise<never>((_, reject) => {
                  timer = setTimeout(() => reject(new Error("Owned worktree job did not close")), 5_000);
                })]);
              } finally { clearTimeout(escalation); clearTimeout(timer); }
            }
          }
        }
        if (gatedInput || afterAck === "stop claim replaced" || afterAck === "queued body") expect(errors, "zero console/page errors including shutdown").toEqual([]);
      }
    }
    expect(errors).toEqual([]);
  });

  test("T20a UI reassigned schedule survives GUI restart and force requeue with live Busy and Works", async ({ page }, info) => {
    // Setup, GUI restart and two bounded launches reached proof collection at
    // 242s on Windows. Budget their aggregate plus cleanup; retain phase limits.
    test.setTimeout(420_000);
    const fixture = await startIssueQueueLaunchFixture(info, { page });
    const errors: string[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const context = page.context();
    let current = page;
    const capture = (target: typeof page) => {
      target.on("pageerror", error => errors.push(error.message));
      target.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    };
    let rosterRequestSequence = 0;
    const request = async (event: unknown, kind: string) => {
      const requestId = kind === "agent_roster" ? `t20a-roster-${++rosterRequestSequence}` : null;
      const after = await current.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      await sendLiveGwtEvent(current, requestId
        ? { ...(event as Record<string, unknown>), request_id: requestId } : event);
      return (await current.waitForFunction(({ after, kind, requestId }) => {
        const entry = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.sequence > after && [kind, `${kind}_error`].includes(entry.payload.kind)
          && (requestId === null || entry.payload.request_id === requestId));
        return entry?.payload || null;
      }, { after, kind, requestId }, { timeout: 30_000 })).jsonValue();
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
      expect(roster.members).toHaveLength(2);
      // Registration and launch eligibility are separate: explicitly select
      // both fixed-tool people while retaining the legacy profile's tuning.
      const preferences = JSON.parse((await fixture.monitorPreferencesBytes()).toString());
      const baseProfile = preferences.launch_profiles[0];
      expect(baseProfile.agent_id).toBe("codex");
      expect(baseProfile.agent_member_id).toBe(initial.members[0].agent_member_id);
      const selectedProfiles = roster.members.map((member: any) => {
        expect(member.agent_id).toBe("codex");
        return { ...baseProfile, agent_member_id: member.agent_member_id, agent_id: member.agent_id,
          skip_permissions: true, prefer_for: ["kind:issue"] };
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
      let observed = schedule();
      const source = owner(observed);
      expect(source).toBeTruthy();
      const target = roster.members.find((member: any) => member.agent_member_id !== source.agent_member_id);
      expect(target).toBeTruthy();
      // A separate project changes only this person's common saved core. The
      // primary Monitor still owns its candidate order, permissions and routing.
      const profileSourceRoot = join(fixture.home, "shared-profile-source");
      await mkdir(profileSourceRoot);
      const primaryScope = fixture.fixtureOperation("issue.monitor.status");
      expect(primaryScope.ok, JSON.stringify(primaryScope)).toBe(true);
      const sourceProfile = { agent_id: "codex", agent_member_id: target.agent_member_id,
        model: "gpt-6-sol", reasoning: "high", fast_mode: true, codex_speed: "fast", runtime_target: "Host",
        skip_permissions: false, prefer_for: ["label:source-only"] };
      const sourceSave = fixture.fixtureOperation("issue.monitor.profiles.set", { profiles: [sourceProfile] }, profileSourceRoot);
      expect(sourceSave.ok, JSON.stringify(sourceSave)).toBe(true);
      expect(sourceSave.project_store.hash).not.toBe(primaryScope.project_store.hash);
      expect(JSON.parse(sourceSave.output).launch_profiles).toContainEqual(expect.objectContaining(sourceProfile));
      // The core save advances the shared settings CAS, so obtain a fresh plan
      // before exercising the existing UI reassignment transaction.
      observed = schedule();
      expect(owner(observed).agent_member_id).toBe(source.agent_member_id);
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
      const rosterBeforeStale = durableRoster(await readCanonicalRoster(current));
      const stale = fixture.scheduleOperation("issue.monitor.schedule.reassign", reassignment);
      expect(stale.ok).toBe(false);
      const afterStale = await fixture.monitorPreferencesBytes();
      await info.attach("agent-schedule-stale-prefs", { body: JSON.stringify({
        before: beforeStale.toString(), after: afterStale.toString(),
      }, null, 2), contentType: "application/json" });
      const plan = (bytes: Buffer) => {
        const prefs = JSON.parse(bytes.toString());
        return { agent_issue_schedule: prefs.agent_issue_schedule,
          terminal_queues: Object.fromEntries(Object.entries(prefs.terminal_queues)
            .map(([host, queue]) => [host, (queue as any).entries])) };
      };
      expect(plan(afterStale), "rejected stale move preserves project member plans and queue order").toEqual(plan(beforeStale));
      expect(durableRoster(await readCanonicalRoster(current)), "rejected stale move preserves canonical roster").toEqual(rosterBeforeStale);
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
      expect(await fixture.launches(), "disabled Issues stays idle across restart").toEqual([]);
      await info.attach(`agent-schedule-ui-restored-${theme}`, { body: await current.screenshot(), contentType: "image/png" });
      await sendLiveGwtEvent(current, { kind: "set_issue_monitor_enabled", enabled: true });
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 90_000, message: "normal Issues launches the reassigned queued Issue" }).toBe(1);
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
        await openWorks(current);
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
      expect(field(record, "session_mode")).toBe("Normal");
      expect(field(record, "model")).toBe(sourceProfile.model);
      expect(field(record, "reasoning_level")).toBe(sourceProfile.reasoning);
      // Legacy flags stay Off; explicit speed is persisted in launch arguments.
      expect(record).toMatch(/^fast_mode\s*=\s*false$/m);
      expect(record).toMatch(/^codex_fast_mode\s*=\s*false$/m);
      const persistedLaunchArgs = /^launch_args\s*=\s*\[([\s\S]*?)\]/m.exec(record)?.[1];
      expect(persistedLaunchArgs).toContain('"service_tier=fast"');
      expect(persistedLaunchArgs).toContain('"features.fast_mode=true"');
      expect(record).toMatch(/^skip_permissions\s*=\s*true$/m);
      expect(launch.argv).toContain(`--model=${sourceProfile.model}`);
      expect(launch.argv).toContain(`model_reasoning_effort=${sourceProfile.reasoning}`);
      expect(launch.argv).toContain("service_tier=fast");
      expect(launch.argv).toContain("features.fast_mode=true");
      expect(launch.argv).toContain("--yolo");
      expect(launch.argv).not.toContain("resume");
      const primaryTarget = JSON.parse((await fixture.monitorPreferencesBytes()).toString()).launch_profiles
        .find((profile: any) => profile.agent_member_id === target.agent_member_id);
      expect(primaryTarget).toMatchObject({ skip_permissions: true, prefer_for: ["kind:issue"] });
      expect(record).toMatch(new RegExp(`^linked_issue_number\\s*=\\s*${fixture.issueNumber}$`, "m"));
      const firstObservation = await assertObservedWork(launch.gwt_session_id!, target.agent_member_id);
      fixture.auditHooks(launch.cwd);
      // Join the GUI input alias and Monitor's physical pane through the exact
      // Session, rather than comparing IDs from different namespaces.
      const primaryProjectKey = primaryScope.project_store.hash;
      expect(firstObservation.activity.project_key).toBe(primaryProjectKey);
      const physicalWorkspace = JSON.parse(await readFile(join(fixture.home, ".gwt", "projects",
        primaryProjectKey, "workspace.json"), "utf8"));
      const physicalRows = physicalWorkspace.windows.filter((row: any) => row.session_id === launch.gwt_session_id);
      expect(physicalRows, "exact launched Session has one durable owner pane").toHaveLength(1);
      expect(physicalRows[0]).toMatchObject({ preset: "agent", linked_issue_number: fixture.issueNumber });
      const firstPhysicalWindowId = `${primaryProjectKey}::${physicalRows[0].id}`;
      const beforeExit = JSON.parse((await fixture.monitorPreferencesBytes()).toString());
      expect(beforeExit.launched_issues.filter((entry: any) => entry.issue_number === fixture.issueNumber),
        "Monitor owns the same Session's exact physical pane before exit").toEqual([
        { issue_number: fixture.issueNumber, window_id: firstPhysicalWindowId },
      ]);

      // Stop only the owned provider, then recover through the real authenticated
      // Issues path. No incoming census or roster frame is manufactured here.
      // Keep the launch binding until the exact terminal receipt is accepted.
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
            && entry.window_id === firstPhysicalWindowId);
      }, { timeout: 60_000, message: "the Host records the exact predecessor exit and releases its launch" }).toBe(true);
      // OFF revokes the launch identity, so record its exact failure first.
      await sendLiveGwtEvent(current, { kind: "set_issue_monitor_enabled", enabled: false });
      await expect.poll(async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString()).enabled).toBe(false);
      const after = await current.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      const requeued = fixture.fixtureOperation("issue.monitor.requeue", {
        number: fixture.issueNumber, reason: "Issue #6362 real observer force-requeue regression", force: true,
      });
      expect(requeued.ok, JSON.stringify(requeued)).toBe(true);
      await sendLiveGwtEvent(current, { kind: "set_issue_monitor_enabled", enabled: true });
      await expect.poll(async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString()).enabled,
        { timeout: 30_000, message: "Issues enable is durable before the separate launch_now operation" }).toBe(true);
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
        target: { agent_member_id: target.agent_member_id, name: target.name }, launch, firstObservation, firstPhysicalWindowId,
        shared_profile_source: { project_key: sourceSave.project_store.hash, profile: sourceProfile },
        primary_project_key: primaryScope.project_store.hash, primary_target: primaryTarget,
        requeued, immediate, successor, successorObservation }, null, 2), contentType: "application/json" });
      expect(errors, "zero console/page errors across UI reassignment, GUI restart and real launch").toEqual([]);
    } finally {
      try { await current.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned fixture cleanup").toEqual([]);
  });

  test("Issue #7647 a registered agent starts on the project's environment whatever tool it was registered with", async ({ page }, info) => {
    // The agents are registered with Claude Code, which this fixture does not
    // provide: only the project's saved environment (Codex) can start.
    const fixture = await startIssueQueueLaunchFixture(info, { page, monitorPreferences: {
      launch_profile: { agent_id: "claude", model: "claude-only-model", runtime_target: "Host", skip_permissions: true },
      execution_environments: [{ environment_id: "codex", tool: "codex" }],
    } });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] });
      await expect.poll(async () => (await fixture.launches()).length,
        { timeout: 90_000, message: "the registered agent starts with the environment's tool" }).toBe(1);
      const launch = (await fixture.launches())[0];
      const record = await readFile(join(fixture.home, ".gwt/sessions", `${launch.gwt_session_id}.toml`), "utf8");
      expect(record).toMatch(/^\[agent_id\]\r?\ntype = "Codex"/m);
      expect(record).not.toContain("claude-only-model");
      expect(record).toContain(`linked_issue_number = ${fixture.issueNumber}`);
      const roster = await readCanonicalRoster(page);
      const member = roster.members.find((member: any) => member.assignment?.issue_number === fixture.issueNumber);
      expect(member, "the session belongs to a registered agent").toBeTruthy();
      expect(member.agent_id, "the agent keeps the tool it was registered with").toBe("claude");
      expect(record).toContain(member.agent_member_id);
      const prefs = JSON.parse((await fixture.monitorPreferencesBytes()).toString());
      expect(prefs.launch_profiles.map((profile: any) => profile.agent_id)).toContain("claude");
      expect(prefs.launch_profiles.map((profile: any) => profile.agent_id)).not.toContain("codex");
      await info.attach(`environment-tool-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      expect(errors).toEqual([]);
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned process cleanup").toEqual([]);
  });

  test("AC34 actual provider exit removes the local Active agent without closing its Issue", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, { page });
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
      await openWorks(page);
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

  for (const stopBeforeExit of [false, true]) test(stopBeforeExit
    ? "T94416 explicit Stop prevents admission after provider exit until Start"
    : "T94415 ordinary provider exit admits the queued Issue without operator rescue", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, {
      additionalIssueNumbers: [44], autonomousMode: true, terminalConvergence: true,
      monitorPreferences: { max_active_agents: 1 },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const nativeSession = async (id: string) => (await readFile(
      join(fixture.home, ".gwt", "sessions", `${id}.toml`), "utf8"))
      .match(/^agent_session_id\s*=\s*"([^"]*)"/m)?.[1];
    const activity = () => page.evaluate(() =>
      ((window as any).__gwtPlaywrightMessages ?? [])
        .map(({ payload }: any) => payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity)
        .filter((snapshot: any) => snapshot?.project_key === location.pathname.split("/")[2])
        .at(-1));
    const sourceActivity = async (sessionId: string) =>
      (await activity())?.agents?.find((agent: any) => agent.session_id === sessionId);
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await page.locator('[data-surface="issues"]').click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const issues = page.locator('.workspace-window[data-preset="issue"] .issue-bridge-root');
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 });
      await expect.poll(async () => JSON.parse(await readFile(
        join(fixture.home, ".gwt/machine-state/agent-settings.json"), "utf8")), { timeout: 30_000 })
        .toMatchObject({ mode: "manual", manual_limit: 1,
          last_change: { source: "gui", max_active_agents_override: 1 } });
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [43, 44] });
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43]);
      const source = (await fixture.launches())[0];
      await expect(issues.locator('[data-queue-column="active"] [data-issue-number="43"]')).toBeVisible();
      await expect(issues.locator('[data-queue-column="queued"] [data-issue-number="44"]')).toBeVisible();
      expect(source.gwt_session_id).toBeTruthy();
      await expect.poll(() => nativeSession(source.gwt_session_id!), { timeout: 45_000 }).toBe(source.provider_session_id);
      await expect.poll(() => sourceActivity(source.gwt_session_id!)).toMatchObject({ issue_number: 43 });
      const sourceWindow = (await sourceActivity(source.gwt_session_id!)).window_id;
      expect(sourceWindow).toBeTruthy();
      expect((await preferences()).autonomous_mode).toBe(true);
      expect(() => process.kill(source.pid, 0)).not.toThrow();
      expect(await fixture.launchIssues(), "the occupied slot keeps the successor queued").toEqual([43]);

      const controls = page.getByRole("group", { name: "Issues controls", exact: true });
      if (stopBeforeExit) {
        await controls.getByRole("button", { name: "Stop", exact: true }).click();
        await expect.poll(preferences).toMatchObject({ enabled: false, autonomous_mode: true });
        await expect(controls.getByRole("button", { name: "Start", exact: true })).toBeVisible();
        expect(() => process.kill(source.pid, 0), "Stop preserves the running provider").not.toThrow();
      }

      await sendLiveGwtEvent(page, { kind: "terminal_input", id: sourceWindow, data: "GWT_FIXTURE_EXIT\r" });
      // Every observation after exit is passive: no refresh, Switch, requeue or launch_now.
      await expect.poll(() => {
        try { process.kill(source.pid, 0); return false; } catch { return true; }
      }, { timeout: 60_000, message: "the exact source process exited and was reaped" }).toBe(true);
      if (stopBeforeExit) {
        // OFF clears Monitor bindings itself, so also require physical exit and live-census removal.
        await expect.poll(async () => (await activity())?.agents, { timeout: 60_000 }).toEqual([]);
        await expect(issues.locator('[data-metric="active"]')).toHaveText("Active 0 · 0 issues · PM 0");
        await expect(issues.locator('[data-queue-column="queued"] [data-issue-number="44"]')).toBeVisible();
        expect((await preferences()).enabled).toBe(false);
        expect(await fixture.launchIssues(), "explicit Stop prevents successor launch").toEqual([43]);
        const successorClaims = fixture.requests.filter(request => request.method === "POST"
          && request.url === "/repos/fixture/queue/issues/44/comments");
        expect(successorClaims, "explicit Stop prevents successor claim").toEqual([]);
        await info.attach(`stopped-after-exit-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
        await info.attach("stopped-after-exit", { body: JSON.stringify({ enabled: false,
          source, launches: await fixture.launches(), successorClaims }), contentType: "application/json" });
        await controls.getByRole("button", { name: "Start", exact: true }).click();
        await expect.poll(preferences).toMatchObject({ enabled: true, autonomous_mode: true });
      }
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000,
        message: "ordinary exit releases the sole slot and automatically admits Issue 44" }).toEqual([43, 44]);
      const target = (await fixture.launches())[1];
      expect(target.pid).not.toBe(source.pid);
      expect(target.gwt_session_id).toBeTruthy();
      expect(target.gwt_session_id).not.toBe(source.gwt_session_id);
      expect(target.provider_session_id).not.toBe(source.provider_session_id);
      await expect.poll(() => nativeSession(target.gwt_session_id!), { timeout: 45_000 }).toBe(target.provider_session_id);
      expect(() => process.kill(target.pid, 0)).not.toThrow();
      await expect(issues.locator('[data-queue-column="active"] [data-issue-number="44"]')).toBeVisible();
      await expect(issues.locator('[data-queue-column="active"] .knowledge-row')).toHaveCount(1);
      await expect(issues.locator('[data-metric="active"]')).toHaveText("Active 1 · 1 issue · PM 0");
      const sourceRow = issues.locator('[data-issue-number="43"]');
      await expect(sourceRow).toBeVisible();
      await expect(sourceRow.locator('[data-action="focus-surface-window"]')).toHaveCount(0);
      await expect(sourceRow).not.toContainText("Shown in Agents");
      expect(await fixture.launchIssues()).toEqual([43, 44]);
      await info.attach("ordinary-exit-next-admission", {
        body: JSON.stringify({ source, target, launches: await fixture.launches() }), contentType: "application/json",
      });
      await info.attach(`ordinary-exit-next-admission-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      expect(errors).toEqual([]);
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned fixture cleanup").toEqual([]);
  });

  test("T94417 ordinary exit preserves a busy fixed assignment until its member is free", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, {
      additionalIssueNumbers: [44, 45], autonomousMode: true, terminalConvergence: true,
      monitorPreferences: { enabled: false, max_active_agents: 1 },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const record = (id: string) => readFile(join(fixture.home, ".gwt", "sessions", `${id}.toml`), "utf8");
    const field = (text: string, key: string) => text.match(new RegExp(`^${key}\\s*=\\s*"([^"]*)"`, "m"))?.[1];
    const activity = () => page.evaluate(() =>
      ((window as any).__gwtPlaywrightMessages ?? [])
        .map(({ payload }: any) => payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity)
        .filter((snapshot: any) => snapshot?.project_key === location.pathname.split("/")[2])
        .at(-1));
    let rosterRequestSequence = 0;
    const request = async (event: unknown, kind: string) => {
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
      const requestId = kind === "agent_roster" ? `t94417-${++rosterRequestSequence}` : null;
      await sendLiveGwtEvent(page, requestId ? { ...(event as object), request_id: requestId } : event);
      return (await page.waitForFunction(({ after, kind, requestId }) =>
        (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.sequence > after && [kind, `${kind}_error`].includes(entry.payload.kind)
          && (!requestId || entry.payload.request_id === requestId))?.payload,
      { after, kind, requestId }, { timeout: 30_000 })).jsonValue();
    };
    const exited = (pid: number) => { try { process.kill(pid, 0); return false; } catch { return true; } };
    const reservation = async () => (await preferences()).agent_issue_schedule.reservations
      .find((entry: any) => entry.issue_number === 45);
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const initial = (await request({ kind: "agent_roster", action: { kind: "read" } }, "agent_roster")).roster;
      const roster = (await request({ kind: "agent_roster", action: { kind: "add" },
        expected_revision: initial.revision }, "agent_roster")).roster;
      expect(roster.error).toBeFalsy();
      expect(roster.members).toHaveLength(2);
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 2 });
      await expect.poll(async () => JSON.parse(await readFile(
        join(fixture.home, ".gwt/machine-state/agent-settings.json"), "utf8")))
        .toMatchObject({ mode: "manual", manual_limit: 2 });
      const baseProfile = (await preferences()).launch_profiles[0];
      const profiles = roster.members.map((member: any) => {
        expect(member.agent_id).toBe("codex");
        return { ...baseProfile, agent_member_id: member.agent_member_id, agent_id: member.agent_id };
      });
      const profileReply = await request({ kind: "issue_monitor_profiles_set", request_id: 94417, profiles },
        "issue_monitor_profiles");
      expect(profileReply.profiles).toEqual(profiles);
      await expect.poll(async () => (await preferences()).launch_profiles).toEqual(profiles);
      await page.locator('[data-surface="issues"]').click();
      const issues = page.locator('.workspace-window[data-preset="issue"] .issue-bridge-root');
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [43, 44] });
      await page.getByRole("group", { name: "Issues controls", exact: true })
        .getByRole("button", { name: "Start", exact: true }).click();
      await expect.poll(async () => (await fixture.launchIssues()).sort(), { timeout: 90_000 }).toEqual([43, 44]);
      const launches = await fixture.launches();
      for (const launch of launches) {
        expect(launch.gwt_session_id).toBeTruthy();
        await expect.poll(async () => field(await record(launch.gwt_session_id!), "agent_session_id"),
          { timeout: 45_000 }).toBe(launch.provider_session_id);
        expect(() => process.kill(launch.pid, 0)).not.toThrow();
      }
      await expect.poll(async () => (await activity())?.agents?.length).toBe(2);
      await expect.poll(async () => (await readCanonicalRoster(page)).members
        .filter((member: any) => launches.some(launch => launch.gwt_session_id === member.assignment?.session_id)).length).toBe(2);
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [45] });
      await expect(issues.locator('[data-queue-column="queued"] [data-issue-number="45"]')).toBeVisible();
      const reply = fixture.scheduleOperation("issue.monitor.schedule.read");
      expect(reply.ok, JSON.stringify(reply)).toBe(true);
      const schedule = JSON.parse(reply.output);
      const automatic = schedule.members.find((member: any) => member.issues.some((issue: any) => issue.issue_number === 45));
      expect(automatic).toBeTruthy();
      const selected = (await readCanonicalRoster(page)).members.find((member: any) =>
        member.assignment && member.agent_member_id !== automatic.agent_member_id);
      expect(selected).toBeTruthy();
      const selectedLaunch = launches.find(launch => launch.gwt_session_id === selected.assignment.session_id)!;
      const otherLaunch = launches.find(launch => launch.gwt_session_id !== selected.assignment.session_id)!;
      await page.locator('.op-rail__surface[data-surface="agents"]').click();
      const agents = page.getByRole("region", { name: "Main pane", exact: true })
        .locator(".agents-surface:not(.agents-surface--preview)");
      await agents.locator(`.roster-row[data-member-id="${automatic.agent_member_id}"]`).click();
      const plan = agents.locator('.roster-plan[data-issue-number="45"]');
      await plan.getByRole("combobox", { name: "Assign issue #45 to member", exact: true }).selectOption(selected.agent_member_id);
      await plan.getByRole("button", { name: "Reassign issue #45", exact: true }).click();
      await expect.poll(reservation).toMatchObject({ agent_member_id: selected.agent_member_id, reason: "assigned_by_you" });
      const fixed = await reservation();
      await agents.locator(`.roster-row[data-member-id="${selected.agent_member_id}"]`).click();
      await expect(agents.locator('.roster-plan[data-issue-number="45"] .roster-plan-reason')).toHaveText("Assigned by you");
      await page.locator('[data-surface="issues"]').click();

      const otherWindow = (await activity()).agents.find((agent: any) => agent.session_id === otherLaunch.gwt_session_id).window_id;
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: otherWindow, data: "GWT_FIXTURE_EXIT\r" });
      // After exit, observe only real output, census and durable state; never drive a scan by reading it.
      await expect.poll(() => exited(otherLaunch.pid), { timeout: 60_000 }).toBe(true);
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([selectedLaunch.gwt_session_id]);
      await expect.poll(async () => (await readCanonicalRoster(page)).members
        .find((member: any) => member.agent_member_id === automatic.agent_member_id)?.assignment).toBeNull();
      const previousScan = (await preferences()).last_scan_at;
      await expect.poll(async () => {
        const prefs = await preferences();
        return !!prefs.last_scan_at && prefs.last_scan_at !== previousScan && !prefs.last_scan_error;
      }, { timeout: 90_000, message: "a successful scan observes the genuinely free other slot" }).toBe(true);
      expect((await preferences()).enabled).toBe(true);
      expect((await preferences()).autonomous_mode).toBe(true);
      expect(await reservation()).toEqual(fixed);
      expect((await readCanonicalRoster(page)).members.find((member: any) =>
        member.agent_member_id === selected.agent_member_id).assignment).toEqual(selected.assignment);
      expect(() => process.kill(selectedLaunch.pid, 0)).not.toThrow();
      const queued = issues.locator('[data-queue-column="queued"] [data-issue-number="45"]');
      await expect(queued).toBeVisible();
      expect(await fixture.launchIssues()).toHaveLength(2);
      const fixedClaims = fixture.requests.filter(request => request.method === "POST"
        && request.url === "/repos/fixture/queue/issues/45/comments");
      expect(fixedClaims, "the free other member cannot claim the user-assigned Issue").toEqual([]);
      await info.attach(`fixed-assignment-wait-${theme}`, { body: await page.screenshot(), contentType: "image/png" });

      const selectedWindow = (await activity()).agents.find((agent: any) => agent.session_id === selectedLaunch.gwt_session_id).window_id;
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: selectedWindow, data: "GWT_FIXTURE_EXIT\r" });
      await expect.poll(() => exited(selectedLaunch.pid), { timeout: 60_000 }).toBe(true);
      await expect.poll(async () => (await fixture.launchIssues()).sort(), { timeout: 90_000 }).toEqual([43, 44, 45]);
      const successor = (await fixture.launches())[2];
      expect(launches.map(launch => launch.pid)).not.toContain(successor.pid);
      expect(launches.map(launch => launch.gwt_session_id)).not.toContain(successor.gwt_session_id);
      await expect.poll(async () => field(await record(successor.gwt_session_id!), "agent_session_id"),
        { timeout: 45_000 }).toBe(successor.provider_session_id);
      expect(field(await record(successor.gwt_session_id!), "agent_member_id")).toBe(selected.agent_member_id);
      expect(() => process.kill(successor.pid, 0)).not.toThrow();
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([successor.gwt_session_id]);
      await expect(issues.locator('[data-queue-column="active"] [data-issue-number="45"]')).toBeVisible();
      await expect(issues.locator('[data-metric="active"]')).toHaveText("Active 1 · 1 issue · PM 0");
      await info.attach("fixed-assignment-exit-admission", { body: JSON.stringify({
        fixed, selected, otherLaunch, selectedLaunch, successor, fixedClaims,
      }, null, 2), contentType: "application/json" });
      await info.attach(`fixed-assignment-admitted-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned fixture cleanup").toEqual([]);
  });

  test("T94419 ordinary exit skips a foreign live claim and admits the next Issue", async ({ page }, info) => {
    const now = new Date();
    const foreignClaim = {
      claim_id: "gwt-auto-improve:t94419-foreign", owner: "foreign-host/foreign-session", issue_number: 44,
      status: "active", heartbeat_at: now.toISOString(),
      expires_at: new Date(now.getTime() + 60 * 60 * 1000).toISOString(),
      launched_work_id: "work/issue-44",
    };
    const foreignBody = `<!-- gwt-auto-improve-claim v1 -->\n\`\`\`json\n${JSON.stringify(foreignClaim)}\n\`\`\`\n<!-- /gwt-auto-improve-claim -->`;
    const fixture = await startIssueQueueLaunchFixture(info, {
      additionalIssueNumbers: [44, 45], autonomousMode: true, terminalConvergence: true,
      monitorPreferences: { max_active_agents: 1 },
      initialComments: { 44: [{ body: foreignBody, updated_at: now.toISOString() }] },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const nativeSession = async (id: string) => (await readFile(
      join(fixture.home, ".gwt", "sessions", `${id}.toml`), "utf8"))
      .match(/^agent_session_id\s*=\s*"([^"]*)"/m)?.[1];
    const activity = () => page.evaluate(() =>
      ((window as any).__gwtPlaywrightMessages ?? [])
        .map(({ payload }: any) => payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity)
        .filter((snapshot: any) => snapshot?.project_key === location.pathname.split("/")[2])
        .at(-1));
    const blocked = () => page.evaluate(() =>
      ((window as any).__gwtPlaywrightMessages ?? []).findLast(({ payload }: any) =>
        payload.kind === "issue_monitor_inbox")?.payload.items.find((row: any) => row.issue.number === 44));
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await page.locator('[data-surface="issues"]').click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const issues = page.locator('.workspace-window[data-preset="issue"] .issue-bridge-root');
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 });
      await expect.poll(async () => JSON.parse(await readFile(
        join(fixture.home, ".gwt/machine-state/agent-settings.json"), "utf8")))
        .toMatchObject({ mode: "manual", manual_limit: 1 });
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [43] });
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43]);
      const source = (await fixture.launches())[0];
      expect(source.gwt_session_id).toBeTruthy();
      await expect.poll(() => nativeSession(source.gwt_session_id!), { timeout: 45_000 }).toBe(source.provider_session_id);
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([source.gwt_session_id]);
      expect(() => process.kill(source.pid, 0)).not.toThrow();
      const sourceWindow = (await activity()).agents[0].window_id;
      const originalComments = fixture.commentsForIssue(44);
      expect(originalComments).toEqual([expect.objectContaining({ body: foreignBody })]);
      const foreignComment = originalComments[0];
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [44, 45] });
      await expect(issues.locator('[data-queue-column="queued"] [data-issue-number="45"]')).toBeVisible();
      const previousScan = (await preferences()).last_scan_at;
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: sourceWindow, data: "GWT_FIXTURE_EXIT\r" });
      await expect.poll(() => {
        try { process.kill(source.pid, 0); return false; } catch { return true; }
      }, { timeout: 60_000 }).toBe(true);
      // Observe the natural claim/launch path without a status call that could drive it.
      await expect.poll(blocked, { timeout: 90_000 }).toMatchObject({
        state: "blocked_by_claim", blocked_by_owner: foreignClaim.owner,
        blocked_by_claim_id: foreignClaim.claim_id, claim_expires_at: foreignClaim.expires_at,
      });
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43, 45]);
      const target = (await fixture.launches())[1];
      expect(target.pid).not.toBe(source.pid);
      expect(target.gwt_session_id).toBeTruthy();
      expect(target.gwt_session_id).not.toBe(source.gwt_session_id);
      expect(target.provider_session_id).not.toBe(source.provider_session_id);
      await expect.poll(() => nativeSession(target.gwt_session_id!), { timeout: 45_000 }).toBe(target.provider_session_id);
      expect(() => process.kill(target.pid, 0)).not.toThrow();
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([target.gwt_session_id]);
      await expect.poll(async () => {
        const prefs = await preferences();
        return !!prefs.last_scan_at && prefs.last_scan_at !== previousScan && !prefs.last_scan_error;
      }, { timeout: 90_000 }).toBe(true);
      expect(await preferences()).toMatchObject({ enabled: true, autonomous_mode: true });
      const foreignRow = issues.locator('[data-issue-number="44"]');
      await expect(foreignRow).toContainText("Claimed elsewhere");
      await expect(foreignRow.locator('[data-action="focus-surface-window"]')).toHaveCount(0);
      await expect(issues.locator('[data-queue-column="active"] .knowledge-row')).toHaveCount(1);
      await expect(issues.locator('[data-queue-column="active"] [data-issue-number="45"]')).toBeVisible();
      await expect(issues.locator('[data-metric="active"]')).toHaveText("Active 1 · 1 issue · PM 0");
      await expect(issues.locator('[data-issue-number="43"]')).toBeVisible();
      expect(fixture.requests.some(request => request.method === "POST"
        && request.url === "/repos/fixture/queue/issues/44/comments")).toBe(false);
      expect(fixture.requests.some(request => request.method === "PATCH"
        && request.url === `/repos/fixture/queue/issues/comments/${foreignComment.id}`)).toBe(false);
      expect(fixture.commentsForIssue(44), "the foreign claim remains byte-for-byte unchanged").toEqual(originalComments);
      expect(await fixture.launchIssues()).toEqual([43, 45]);
      await info.attach("foreign-claim-next-admission", { body: JSON.stringify({ foreignClaim,
        source, target, blocked: await blocked(), requests: fixture.requests }, null, 2), contentType: "application/json" });
      await info.attach(`foreign-claim-next-admission-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned fixture cleanup").toEqual([]);
  });

  test("T94420 ordinary exit preserves an unfinished dependency and admits the next Issue", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, {
      additionalIssueNumbers: [44, 45], autonomousMode: true, terminalConvergence: true,
      monitorPreferences: { max_active_agents: 1 },
      initialBodies: { 44: "## Acceptance Criteria\n- [ ] AC-1: Record the provider handoff.\n\n## Dependencies\nBlocked-by: #43" },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const nativeSession = async (id: string) => (await readFile(
      join(fixture.home, ".gwt", "sessions", `${id}.toml`), "utf8"))
      .match(/^agent_session_id\s*=\s*"([^"]*)"/m)?.[1];
    const activity = () => page.evaluate(() =>
      ((window as any).__gwtPlaywrightMessages ?? [])
        .map(({ payload }: any) => payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity)
        .filter((snapshot: any) => snapshot?.project_key === location.pathname.split("/")[2])
        .at(-1));
    const inbox = () => page.evaluate(() =>
      ((window as any).__gwtPlaywrightMessages ?? []).findLast(({ payload }: any) =>
        payload.kind === "issue_monitor_inbox")?.payload.items);
    const dependent = async () => (await inbox())?.find((row: any) => row.issue.number === 44);
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await page.locator('[data-surface="issues"]').click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const issues = page.locator('.workspace-window[data-preset="issue"] .issue-bridge-root');
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 });
      await expect.poll(async () => JSON.parse(await readFile(
        join(fixture.home, ".gwt/machine-state/agent-settings.json"), "utf8")))
        .toMatchObject({ mode: "manual", manual_limit: 1 });
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [43] });
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43]);
      const source = (await fixture.launches())[0];
      expect(source.gwt_session_id).toBeTruthy();
      await expect.poll(() => nativeSession(source.gwt_session_id!), { timeout: 45_000 }).toBe(source.provider_session_id);
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([source.gwt_session_id]);
      expect(() => process.kill(source.pid, 0)).not.toThrow();
      const sourceWindow = (await activity()).agents[0].window_id;
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [44, 45] });
      await expect(issues.locator('[data-queue-column="queued"] [data-issue-number="45"]')).toBeVisible();
      await expect.poll(dependent, { timeout: 90_000 }).toMatchObject({
        state: "blocked_by_dependency", dependency: { blocked_by: [{ number: 43 }] },
      });
      expect(await fixture.launchIssues()).toEqual([43]);
      const previousScan = (await preferences()).last_scan_at;
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: sourceWindow, data: "GWT_FIXTURE_EXIT\r" });
      await expect.poll(() => {
        try { process.kill(source.pid, 0); return false; } catch { return true; }
      }, { timeout: 60_000 }).toBe(true);
      // Observe the natural claim/launch path without a status call that could drive it.
      await expect.poll(dependent, { timeout: 90_000 }).toMatchObject({
        state: "blocked_by_dependency", dependency: { blocked_by: [{ number: 43 }] },
      });
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43, 45]);
      const target = (await fixture.launches())[1];
      expect(target.pid).not.toBe(source.pid);
      expect(target.gwt_session_id).toBeTruthy();
      expect(target.gwt_session_id).not.toBe(source.gwt_session_id);
      expect(target.provider_session_id).not.toBe(source.provider_session_id);
      await expect.poll(() => nativeSession(target.gwt_session_id!), { timeout: 45_000 }).toBe(target.provider_session_id);
      expect(() => process.kill(target.pid, 0)).not.toThrow();
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([target.gwt_session_id]);
      await expect.poll(async () => {
        const prefs = await preferences();
        return !!prefs.last_scan_at && prefs.last_scan_at !== previousScan && !prefs.last_scan_error;
      }, { timeout: 90_000 }).toBe(true);
      expect(await preferences()).toMatchObject({ enabled: true, autonomous_mode: true });
      await expect.poll(async () => (await inbox())?.find((row: any) => row.issue.number === 43)?.issue.state)
        .toBe("open");
      const dependentRow = issues.locator('[data-queue-column="queued"] [data-issue-number="44"]');
      await expect(dependentRow).toContainText("Blocked by dependency");
      await expect(dependentRow).toContainText("Blocked by #43");
      await expect(dependentRow.locator('[data-action="focus-surface-window"]')).toHaveCount(0);
      await expect(issues.locator('[data-queue-column="active"] .knowledge-row')).toHaveCount(1);
      await expect(issues.locator('[data-queue-column="active"] [data-issue-number="45"]')).toBeVisible();
      await expect(issues.locator('[data-metric="active"]')).toHaveText("Active 1 · 1 issue · PM 0");
      await expect(issues.locator('[data-issue-number="43"]')).toBeVisible();
      expect(fixture.requests.some(request => request.method === "POST"
        && request.url === "/repos/fixture/queue/issues/44/comments")).toBe(false);
      expect(fixture.commentsForIssue(44), "the dependent Issue never acquires a claim").toEqual([]);
      expect(fixture.requests.some(request => request.method === "PATCH"
        && request.url === "/repos/fixture/queue/issues/43")).toBe(false);
      await expect.poll(dependent).toMatchObject({
        state: "blocked_by_dependency", dependency: { blocked_by: [{ number: 43 }] },
      });
      expect(await fixture.launchIssues()).toEqual([43, 45]);
      await info.attach("unfinished-dependency-next-admission", { body: JSON.stringify({
        source, target, dependent: await dependent(), inbox: await inbox(), requests: fixture.requests }, null, 2), contentType: "application/json" });
      await info.attach(`unfinished-dependency-next-admission-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned fixture cleanup").toEqual([]);
  });

  test("T94418 operator hold survives ordinary exit until explicit requeue", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, {
      additionalIssueNumbers: [44], autonomousMode: true, terminalConvergence: true,
      monitorPreferences: { max_active_agents: 1 },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const nativeSession = async (id: string) => (await readFile(
      join(fixture.home, ".gwt", "sessions", `${id}.toml`), "utf8"))
      .match(/^agent_session_id\s*=\s*"([^"]*)"/m)?.[1];
    const activity = () => page.evaluate(() =>
      ((window as any).__gwtPlaywrightMessages ?? [])
        .map(({ payload }: any) => payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity)
        .filter((snapshot: any) => snapshot?.project_key === location.pathname.split("/")[2])
        .at(-1));
    const exited = (pid: number) => { try { process.kill(pid, 0); return false; } catch { return true; } };
    const sourceClaims = () => fixture.comments.filter(comment =>
      comment.body.startsWith("<!-- gwt-auto-improve-claim v1 -->")).length;
    const hold = async () => (await preferences()).failed_issues?.find((entry: any) => entry.issue_number === 43);
    const reason = "T94418 explicit operator hold";
    try {
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await page.locator('[data-surface="issues"]').click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const issues = page.locator('.workspace-window[data-preset="issue"] .issue-bridge-root');
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 });
      await expect.poll(async () => JSON.parse(await readFile(
        join(fixture.home, ".gwt/machine-state/agent-settings.json"), "utf8")))
        .toMatchObject({ mode: "manual", manual_limit: 1 });
      await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [43, 44] });
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43]);
      const source = (await fixture.launches())[0];
      expect(source.gwt_session_id).toBeTruthy();
      await expect.poll(() => nativeSession(source.gwt_session_id!), { timeout: 45_000 }).toBe(source.provider_session_id);
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([source.gwt_session_id]);
      const sourceWindow = (await activity()).agents[0].window_id;
      expect(() => process.kill(source.pid, 0)).not.toThrow();
      const beforeClaims = sourceClaims();
      expect(beforeClaims).toBeGreaterThan(0);
      const projectKey = await page.evaluate(() => location.pathname.split("/")[2]);
      await expect.poll(async () => {
        const confirmation = (await preferences()).launch_confirmations?.[43];
        const workspace = JSON.parse(await readFile(join(fixture.home, ".gwt", "projects", projectKey, "workspace.json"), "utf8"));
        const rows = workspace.windows.filter((row: any) => row.session_id === source.gwt_session_id);
        return !!confirmation?.claim_id && !!confirmation?.delivery_id && rows.length === 1
          && confirmation.window_id === `${projectKey}::${rows[0].id}`;
      }, { timeout: 30_000, message: "the exact physical launch has a durable Monitor receipt" }).toBe(true);
      const statusReply = fixture.fixtureOperation("issue.monitor.status");
      expect(statusReply.ok, JSON.stringify(statusReply)).toBe(true);
      const identity = JSON.parse(statusReply.output).inbox.find((row: any) => row.issue_number === 43);
      expect(identity.claim_id).toBeTruthy();
      expect(identity.launched_window_id).toBeTruthy();
      const confirmation = (await preferences()).launch_confirmations[43];
      expect(confirmation).toMatchObject({ claim_id: identity.claim_id, window_id: identity.launched_window_id });
      expect(confirmation.delivery_id).toBeTruthy();
      // Stop reads the loopback GitHub service on this worker; a synchronous child would starve it.
      const stoppedReply = await fixture.operationAsync("issue.monitor.stop", {
        number: 43, reason, remove_from_queue: false, claim_id: identity.claim_id,
        delivery_id: confirmation.delivery_id, window_id: identity.launched_window_id,
      });
      expect(stoppedReply.ok, JSON.stringify(stoppedReply)).toBe(true);
      const stopped = JSON.parse(stoppedReply.output);
      expect(stopped).toMatchObject({ status: "stopped", removed_from_queue: false, runtime_stop: "requested" });
      expect(stopped.stopped_window_id).toBe(identity.launched_window_id);
      await expect.poll(() => exited(source.pid), { timeout: 60_000 }).toBe(true);
      // Remove the exited diagnostic pane so only the hold can prevent a duplicate launch.
      const windows = async () => JSON.parse(await readFile(join(fixture.home, ".gwt", "projects",
        statusReply.project_store.hash, "workspace.json"), "utf8")).windows;
      expect((await windows()).some((window: any) => window.session_id === source.gwt_session_id)).toBe(true);
      await sendLiveGwtEvent(page, { kind: "close_window", id: sourceWindow });
      await expect.poll(async () => (await windows()).some((window: any) =>
        window.session_id === source.gwt_session_id), { timeout: 30_000 }).toBe(false);
      await expect.poll(hold).toMatchObject({ message: `stopped: ${reason}` });
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43, 44]);
      const other = (await fixture.launches())[1];
      expect(other.gwt_session_id).toBeTruthy();
      expect(other.gwt_session_id).not.toBe(source.gwt_session_id);
      expect(other.provider_session_id).not.toBe(source.provider_session_id);
      await expect.poll(() => nativeSession(other.gwt_session_id!), { timeout: 45_000 }).toBe(other.provider_session_id);
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([other.gwt_session_id]);
      expect(() => process.kill(other.pid, 0)).not.toThrow();
      const otherWindow = (await activity()).agents[0].window_id;
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: otherWindow, data: "GWT_FIXTURE_EXIT\r" });
      await expect.poll(() => exited(other.pid), { timeout: 60_000 }).toBe(true);
      await expect.poll(async () => (await activity())?.agents, { timeout: 60_000 }).toEqual([]);
      const previousScan = (await preferences()).last_scan_at;
      // Observe a real successful scan after physical exit, without driving it with a status read.
      await expect.poll(async () => {
        const prefs = await preferences();
        return !!prefs.last_scan_at && prefs.last_scan_at !== previousScan && !prefs.last_scan_error;
      }, { timeout: 90_000 }).toBe(true);
      const held = await hold();
      expect(held).toMatchObject({ message: `stopped: ${reason}` });
      const prefs = await preferences();
      expect(prefs).toMatchObject({ enabled: true, autonomous_mode: true });
      expect(Object.values(prefs.terminal_queues).flatMap((queue: any) => queue.entries)
        .some((entry: any) => entry.number === 43)).toBe(true);
      expect(sourceClaims(), "ordinary exit and rescan must not claim the held Issue").toBe(beforeClaims);
      expect(await fixture.launchIssues()).toEqual([43, 44]);
      await expect(issues.locator('[data-metric="active"]')).toHaveText("Active 0 · 0 issues · PM 0");
      await expect(issues.locator('[data-issue-number="43"]')).toBeVisible();
      await info.attach(`operator-hold-${theme}`, { body: await page.screenshot(), contentType: "image/png" });

      const requeued = fixture.fixtureOperation("issue.monitor.requeue", { number: 43, reason: "T94418 explicit release" });
      expect(requeued.ok, JSON.stringify(requeued)).toBe(true);
      const release = JSON.parse(requeued.output);
      expect(release.status).toBe("requeued");
      await info.attach("operator-hold-release", { body: JSON.stringify(release), contentType: "application/json" });
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43, 44, 43]);
      const resumed = (await fixture.launches())[2];
      expect(resumed.gwt_session_id).toBeTruthy();
      expect([source.gwt_session_id, other.gwt_session_id]).not.toContain(resumed.gwt_session_id);
      expect([source.provider_session_id, other.provider_session_id]).not.toContain(resumed.provider_session_id);
      expect([source.pid, other.pid]).not.toContain(resumed.pid);
      await expect.poll(() => nativeSession(resumed.gwt_session_id!), { timeout: 45_000 }).toBe(resumed.provider_session_id);
      expect(() => process.kill(resumed.pid, 0)).not.toThrow();
      await expect.poll(async () => (await activity())?.agents?.map((agent: any) => agent.session_id))
        .toEqual([resumed.gwt_session_id]);
      expect(await hold()).toBeUndefined();
      await expect(issues.locator('[data-queue-column="active"] [data-issue-number="43"]')).toBeVisible();
      await expect(issues.locator('[data-metric="active"]')).toHaveText("Active 1 · 1 issue · PM 0");
      await info.attach("operator-hold-exit-requeue", { body: JSON.stringify({
        source, other, stopped, held, beforeClaims, resumed, launches: await fixture.launches(),
      }, null, 2), contentType: "application/json" });
      await info.attach(`operator-hold-released-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned fixture cleanup").toEqual([]);
  });

  test("Issue #7451 shared HOME returns one machine slot to a second project and resumes the original owner", async ({ page }, info) => {
    // Each admission after a refusal waits for the Monitor's ordinary retry backoff.
    test.setTimeout(1_500_000);
    const fixture = await startIssueQueueLaunchFixture(info, {
      autonomousMode: true, secondProject: { issueNumbers: [45] },
      // Keep the ordinary retry path, with a wait short enough for a native CI host.
      monitorPreferences: { max_active_agents: 1, autonomous_tuning: { max_attempts: 3, stuck_timeout_secs: 1800,
        heartbeat_interval_secs: 120, merge_watch_timeout_secs: 3600, deliver_fix_loop_cap: 5,
        retry_backoff_base_secs: 5, retry_backoff_cap_secs: 10 } },
    });
    const secondPage = await page.context().newPage().catch(async error => { await fixture.stop(); throw error; });
    const errors: string[] = [];
    for (const target of [page, secondPage]) {
      target.on("pageerror", error => errors.push(error.message));
      target.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    }
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
    const secondPreferences = async () => JSON.parse((await fixture.second.preferencesBytes()).toString());
    const machine = async (name: string) => JSON.parse(await readFile(join(fixture.home, ".gwt/machine-state", name), "utf8"));
    type Launch = Awaited<ReturnType<typeof fixture.launches>>[number];
    type RecordedProcess = { pid: number; birth: string; session_id: string | null };
    const timeline: Record<string, unknown>[] = [];
    const mark = (stage: string, details: Record<string, unknown> = {}) =>
      timeline.push({ stage, at: new Date().toISOString(), ...details });
    const capture = (stage: string, launch: Launch): RecordedProcess => {
      const process = { pid: launch.pid, birth: fixture.providerProcessBirth(launch.pid), session_id: launch.gwt_session_id };
      mark(stage, { process, launch_at: launch.at });
      expect(process.birth, "creation time of the fixture-owned provider").toBeTruthy();
      return process;
    };
    const liveProcesses = (stage: string, ...providers: RecordedProcess[]) => {
      const observed = providers.map(provider => ({ ...provider, observed_birth: fixture.processBirth(provider.pid) }));
      mark(stage, { observed });
      return observed.filter(provider => provider.observed_birth === provider.birth)
        .map(({ observed_birth: _observedBirth, ...provider }) => provider);
    };
    const waitForExit = async (stage: string, provider: RecordedProcess) => {
      let observedBirth: string;
      await expect.poll(() => {
        observedBirth = fixture.processBirth(provider.pid);
        return observedBirth;
      }, { timeout: 60_000, message: stage }).not.toBe(provider.birth);
      mark(stage, { process: provider, observed_birth: observedBirth! });
    };
    // A status read refused by a busy daemon is repeated within a bound; each refusal stays in the timeline.
    const status = (project?: string) => readOperationOutput("issue.monitor.status",
      () => fixture.operationAsync("issue.monitor.status", {}, project),
      { onRefusal: (refusal: Record<string, unknown>) => mark("operation refused", refusal) });
    const inboxState = async (number: number, project?: string) =>
      (await status(project)).inbox.find((row: any) => row.issue_number === number)?.state;
    const activity = (target: Page) => target.evaluate(() =>
      ((window as any).__gwtPlaywrightMessages ?? [])
        .map(({ payload }: any) => payload.kind === "local_agent_activity" ? payload.activity : payload.projection?.local_agent_activity)
        .filter((snapshot: any) => snapshot?.project_key === location.pathname.split("/")[2])
        .at(-1));
    // Structured log rows from the shared HOME and from each project's Host.
    const logRows = async (matches: (row: any) => boolean) => {
      const rows: any[] = [];
      const projects = join(fixture.home, ".gwt", "projects");
      const directories = [join(fixture.home, ".gwt", "logs"),
        ...(await readdir(projects)).map(key => join(projects, key, "logs"))];
      for (const directory of directories) {
        for (const file of await readdir(directory).catch(() => [])) {
          if (!/\.log(?:\.|$)/.test(file)) continue;
          for (const line of (await readFile(join(directory, file), "utf8")).split("\n")) {
            let row;
            try { row = JSON.parse(line); } catch { continue; }
            if (row?.fields && matches(row)) rows.push(row);
          }
        }
      }
      return rows;
    };
    // The shared budget's own refusal, as each project's Host recorded it.
    const capacityRefusals = async (number: number) =>
      (await logRows(row => row.fields.issue_number === number
        && String(row.fields.reason ?? "").startsWith("Machine Agent capacity is occupied;")))
        .map(row => row.timestamp);
    // Issue #7758 AC-1: where each daemon control's time to ACK went on this host.
    const controlTimings = async () =>
      (await logRows(row => row.fields.control_apply_ms !== undefined || row.fields.control_prefs_commit_ms !== undefined))
        .map(row => ({ timestamp: row.timestamp, ...row.fields }))
        .sort((left, right) => String(left.timestamp).localeCompare(String(right.timestamp)));
    // Two completed scans: the decision under test was taken with the slot already occupied.
    const scans = async (read: () => Promise<any>, subject: string) => {
      for (const pass of ["first", "second"]) {
        const previous = (await read()).last_scan_at;
        await expect.poll(async () => {
          const prefs = await read();
          return !!prefs.last_scan_at && prefs.last_scan_at !== previous && !prefs.last_scan_error;
        }, { timeout: 180_000, message: `${pass} scan of the ${subject}` }).toBe(true);
      }
    };
    const queue = async (target: Page, number: number, read: () => Promise<any>) => {
      await expect.poll(async () => {
        const prefs = await read();
        return !!prefs.last_scan_at && !prefs.last_scan_error;
      }, { timeout: 180_000, message: `the Issue #${number} Monitor has completed its initial scan` }).toBe(true);
      await expect.poll(async () => {
        const after = await target.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
        mark("queue request", { issue_number: number });
        await sendLiveGwtEvent(target, { kind: "issue_monitor_queue_push", issue_numbers: [number] });
        const handle = await target.waitForFunction(({ after, number }) =>
          (window as any).__gwtPlaywrightMessages?.find(({ sequence, payload }: any) =>
            sequence > after && payload.kind === "issue_monitor_queue_result" && payload.operation === "push"
            && payload.issue_numbers?.length === 1 && payload.issue_numbers[0] === number)?.payload,
        { after, number }, { timeout: 30_000 });
        const { error } = await handle.jsonValue();
        await handle.dispose();
        mark("queue response", { issue_number: number, error });
        // This refusal is emitted before the daemon connection is attempted.
        // Any other error must fail the test.
        if (error && !/^publish budget exhausted during scope\/bootstrap resolution after \d+ms$/.test(error)) {
          throw new Error(`Queue push for Issue #${number} failed: ${error}`);
        }
        return error;
      }, { timeout: 120_000, message: `queue push for Issue #${number} is acknowledged` }).toBeNull();
    };
    const shots = async (stage: string) => {
      await info.attach(`shared-pool-${stage}-first-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      await info.attach(`shared-pool-${stage}-second-${theme}`, { body: await secondPage.screenshot(), contentType: "image/png" });
    };
    let closeRequest: { windowId: string; at: string } | undefined;
    const closeResult = () => closeRequest
      ? fixture.ownerCloseResult(closeRequest.windowId, closeRequest.at)
      : Promise.resolve(null);
    try {
      for (const target of [page, secondPage])
        await target.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      const firstKey = fixture.fixtureOperation("issue.monitor.status").project_store.hash;
      expect(fixture.second.key).toBeTruthy();
      expect(fixture.second.key).not.toBe(firstKey);
      await gotoLiveGwt(page, liveGwtProjectUrl(fixture.url, firstKey), { enableTestBridge: true });
      await page.locator('[data-surface="issues"]').click();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await sendLiveGwtEvent(page, { kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 });
      await expect.poll(() => machine("agent-settings.json")).toMatchObject({ mode: "manual", manual_limit: 1 });
      await gotoLiveGwt(secondPage, liveGwtProjectUrl(fixture.url, fixture.second.key), { enableTestBridge: true });
      await secondPage.locator('[data-surface="issues"]').click();
      await expect(secondPage.locator("html")).toHaveAttribute("data-theme", theme);

      // 1. The original owner takes the only machine slot.
      await queue(page, 43, preferences);
      await expect.poll(() => fixture.launchIssues(), { timeout: 90_000 }).toEqual([43]);
      const source = (await fixture.launches())[0];
      const sourceProcess = capture("original owner launched", source);
      await expect.poll(async () => !!(await preferences()).launch_confirmations?.[43]?.delivery_id, { timeout: 30_000 }).toBe(true);
      await expect.poll(async () => (await activity(page))?.agents?.map((agent: any) => agent.session_id),
        { timeout: 60_000 }).toEqual([source.gwt_session_id]);
      const sourceRuntime = (await activity(page)).agents[0];
      const sourceWindow = sourceRuntime.window_id;
      const sourceAssignment = (await machine("agent-registry.json")).project_assignments[firstKey];
      expect(Object.values(sourceAssignment)).toEqual([expect.objectContaining({
        project_key: firstKey, issue_number: 43, session_id: source.gwt_session_id })]);

      // 2. The second project is ready, but the shared limit keeps it out. A Host whose
      // capacity sample is still warming attempts the launch and the shared registry
      // refuses it; a Host with a settled sample counts the other project and waits.
      await queue(secondPage, 45, secondPreferences);
      const withheld = async () => {
        const second = await status(fixture.second.project);
        const refusals = await capacityRefusals(45);
        const counted = second.agent_capacity.other_live_agents === 1 && second.launch_capacity.admission_saturated === true;
        return { second, refusals, protectedBy: refusals.length > 0 ? "refused" : counted ? "counted" : null };
      };
      await expect.poll(async () => (await withheld()).protectedBy, { timeout: 180_000,
        message: "the shared limit refuses the second project's launch or makes it count the original owner" }).not.toBeNull();
      // The decision must hold across further scans, not only at one instant.
      await scans(secondPreferences, "second project while the slot is occupied");
      const { second: held, refusals: refusedSecond, protectedBy } = await withheld();
      await info.attach("issue-7451-second-project-held", { body: JSON.stringify({
        protected_by: protectedBy, refusals: refusedSecond, agent_capacity: held.agent_capacity,
        launch_capacity: held.launch_capacity, inbox: held.inbox,
      }, null, 2), contentType: "application/json" });
      expect(protectedBy).not.toBeNull();
      expect(held.inbox.find((row: any) => row.issue_number === 45)?.state).not.toBe("launched");
      expect(await fixture.launchIssues()).toEqual([43]);
      expect(liveProcesses("original owner still live", sourceProcess)).toEqual([sourceProcess]);
      expect((await machine("agent-registry.json")).project_assignments[fixture.second.key] ?? {}).toEqual({});
      await shots("occupied");

      // 3. The original owner returns the slot; its Issue stays held.
      const identity = (await status()).inbox.find((row: any) => row.issue_number === 43);
      const confirmation = (await preferences()).launch_confirmations[43];
      mark("stop original owner");
      const stopped = operationOutput("issue.monitor.stop", await fixture.operationAsync("issue.monitor.stop", {
        number: 43, reason: "Issue #7451 slot return", remove_from_queue: false, claim_id: identity.claim_id,
        delivery_id: confirmation.delivery_id, window_id: identity.launched_window_id }));
      expect(stopped).toMatchObject({ status: "stopped", removed_from_queue: false, runtime_stop: "requested" });
      await waitForExit("original owner exited", sourceProcess);
      // Remove the exited diagnostic pane so only the hold keeps the Issue from relaunching.
      const windows = async () => JSON.parse(await readFile(join(fixture.home, ".gwt", "projects", firstKey, "workspace.json"), "utf8")).windows;
      expect((await windows()).some((window: any) => window.session_id === source.gwt_session_id)).toBe(true);
      closeRequest = { windowId: sourceWindow, at: new Date().toISOString() };
      mark("close original owner window", closeRequest);
      await sendLiveGwtEvent(page, { kind: "close_window", id: sourceWindow });
      await expect.poll(closeResult, { timeout: 30_000, message: "the owner answers the exact window Close" }).not.toBeNull();
      const reply = await closeResult();
      mark("owner close response", { reply });
      expect(reply).toMatchObject({
        ok: true, owner_pane_id: identity.launched_window_id, incarnation: sourceRuntime.runtime_incarnation,
      });
      await expect.poll(async () => (await windows()).some((window: any) => window.session_id === source.gwt_session_id),
        { timeout: 30_000 }).toBe(false);
      const hold = async () => (await preferences()).failed_issues?.find((entry: any) => entry.issue_number === 43);
      await expect.poll(hold, { timeout: 30_000 }).toMatchObject({ message: "stopped: Issue #7451 slot return" });

      // 4. The second project is admitted by its own ordinary retry.
      await expect.poll(() => fixture.launchIssues(), { timeout: 420_000,
        message: "the second project takes the returned slot without a rescue operation" }).toEqual([43, 45]);
      const other = (await fixture.launches())[1];
      const otherProcess = capture("second project launched", other);
      expect(liveProcesses("only the second project is live", sourceProcess, otherProcess)).toEqual([otherProcess]);
      expect(other.gwt_session_id).not.toBe(source.gwt_session_id);
      expect(await realpath(other.cwd)).not.toBe(await realpath(source.cwd));
      await expect.poll(async () => Object.values((await machine("agent-registry.json")).project_assignments[fixture.second.key] ?? {}),
        { timeout: 60_000 }).toEqual([expect.objectContaining({
        project_key: fixture.second.key, issue_number: 45, session_id: other.gwt_session_id })]);
      expect(await hold(), "admitting the other project does not release the operator hold").toBeTruthy();
      await expect.poll(async () => (await activity(secondPage))?.agents?.map((agent: any) => agent.session_id),
        { timeout: 60_000 }).toEqual([other.gwt_session_id]);
      const otherWindow = (await activity(secondPage)).agents[0].window_id;
      await shots("admitted");

      // 5. An explicit resume of the original owner waits for the slot too.
      mark("requeue original owner");
      const requeued = operationOutput("issue.monitor.requeue", await fixture.operationAsync("issue.monitor.requeue", {
        number: 43, reason: "Issue #7451 resume the original owner" }));
      expect(requeued.status).toBe("requeued");
      await expect.poll(hold, { timeout: 30_000 }).toBeUndefined();
      await expect.poll(async () => {
        const first = await status();
        return first.agent_capacity.other_live_agents === 1 && first.launch_capacity.admission_saturated === true;
      }, { timeout: 180_000, message: "the original project counts the second project's live worker" }).toBe(true);
      await scans(preferences, "original project while the slot is occupied");
      const waiting = await status();
      expect(waiting.agent_capacity.other_live_agents, "the original project counts the other project's live worker").toBe(1);
      expect(waiting.launch_capacity).toMatchObject({ admission_saturated: true, configured_max_active: 0 });
      expect(waiting.inbox.find((row: any) => row.issue_number === 43)?.state).not.toBe("launched");
      expect(await fixture.launchIssues()).toEqual([43, 45]);
      expect(liveProcesses("second project still live", otherProcess)).toEqual([otherProcess]);

      // 6. The second project returns the slot and the original owner resumes.
      mark("request second project exit");
      await sendLiveGwtEvent(secondPage, { kind: "terminal_input", id: otherWindow, data: "GWT_FIXTURE_EXIT\r" });
      await waitForExit("second project exited", otherProcess);
      await expect.poll(() => fixture.launchIssues(), { timeout: 420_000,
        message: "the original owner resumes in the returned slot" }).toEqual([43, 45, 43]);
      const resumed = (await fixture.launches())[2];
      const resumedProcess = capture("original owner resumed", resumed);
      expect(liveProcesses("only the resumed owner is live", sourceProcess, otherProcess, resumedProcess)).toEqual([resumedProcess]);
      expect(await realpath(resumed.cwd), "the original owner continues in its saved worktree").toBe(await realpath(source.cwd));
      await expect.poll(() => inboxState(43), { timeout: 60_000 }).toBe("launched");
      expect(await hold()).toBeUndefined();
      expect(await fixture.launchIssues()).toEqual([43, 45, 43]);
      await shots("resumed");
      await info.attach("issue-7451-shared-pool", { body: JSON.stringify({
        platform: process.platform, theme, first_project: firstKey, second_project: fixture.second.key,
        machine_policy: await machine("agent-settings.json"), source, stopped, other, requeued, resumed,
        second_project_protected_by: protectedBy, refused_second_project: refusedSecond,
        second_project_waiting: held.launch_capacity,
        original_owner_waiting: waiting.launch_capacity,
        launches: await fixture.launches(), timeline,
      }, null, 2), contentType: "application/json" });
      await info.attach("issue-7758-control-commit-timing", { body: JSON.stringify({
        platform: process.platform, theme, timeline: timeline.filter(row => String(row.stage).startsWith("queue ")),
        controls: await controlTimings(),
      }, null, 2), contentType: "application/json" });
    } catch (error) {
      // A native runner's HOME is not retained; keep both projects' view of the shared pool.
      const observedAt = new Date().toISOString();
      const [launches, first, second, policy, registry, capacity, ownerClose] = await Promise.all([
        fixture.launches(), status(), status(fixture.second.project),
        machine("agent-settings.json"), machine("agent-registry.json"), machine("agent-capacity.json"),
        closeResult(),
      ].map(promise => promise.catch(failure => String(failure))));
      await info.attach("issue-7451-failure-state", { body: JSON.stringify({
        observed_at: observedAt, launches, first, second, policy, registry, capacity, owner_close: ownerClose, timeline,
        control_commit_timing: await controlTimings().catch(failure => String(failure)),
      }, null, 2), contentType: "application/json" }).catch(() => {});
      throw error;
    } finally {
      try { await secondPage.close(); await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "zero console/page errors including owned fixture cleanup").toEqual([]);
  });

  test("Backlog drag launches once and Blocked generation stays stopped after app restart", async ({ page }, info) => {
    const fixture = await startIssueQueueLaunchFixture(info, { page });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    try {
      await page.addInitScript(theme => {
        if (location.protocol === "http:" || location.protocol === "https:") localStorage.setItem("gwt:ui:theme", theme);
      }, theme);
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
      await fixture.restartHost();
      expect(fixture.ownedPid).not.toBe(previousPid);
      const restored = await context.newPage();
      restored.on("pageerror", error => errors.push(error.message));
      restored.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      await restored.addInitScript(theme => {
        if (location.protocol === "http:" || location.protocol === "https:") localStorage.setItem("gwt:ui:theme", theme);
      }, theme);
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
      const retained = await agentWindow(restored);
      expect(retained.session_id).toBe(launch.gwt_session_id);
      expect(typeof retained.id === "string" && retained.id.length > 0).toBe(true);
      // The stopped diagnostic remains in the catalog; Works only shows live census members.
      await expect(restored.locator(`.workspace-window[data-id="${retained.id}"]`)).toHaveCount(1);
      await openWorks(restored);
      if (await restored.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
        await restored.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
      await expect(restored.locator(`.work-row[data-window-id="${retained.id}"]`)).toHaveCount(0);
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
    const fixture = await startIssueQueueLaunchFixture(info, { page });
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
      await page.addInitScript(theme => {
        if (location.protocol === "http:" || location.protocol === "https:") localStorage.setItem("gwt:ui:theme", theme);
      }, theme);
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
      await restored.addInitScript(theme => {
        if (location.protocol === "http:" || location.protocol === "https:") localStorage.setItem("gwt:ui:theme", theme);
      }, theme);
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
      await openWorks(restored);
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
