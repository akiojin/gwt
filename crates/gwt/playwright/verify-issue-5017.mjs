// Issue #5017: real headed Chromium, checkout binaries, fixture-only installers.
// Usage: node crates/gwt/playwright/verify-issue-5017.mjs [--skip-build] [Playwright arguments]
import { spawn } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const gwt = join(root, "target/debug/gwt");
const gwtd = join(root, "target/debug/gwtd");
const args = process.argv.slice(2);
const playwrightArgs = args.filter(argument => argument !== "--skip-build");
const owned = [];
const backends = [];
const exists = async path => access(path).then(() => true, () => false);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let evidence;
let cleanupPromise;

function start(file, arguments_, env = process.env, cwd = root, input = "", stream = false) {
  const child = spawn(file, arguments_, { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  const record = { child, file, arguments: arguments_, stdout: "", stderr: "", finished: false };
  owned.push(record); // Capture this exact owned PID before readiness or auditing.
  record.closed = new Promise(resolve => {
    child.once("error", error => { record.error = error; });
    child.once("close", (code, signal) => { record.finished = true; record.signal = signal; resolve(code); });
  });
  child.stdout.on("data", data => { record.stdout += data; if (stream) process.stdout.write(data); });
  child.stderr.on("data", data => { record.stderr += data; if (stream) process.stderr.write(data); });
  child.stdin.on("error", () => {});
  child.stdin.end(input);
  return record;
}

async function finish(record, timeout = 45_000) {
  let timer;
  try {
    const code = await Promise.race([record.closed, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(`Owned PID ${record.child.pid} timed out`)), timeout);
    })]);
    if (record.error || code !== 0) throw new Error(`${record.error ?? `Process exited ${code} (${record.signal ?? "no signal"})`}\n${record.stdout}\n${record.stderr}`);
    return record.stdout;
  } finally { clearTimeout(timer); }
}

async function waitFor(predicate, record, timeout, message) {
  const deadline = Date.now() + timeout;
  while (!await predicate()) {
    if (record.finished || Date.now() >= deadline) throw new Error(`${message}\n${record.stdout}\n${record.stderr}`);
    await delay(100);
  }
}

// browser-check-process-cleanup-begin
async function stop(record) {
  if (record.finished || !record.child.pid) return;
  record.child.kill("SIGTERM");
  let timer;
  const stopped = await Promise.race([record.closed.then(() => true), new Promise(resolve => {
    timer = setTimeout(() => resolve(false), 10_000);
  })]);
  clearTimeout(timer);
  if (!stopped) {
    record.child.kill("SIGKILL");
    await Promise.race([record.closed, delay(10_000).then(() => { throw new Error(`Owned PID ${record.child.pid} remains`); })]);
  }
}

async function cleanup() {
  cleanupPromise ??= (async () => {
    const failures = [];
    for (const backend of backends) backend.hub?.close();
    for (const record of [...owned].reverse()) {
      try { await stop(record); } catch (error) { failures.push(String(error)); }
    }
    for (const backend of backends) {
      // PTYs own separate process groups. Only this fixture's recorded provider
      // PIDs, still carrying its unique executable path, may need final cleanup.
      const launches = (await readFile(join(backend.home, "provider-argv.jsonl"), "utf8").catch(() => ""))
        .split("\n").filter(Boolean).map(line => JSON.parse(line));
      for (const launch of launches) {
        const ps = start("/bin/ps", ["-p", String(launch.pid), "-o", "command="]);
        await ps.closed;
        if (!ps.stdout.includes(join(backend.home, "bin", "claude"))) continue;
        try { process.kill(launch.pid, "SIGTERM"); } catch (error) { if (error.code !== "ESRCH") failures.push(String(error)); }
        const deadline = Date.now() + 10_000;
        while (Date.now() < deadline) {
          try { process.kill(launch.pid, 0); } catch { break; }
          await delay(100);
        }
        try { process.kill(launch.pid, 0); failures.push(`Owned provider PID ${launch.pid} remains`); } catch { /* reaped */ }
      }
      for (const [name, record] of [["gwt", backend.gui], ["gwtd", backend.daemon]]) {
        if (!record) continue;
        await writeFile(join(backend.home, `${name}.stdout.log`), record.stdout);
        await writeFile(join(backend.home, `${name}.stderr.log`), record.stderr);
      }
    }
    if (evidence) {
      await writeFile(join(evidence, "cleanup.json"), JSON.stringify({
        failures, processes: owned.map(record => ({ pid: record.child.pid, file: record.file, finished: record.finished, signal: record.signal })),
      }, null, 2));
      console.log(`Issue #5017 evidence retained: ${evidence}`);
    }
    if (failures.length) throw new Error(failures.join("\n"));
  })();
  return cleanupPromise;
}
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.once(signal, () => {
  void cleanup().finally(() => process.exit(1));
});
// browser-check-process-cleanup-end

const fixtureSource = String.raw`
const fs = require('node:fs'), path = require('node:path'), readline = require('node:readline');
const bin = __dirname, home = path.dirname(bin), command = path.basename(process.argv[1]);
const args = process.argv.slice(2), statePath = path.join(home, 'agent-versions.json');
const versions = () => JSON.parse(fs.readFileSync(statePath, 'utf8'));
const packages = {'@anthropic-ai/claude-code':'claude','@openai/codex':'codex','@xai-official/grok':'grok','opencode-ai':'opencode','openclaw':'openclaw'};
const latest = {claude:'2.1.0',codex:'1.0.0',grok:'2.0.0',opencode:'1.0.0',openclaw:'1.0.0'};
const log = (name, value) => fs.appendFileSync(path.join(home, name), JSON.stringify(value) + '\n');
const refuse = () => { log('rejected-operations.jsonl', {command,args}); console.error('Unsupported fixture operation: ' + command + ' ' + args.join(' ')); process.exit(64); };
if (command === 'npm') {
  log('npm-argv.jsonl', args);
  if (args[0] === 'view' && packages[args[1]] && args[2] === 'version'
    && args.slice(3).every(arg => ['--json','--fetch-timeout=5000','--fetch-retries=0'].includes(arg))) {
    console.log(JSON.stringify(latest[packages[args[1]]])); process.exit(0);
  }
  if (args.length === 3 && args[0] === 'install' && args[1] === '-g') {
    const entry = Object.entries(packages).find(([pkg]) => args[2] === pkg || args[2].startsWith(pkg + '@'));
    if (!entry) refuse();
    const [pkg, id] = entry, selected = args[2].slice(pkg.length).replace(/^@/, '');
    if (selected && selected !== 'latest' && selected !== latest[id]) refuse();
    if (id === 'opencode' && !fs.existsSync(path.join(home, 'opencode-first-attempt'))) {
      fs.writeFileSync(path.join(home, 'opencode-first-attempt'), 'failed');
      console.error('Permission denied writing fixture prefix. Fix npm global prefix permissions and retry.'); process.exit(7);
    }
    const state = versions(); state[id] = latest[id]; fs.writeFileSync(statePath, JSON.stringify(state));
    fs.writeFileSync(path.join(bin, id), '#!' + process.execPath + '\nrequire("./fixture.cjs");\n', {mode:0o755});
    console.log('Installed fixture ' + id + ' ' + state[id]); process.exit(0);
  }
  refuse();
}
if (command === 'curl' || command === 'npx' || command === 'bunx') refuse();
if (command === 'gh') {
  if ((args.length === 1 && args[0] === '--version') || (args.length === 2 && args[0] === 'copilot' && args[1] === '--version')) {
    console.log('gh version 2.80.0'); process.exit(0);
  }
  refuse();
}
if (args.length === 1 && ['--version','-V'].includes(args[0])) {
  const version = versions()[command];
  if (command === 'codex') { console.error('Fixture version unavailable'); process.exit(1); }
  if (!version) refuse();
  console.log(command + ' ' + version); process.exit(0);
}
if (command !== 'claude') refuse();
log('provider-argv.jsonl', {pid:process.pid,argv:args,cwd:process.cwd(),at:new Date().toISOString()});
console.log('GWT_MAINTENANCE_FIXTURE_READY ' + process.pid);
readline.createInterface({input:process.stdin}).on('line', line => {
  log('provider-input.jsonl', {pid:process.pid,line});
  console.log('GWT_MAINTENANCE_FIXTURE_ACK ' + line);
});
`;

try {
  if (process.platform === "win32") throw new Error("Issue #5017 fixture requires POSIX executables");
  if (!args.includes("--skip-build")) await finish(start("cargo", ["build", "-p", "gwt", "--bin", "gwt", "--bin", "gwtd"], process.env, root, "", true), 1_800_000);
  if (!await exists(gwt) || !await exists(gwtd)) throw new Error("Build this checkout's gwt and gwtd first.");
  // Canonical verification owns and removes TMPDIR after the command. Keep
  // this runner's screenshots, traces and owned-process logs for review.
  evidence = await mkdtemp(join(root, "target/gwt-issue-5017-"));

  // browser-check-hook-authority-begin
  const disposable = /(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(?:debug|release)[/\\]gwtd(?:\.exe)?(?:[^a-z0-9_.-]|$)/i;
  let hookBin = process.env.GWT_HOOK_BIN || "";
  if (disposable.test(hookBin) || (hookBin && hookBin !== "gwtd" && !await exists(hookBin))) hookBin = "";
  if (!hookBin) {
    const probe = start("/bin/sh", ["-c", "command -v gwtd || true"]);
    const candidate = (await finish(probe)).trim();
    if (candidate && !disposable.test(candidate)) hookBin = "gwtd";
  }
  if (!hookBin && await exists("/Applications/GWT.app/Contents/MacOS/gwtd")) hookBin = "/Applications/GWT.app/Contents/MacOS/gwtd";
  hookBin ||= "gwtd";
  // browser-check-hook-authority-end

  for (const theme of ["dark", "light"]) {
    const home = await mkdtemp(join(evidence, `${theme}-home-`));
    const project = join(home, "project"), bin = join(home, "bin");
    const backend = { theme, home, project, bin };
    backends.push(backend);
    await Promise.all([bin, project, join(home, ".gwt"), join(home, ".codex"), join(project, ".codex")].map(path => mkdir(path, { recursive: true })));
    const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
      && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())));
    const fixturePath = `${bin}:/usr/bin:/bin:${dirname(process.execPath)}`;
    Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"), PATH: fixturePath,
      SHELL: "/bin/bash", GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_HOOK_BIN: hookBin,
      GWT_TEST_GH: join(bin, "gh"), GWT_DISABLE_BACKGROUND_INDEX: "1", GWT_PROJECT_ROOT: project });
    const runtime = join(process.env.HOME || homedir(), ".gwt/runtime");
    if (await exists(runtime)) await symlink(runtime, join(home, ".gwt/runtime"));
    await writeFile(join(bin, "fixture.cjs"), fixtureSource);
    // Mask every supported provider, including the intentionally missing CLI:
    // macOS path_helper may append real host tools even to an isolated PATH.
    for (const command of ["claude", "codex", "grok", "agy", "opencode", "openclaw", "hermes", "gh", "npm", "npx", "bunx", "curl"]) {
      await writeFile(join(bin, command), ["opencode", "openclaw"].includes(command)
        ? `#!${home}/nonexistent-interpreter\n`
        : `#!${process.execPath}\nrequire("./fixture.cjs");\n`, { mode: 0o755 });
    }
    await writeFile(join(home, "agent-versions.json"), JSON.stringify({ claude: "2.1.0", codex: null, grok: "1.0.0", agy: "1.0.0", opencode: null, openclaw: null, hermes: "1.0.0" }));
    await writeFile(join(home, "provider-argv.jsonl"), "");
    await writeFile(join(home, "npm-argv.jsonl"), "");
    await writeFile(join(home, ".gwt/config.toml"), `[profiles]\nactive = "Host"\n[[profiles.profiles]]\nname = "Host"\ndescription = "Isolated supported-agent fixture"\n[profiles.profiles.env_vars]\nPATH = ${JSON.stringify(fixturePath)}\n`);
    for (const arguments_ of [["init", "-q", "-b", "fixture-main"], ["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-q", "-m", "fixture"]]) await finish(start("git", arguments_, env, project), 10_000);
    async function envelope(operation, params = {}, outer = false) {
      const response = JSON.parse(await finish(start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation, params }))));
      if (!response.ok) throw new Error(`${operation} failed: ${JSON.stringify(response)}`);
      if (outer) return response;
      return operation === "daemon.status" || operation === "hook.register_codex_managed_hook_trust" ? response.output : JSON.parse(response.output);
    }
    const hash = (await envelope("issue.monitor.status", {}, true)).project_store.hash;
    if (!hash) throw new Error("Missing canonical project_store.hash");
    // browser-check-agent-seed-begin
    const preferences = join(home, ".gwt/projects", hash, "project-state");
    await mkdir(preferences, { recursive: true });
    await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
    await writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] }));
    // browser-check-agent-seed-end
    await writeFile(join(home, ".gwt/session.json"), JSON.stringify({ tabs: [{ id: "supported-agents", title: "Supported agents fixture", project_root: project, kind: "git" }], active_tab_id: "supported-agents", recent_projects: [] }));
    await writeFile(join(project, ".codex/hooks.json"), "{}");
    async function audit(repair) {
      const params = { expected_hook_bin: hookBin, runtime_state_path: join(home, ".gwt/browser-check-missing-runtime-state.json") };
      if (repair) {
        await envelope("hook.doctor", { ...params, repair: true });
        await envelope("hook.register_codex_managed_hook_trust", { project_root: project, codex_config: join(home, ".codex/config.toml"), codex_hook_discovery: "both" });
      }
      const health = await envelope("hook.health", params);
      await writeFile(join(home, repair ? "hook-repair.json" : "hook-audit.json"), JSON.stringify(health, null, 2));
      const blocking = health.issues.filter(issue => !(hookBin === "gwtd" && /^managed hook binary missing: .* uses gwtd$/.test(issue))
        && !/^managed hook failure: .*state=fail-open( |$)/.test(issue));
      if (health.status === "inactive" || blocking.length) throw new Error(`Hook convergence failed: ${JSON.stringify(health)}`);
    }
    // browser-check-hook-repair-begin
    await audit(true);
    // browser-check-hook-repair-end
    backend.daemon = start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }));
    await waitFor(async () => new RegExp(`^running pid=${backend.daemon.child.pid} .*probe=ok`).test(await envelope("daemon.status")), backend.daemon, 30_000, "Isolated daemon did not become ready");
    env.GWT_BROWSER_URL_FILE = join(home, "url");
    // browser-check-launch-begin
    backend.gui = start(gwt, ["--no-tray", "--no-open"], env, project);
    console.log(`Fresh ${theme} gwt PID: ${backend.gui.child.pid}; HOME: ${home}`);
    // browser-check-launch-end
    await waitFor(() => exists(env.GWT_BROWSER_URL_FILE), backend.gui, 90_000, "Fresh checkout did not publish its URL");
    backend.url = (await readFile(env.GWT_BROWSER_URL_FILE, "utf8")).trim();
    if (!(await fetch(backend.url, { method: "HEAD", signal: AbortSignal.timeout(10_000) })).ok) throw new Error("Fresh URL failed readiness");
    if (/another tray-resident gwt instance is already running/i.test(backend.gui.stdout + backend.gui.stderr)) throw new Error("Fresh instance isolation failed");
    // #5219: keep a real Hub connection while test pages navigate/disconnect.
    backend.hub = new WebSocket(new URL("/ws", backend.url).toString().replace(/^http/, "ws"));
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("Hub keepalive failed")), 10_000);
      backend.hub.addEventListener("open", () => { clearTimeout(timer); backend.hub.send(JSON.stringify({ kind: "frontend_ready" })); resolve(); }, { once: true });
      backend.hub.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Hub keepalive failed")); }, { once: true });
    });
    // browser-check-hook-audit-begin
    await audit(false);
    // browser-check-hook-audit-end
  }
  const metadata = Object.fromEntries(backends.map(backend => [backend.theme, {
    url: backend.url, home: backend.home, project: backend.project, pid: backend.gui.child.pid,
  }]));
  await writeFile(join(evidence, "isolated-backends.json"), JSON.stringify({ checkout: root, gwt, gwtd, hook_bin: hookBin, backends: metadata }, null, 2));
  // Keep the runner's real HOME/browser cache and canonical measured reporter.
  const testEnv = { ...process.env, GWT_L3_BACKENDS: JSON.stringify(metadata), GWT_L3_AGENT_FIXTURES: "1",
    GWT_L3_MISSING_AGENT: "opencode", GWT_L3_SCREENSHOT_DIR: evidence, GWT_PLAYWRIGHT_CHECKOUT_ROOT: root };
  const arguments_ = [join(root, "scripts/run-visual-tests.sh"), "settings-supported-agents-live.spec.ts", "--workers=1", "--retries=0", "--trace=on", "--output", join(evidence, "playwright"), ...playwrightArgs];
  if (!arguments_.includes("--headed")) arguments_.push("--headed");
  await finish(start("bash", arguments_, testEnv, root, "", true), 600_000);
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  try { await cleanup(); } catch (error) { console.error(error); process.exitCode = 1; }
}
