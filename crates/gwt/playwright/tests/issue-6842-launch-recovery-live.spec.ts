import { execFileSync } from "node:child_process";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { startIssueQueueLaunchFixture } from "./_helpers/issue-queue-launch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.skip(process.env.GWT_PLAYWRIGHT_ISSUE_6842 !== "1",
  "requires checkout binaries and the isolated Monitor/provider fixture");
test.use({ headless: false, viewport: { width: 1600, height: 1100 } });

for (const issueNumber of [5390, 3340]) {
test(`Issue #6861/#6842: saved runtime-less pane permits Issue #${issueNumber} launch`, async ({ page }, info) => {
  test.setTimeout(240_000);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  let projectState = "";
  const rawId = "agent-saved-without-runtime";
  const fixture = await startIssueQueueLaunchFixture(info, {
    issueNumber,
    async prepare({ home, bin, project, env }) {
      const daemon = join(bin, process.platform === "win32" ? "gwtd.exe" : "gwtd");
      const envelope = JSON.parse(execFileSync(daemon, [], { cwd: project, env,
        encoding: "utf8", timeout: 30_000, windowsHide: true,
        input: JSON.stringify({ schema_version: 1, operation: "issue.monitor.status", params: {} }),
      }));
      expect(envelope.ok).toBe(true);
      const key = envelope.project_store?.hash;
      expect(key).toMatch(/^[a-zA-Z0-9_-]+$/);
      projectState = join(home, ".gwt/projects", key, "project-state");
      await mkdir(projectState, { recursive: true });
      // Saved presentation is the only fabricated state. There is no Session,
      // execution ledger, owner runtime, launch capability, or delivery ACK.
      await writeFile(join(projectState, "../workspace.json"), JSON.stringify({
        windows: [{ id: rawId, title: "Saved Issue Agent", preset: "agent", status: "running",
          z_index: 1, agent_id: "codex", linked_issue_number: issueNumber,
          dynamic_title_detail: "Saved presentation without a native process" }],
        next_z_index: 2,
      }));
      // The recorder submits the launch prompt through its own generated hooks,
      // using only the native provider's ambient launch identity and environment.
      const providerPath = join(bin, process.platform === "win32" ? "codex.cjs" : "codex");
      const providerSource = await readFile(providerPath, "utf8");
      const readyMarker = "console.log('GWT_FAKE_PROVIDER_READY ' + native);";
      expect(providerSource.split(readyMarker).length - 1, "provider READY insertion point").toBeGreaterThanOrEqual(1);
      const submitPrompt = String.raw`
try {
  const hooks = JSON.parse(fs.readFileSync(path.join(process.cwd(), '.codex', 'hooks.json'), 'utf8'));
  const input = JSON.stringify({session_id:native,hook_event_name:'UserPromptSubmit',cwd:process.cwd(),
    transcript_path:rollout,prompt:args.at(-1),stop_hook_active:false});
  let delivered = false;
  for (const group of hooks.hooks?.UserPromptSubmit ?? []) for (const hook of group.hooks ?? []) {
    if (hook.type !== 'command') continue;
    const windows = process.platform === 'win32';
    const result = require('node:child_process').spawnSync(windows ? (process.env.ComSpec || 'cmd.exe') : '/bin/sh',
      windows ? ['/d','/s','/c',hook.command] : ['-c',hook.command],
      {input,cwd:process.cwd(),env:process.env,stdio:['pipe','ignore','inherit'],timeout:60000,
        windowsHide:true,windowsVerbatimArguments:windows});
    if (result.error || result.status !== 0) throw new Error('UserPromptSubmit hook failed: ' + (result.error?.message || result.status));
    delivered = true;
  }
  if (!delivered) throw new Error('UserPromptSubmit command hook is missing');
} catch (error) {
  console.error('GWT_FAKE_PROVIDER_USER_PROMPT_HOOK_ERROR ' + error.message);
  process.exit(1);
}
`;
      await writeFile(providerPath, providerSource.replace(readyMarker, submitPrompt + readyMarker));
    },
  });
  const preferences = async () => JSON.parse((await fixture.monitorPreferencesBytes()).toString());
  let evidence: Record<string, unknown> = { theme, issue_number: issueNumber, isolated_home: fixture.home, project: fixture.project };
  let passed = false;
  try {
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const saved = (await windows(page)).find((window: any) => window.id.endsWith(`::${rawId}`));
    expect(saved, "the saved Issue-linked Agent remains visible before launch").toBeTruthy();
    expect(saved.linked_issue_number).toBe(fixture.issueNumber);
    // Cold startup deliberately pauses saved process windows. Rust acceptance
    // separately covers a stale Running projection with no runtime handle.
    expect(saved.status).toBe("stopped");
    expect(saved.session_id ?? null).toBeNull();
    expect(saved.runtime_started_at_ms ?? null).toBeNull();
    expect(await fixture.launches(), "saved metadata has not spawned a provider").toEqual([]);
    evidence = { ...evidence, saved_window: saved };

    const started = Date.now();
    const startedAt = new Date(started).toISOString();
    await sendLiveGwtEvent(page, { kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] });
    let launch: any;
    let bound: any;
    let confirmation: any;
    await expect.poll(async () => {
      const providers = await fixture.launches();
      expect(providers.length, "only one provider is launched for this delivery").toBeLessThanOrEqual(1);
      const prefs = await preferences();
      expect(prefs.failed_issues ?? [], "no timeout or prepare-budget failure").toEqual([]);
      if (providers.length !== 1) return false;
      launch = providers[0];
      confirmation = prefs.launch_confirmations?.[fixture.issueNumber];
      Object.assign(evidence, { started_at: startedAt, elapsed_ms: Date.now() - started,
        provider: launch, confirmation: confirmation ?? null });
      bound = (await windows(page)).find((window: any) => window.session_id === launch.gwt_session_id);
      Object.assign(evidence, { bound_window: bound ?? null, gui_window_id: bound?.id ?? null,
        canonical_window_id: confirmation?.window_id ?? null });
      return !!bound && ["running", "idle"].includes(bound.status)
        && !!confirmation?.window_id && !!confirmation.delivery_id && !!confirmation.claim_id
        && prefs.launch_bindings?.[confirmation.window_id] === fixture.issueNumber
        && !prefs.pending_launch_deliveries?.some((delivery: any) => delivery.issue_number === fixture.issueNumber);
    }, { timeout: Math.max(1, 120_000 - (Date.now() - started)),
      message: "real provider, Session, running window and exact Monitor ACK within 120 seconds" }).toBe(true);
    const elapsedMs = Date.now() - started;
    expect(elapsedMs).toBeLessThan(120_000);
    expect(launch.pid).toBeGreaterThan(0);
    expect(() => process.kill(launch.pid, 0), "the recorded native provider PID is alive").not.toThrow();
    expect(launch.gwt_session_id).toBeTruthy();
    expect(bound.id).not.toBe(saved.id);
    expect(bound.linked_issue_number).toBe(fixture.issueNumber);
    const statusEnvelope = fixture.fixtureOperation("issue.monitor.status");
    expect(statusEnvelope.ok).toBe(true);
    const monitorStatus = typeof statusEnvelope.output === "string"
      ? JSON.parse(statusEnvelope.output) : statusEnvelope.output;
    Object.assign(evidence, { monitor_status: monitorStatus });
    expect(monitorStatus.active_launch_details, "canonical Monitor pane and GUI alias share the provider Session")
      .toContainEqual(expect.objectContaining({ issue_number: fixture.issueNumber,
        window_id: confirmation.window_id, session_id: launch.gwt_session_id }));
    const session = await readFile(join(fixture.home, ".gwt/sessions", `${launch.gwt_session_id}.toml`), "utf8");
    expect(session).toContain(`linked_issue_number = ${fixture.issueNumber}`);
    const phases = (await launchLogs(fixture.home, projectState)).filter(row =>
      row.target === "gwt.launch.timing" && Date.parse(row.timestamp) >= started
      && (row.fields?.delivery_id ?? row.span?.delivery_id) === confirmation.delivery_id);
    const spawn = phases.filter(row => row.fields?.phase === "monitor_gui_spawn");
    expect(spawn, "the exact delivery reaches native spawn once").toHaveLength(1);
    expect(phases.some(row => row.fields?.phase === "monitor_gui_reprepare"),
      "saved metadata must not force a same-delivery prepare loop").toBe(false);
    expect(fixture.comments.some(comment => comment.body.includes(confirmation.claim_id)),
      "the ACK belongs to the real loopback upstream claim").toBe(true);
    expect(errors, "zero console/page errors").toEqual([]);
    evidence = { ...evidence, started_at: startedAt, elapsed_ms: elapsedMs, provider: launch,
      bound_window: bound, confirmation, launch_phases: phases, hook_audits: fixture.hookAudits };
    await info.attach(`launch-recovery-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
    passed = true;
  } finally {
    try { await page.close(); } finally {
      try { await fixture.stop(); } finally {
        await info.attach(`launch-recovery-evidence-${theme}`, {
          body: JSON.stringify({ ...evidence, passed, browser_errors: errors,
            "User Verification Result": "n/a (autonomous)", "Agent Visual Check": passed ? "pass" : "fail",
            limitation: "Cold startup pauses saved Running windows. This real checkout/owner/PTY run verifies delivery past a retained runtime-less placeholder; Rust acceptance covers the stale Running projection and bounded repeated preparation directly. GUI combined window IDs are presentation aliases; canonical Monitor window identity is verified through active_launch_details with the same provider Session.",
          }, null, 2), contentType: "application/json",
        });
      }
    }
  }
  expect(errors, "zero console/page errors including owned cleanup").toEqual([]);
});
}

async function windows(page: Page): Promise<any[]> {
  const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
  await sendLiveGwtEvent(page, { kind: "list_windows" });
  return (await page.waitForFunction(after => (window as any).__gwtPlaywrightMessages
    ?.findLast((entry: any) => entry.sequence > after && entry.payload.kind === "window_list")?.payload.windows,
  after, { timeout: 10_000 })).jsonValue();
}

async function launchLogs(home: string, state: string): Promise<any[]> {
  const rows: any[] = [];
  for (const directory of [join(home, ".gwt/logs"), join(state, "../logs")]) {
    for (const name of await readdir(directory).catch(() => [] as string[])) {
      if (!/^gwt\.log\./.test(name)) continue;
      for (const line of (await readFile(join(directory, name), "utf8")).split("\n")) {
        try { rows.push(JSON.parse(line)); } catch { /* In-flight final log line. */ }
      }
    }
  }
  return rows;
}
