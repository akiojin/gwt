import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import { createWriteStream } from "node:fs";
import { access, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import type { TestInfo } from "@playwright/test";

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
  if (process.platform === "win32") throw new Error("Exact relaunch fixture requires POSIX executables");
  const root = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
  const gwt = join(root, "target/debug/gwt");
  const gwtd = join(root, "target/debug/gwtd");
  await Promise.all([access(gwt), access(gwtd)]);
  const home = await mkdtemp(join(tmpdir(), "gwt-exact-relaunch-"));
  const bin = join(home, "bin");
  const state = join(home, ".gwt");
  const argvLog = join(home, "provider-argv.jsonl");
  const nativeId = randomUUID();
  await Promise.all([mkdir(bin), mkdir(state), mkdir(join(home, ".codex"))]);
  await writeFile(argvLog, "");
  // The project is a throwaway repository, not this checkout: startup restore
  // refuses a worktree whose HEAD already landed on origin/develop, and the
  // launch materializes hooks into the project, which must not touch the checkout.
  const project = join(home, "project");
  const branch = "main";
  await mkdir(project);
  const git = (...args: string[]) => spawnSync("git", args, { cwd: project, encoding: "utf8" });
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
    if (hook.type === 'command') require('node:child_process').spawnSync('/bin/sh', ['-c', hook.command], {input, stdio:['pipe','ignore','inherit'], timeout:60000});
  }
} catch (error) { console.error('GWT_FAKE_PROVIDER_HOOK_ERROR ' + error.message); }
console.log('GWT_FAKE_PROVIDER_READY ' + native);
readline.createInterface({input:process.stdin}).on('line', line => {
  if (line.trim() === 'GWT_FIXTURE_EXIT') process.exit(0);
  console.log('GWT_FAKE_PROVIDER_READY ' + native);
});
`;
  // Only the child holds writable fixture FDs, and exits before any spawn.
  // Issue #5028: awaiting writeFile cannot prevent sibling fork inheritance.
  const writeExecutable = (file: string, contents: string) => {
    const result = spawnSync(process.execPath, ["-e",
      "const fs = require('node:fs'); fs.writeFileSync(process.argv[1], fs.readFileSync(0)); fs.chmodSync(process.argv[1], 0o755);",
      file], { input: contents, encoding: "utf8" });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`Fixture executable writer failed: ${result.stderr}`);
  };
  writeExecutable(join(bin, "codex"), provider);
  // Hooks resolve `gwtd` through PATH when GWT_BIN_PATH is absent; it must be
  // the checkout build under test, never an installed binary.
  await symlink(gwtd, join(bin, "gwtd"));
  // An accidentally selected package runner must never contact a registry or
  // bypass the fixture's provider. Installed Codex is the intended test route.
  for (const runner of ["bunx", "npx", "npm"]) {
    writeExecutable(join(bin, runner), "#!/bin/sh\necho 'Exact relaunch fixture requires installed Codex' >&2\nexit 64\n");
  }
  try {
    const runtime = join(homedir(), ".gwt/runtime");
    await access(runtime);
    await symlink(runtime, join(state, "runtime"));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  const env: NodeJS.ProcessEnv = { ...process.env };
  // No ambient parent Session, capability, execution, transport, or config
  // identity may enter this independent test instance.
  for (const key of Object.keys(env)) {
    if (key.startsWith("GWT_") || key.startsWith("CODEX_") || key.startsWith("CLAUDE_")
      || ["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key)) delete env[key];
  }
  Object.assign(env, {
    HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
    PATH: `${bin}:${process.env.PATH ?? ""}`,
    GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1",
    GWT_HOOK_BIN: "gwtd", GWT_PROJECT_ROOT: project,
    GWT_DISABLE_BACKGROUND_INDEX: "1",
  });
  await setup.prepare?.({ home, bin, project, env });
  const status = spawnSync(gwtd, [], {
    cwd: project, env, encoding: "utf8", timeout: 30_000,
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
    for (const launch of await launches()) {
      const command = spawnSync("ps", ["-p", String(launch.pid), "-o", "command="], { encoding: "utf8" });
      if (command.stdout.includes(join(bin, "codex"))) {
        try { process.kill(launch.pid, "SIGTERM"); } catch { /* exited */ }
      }
    }
    if (ownedPid) {
      const remaining = spawnSync("ps", ["-p", String(ownedPid), "-o", "pid="], { encoding: "utf8" });
      await testInfo.attach("fixture-process-cleanup", { body: JSON.stringify({ pid: ownedPid, ps_status: remaining.status }), contentType: "application/json" });
      if (remaining.status !== 1) throw new Error(`Fixture gwt process ${ownedPid} remained after shutdown`);
    }
    child = undefined;
  }
  async function start() {
    const urlFile = join(home, `url-${++incarnation}.txt`);
    const logFile = join(home, `gwt-${incarnation}.log`);
    const stream = createWriteStream(logFile);
    try {
      child = spawn(gwt, ["--no-tray", "--no-open"], {
        cwd: project, env: { ...env, GWT_BROWSER_URL_FILE: urlFile }, stdio: ["ignore", "pipe", "pipe"],
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
