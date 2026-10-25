import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import { createWriteStream } from "node:fs";
import { access, copyFile, mkdir, mkdtemp, readFile, realpath, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import type { TestInfo } from "@playwright/test";

// Rust's bare-command search on Windows requires a PE executable for git/gh;
// the provider itself uses the production-supported .cmd launch route.
export async function writeFixtureNodeCommand(bin: string, name: string, source: string, native = false) {
  const windows = process.platform === "win32";
  const script = join(bin, windows ? `${name}.cjs` : name);
  // Issue #5028: close writable fixture FDs in a child before any PTY spawn.
  const written = spawnSync(process.execPath, ["-e",
    "const fs=require('node:fs');fs.writeFileSync(process.argv[1],fs.readFileSync(0));fs.chmodSync(process.argv[1],0o755);",
    script], { input: source, encoding: "utf8", windowsHide: true });
  if (written.error) throw written.error;
  if (written.status !== 0) throw new Error(`Fixture executable writer failed: ${written.stderr}`);
  if (!windows) return script;
  if (!native) {
    const command = join(bin, `${name}.cmd`);
    await writeFile(command, `@echo off\r\n"${process.execPath}" "%~dp0${name}.cjs" %*\r\n`);
    return command;
  }
  const adapter = join(bin, "fixture-node.exe");
  if (!await access(adapter).then(() => true, () => false)) {
    const rust = join(bin, "fixture-node.rs");
    await writeFile(rust, `fn main() {
    let script = std::env::current_exe().unwrap().with_extension("cjs");
    let status = std::process::Command::new(${JSON.stringify(process.execPath)})
        .arg(script).args(std::env::args_os().skip(1)).status().expect("fixture Node command");
    std::process::exit(status.code().unwrap_or(1));
}\n`);
    const built = spawnSync("rustc", ["--crate-name", "fixture_node", "-C", "debuginfo=0", rust, "-o", adapter],
      { encoding: "utf8", windowsHide: true });
    if (built.error) throw built.error;
    if (built.status !== 0) throw new Error(`Fixture native adapter failed: ${built.stderr}`);
  }
  const command = join(bin, `${name}.exe`);
  await copyFile(adapter, command);
  return command;
}

export type ExactRelaunchInvocation = {
  pid: number;
  argv: string[];
  cwd: string;
  gwt_session_id: string | null;
  provider_session_id: string;
  session_id_argument: string | null;
  resume_session_id: string | null;
  at: string;
};

/** Real checkout/backend/PTY; only the provider executable is a recorder.
 * Never writes durable gwt Sessions, execution ledgers, or Work records.
 * Each invocation owns a fresh HOME, retained as evidence after cleanup.
 */
export type ExactRelaunchSetup = {
  prepare?: (context: { home: string; bin: string; project: string; env: NodeJS.ProcessEnv }) => Promise<void>;
  monitorPreferences?: Record<string, unknown>;
  beforeStart?: (context: { home: string; project: string; env: NodeJS.ProcessEnv; preferences: string }) => Promise<void>;
};

export async function startExactRelaunchFixture(testInfo: TestInfo, setup: ExactRelaunchSetup = {}) {
  const windows = process.platform === "win32";
  const suffix = windows ? ".exe" : "";
  const root = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
  const gwt = join(root, `target/debug/gwt${suffix}`);
  const gwtd = join(root, `target/debug/gwtd${suffix}`);
  await Promise.all([access(gwt), access(gwtd)]);
  const home = await realpath(await mkdtemp(join(tmpdir(), "gwt-exact-relaunch-")));
  const bin = join(home, "bin");
  const state = join(home, ".gwt");
  const argvLog = join(home, "provider-argv.jsonl");
  const nativeId = randomUUID();
  const env: NodeJS.ProcessEnv = { ...process.env };
  // No ambient parent Session, capability, Git selection, or credentials may
  // enter this independent instance, including its initial Git operations.
  for (const key of Object.keys(env)) {
    if (/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
      || ["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())
      || key.toUpperCase() === "PATH") delete env[key];
  }
  Object.assign(env, {
    HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
    PATH: [bin, process.env.PATH ?? process.env.Path ?? ""].join(delimiter),
    GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1",
    GWT_HOOK_BIN: "gwtd", GWT_PROJECT_ROOT: join(home, "project"),
    GWT_DISABLE_BACKGROUND_INDEX: "1",
  });
  await Promise.all([mkdir(bin), mkdir(state), mkdir(join(home, ".codex"))]);
  await writeFile(argvLog, "");
  // The project is a throwaway repository, not this checkout: startup restore
  // refuses a worktree whose HEAD already landed on origin/develop, and the
  // launch materializes hooks into the project, which must not touch the checkout.
  const project = join(home, "project");
  const branch = "main";
  await mkdir(project);
  const git = (...args: string[]) => spawnSync("git", args, { cwd: project, env, encoding: "utf8", windowsHide: true });
  for (const args of [["init", "-q", "-b", branch], ["-c", "user.name=gwt", "-c", "user.email=gwt@example.invalid",
    "commit", "-q", "--allow-empty", "-m", "exact relaunch fixture"]]) {
    const result = git(...args);
    if (result.status !== 0) throw new Error(`Fixture project git ${args[0]} failed: ${result.stderr}`);
  }

  // The fake provider owns only native conversation metadata. In particular,
  // its native conversation UUID is independent from GWT_SESSION_ID.
  const provider = `#!${process.execPath}
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');
const args = process.argv.slice(2);
if (args.includes('--version') || args.includes('-V')) {
  console.log('codex-cli 0.116.0'); process.exit(0);
}
// Configuration preflight is not an interactive provider session.
const configOptions = ['--config','-c','--profile','-p','--enable','--disable'];
let configEnd = 0;
while (configEnd < args.length - 2) {
  if (configOptions.includes(args[configEnd])) configEnd += 2;
  else if (/^--(?:config|profile|enable|disable)=/.test(args[configEnd])) configEnd += 1;
  else break;
}
if (configEnd === args.length - 2 && args.at(-2) === 'features' && args.at(-1) === 'list') {
  process.exit(0);
}
const option = name => {
  const index = args.indexOf(name);
  return index < 0 ? (args.find(a => a.startsWith(name + '='))?.slice(name.length + 1) ?? null)
    : (args[index + 1] ?? null);
};
const resumeAt = args.indexOf('resume');
const resumed = resumeAt < 0 ? null : (args.slice(resumeAt + 1).find(a => /^[0-9a-f]{8}-[0-9a-f-]{27}$/i.test(a)) ?? null);
const requested = option('--session-id');
const native = resumed || requested || ${JSON.stringify(nativeId)};
const at = new Date().toISOString();
const directory = path.join(process.env.HOME, '.codex', 'sessions', ...at.slice(0,10).split('-'));
fs.mkdirSync(directory, {recursive:true});
const rollout = path.join(directory, 'rollout-' + at.replaceAll(':','-') + '-' + native + '.jsonl');
fs.writeFileSync(rollout, JSON.stringify({timestamp:at,type:'session_meta',payload:{id:native,timestamp:at,cwd:process.cwd(),originator:'codex_cli_rs',cli_version:'0.116.0',source:'cli',model_provider:'openai'}}) + '\\n');
fs.appendFileSync(${JSON.stringify(argvLog)}, JSON.stringify({pid:process.pid,argv:args,cwd:process.cwd(),gwt_session_id:process.env.GWT_SESSION_ID ?? null,provider_session_id:native,session_id_argument:requested,resume_session_id:resumed,at}) + '\\n');
// Like the real provider, report the native conversation to gwt through the
// SessionStart hooks materialized in the worktree; this is how gwt learns the
// exact resume handle (it never derives it from rollout files).
try {
  const hooks = JSON.parse(fs.readFileSync(path.join(process.cwd(), '.codex', 'hooks.json'), 'utf8'));
  const input = JSON.stringify({session_id:native,hook_event_name:'SessionStart',source:resumed ? 'resume' : 'startup',cwd:process.cwd(),transcript_path:rollout,model:option('--model')});
  for (const group of hooks.hooks?.SessionStart ?? []) for (const hook of group.hooks ?? []) {
    if (hook.type !== 'command') continue;
    const windows = process.platform === 'win32';
    const result = require('node:child_process').spawnSync(windows ? (process.env.ComSpec || 'cmd.exe') : '/bin/sh',
      windows ? ['/d','/s','/c',hook.command] : ['-c',hook.command],
      {input, stdio:['pipe','ignore','inherit'], timeout:60000, windowsHide:true, windowsVerbatimArguments:windows});
    if (result.error || result.status !== 0) throw new Error('SessionStart hook failed: ' + (result.error?.message || result.status));
  }
} catch (error) { console.error('GWT_FAKE_PROVIDER_HOOK_ERROR ' + error.message); }
console.log('GWT_FAKE_PROVIDER_READY ' + native);
readline.createInterface({input:process.stdin}).on('line', line => {
  if (line.trim() === 'GWT_FIXTURE_EXIT') process.exit(0);
  if (line.trim() === 'GWT_FIXTURE_DONE') {
    const result = require('node:child_process').spawnSync(process.env.GWT_BIN_PATH, [], {
      cwd:process.cwd(), env:process.env, encoding:'utf8', timeout:30000, windowsHide:true,
      input:JSON.stringify({schema_version:1,operation:'workspace.update',params:{status:'done'}}),
    });
    let done = false;
    try { done = result.status === 0 && JSON.parse(result.stdout).ok; } catch {}
    console.log(done ? 'GWT_FAKE_PROVIDER_DONE' : 'GWT_FAKE_PROVIDER_DONE_FAILED');
    return;
  }
  if (line.trim() === 'GWT_FIXTURE_BLOCK') {
    // Exercise the provider's real authenticated execution boundary. Never
    // copy its launch capability into the test runner or print the envelope.
    const result = require('node:child_process').spawnSync(process.env.GWT_BIN_PATH, [], {
      cwd:process.cwd(), env:process.env, encoding:'utf8', timeout:30000, windowsHide:true,
      input:JSON.stringify({schema_version:1,operation:'execution.blocked',params:{reason:'Fixture execution intentionally blocked'}}),
    });
    let blocked = false;
    try { blocked = result.status === 0 && JSON.parse(result.stdout).ok; } catch {}
    console.log(blocked ? 'GWT_FAKE_PROVIDER_BLOCKED' : 'GWT_FAKE_PROVIDER_BLOCK_FAILED');
    return;
  }
  console.log('GWT_FAKE_PROVIDER_READY ' + native);
});
`;
  await writeFixtureNodeCommand(bin, "codex", provider);
  // Hooks resolve `gwtd` through PATH when GWT_BIN_PATH is absent; it must be
  // the checkout build under test, never an installed binary.
  if (windows) await copyFile(gwtd, join(bin, `gwtd${suffix}`));
  else await symlink(gwtd, join(bin, "gwtd"));
  // An accidentally selected package runner must never contact a registry or
  // bypass the fixture's provider. Installed Codex is the intended test route.
  for (const runner of ["bunx", "npx", "npm"]) {
    await writeFixtureNodeCommand(bin, runner,
      `#!${process.execPath}\nconsole.error('Exact relaunch fixture requires installed Codex');process.exit(64);\n`);
  }
  try {
    const runtime = join(homedir(), ".gwt/runtime");
    await access(runtime);
    await symlink(runtime, join(state, "runtime"), windows ? "junction" : "dir");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  await setup.prepare?.({ home, bin, project, env });
  const status = spawnSync(gwtd, [], {
    cwd: project, env, encoding: "utf8", timeout: 30_000, windowsHide: true,
    input: JSON.stringify({ schema_version: 1, operation: "issue.monitor.status", params: {} }),
  });
  if (status.status !== 0) throw new Error(`Fixture project lookup failed: ${status.stderr}`);
  const envelope = JSON.parse(status.stdout);
  if (!envelope.ok) throw new Error("Fixture issue.monitor.status refused");
  // The envelope reports the operation's project store beside `output`.
  const hash = envelope.project_store?.hash;
  if (typeof hash !== "string" || !/^[a-zA-Z0-9_-]+$/.test(hash)) throw new Error("Missing canonical project_store.hash");
  const preferences = join(state, "projects", hash, "project-state");
  await mkdir(preferences, { recursive: true });
  await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [], ...setup.monitorPreferences }));
  await writeFile(join(state, "session.json"), JSON.stringify({
    tabs: [{ id: "exact-relaunch", title: "exact-relaunch", project_root: project, kind: "git" }],
    active_tab_id: "exact-relaunch", recent_projects: [],
  }));

  await setup.beforeStart?.({ home, project, env, preferences });

  let child: ChildProcess | undefined;
  let incarnation = 0;
  let url = "";
  async function launches(): Promise<ExactRelaunchInvocation[]> {
    return (await readFile(argvLog, "utf8")).split("\n").filter(Boolean).map(line => JSON.parse(line));
  }
  function processCommand(pid: number) {
    const result = windows
      ? spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
        `[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); (Get-CimInstance Win32_Process -Filter 'ProcessId = ${pid}').CommandLine`],
      { encoding: "utf8", windowsHide: true })
      : spawnSync("ps", ["-p", String(pid), "-o", "command="], { encoding: "utf8" });
    if (result.error) throw result.error;
    if (result.status !== 0 && (windows || result.status !== 1)) throw new Error(`Cannot inspect owned fixture PID ${pid}: ${result.stderr}`);
    return result.stdout.trim();
  }
  async function stop() {
    const ownedPid = child?.pid;
    if (child?.pid && child.exitCode === null && child.signalCode === null) {
      const current = child;
      current.kill("SIGTERM");
      const deadline = Date.now() + 8_000;
      while (current.exitCode === null && current.signalCode === null && Date.now() < deadline) await delay(100);
      if (current.exitCode === null && current.signalCode === null) {
        current.kill("SIGKILL");
        const forcedDeadline = Date.now() + 8_000;
        while (current.exitCode === null && current.signalCode === null && Date.now() < forcedDeadline) await delay(100);
      }
    }
    // PTYs can create their own process groups; clean only recorded fixture
    // providers still carrying our unique argv-recorder path in their command.
    const providerPaths = [join(bin, windows ? "codex.cjs" : "codex"), join(bin, windows ? "claude.cjs" : "claude")];
    const providerPids: number[] = [];
    for (const launch of await launches()) {
      const command = processCommand(launch.pid);
      if (providerPaths.some(path => command.includes(path))) {
        providerPids.push(launch.pid);
        try { process.kill(launch.pid, "SIGTERM"); } catch { /* exited */ }
      }
    }
    await Promise.all(providerPids.map(async pid => {
      const deadline = Date.now() + 5_000;
      while (Date.now() < deadline) {
        const command = processCommand(pid);
        if (!providerPaths.some(path => command.includes(path))) return;
        await delay(100);
      }
      throw new Error(`Fixture provider ${pid} remained after shutdown`);
    }));
    if (ownedPid) {
      const remaining = processCommand(ownedPid);
      await testInfo.attach("fixture-process-cleanup", { body: JSON.stringify({ pid: ownedPid, exited: !remaining }), contentType: "application/json" });
      if (remaining) throw new Error(`Fixture gwt process ${ownedPid} remained after shutdown`);
    }
    child = undefined;
  }
  async function start() {
    const urlFile = join(home, `url-${++incarnation}.txt`);
    const logFile = join(home, `gwt-${incarnation}.log`);
    const stream = createWriteStream(logFile);
    try {
      child = spawn(gwt, ["--no-tray", "--no-open"], {
        cwd: project, env: { ...env, GWT_BROWSER_URL_FILE: urlFile }, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
      });
      const current = child;
      let failure: Error | undefined;
      current.on("error", error => { failure = error; });
      current.stdout?.pipe(stream, { end: false });
      current.stderr?.pipe(stream, { end: false });
      current.on("close", () => stream.end());
      const deadline = Date.now() + 60_000;
      while (Date.now() < deadline) {
        if (failure) throw failure;
        if (current.exitCode !== null || current.signalCode !== null) throw new Error(`Fresh gwt exited; see ${logFile}`);
        let readyUrl: string | undefined;
        try {
          const candidate = (await readFile(urlFile, "utf8")).trim();
          if (candidate && (await fetch(candidate, { method: "HEAD", signal: AbortSignal.timeout(2_000) })).ok) {
            readyUrl = candidate;
          }
        } catch { /* Fresh process has not published its URL yet. */ }
        if (readyUrl) {
          url = readyUrl;
          await testInfo.attach("exact-relaunch-fixture", {
            body: JSON.stringify({ home, argvLog, checkout: root, project, url, pid: current.pid }), contentType: "application/json",
          });
          return url;
        }
        await delay(100);
      }
      throw new Error(`Fresh gwt readiness timed out; see ${logFile}`);
    } catch (error) {
      await stop();
      throw error;
    }
  }
  await start();
  return { get url() { return url; }, get ownedPid() { return child?.pid; }, home, project, branch, argvLog, launches, stop,
    async restart() { await stop(); return start(); } };
}
