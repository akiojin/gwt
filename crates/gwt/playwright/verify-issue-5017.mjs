// Issue #5017: real headed Chromium, checkout binaries, fixture-only installers.
// Usage: node crates/gwt/playwright/verify-issue-5017.mjs [--skip-build] [Playwright arguments]
import { spawn } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const windows = process.platform === "win32";
const gwt = join(root, "target/debug/gwt" + (windows ? ".exe" : ""));
const gwtd = join(root, "target/debug/gwtd" + (windows ? ".exe" : ""));
const args = process.argv.slice(2);
const playwrightArgs = args.filter(argument => argument !== "--skip-build");
const owned = [];
const backends = [];
const exists = async path => access(path).then(() => true, () => false);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let evidence;
let cleanupPromise;

function start(file, arguments_, env = process.env, cwd = root, input = "", stream = false) {
  const child = spawn(file, arguments_, { cwd, env, windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
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
        if (!Number.isInteger(launch.pid) || launch.pid <= 0) continue;
        const ps = windows
          ? start(join(process.env.SystemRoot, "System32/WindowsPowerShell/v1.0/powershell.exe"), ["-NoProfile", "-Command",
            `(Get-CimInstance Win32_Process -Filter 'ProcessId = ${launch.pid}').CommandLine`])
          : start("/bin/ps", ["-p", String(launch.pid), "-o", "command="]);
        await ps.closed;
        if (!ps.stdout.includes(join(backend.home, "bin", windows ? "claude.cjs" : "claude"))) continue;
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
const bin = __dirname, home = path.dirname(bin), command = path.basename(process.argv[1]).replace(/\.cjs$/, '');
const args = process.argv.slice(2), statePath = path.join(home, 'agent-versions.json');
const versions = () => JSON.parse(fs.readFileSync(statePath, 'utf8'));
const packages = {'@openai/codex':'codex','@xai-official/grok':'grok','opencode-ai':'opencode','openclaw':'openclaw'};
const latest = {codex:'1.0.0',grok:'2.0.0',agy:'1.3.5',opencode:'1.0.0',openclaw:'1.0.0'};
const log = (name, value) => fs.appendFileSync(path.join(home, name), JSON.stringify(value) + '\n');
const refuse = () => { log('rejected-operations.jsonl', {command,args}); console.error('Unsupported fixture operation: ' + command + ' ' + args.join(' ')); process.exit(64); };
if (command === 'npm') {
  log('npm-argv.jsonl', args);
  if (args[0] === 'view' && packages[args[1]] && args[2] === 'version'
    && args.slice(3).every(arg => ['--json','--fetch-timeout=5000','--fetch-retries=0'].includes(arg))) {
    const id = packages[args[1]], marker = path.join(home, 'codex-metadata-started');
    log('metadata-events.jsonl', {id,stage:'started',pid:process.pid});
    if (id === 'codex') {
      fs.writeFileSync(marker, 'started');
      console.error('Fixture Codex metadata unavailable'); process.exit(8);
    }
    if (id === 'openclaw') {
      // A sequential catalog cannot reach Codex while this query is waiting.
      const deadline = Date.now() + 10_000;
      while (!fs.existsSync(marker)) {
        if (Date.now() >= deadline) {
          console.error('Codex metadata did not start while OpenClaw was waiting'); process.exit(9);
        }
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
      }
    }
    log('metadata-events.jsonl', {id,stage:'finished',pid:process.pid});
    console.log(JSON.stringify(latest[id])); process.exit(0);
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
    if (process.platform === 'win32') {
      fs.writeFileSync(path.join(bin, id + '.cjs'), 'require("./fixture.cjs");\n');
      fs.writeFileSync(path.join(bin, id + '.cmd'), '@echo off\r\nset "dp0=%~dp0"\r\n"' + process.execPath + '" "%dp0%\\' + id + '.cjs" %*\r\n');
    } else fs.writeFileSync(path.join(bin, id), '#!' + process.execPath + '\nrequire("./fixture.cjs");\n', {mode:0o755});
    console.log('Installed fixture ' + id + ' ' + state[id]); process.exit(0);
  }
  refuse();
}
if (command === 'curl') {
  log('curl-argv.jsonl', args);
  const manifest = /^https:\/\/antigravity-cli-auto-updater-974169037036\.us-central1\.run\.app\/manifests\/(?:(?:darwin|windows)_(?:amd64|arm64)|linux_(?:amd64|arm64)(?:_musl)?)\.json$/;
  if (!manifest.test(args.at(-1))) refuse();
  const flags = ['--fail','--silent','--show-error','--location','--proto','=https','--proto-redir','=https','--connect-timeout','5','--max-time','10'];
  if (JSON.stringify(args.slice(0, -1)) !== JSON.stringify(flags)) refuse();
  if (fs.existsSync(path.join(home, 'non-npm-metadata-unavailable'))) {
    console.error('Fixture Antigravity metadata unavailable'); process.exit(8);
  }
  log('metadata-events.jsonl', {id:'agy',stage:'finished',pid:process.pid});
  console.log(JSON.stringify({version:latest.agy,url:'https://storage.googleapis.com/antigravity-public/fixture/agy',sha512:'a'.repeat(128)})); process.exit(0);
}
if (command === 'npx' || command === 'bunx') refuse();
if (command === 'gh') {
  log('gh-argv.jsonl', args);
  if (args.length === 1 && args[0] === '--version') {
    console.log('gh version 2.80.0'); process.exit(0);
  }
  if (args.length === 2 && args[0] === 'copilot' && args[1] === '--version') {
    console.log('gh copilot version ' + versions().gh); process.exit(0);
  }
  refuse();
}
if (command === 'agy' && args.length === 1 && args[0] === 'update') {
  log('native-update-argv.jsonl', {command,args});
  const state = versions(); state.agy = latest.agy; fs.writeFileSync(statePath, JSON.stringify(state));
  console.log('Updated fixture Antigravity CLI ' + state.agy); process.exit(0);
}
if (args.length === 1 && ['--version','-V'].includes(args[0])) {
  const version = versions()[command];
  if (command === 'codex') { console.error('Fixture version unavailable'); process.exit(1); }
  if (!version) refuse();
  console.log(command + ' ' + version); process.exit(0);
}
if (command !== 'claude') refuse();
log('provider-argv.jsonl', {pid:process.pid,ppid:process.ppid,argv:args,cwd:process.cwd(),at:new Date().toISOString()});
console.log('GWT_MAINTENANCE_FIXTURE_READY ' + process.pid);
readline.createInterface({input:process.stdin}).on('line', line => {
  log('provider-input.jsonl', {pid:process.pid,line});
  console.log('GWT_MAINTENANCE_FIXTURE_ACK ' + line);
});
`;

try {
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
    const resolverEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^GWT_(BIN_PATH|HOOK_BIN)$/i.test(key)));
    const result = JSON.parse(await finish(start(gwtd, [], resolverEnv, root,
      JSON.stringify({ schema_version: 1, operation: "hook.doctor", params: {} }))));
    if (!result.ok) throw new Error(`Managed hook fallback resolution failed: ${JSON.stringify(result)}`);
    hookBin = JSON.parse(result.output).managed_hook_bin;
    if (!hookBin || disposable.test(hookBin)) throw new Error("Managed hook resolver returned no stable fallback");
  }
  // browser-check-hook-authority-end

  const git = windows ? (await finish(start("where.exe", ["git.exe"]))).trim().split(/\r?\n/)[0] : "git";

  for (const theme of ["dark", "light"]) {
    const home = await mkdtemp(join(evidence, `${theme}-home-`));
    const project = join(home, "project"), bin = join(home, "bin");
    const backend = { theme, home, project, bin };
    backends.push(backend);
    await Promise.all([bin, project, join(home, ".gwt"), join(home, ".codex"), join(project, ".codex")].map(path => mkdir(path, { recursive: true })));
    const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
      && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())));
    const fixturePath = (windows
      ? [bin, dirname(process.execPath), dirname(git), join(process.env.SystemRoot, "System32"),
        join(process.env.SystemRoot, "System32/WindowsPowerShell/v1.0")]
      : [bin, "/usr/bin", "/bin", dirname(process.execPath)]).join(delimiter);
    // Windows environment keys are case insensitive; do not retain an earlier Path.
    for (const key of Object.keys(env)) if (/^(PATH|SHELL)$/i.test(key)) delete env[key];
    Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"), PATH: fixturePath,
      SHELL: windows ? join(process.env.SystemRoot, "System32/cmd.exe") : "/bin/bash",
      GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_HOOK_BIN: hookBin,
      GWT_TEST_GH: join(bin, windows ? "gh.cmd" : "gh"), GWT_DISABLE_BACKGROUND_INDEX: "1", GWT_PROJECT_ROOT: project });
    const runtime = join((windows ? process.env.USERPROFILE : process.env.HOME) || homedir(), ".gwt/runtime");
    if (await exists(runtime)) await symlink(runtime, join(home, ".gwt/runtime"), windows ? "junction" : "dir");
    await writeFile(join(bin, "fixture.cjs"), fixtureSource);
    // Mask every supported provider, including the intentionally missing CLI:
    // macOS path_helper may append real host tools even to an isolated PATH.
    for (const command of ["claude", "codex", "grok", "agy", "opencode", "openclaw", "hermes", "gh", "npm", "npx", "bunx", "curl"]) {
      if (windows) {
        // The restricted PATH contains no host providers. Missing CLIs stay absent
        // until the fixture installer writes their native npm shim.
        if (["opencode", "openclaw"].includes(command)) continue;
        await writeFile(join(bin, `${command}.cjs`), 'require("./fixture.cjs");\n');
        await writeFile(join(bin, `${command}.cmd`),
          `@echo off\r\nset "dp0=%~dp0"\r\n"${process.execPath}" "%dp0%\\${command}.cjs" %*\r\n`);
      } else await writeFile(join(bin, command), ["opencode", "openclaw"].includes(command)
        ? `#!${home}/nonexistent-interpreter\n`
        : `#!${process.execPath}\nrequire("./fixture.cjs");\n`, { mode: 0o755 });
    }
    await writeFile(join(home, "agent-versions.json"), JSON.stringify({ claude: "2.1.0", codex: null, grok: "1.0.0", agy: "1.0.0", opencode: null, openclaw: null, hermes: "1.0.0", gh: "1.0.0" }));
    await writeFile(join(home, "provider-argv.jsonl"), "");
    await writeFile(join(home, "npm-argv.jsonl"), "");
    await writeFile(join(home, ".gwt/config.toml"), `[profiles]\nactive = "Host"\n[[profiles.profiles]]\nname = "Host"\ndescription = "Isolated supported-agent fixture"\n[profiles.profiles.env_vars]\nPATH = ${JSON.stringify(fixturePath)}\n`);
    for (const arguments_ of [["init", "-q", "-b", "fixture-main"], ["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-q", "-m", "fixture"]]) await finish(start(git, arguments_, env, project), 10_000);
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
  const arguments_ = ["settings-supported-agents-live.spec.ts", "--workers=1", "--retries=0", "--trace=on", "--output", join(evidence, "playwright"), ...playwrightArgs];
  if (!arguments_.includes("--headed")) arguments_.push("--headed");
  if (windows) {
    const version = (await readFile(join(root, "scripts/playwright-version.txt"), "utf8")).trim();
    const deps = process.env.GWT_PLAYWRIGHT_DEPS_DIR || join(tmpdir(), `gwt-playwright-${version}`);
    const playwright = join(deps, "node_modules/@playwright/test/cli.js");
    if (!await exists(playwright)) throw new Error(`Pinned Playwright is missing: ${playwright}`);
    const resolver = join(evidence, "playwright-resolver.cjs");
    await writeFile(resolver, `const Module = require("module"), path = require("node:path");
const original = Module._resolveFilename;
Module._resolveFilename = function(request, parent, ...rest) {
  if (request === "@playwright/test" || request.startsWith("@playwright/test/")) {
    return original.call(this, path.join(process.env.GWT_PLAYWRIGHT_NODE_MODULES, request), parent, ...rest);
  }
  return original.call(this, request, parent, ...rest);
};
`);
    testEnv.GWT_PLAYWRIGHT_NODE_MODULES = join(deps, "node_modules");
    testEnv.NODE_OPTIONS = `${testEnv.NODE_OPTIONS || ""} --require ${JSON.stringify(resolver.replaceAll("\\", "/"))}`;
    await finish(start(process.execPath, [playwright, "test", "--config", join(root, "crates/gwt/playwright/playwright.config.ts"),
      ...arguments_], testEnv, root, "", true), 600_000);
  } else await finish(start("bash", [join(root, "scripts/run-visual-tests.sh"), ...arguments_], testEnv, root, "", true), 600_000);
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  try { await cleanup(); } catch (error) { console.error(error); process.exitCode = 1; }
}
