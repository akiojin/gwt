/** Issue #6934 AC-5: one real owner, four real Agent PTYs, continuous headed observation. */
import { readFile, readdir, writeFile } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { clearLiveLaunchWizard, gotoLiveGwt, sendLiveGwtEvent } from "../../crates/gwt/playwright/tests/_helpers/live-gwt";

const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME || "";
const base = process.env.GWT_PLAYWRIGHT_BASE_URL || "";
const delay = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));
type Agent = { id: string; session_id: string; index: number };

test("four Agent windows retain input and snapshot service without owner drain", async ({ page }, info) => {
  test.skip(!home || !base || process.platform !== "win32", "requires issue-6934-browser-check.ps1");
  const fixture = JSON.parse(await readFile(join(home, "fixture.json"), "utf8"));
  const theme = info.project.name.endsWith("light") ? "light" : "dark";
  const seconds = theme === "dark" ? fixture.duration_seconds : Math.min(10, fixture.duration_seconds);
  test.setTimeout((seconds + 600) * 1000);
  const errors: string[] = [], samples: any[] = [], agents: Agent[] = [];
  let startedAt = 0, passed = false;
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("pageerror", error => errors.push(error.message));
  try {
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    await gotoLiveGwt(page, base, { enableTestBridge: true, projectKey: fixture.repo_hash });
    await clearLiveLaunchWizard(page);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await prepareWorkloadMembers(page);
    for (let index = 0; index < 4; index++) {
      const agent = await launch(page, index, fixture.repo_hash, theme);
      agents.push({ ...agent, index });
      await expect.poll(async () => (await receipts()).some(receipt => receipt.session === agent.session_id),
        { timeout: 60_000, message: "a real custom Agent process starts" }).toBe(true);
    }
    expect(new Set(agents.map(agent => agent.session_id)).size).toBe(4);
    await page.locator('.op-rail__surface[data-surface="works"]').click();
    if (await page.locator(".works-surface:not(.works-surface--preview) .works-full:not([hidden])").isVisible())
      await page.locator('.works-surface:not(.works-surface--preview) [data-action="back"]').click();
    for (const agent of agents) await expect(page.locator(`.work-row[data-window-id="${agent.id}"]`)).toBeVisible();
    await page.evaluate(() => {
      (window as any).__gwt6934HealthFailures = [];
      for (const socket of (window as any).__gwtPlaywrightSockets as WebSocket[]) socket.addEventListener("message", event => {
        let payload;
        try { payload = JSON.parse(String(event.data)); } catch { return; }
        if (payload.kind === "project_owner_health" && payload.health?.state !== "healthy")
          (window as any).__gwt6934HealthFailures.push(payload.health);
      });
    });
    startedAt = Date.now();
    console.log(`Owner observation ${theme}: four real Agent PTYs, ${seconds}s`);
    let checkpoint = 0;
    const previousSequences = new Map<string, number>();
    while (true) {
      const sample: any = { observed_at: new Date().toISOString(), health: await ownerCommand(fixture.repo_hash, { type: "health" }) };
      expect(sample.health.project_key).toBe(fixture.repo_hash);
      expect(sample.health.draining, "owner never enters draining during the observation").toBe(false);
      expect(sample.health.reconciliation_required, "no unresolved exact cleanup during the observation").toBe(false);
      expect(sample.health.panes).toBeGreaterThanOrEqual(4);
      const listed = await ownerCommand(fixture.repo_hash, { type: "list_panes" });
      const panes = agents.map(agent => listed.panes.find((pane: any) => pane.session_id === agent.session_id));
      expect(panes.filter(Boolean), "all four original Sessions retain actual owner runtimes").toHaveLength(4);
      for (const pane of panes) {
        expect(pane.child_pid).toBeGreaterThan(0);
        expect(["stopped", "error"]).not.toContain(pane.status);
        expect(pane.exit_confirmed).toBe(false);
        if (previousSequences.has(pane.session_id))
          expect(pane.screen_seq, "PTY output sequence never moves backwards").toBeGreaterThanOrEqual(previousSequences.get(pane.session_id)!);
        previousSequences.set(pane.session_id, pane.screen_seq);
      }
      sample.sessions = panes.map((pane: any) => ({ session_id: pane.session_id, child_pid: pane.child_pid, screen_seq: pane.screen_seq }));
      if (checkpoint % 6 === 0 || Date.now() - startedAt >= seconds * 1000) {
        sample.responses = [];
        for (const [index, agent] of agents.entries()) {
          const pane = panes[index], token = `${theme}_${checkpoint}_${index}`;
          await sendLiveGwtEvent(page, { kind: "terminal_input", id: agent.id, data: `GWT6934_PING_${token}\r` });
          let snapshot: any;
          await expect.poll(async () => {
            snapshot = await ownerCommand(fixture.repo_hash, { type: "snapshot", id: pane.id, incarnation: pane.incarnation });
            return snapshot.screen;
          }, { timeout: 10_000, intervals: [100, 250] }).toContain(`GWT6934_ACK_${token}:`);
          expect(snapshot.session_id).toBe(agent.session_id);
          sample.responses.push({ session_id: agent.session_id, screen_seq: snapshot.screen_seq, acknowledged: true });
        }
      }
      samples.push(sample);
      await writeFile(join(home, `owner-${theme}.samples.jsonl`), JSON.stringify(sample) + "\n", { flag: "a" });
      expect(errors, "zero browser console/page errors").toEqual([]);
      expect(await page.evaluate(() => (window as any).__gwt6934HealthFailures)).toEqual([]);
      if (Date.now() - startedAt >= seconds * 1000) break;
      checkpoint++;
      await delay(Math.min(5000, seconds * 1000 - (Date.now() - startedAt)));
    }
    expect(Date.now() - startedAt).toBeGreaterThanOrEqual(seconds * 1000);
    expect(samples.length, "health and physical PTYs are observed throughout the run").toBeGreaterThanOrEqual(2);
    for (const session of samples[0].sessions) {
      expect(previousSequences.get(session.session_id), "each original PTY produces output during the observation")
        .toBeGreaterThan(session.screen_seq);
    }
    for (const agent of agents) await expect(page.locator(`.work-row[data-window-id="${agent.id}"]`)).toBeVisible();
    const logFailures = await ownerLogFailures(startedAt);
    expect(logFailures, "zero owner draining/reconciliation errors in the isolated logs").toEqual([]);
    await info.attach(`owner-windows-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    passed = true;
  } finally {
    if (!passed) await page.screenshot({ path: info.outputPath("owner-fixture-failure.png") }).catch(() => undefined);
    const evidence = { issue: 6934, fixture, theme, passed, started_at: startedAt ? new Date(startedAt).toISOString() : null,
      ended_at: new Date().toISOString(), actual_elapsed_ms: startedAt ? Date.now() - startedAt : 0,
      full_acceptance_observation: passed && theme === "dark" && seconds >= 1800,
      duration_seconds: seconds, agents, producer_receipts: await receipts(), samples, browser_errors: errors,
      owner_log_failures: startedAt ? await ownerLogFailures(startedAt) : [],
      user_verification_result: "n/a (autonomous)", agent_visual_check: passed ? "pass" : "fail",
      limitations: "Real command Agent PTYs without model inference. Dark measures the continuous soak; light is a separate functional check. Forced recovery is covered by focused Rust tests. Exact geometry is outside this owner-drain acceptance check." };
    const body = JSON.stringify(evidence, null, 2) + "\n";
    await writeFile(join(home, `owner-${theme}.json`), body);
    await info.attach(`owner-evidence-${theme}`, { body, contentType: "application/json" });
    for (const agent of agents) await sendLiveGwtEvent(page, { kind: "close_window", id: agent.id }).catch(() => undefined);
    await expect.poll(async () => {
      const listed = await ownerCommand(fixture.repo_hash, { type: "list_panes" });
      return listed.panes.filter((pane: any) => agents.some(agent => agent.session_id === pane.session_id)).length;
    }, { timeout: 60_000 }).toBe(0).catch(error => {
      if (passed) throw error;
      console.error(`Cleanup poll failed after an earlier failure: ${error}`);
    });
  }
});

async function receipts(): Promise<any[]> {
  const directory = join(home, "workload");
  return Promise.all((await readdir(directory)).filter(name => /^\d+\.json$/.test(name))
    .map(async name => JSON.parse(await readFile(join(directory, name), "utf8"))));
}

async function prepareWorkloadMembers(page: Page): Promise<void> {
  // A registered command tool also needs an enabled, idle fixed-tool person.
  // Allocate through the normal correlated roster API so revisions and member
  // reservations have exactly the same checks as an ordinary Agent launch.
  const request = async (action: unknown, expectedRevision: number | null = null) => {
    const requestId = randomUUID();
    await sendLiveGwtEvent(page, { kind: "agent_roster", action, expected_revision: expectedRevision, request_id: requestId });
    const reply = await page.waitForFunction(requestId => {
      const entry = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
        entry.payload.request_id === requestId && ["agent_roster", "agent_roster_error"].includes(entry.payload.kind));
      return entry?.payload;
    }, requestId, { timeout: 30_000 }).then(handle => handle.jsonValue());
    expect(reply.kind, reply.message ?? "normal workload roster allocation").toBe("agent_roster");
    expect(reply.roster.error).toBeFalsy();
    return reply.roster;
  };
  let roster = await request({ kind: "read" });
  for (let index = 0; index < 4; index++) {
    const agentId = `owner-workload-6934-${index}`;
    const available = () => roster.members.some((member: any) =>
      member.agent_id === agentId && member.enabled && !member.assignment);
    if (!available()) roster = await request({ kind: "add", agent_id: agentId }, roster.revision);
    expect(available(), `idle fixed-tool member for ${agentId}`).toBe(true);
  }
}

async function ownerCommand(projectKey: string, command: unknown): Promise<any> {
  // Private credentials stay in the Node controller, never the browser or evidence.
  const directory = join(home, ".gwt/projects", projectKey, "runtime/daemon");
  const endpoints = await Promise.all((await readdir(directory)).filter(name => name.endsWith(".json"))
    .map(async name => JSON.parse(await readFile(join(directory, name), "utf8"))));
  const endpoint = endpoints.find(value => value.scope.repo_hash === projectKey && value.project_transport_url);
  if (!endpoint) throw new Error("isolated project owner endpoint missing");
  const response = await fetch(`${endpoint.project_transport_url}/private/project/command`, { method: "POST",
    headers: { Authorization: `Bearer ${endpoint.auth_token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ project_key: projectKey, command }), signal: AbortSignal.timeout(10_000) });
  const result = await response.json();
  if (!response.ok) throw new Error(`owner command refused: ${result.error}`);
  return result;
}

async function ownerLogFailures(from: number): Promise<unknown[]> {
  const directory = join(home, ".gwt/logs"), failures: unknown[] = [];
  for (const name of (await readdir(directory).catch(() => [])).filter(name => name.startsWith("gwt.log."))) {
    for (const raw of (await readFile(join(directory, name), "utf8")).split("\n")) {
      let row;
      try { row = JSON.parse(raw); } catch { continue; }
      if (Date.parse(row.timestamp) < from) continue;
      const error = [row.fields?.error, row.fields?.reason, row.fields?.message].filter(Boolean).join(" ");
      if (/owner is draining|owner is shutting down|cleanup.*reconciliation|owner.*reconciliation.*(fail|pending)|error sending request.*private\/project\/command/i.test(error))
        failures.push(row);
    }
  }
  return failures;
}

async function launch(page: Page, index: number, projectKey: string, theme: string): Promise<{ id: string; session_id: string }> {
  const board = page.locator('.workspace-window[data-preset="agent_kanban"]').first();
  if (await board.count() === 0)
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "agent_kanban", bounds: { x: 20, y: 20, width: 800, height: 500 } });
  // Launch switches to Works. The setup board remains registered off-surface.
  await expect(board).toBeAttached();
  const request = (detail: unknown, requireOpen = false) => page.evaluate(({ detail, requireOpen }) => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse()
      .find(socket => socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("project socket missing");
    return new Promise<any>((resolve, reject) => {
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener("message", reply); };
      const reply = (event: MessageEvent) => {
        let payload;
        try { payload = JSON.parse(String(event.data)); } catch { return; }
        if (payload.kind === "launch_wizard_open_error") { cleanup(); reject(new Error(payload.message)); return; }
        if (payload.kind !== "launch_wizard_state" || (requireOpen && !payload.wizard) || (payload.wizard &&
          (payload.wizard.is_hydrating || payload.wizard.runtime_resolution_pending || payload.wizard.launch_materialization_pending))) return;
        cleanup(); resolve(payload);
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error("launch wizard timed out")); }, 90_000);
      socket.addEventListener("message", reply);
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
    });
  }, { detail, requireOpen });
  await request({ kind: "open_agent_kanban_launch_wizard", board_id: await board.getAttribute("data-id"), lane_id: "active" }, true);
  const action = (action: unknown) => request({ kind: "launch_wizard_action", action, bounds: { x: 20, y: 30, width: 900, height: 600 } });
  await action({ kind: "set_launch_path", path: "manual_setup" });
  await action({ kind: "set_branch_mode", create_new: true });
  await action({ kind: "set_branch_name", value: `fixture/owner-${theme}-${index}` });
  let state = await action({ kind: "set_agent", agent_id: `owner-workload-6934-${index}` });
  state = await action({ kind: "clear_linked_issue" });
  for (let step = 0; step < 12 && state.wizard; step++) {
    expect(state.wizard.error).toBeFalsy();
    if (state.wizard.selected_runtime_target !== "host") state = await action({ kind: "set_runtime_target", target: "Host" });
    else {
      expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
      state = await action({ kind: "submit" });
    }
  }
  expect(state.wizard).toBeNull();
  let physical: any;
  await expect.poll(async () => {
    const panes = await ownerCommand(projectKey, { type: "list_panes" });
    physical = panes.panes.find((pane: any) => pane.agent_id === `owner-workload-6934-${index}` && pane.session_id && pane.child_pid);
    // Command Agents have no provider hook to advance their projected status
    // from starting. The real process receipt and input/output prove liveness.
    return Boolean(physical && physical.screen_seq > 0 && !["stopped", "error"].includes(physical.status));
  }, { timeout: 90_000, message: "the actual owner PTY is running before using its final projected window identity" }).toBe(true);
  const projected = await page.waitForFunction(session => {
    const message = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) => entry.payload.kind === "workspace_state");
    return message?.payload.workspace.tabs.flatMap((tab: any) => tab.workspace.windows)
      .find((pane: any) => pane.session_id === session && !["stopped", "error"].includes(pane.status));
  }, physical.session_id, { timeout: 60_000 }).then(handle => handle.jsonValue());
  return { id: projected.id, session_id: projected.session_id };
}
