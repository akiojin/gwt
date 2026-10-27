/** #4735: real independent project owners, PTYs, producing Sessions and Hub recovery.
 * Run against browser-check's checkout binary and fresh HOME. No inbound health
 * frames are injected. Only the captured disposable Session runtime sidecar is
 * temporarily changed to reproduce an exact retirement failure (AC-6).
 */
import { spawn, execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, realpath, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { expect, test, type Page, type TestInfo } from "@playwright/test";
import { gotoLiveGwt, readLiveHubCatalog, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";
const ROOT = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
const GWTD = process.env.GWT_PLAYWRIGHT_GWTD ?? join(ROOT, `target/debug/gwtd${process.platform === "win32" ? ".exe" : ""}`);
const SHELL = process.platform === "win32" ? process.env.ComSpec ?? "cmd.exe" : "/bin/sh";
const SHELL_ARGS = process.platform === "win32" ? ["/d", "/q"] : ["-i"];
const ISSUE = 473501;
type Project = { root: string; key: string; endpoint: any; endpointFile: string; birth: string; owner: any; intentionallyKilled: boolean; pane?: Pane };
type Pane = { id: string; gui_id: string; incarnation: number; session_id: string; child_pid: number; child_birth: string };
type Fixture = { a: Project; b: Project; pa: Pane; pb: Pane; pages: Page[]; errors: string[]; theme: string };

test.skip(!BASE || !HOME, "requires a browser-check checkout and fresh HOME");
test.use({ headless: false, viewport: { width: 1440, height: 960 } });
test.setTimeout(240_000);

function isolatedEnv() {
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("GWT_")
    && !["GH_TOKEN", "GITHUB_TOKEN"].includes(key)));
  return { ...env, HOME, USERPROFILE: HOME, GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1" };
}

async function cli(project: string, operation: string, params: Record<string, unknown> = {}) {
  const child = spawn(GWTD, [], { cwd: project, env: isolatedEnv(), stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
  let output = "", error = "";
  child.stdout.on("data", data => { output += data; });
  child.stderr.on("data", data => { error += data; });
  const code = await new Promise<number | null>((resolve, reject) => {
    const timer = setTimeout(() => { child.kill(); reject(new Error(`${operation} timed out`)); }, 45_000);
    child.once("error", e => { clearTimeout(timer); reject(e); });
    child.once("close", code => { clearTimeout(timer); resolve(code); });
    child.stdin.end(JSON.stringify({ schema_version: 1, operation, params }) + "\n");
  });
  expect(code, `${operation}: ${output}\n${error}`).toBe(0);
  const result = JSON.parse(output);
  expect(result.ok, `${operation}: ${output}`).toBe(true);
  return { envelope: result, value: typeof result.output === "string" ? JSON.parse(result.output) : result.output };
}

function processBirth(pid: number) {
  if (!Number.isInteger(pid) || pid <= 0) return "";
  try {
    return process.platform === "win32"
      ? execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
        `(Get-Process -Id ${pid} -ErrorAction Stop).StartTime.ToUniversalTime().Ticks`],
      { encoding: "utf8", windowsHide: true }).trim()
      : execFileSync("ps", ["-o", "lstart=", "-p", String(pid)], { encoding: "utf8" }).trim();
  }
  catch { return ""; }
}

function processParent(pid: number) {
  expect(Number.isInteger(pid) && pid > 0, "real physical child PID").toBe(true);
  return process.platform === "win32"
    ? execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
      `(Get-CimInstance Win32_Process -Filter 'ProcessId = ${pid}').ParentProcessId`],
    { encoding: "utf8", windowsHide: true }).trim()
    : execFileSync("ps", ["-o", "ppid=", "-p", String(pid)], { encoding: "utf8" }).trim();
}

async function command(project: Project, command: Record<string, unknown>, timeout = 15_000) {
  const response = await fetch(`${project.endpoint.project_transport_url}/private/project/command`, {
    method: "POST", headers: { Authorization: `Bearer ${project.endpoint.auth_token}`, "Content-Type": "application/json" },
    body: JSON.stringify({ project_key: project.key, command }), signal: AbortSignal.timeout(timeout),
  }).catch(error => { throw new Error(`${project.key} ${command.type}: ${error}`, { cause: error }); });
  const body = await response.json();
  expect(response.ok, `${command.type}: ${JSON.stringify(body)}`).toBe(true);
  return body as any;
}

async function prepareProject(page: Page, directory: string, label: string, memberName: string): Promise<Project> {
  const root = join(directory, label);
  const source = join(directory, `${label}-source`);
  await mkdir(source);
  for (const args of [["init", "-q", "-b", "main"], ["-c", "user.name=gwt fixture", "-c", "user.email=gwt@example.invalid",
    "commit", "--allow-empty", "-qm", "supervisor isolation fixture"],
  ["remote", "add", "origin", `https://github.com/fixture/supervisor-${randomUUID()}.git`]]) {
    execFileSync("git", args, { cwd: source, env: isolatedEnv() });
  }
  // A real linked worktree is already admitted by the application. A normal
  // repository instead enters the migration dialog and has no owner yet.
  execFileSync("git", ["worktree", "add", "-q", "-b", "work/isolation", root, "main"],
    { cwd: source, env: isolatedEnv() });
  const status = await cli(root, "issue.monitor.status");
  const key = status.envelope.project_store?.hash;
  expect(key, "canonical project_store.hash").toMatch(/^[a-f0-9]{16}$/);
  const prefs = join(HOME, ".gwt/projects", key, "project-state");
  await mkdir(prefs, { recursive: true });
  await writeFile(join(prefs, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(prefs, "issue-monitor.json"), JSON.stringify({ enabled: false, autonomous_mode: false,
    terminal_queue_auto_refill: false, max_active_agents: 1, priority_order: [],
    // Stable fixture members still pass through the real owner's reservation.
    // Random catalog initials make independent fixture projects collide.
    agent_roster: { schema_version: 1, revision: 1, members: [{
      agent_member_id: `fixture-${label}-${randomUUID()}`, agent_id: "supervisor-isolation-fixture", name: memberName,
      enabled: true, created_at: new Date().toISOString(), origin_session_id: null,
      last_issue_number: null, last_spec_number: null, assignment: null,
    }] } }));
  try {
  await readLiveHubCatalog(page, BASE, { send: { kind: "reopen_recent_project", path: root },
    until: (catalog, key) => catalog.projects.some(project => project.project_key === key), arg: key });
  // Keep this project's real GUI attachment alive before opening the next
  // project. A catalog entry alone does not retain a GuiManaged owner.
  await gotoLiveGwt(page, BASE, { projectKey: key, enableTestBridge: true });
  await expect(page.locator("[data-project-owner-health]")).toHaveAttribute("data-state", "healthy", { timeout: 45_000 });
  let endpoint: any, endpointFile = "", owner: any, birth = "";
  await expect.poll(async () => {
    const directory = join(HOME, ".gwt/projects", key, "runtime/daemon");
    for (const name of await readdir(directory).catch(() => [])) {
      if (!name.endsWith(".json")) continue;
      const candidate = JSON.parse(await readFile(join(directory, name), "utf8"));
      if (candidate.scope?.repo_hash === key && candidate.project_transport_url) {
        let health: any;
        try { health = await command({ key, endpoint: candidate } as Project, { type: "health" }); }
        catch { continue; }
        if (health.pid !== candidate.pid || health.project_key !== key
          || health.startup_intent !== "gui_managed" || health.gui_presence_count < 1) continue;
        const observedBirth = processBirth(candidate.pid);
        if (!observedBirth) continue;
        endpoint = candidate;
        endpointFile = join(directory, name);
        owner = health;
        birth = observedBirth;
        return true;
      }
    }
    return false;
  }, { timeout: 45_000 }).toBe(true);
  const project = { root, key, endpoint, endpointFile, birth, owner, intentionallyKilled: false };
  expect(project.birth).not.toBe("");
  expect(project.owner).toMatchObject({ pid: endpoint.pid, project_key: key, startup_intent: "gui_managed" });
  expect(project.owner.gui_presence_count).toBeGreaterThan(0);
  expect(project.owner.host_started_at).toBeGreaterThan(0);
  expect(project.owner.host_instance_id).not.toBe("");
  return project;
  } catch (error) {
    await closeProject(page, { key }).catch(() => {});
    throw error;
  }
}

// Immutable launch metadata enters the real owner; it creates every Session,
// execution binding, registry row, capability and physical child itself.
function launchConfig(project: Project) {
  return {
    agent_id: { type: "Custom", value: "supervisor-isolation-fixture" }, command: SHELL, args: SHELL_ARGS,
    pending_initial_prompt: null, env_vars: { TERM: "xterm-256color" }, codex_auth_root: null, remove_env: [],
    working_dir: project.root, branch: "work/isolation", base_branch: "main", display_name: "Supervisor isolation fixture", color: "gray",
    model: null, tool_version: null, reasoning_level: null, session_mode: "Normal", resume_session_id: null,
    predecessor_session_id: null, skip_permissions: true, fast_mode: false, codex_fast_mode: false,
    runtime_target: "Host", docker_service: null, docker_lifecycle_intent: "Connect", linked_issue_number: ISSUE,
    windows_shell: null, is_ephemeral: false, ephemeral_base_ref: null, suppress_execution_control: false,
    explicit_follow_up: true, launch_route: "manual", automatic_restore: false,
    permission_decision: { outcome: "skip_forced_ready", provider: "custom:supervisor-isolation-fixture", source: "start_work",
      entrypoint: "project supervisor isolation fixture", launch_route: "manual", requested_skip_permissions: true,
      skip_forced: true, effective_skip_permissions: true, interactive_request_ignored: false,
      reason: "The local shell fixture has no permission prompts.", recovery_action: "",
      provider_mapping: { provider: "custom:supervisor-isolation-fixture", supported: true, args: SHELL_ARGS, env_overlay: [] },
      decided_at: new Date().toISOString() },
  };
}

async function launch(project: Project): Promise<Pane> {
  const id = `${project.key}::isolation-agent`;
  const pane = await command(project, { type: "launch_agent", id, operation_id: randomUUID(), tab_id: project.key,
    config: launchConfig(project), cols: 100, rows: 28 }, 120_000);
  expect(pane).toMatchObject({ id, host_pid: project.endpoint.pid, awaiting_readiness: false });
  expect(pane.session_id).not.toBe("");
  expect(pane.incarnation).toBeGreaterThan(0);
  pane.child_birth = processBirth(pane.child_pid);
  project.pane = pane;
  expect(pane.child_birth, "the actual physical fixture child is alive").not.toBe("");
  const session = await readFile(join(HOME, ".gwt/sessions", `${pane.session_id}.toml`), "utf8");
  expect(session, "a producing Session must exist; absent bindings cannot pass AC-3").toMatch(/\[execution_binding(?:\.|\])/);
  expect(processParent(pane.child_pid)).toBe(String(project.endpoint.pid));
  await assertLiveSession(project, pane);
  return pane;
}

function runtimePath(project: Project, pane: Pane) {
  return join(HOME, ".gwt/sessions/runtime", String(project.endpoint.pid), `${pane.session_id}.json`);
}

async function assertLiveSession(project: Project, pane: Pane) {
  const runtime = JSON.parse(await readFile(runtimePath(project, pane), "utf8"));
  // The runtime namespace directory carries host PID; the sidecar carries
  // its birth proof and physical child, rather than duplicating host_pid.
  expect(runtime).toMatchObject({ host_started_at: project.owner.host_started_at,
    child_pid: pane.child_pid, runtime_incarnation: pane.incarnation });
  expect(processBirth(project.endpoint.pid)).toBe(project.birth);
  const diagnosis = (await cli(project.root, "execution.status", { issue: ISSUE })).value;
  expect(diagnosis).toMatchObject({ holder_session_id: pane.session_id, holder_runtime: "live", reclaimable: false });
  const census = (await cli(project.root, "pane.health", { id: pane.id })).value;
  expect(census.panes).toHaveLength(1);
  expect(census.panes[0]).toMatchObject({ id: pane.id, session_id: pane.session_id });
  return { host_pid: project.endpoint.pid, child_pid: runtime.child_pid, runtime_incarnation: runtime.runtime_incarnation,
    holder_runtime: diagnosis.holder_runtime };
}

async function input(page: Page, pane: Pane, text: string) {
  await sendLiveGwtEvent(page, { kind: "terminal_input", id: pane.gui_id, data: text });
}

async function echo(page: Page, project: Project, pane: Pane, label: string) {
  const token = `GWT_ISOLATION_${label}_${randomUUID().replaceAll("-", "")}`;
  const started = Date.now();
  // Split the marker so the PTY's echoed input cannot satisfy the assertion.
  await input(page, pane, `node -e "console.log('GWT_'+'${token.slice(4)}')"\r`);
  await expect.poll(async () => (await command(project, { type: "snapshot", id: pane.id, incarnation: pane.incarnation })).screen,
    { timeout: 10_000, intervals: [100] }).toContain(token);
  // The echo is read through the checkout GUI's actual projection as well.
  try {
    await expect.poll(() => page.evaluate(id => (window as any).__gwtTerminalTestApi?.bufferText(id) ?? "", pane.gui_id),
      { timeout: 10_000, intervals: [100] }).toContain(token);
  } catch (error) {
    const gui = await page.evaluate(id => ({ id,
      hidden: document.hidden,
      metrics: (window as any).__gwtTerminalTestApi?.metrics(id),
      buffer: (window as any).__gwtTerminalTestApi?.bufferText(id),
      frames: ((window as any).__gwtPlaywrightMessages ?? []).slice(-40).map(({ payload }: any) => ({
        kind: payload.kind, id: payload.id, data: payload.data_base64?.slice(0, 250),
        keys: Object.keys(payload),
      })),
    }), pane.gui_id);
    console.log("owner echo projection diagnostic", JSON.stringify(gui));
    await page.screenshot({ path: join(HOME, "echo-projection-failure.png") });
    await test.info().attach("owner-echo-projection", { body: JSON.stringify({ gui,
      owner: await command(project, { type: "snapshot", id: pane.id, incarnation: pane.incarnation }),
    }), contentType: "application/json" });
    throw error;
  }
  return Date.now() - started;
}

async function fixture(page: Page, info: TestInfo): Promise<Fixture> {
  expect(await realpath(HOME)).not.toBe(await realpath(homedir()));
  expect(resolve(GWTD)).toBe(join(ROOT, `target/debug/gwtd${process.platform === "win32" ? ".exe" : ""}`));
  const directory = await realpath(await mkdtemp(join(HOME, "supervisor-isolation-")));
  const projects: Project[] = [];
  const pages = [page];
  try {
  // A killed owner's exact historical view may legitimately retain its name.
  // Respect the real global initial guard across this shared check HOME.
  const members = ["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel",
    "India", "Juliet", "Kilo", "Lima", "Mike", "November", "Oscar", "Papa",
    "Quebec", "Romeo", "Sierra", "Tango", "Uniform", "Victor", "Whiskey", "Xray"];
  const scenario = (info.project.name.includes("light") ? 2 : 0) + (info.title.startsWith("Hub") ? 1 : 0);
  const member = 2 * (4 * info.retry + scenario);
  expect(members[member + 1], "distinct real fixture initials for each scenario/theme/retry").toBeDefined();
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const errors: string[] = [];
  pages.push(await page.context().newPage(), await page.context().newPage());
  for (const current of pages) {
    current.on("pageerror", error => errors.push(error.message));
    current.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await current.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  }
  const a = await prepareProject(pages[0], directory, "alpha", members[member]); projects.push(a);
  const b = await prepareProject(pages[1], directory, "beta", members[member + 1]); projects.push(b);
  expect(a.key).not.toBe(b.key);
  expect(a.endpoint.pid).not.toBe(b.endpoint.pid);
  await gotoLiveGwt(pages[2], BASE, { hub: true });
  for (const current of pages) await expect(current.locator("html")).toHaveAttribute("data-theme", theme);
  const pa = await launch(a), pb = await launch(b);
  for (const [current, pane] of [[pages[0], pa], [pages[1], pb]] as const) {
    await sendLiveGwtEvent(current, { kind: "list_windows" });
    await expect.poll(async () => {
      pane.gui_id = await current.evaluate(session => {
        const entries = (window as any).__gwtPlaywrightMessages ?? [];
        for (const { payload } of [...entries].reverse()) {
          const windows = payload.kind === "window_list" ? payload.windows
            : payload.workspace?.tabs?.flatMap((tab: any) => tab.workspace.windows) ?? [];
          const match = windows.find((window: any) => window.session_id === session);
          if (match) return match.id;
        }
        return "";
      }, pane.session_id);
      return pane.gui_id;
    }, { timeout: 30_000 }).not.toBe("");
    const work = current.locator(`.work-row[data-window-id="${pane.gui_id}"]`);
    await expect(work).toBeVisible({ timeout: 30_000 });
    await work.getByRole("button", { name: "Open terminal", exact: true }).click();
    await expect(current.locator(`.works-full .works-terminal[data-window-id="${pane.gui_id}"] .xterm-helper-textarea`))
      .toBeVisible();
    await expect.poll(() => current.evaluate(id => (window as any).__gwtTerminalTestApi?.metrics(id)?.isReady, pane.gui_id),
      { timeout: 30_000 }).toBe(true);
  }
  // Prove real shell output reaches both the owner and the headed terminal
  // before any latency, saturation or retirement measurement begins.
  await echo(pages[0], a, pa, "A_READY");
  await echo(pages[1], b, pb, "B_READY");
  return { a, b, pa, pb, pages, errors, theme };
  } catch (error) {
    await cleanupProjects(pages.at(-1)!, projects);
    for (const current of pages.slice(1)) await current.close();
    throw error;
  }
}

async function closeProject(page: Page, project: Pick<Project, "key">) {
  const catalog = await readLiveHubCatalog(page, BASE);
  if (!catalog.projects.some(current => current.project_key === project.key)) return;
  await gotoLiveGwt(page, BASE, { hub: true });
  const close = page.locator(`[data-close-project="${project.key}"]`);
  await expect(close).toBeVisible();
  await close.click();
  // Unlike the catalog reader's deliberately conservative empty-only helper,
  // this owned fixture can confirm the real operator preview for its agents.
  try {
  await expect.poll(async () => {
    const confirm = page.locator('[data-role="close-project-confirm"]');
    if (await confirm.isVisible()) await confirm.click();
    return close.count();
  }, { timeout: 30_000 }).toBe(0);
  } catch (error) {
    const alert = await page.locator(".gwt-hub__error").textContent();
    const remaining = await readLiveHubCatalog(page, BASE).then(catalog => catalog.projects.map(current => current.project_key));
    throw new Error(`close ${project.key}: ${String(error)}; Hub alert=${alert}; remaining=${remaining}`, { cause: error });
  }
}
async function cleanupProjects(page: Page, projects: Project[]) {
  const failures: unknown[] = [];
  for (const project of projects) {
    try {
      if (!project.intentionallyKilled && processBirth(project.endpoint.pid) === project.birth) {
        const currentEndpoint = JSON.parse(await readFile(project.endpointFile, "utf8"));
        expect(JSON.stringify(currentEndpoint) === JSON.stringify(project.endpoint), "cleanup retains the captured private endpoint").toBe(true);
        const health = await command(project, { type: "health" });
        expect(health).toMatchObject({ pid: project.endpoint.pid, project_key: project.key,
          host_started_at: project.owner.host_started_at, host_instance_id: project.owner.host_instance_id });
        // Draining owners intentionally reject ListPanes. Their authenticated
        // health projection retains only the exact failed retirement to retry.
        const inventory = health.draining ? health.reconciliation : (await command(project, { type: "list_panes" })).panes;
        for (const exact of inventory) {
          const pane = project.pane;
          expect(pane, "cleanup only closes this fixture's captured producing Session").toBeDefined();
          expect(exact).toMatchObject({ id: pane!.id, incarnation: pane!.incarnation });
          if (health.draining) {
            const runtime = JSON.parse(await readFile(runtimePath(project, pane!), "utf8"));
            expect(runtime).toMatchObject({ host_started_at: project.owner.host_started_at,
              runtime_incarnation: pane!.incarnation, child_pid: pane!.child_pid });
            expect(runtime.execution_identity).toMatchObject({ session_id: pane!.session_id, repo_hash: project.key,
              worktree_path: project.root });
          } else {
            expect(exact).toMatchObject({ session_id: pane!.session_id, child_pid: pane!.child_pid });
          }
          expect(processBirth(project.endpoint.pid)).toBe(project.birth);
          await command(project, { type: "close", id: pane!.id, incarnation: pane!.incarnation });
        }
      }
    } catch (error) { failures.push(error); }
    try {
      await closeProject(page, project);
      // The real GUI owner allows a 15s reconnect grace before supervised
      // drain. This is an exit hang guard, not an assertion at that boundary.
      await expect.poll(() => processBirth(project.endpoint.pid), { timeout: 45_000,
        message: `owned ${project.key} daemon ${project.endpoint.pid} exits after its GUI reconnect grace` }).not.toBe(project.birth);
      if (project.pane) expect(processBirth(project.pane.child_pid), "the captured physical child exits with its owner")
        .not.toBe(project.pane.child_birth);
    } catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, "owned project fixture cleanup failed: " + failures.map(error => String(error)).join("\n"));
}

async function cleanup(f: Fixture) {
  await cleanupProjects(f.pages[2], [f.a, f.b]);
  for (const current of f.pages.slice(1)) await current.close();
}

async function recordHealthTransitions(page: Page, projectKey: string, hub = false) {
  await page.evaluate(({ projectKey, hub }) => {
    const states: string[] = [];
    const record = () => {
      const selector = hub ? `[data-project-health="${projectKey}"]` : "[data-project-owner-health]";
      const state = document.querySelector<HTMLElement>(selector)?.dataset.state;
      if (state && states.at(-1) !== state) states.push(state);
    };
    record();
    new MutationObserver(record).observe(document.body, { subtree: true, childList: true, attributes: true,
      attributeFilter: ["data-state"] });
    (window as any).__gwtIsolationHealthStates = states;
  }, { projectKey, hub });
}

async function floodCounter(project: Project, pane: Pane) {
  const snapshot = await command(project, { type: "snapshot", id: pane.id, incarnation: pane.incarnation });
  return Math.max(-1, ...[...snapshot.screen.matchAll(/GWT_A_FLOOD_(\d+)/g)].map(match => Number(match[1])));
}

test("A output saturation and exact owner kill preserve B Session, pane and Hub", async ({ page }, info) => {
  await withLiveGwtBackendLock(BASE, info, async () => {
    const f = await fixture(page, info);
    const [aPage, bPage, hub] = f.pages;
    let scenarioFailure: unknown;
    try {
      const before = await assertLiveSession(f.b, f.pb);
      const baseline = [await echo(bPage, f.b, f.pb, "BASE1"), await echo(bPage, f.b, f.pb, "BASE2")];
      // Bounded-duration real PTY output. The subprocess stops on its own even
      // when a test assertion fails; no unbounded producer survives teardown.
      await input(aPage, f.pa, `node -e "let n=0;const end=Date.now()+60000;function burst(){if(Date.now()>=end)return;const ok=process.stdout.write('\\x1b[HGWT_A_FLOOD_'+n+++' '+'x'.repeat(1024));if(ok)setImmediate(burst);else process.stdout.once('drain',burst)}burst()"\r`);
      let initialFlood = -1;
      await expect.poll(async () => {
        initialFlood = await floodCounter(f.a, f.pa);
        return initialFlood;
      }, { timeout: 15_000 }).toBeGreaterThanOrEqual(0);
      const loaded = [await echo(bPage, f.b, f.pb, "LOAD1"), await echo(bPage, f.b, f.pb, "LOAD2"), await echo(bPage, f.b, f.pb, "LOAD3")];
      expect(Math.max(...loaded), "B response stays within baseline plus the existing 500ms UI dispatch budget")
        .toBeLessThanOrEqual(Math.max(...baseline) + 500);
      const loadedFlood = await floodCounter(f.a, f.pa);
      expect(loadedFlood, "A's actual output advances while B responds").toBeGreaterThan(initialFlood);
      expect(await assertLiveSession(f.b, f.pb)).toEqual(before);
      const prefsPath = join(HOME, ".gwt/projects", f.b.key, "project-state/issue-monitor.json");
      const beforeScan = JSON.parse(await readFile(prefsPath, "utf8")).last_scan_at;
      const scanStarted = Date.now();
      // The visible operator route owns this setting. ScanNow's CLI route
      // requires an actual PM capability and is not an operator shortcut.
      await sendLiveGwtEvent(bPage, { kind: "set_issue_monitor_enabled", enabled: true });
      let completedScan: any;
      await expect.poll(async () => {
        completedScan = JSON.parse(await readFile(prefsPath, "utf8"));
        return completedScan.last_scan_at && completedScan.last_scan_at !== beforeScan
          && Date.parse(completedScan.last_scan_at) >= Math.floor(scanStarted / 1000) * 1000;
      }, { timeout: 45_000, intervals: [250] }).toBe(true);
      // No real GitHub repository backs the disposable fixture. A completed
      // project lookup failure is explicit worker progression, not evidence
      // of a successful GitHub scan. Keep that scope in the retained report.
      const scanScope = completedScan.last_scan_error || completedScan.last_error
        ? "completed own lookup failure; worker progress only"
        : "completed scan";
      info.annotations.push({ type: "monitor-scan-evidence", description: scanScope });
      let completedFlood = -1;
      await expect.poll(async () => {
        completedFlood = await floodCounter(f.a, f.pa);
        return completedFlood;
      }, { timeout: 10_000 }).toBeGreaterThan(loadedFlood);
      // A is intentionally saturated. Its already authenticated owner identity
      // remains tied to this unchanged private endpoint and exact OS birth;
      // requiring another A RPC would test A responsiveness rather than B isolation.
      const currentEndpoint = JSON.parse(await readFile(f.a.endpointFile, "utf8"));
      expect(JSON.stringify(currentEndpoint) === JSON.stringify(f.a.endpoint),
        "captured private endpoint identity remains unchanged").toBe(true);
      expect(processBirth(f.a.endpoint.pid)).toBe(f.a.birth);
      await recordHealthTransitions(aPage, f.a.key);
      await recordHealthTransitions(hub, f.a.key, true);
      process.kill(f.a.endpoint.pid, "SIGKILL");
      f.a.intentionallyKilled = true;
      // A quick automatic restart may replace degraded before the next test
      // RPC. Observe rendered DOM transitions, rather than requiring a fault
      // to remain active after the real supervisor has recovered it.
      for (const current of [aPage, hub]) await expect.poll(() => current.evaluate(() =>
        (window as any).__gwtIsolationHealthStates), { timeout: 15_000 }).toContain("degraded");
      await expect(hub.locator(`[data-project-health="${f.b.key}"]`)).toHaveAttribute("data-state", "healthy");
      expect(await assertLiveSession(f.b, f.pb)).toEqual(before);
      await echo(bPage, f.b, f.pb, "AFTER_KILL");
      await hub.locator('[data-hub-action="clone"]').click();
      await expect(hub.getByRole("dialog", { name: "Clone Project" })).toBeVisible();
      await hub.getByRole("button", { name: "Cancel", exact: true }).click();
      expect(f.errors, "zero console/page errors across A, B and Hub").toEqual([]);
      await info.attach(`owner-isolation-${f.theme}`, { body: Buffer.from(JSON.stringify({ baseline_ms: baseline,
        saturated_ms: loaded, b_session: before, a_pid: f.a.endpoint.pid, b_pid: f.b.endpoint.pid,
        a_flood: { initial: initialFlood, after_b_response: loadedFlood, after_monitor: completedFlood },
        monitor: { scope: scanScope, before: beforeScan, completed: completedScan.last_scan_at,
          failure: completedScan.last_scan_error ?? completedScan.last_error ?? null } })), contentType: "application/json" });
      await info.attach(`hub-after-owner-failure-${f.theme}`, { body: await hub.screenshot(), contentType: "image/png" });
    } catch (error) { scenarioFailure = error; throw error; }
    finally {
      try { await cleanup(f); }
      catch (error) { throw new Error(`scenario=${String(scenarioFailure)}; cleanup=${String(error)}`, { cause: error }); }
    }
  });
});

test("Hub observes and retries A exact failed retirement while B stays usable", async ({ page }, info) => {
  await withLiveGwtBackendLock(BASE, info, async () => {
    const f = await fixture(page, info);
    const [, bPage, hub] = f.pages;
    const path = runtimePath(f.a, f.pa);
    let saved: string | undefined;
    let scenarioFailure: unknown;
    const retirementHealth: any[] = [];
    try {
      saved = await readFile(path, "utf8");
      const changed = { ...JSON.parse(saved), runtime_incarnation: f.pa.incarnation + 1 };
      await writeFile(path + ".fixture-tmp", JSON.stringify(changed));
      await rename(path + ".fixture-tmp", path);
      // Close the actual GUI project, not just its browser page. The owner
      // retains exact failed retirement evidence after its last GUI lease.
      await f.pages[0].locator("#close-project-button").click();
      // An empty preview is automatically confirmed and navigates to the Hub.
      // Observe that UI completion separately from the existing owner drain
      // assertion; GUI residency is released by the native subscriber.
      await expect.poll(async () => new URL(f.pages[0].url()).pathname === "/"
        || await f.pages[0].locator('[data-role="close-project-confirm"]').isVisible(),
      { timeout: 15_000 }).toBe(true);
      const confirmClose = f.pages[0].locator('[data-role="close-project-confirm"]');
      if (await confirmClose.isVisible()) await confirmClose.click();
      await expect.poll(async () => {
        const health = await command(f.a, { type: "health" });
        expect(health).toMatchObject({ pid: f.a.endpoint.pid, project_key: f.a.key,
          host_started_at: f.a.owner.host_started_at, host_instance_id: f.a.owner.host_instance_id });
        retirementHealth.push({ observed_at_ms: Date.now(), ...health });
        return { draining: health.draining, failed: health.reconciliation_required, presence: health.gui_presence_count };
      }, { timeout: 45_000, intervals: [250] }).toEqual({ draining: true, failed: true, presence: 0 });
      await hub.locator('[data-hub-action="refresh-orphans"]').click();
      const row = hub.locator(".gwt-hub__orphan").filter({ hasText: "alpha" });
      await expect(row).toHaveCount(1);
      const retry = row.locator("[data-orphan-retry]");
      await expect(retry).toBeEnabled();
      await expect(row).toContainText("Shutdown failed");
      const recoveryReason = await row.locator(".gwt-hub__orphan-detail").innerText();
      expect(recoveryReason).not.toBe("");
      await echo(bPage, f.b, f.pb, "ORPHAN_OBSERVED");
      await writeFile(path + ".fixture-tmp", saved);
      await rename(path + ".fixture-tmp", path);
      saved = undefined;
      await retry.click();
      await expect(row).toHaveCount(0, { timeout: 15_000 });
      await expect.poll(() => processBirth(f.a.endpoint.pid), { timeout: 15_000 }).not.toBe(f.a.birth);
      const errors = (await cli(f.a.root, "errors.list", { project_root: f.a.root })).value;
      expect(JSON.stringify(errors), "project-scoped recovery reason is readable").toMatch(/daemon_fault|DaemonFault/);
      expect(JSON.stringify(errors)).toContain(recoveryReason);
      await echo(bPage, f.b, f.pb, "ORPHAN_RECOVERED");
      await assertLiveSession(f.b, f.pb);
      expect(f.errors, "zero console/page errors").toEqual([]);
      await info.attach(`orphan-recovered-${f.theme}`, { body: await hub.screenshot(), contentType: "image/png" });
    } catch (error) {
      scenarioFailure = error;
      await info.attach(`orphan-retirement-${f.theme}`, { body: Buffer.from(JSON.stringify({
        url: f.pages[0].url(), owner_at_capture: f.a.owner, owner_health: retirementHealth, error: String(error),
      })), contentType: "application/json" });
      throw error;
    }
    finally {
      if (saved) { await writeFile(path + ".fixture-tmp", saved); await rename(path + ".fixture-tmp", path); }
      try { await cleanup(f); }
      catch (error) { throw new Error(`scenario=${String(scenarioFailure)}; cleanup=${String(error)}`, { cause: error }); }
    }
  });
});
