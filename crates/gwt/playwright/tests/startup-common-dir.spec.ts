import { expect, test, type ConsoleMessage, type Page, type TestInfo } from "@playwright/test";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { access, mkdir, mkdtemp, readFile, readdir, realpath, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const checkout = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
const suffix = process.platform === "win32" ? ".exe" : "";
const gwtd = join(checkout, `target/debug/gwtd${suffix}`);
const control = join(checkout, `target/startup-common-dir-control${suffix}`);
const treatment = join(checkout, `target/debug/gwt${suffix}`);

// Isolate launch authority and credentials as well as HOME. These fixtures do
// not call providers, GitHub, or the host's Issue Monitor.
function isolatedEnvironment(home: string): NodeJS.ProcessEnv {
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (/^(GWT_|CODEX_|CLAUDE_|GIT_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)/i.test(key)) delete env[key];
  }
  return { ...env, HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
    APPDATA: join(home, "AppData/Roaming"), LOCALAPPDATA: join(home, "AppData/Local"),
    TEMP: join(home, "tmp"), TMP: join(home, "tmp"), CI: "1", RUST_LOG: "info",
    GWT_HOOK_BIN: "gwtd", GWT_DISABLE_BACKGROUND_INDEX: "1", GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1" };
}

function operation(name: string, env: NodeJS.ProcessEnv, cwd: string): any {
  const response = execFileSync(gwtd, [], { env, cwd, windowsHide: true, timeout: 30_000,
    encoding: "utf8", input: JSON.stringify({ schema_version: 1, operation: name, params: {} }) });
  const envelope = JSON.parse(response);
  expect(envelope.ok, `${name} must succeed`).toBe(true);
  return envelope;
}

async function logRows(directory: string): Promise<any[]> {
  const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
  const rows: any[] = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) rows.push(...await logRows(path));
    else if (/\.log(?:\.|$)|\.jsonl$/.test(entry.name)) {
      for (const line of (await readFile(path, "utf8")).split("\n")) {
        try { rows.push(JSON.parse(line)); } catch { /* partial final writes are retried */ }
      }
    }
  }
  return rows;
}

async function runArm(binary: string, arm: string, fixture: string, project: string,
  page: Page, testInfo: TestInfo): Promise<any> {
  const home = join(fixture, `home-${arm}`);
  for (const path of [join(home, ".gwt"), join(home, ".codex"), join(home, "tmp")]) await mkdir(path, { recursive: true });
  await writeFile(join(home, ".codex/config.toml"), "");
  await writeFile(join(home, ".gwt/config.toml"), "[usage]\ncodex_enabled = false\nclaude_account_enabled = false\n");
  const runtime = join(homedir(), ".gwt/runtime");
  if (await access(runtime).then(() => true, () => false)) {
    await symlink(runtime, join(home, ".gwt/runtime"), process.platform === "win32" ? "junction" : "dir");
  }
  const env = isolatedEnvironment(home);
  const key = operation("issue.monitor.status", env, project).project_store?.hash;
  expect(key, "canonical isolated project store").toMatch(/^[0-9a-f]{16}$/);
  const state = join(home, ".gwt/projects", key, "project-state");
  await mkdir(state, { recursive: true });
  await writeFile(join(state, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(state, "issue-monitor.json"), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] }));
  await writeFile(join(home, ".gwt/session.json"), JSON.stringify({ tabs: [{ id: "fixture", title: "Startup common dir", project_root: project, kind: "git" }], recent_projects: [] }));
  const urlFile = join(home, "browser-url.txt");
  env.GWT_BROWSER_URL_FILE = urlFile;
  env.GWT_PROJECT_ROOT = project;
  const pageErrors: string[] = [];
  const consoleErrors: string[] = [];
  const pageError = (error: Error) => pageErrors.push(error.message);
  const consoleError = (message: ConsoleMessage) => { if (message.type() === "error") consoleErrors.push(message.text()); };
  page.on("pageerror", pageError);
  page.on("console", consoleError);
  const child = spawn(binary, ["--no-tray", "--no-open"], { cwd: project, env,
    windowsHide: true, stdio: ["ignore", "ignore", "ignore"] });
  const ownedPid = child.pid;
  let launchError: Error | undefined;
  child.on("error", (error) => { launchError = error; });
  const exited = new Promise<void>((done) => child.once("close", () => done()));
  const stop = () => { if (child.exitCode === null && child.signalCode === null) child.kill(); };
  const interrupt = () => { stop(); process.exit(130); };
  process.once("exit", stop);
  process.once("SIGINT", interrupt);
  process.once("SIGTERM", interrupt);
  let rows: any[] = [];
  let exitedAt = "";
  try {
    expect(ownedPid, "record the owned PID before waiting for readiness").toBeGreaterThan(0);
    let url = "";
    await expect.poll(async () => {
      if (launchError || child.exitCode !== null) throw new Error(`${arm} gwt exited before readiness`);
      url = await readFile(urlFile, "utf8").then((value) => value.trim(), () => "");
      if (!url) return false;
      return fetch(url, { method: "HEAD", signal: AbortSignal.timeout(2_000) }).then((response) => response.status === 200, () => false);
    }, { timeout: 90_000, intervals: [200] }).toBe(true);
    await gotoLiveGwt(page, url, { projectKey: key, enableTestBridge: true });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages?.some((entry: any) =>
      entry.payload?.kind === "workspace_state" && entry.payload.workspace.tabs?.some((tab: any) => tab.id === "fixture")));
    await expect.poll(async () => {
      if (launchError || child.exitCode !== null) throw new Error(`${arm} gwt exited during startup`);
      rows = [...await logRows(join(home, ".gwt/logs")), ...await logRows(join(home, ".gwt/projects", key, "logs"))];
      const phases = new Set(rows.map((row) => row.startup?.phase));
      return phases.has("shell_interactive") && phases.has("restore_drain");
    }, { timeout: 90_000, intervals: [200] }).toBe(true);
    expect(consoleErrors, `${arm} console errors`).toEqual([]);
    expect(pageErrors, `${arm} page errors`).toEqual([]);
    await page.screenshot({ path: testInfo.outputPath(`${arm}.png`) });
  } finally {
    page.off("pageerror", pageError);
    page.off("console", consoleError);
    try {
      // Stop the old page's WebSocket reconnect loop before the next arm.
      if (!page.isClosed()) await page.goto("about:blank", { waitUntil: "commit", timeout: 10_000 });
    } finally {
      stop();
      await Promise.race([exited, delay(10_000, undefined, { ref: false }).then(() => { throw new Error(`Owned gwt PID ${ownedPid} remained after cleanup`); })]);
      exitedAt = new Date().toISOString();
      process.removeListener("exit", stop);
      process.removeListener("SIGINT", interrupt);
      process.removeListener("SIGTERM", interrupt);
    }
  }
  // Freeze all completed commands from this GUI startup, including background
  // completions after restore_drain. The perf observer covers the same window.
  rows = [...await logRows(join(home, ".gwt/logs")), ...await logRows(join(home, ".gwt/projects", key, "logs"))];
  // Query after freezing logs, so the read cannot add measured commands.
  const envelope = operation("perf.startup", env, project);
  const report = (typeof envelope.output === "string" ? JSON.parse(envelope.output) : envelope.output).startup;
  expect(report.phases.some((phase: any) => phase.phase === "restore_drain")).toBe(true);
  expect(rows.some((row) => row.target === "gwt.process.summary" && row.fields?.kind === "git"
    && row.fields.phase === "end"), `${arm} process logging must be observable`).toBe(true);
  const finished = rows.filter((row) => row.target === "gwt.process.summary" && row.fields?.kind === "git"
    && row.fields.phase === "end" && /\brev-parse\b/.test(row.fields.label)
    && /--git-common-dir\b/.test(row.fields.label)
    && Date.parse(row.timestamp) >= Date.parse(report.process_started_at)
    && Date.parse(row.timestamp) <= Date.parse(exitedAt));
  const unique = [...new Map(finished.map((row) => [`${row.timestamp}:${row.fields.spawn_id}`, row])).values()];
  // Persist numeric evidence only; do not copy command labels, HOME paths,
  // credential-bearing environment variables, or raw process output.
  return { arm, owned_pid: ownedPid, cleanup: true, git_common_dir_count: unique.length,
    git_common_dir_ms: unique.reduce((sum, row) => sum + Number(row.fields.duration_ms), 0),
    // The frozen control producer has no command-specific startup phase;
    // a new reader's empty aggregate cannot establish old-producer telemetry.
    perf_git_common_dir_count: arm === "treatment" ? report.git_common_dir_count : null,
    perf_git_common_dir_ms: arm === "treatment" ? report.git_common_dir_ms : null,
    measurement_window: { started_at: report.process_started_at, ended_at: exitedAt,
      completion: "owned_gui_exit_after_shell_interactive_and_restore_drain" },
    phases: report.phases.map((phase: any) => ({ phase: phase.phase, duration_ms: phase.duration_ms })),
    console_errors: consoleErrors.length, page_errors: pageErrors.length };
}

test("startup common-dir control and treatment use one isolated fixture", async ({ page }, testInfo) => {
  test.skip(process.env.GWT_PLAYWRIGHT_STARTUP_COMMON_DIR !== "1", "requires the built checkout and frozen control executable");
  test.setTimeout(300_000);
  expect(process.env.GWT_COMMON_DIR_CONTROL_SHA).toMatch(/^[0-9a-f]{40}$/);
  expect(process.env.GWT_COMMON_DIR_TREATMENT_SHA).toMatch(/^[0-9a-f]{40}$/);
  for (const binary of [control, treatment, gwtd]) await access(binary);
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript((value) => localStorage.setItem("gwt:ui:theme", value), theme);
  const fixture = await realpath(await mkdtemp(join(tmpdir(), "gwt-startup-common-dir-")));
  const project = join(fixture, "repo");
  await mkdir(project);
  const gitEnv = isolatedEnvironment(fixture);
  const git = (...args: string[]) => execFileSync("git", ["-C", project, ...args], { env: gitEnv, windowsHide: true, encoding: "utf8" });
  git("init", "--quiet", "-b", "main");
  git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--quiet", "--allow-empty", "-m", "fixture");
  const fixtureBase = git("rev-parse", "HEAD").trim();
  for (let index = 1; index < 6; index += 1) git("worktree", "add", "--quiet", "--detach", join(fixture, `wt-${index}`), fixtureBase);
  const arms = [];
  try {
    arms.push(await runArm(control, "control", fixture, project, page, testInfo));
    expect(git("rev-parse", "HEAD").trim()).toBe(fixtureBase);
    arms.push(await runArm(treatment, "treatment", fixture, project, page, testInfo));
    expect(git("rev-parse", "HEAD").trim()).toBe(fixtureBase);
    expect(arms[1].git_common_dir_count).toBeLessThanOrEqual(1);
    expect(arms[1].git_common_dir_ms).toBeLessThan(500);
    expect(arms[1].perf_git_common_dir_count).toBe(arms[1].git_common_dir_count);
    expect(arms[1].perf_git_common_dir_ms).toBe(arms[1].git_common_dir_ms);
  } finally {
    const evidence = { schema_version: 1, theme, fixture_base: fixtureBase, worktrees: 6, sessions: 0,
      control_source_sha: process.env.GWT_COMMON_DIR_CONTROL_SHA,
      treatment_source_sha: process.env.GWT_COMMON_DIR_TREATMENT_SHA,
      control_binary_sha256: createHash("sha256").update(await readFile(control)).digest("hex"),
      treatment_binary_sha256: createHash("sha256").update(await readFile(treatment)).digest("hex"), arms };
    const path = testInfo.outputPath("startup-common-dir.json");
    await writeFile(path, `${JSON.stringify(evidence, null, 2)}\n`, { flag: "wx" });
    await testInfo.attach("startup-common-dir", { path, contentType: "application/json" });
  }
});
