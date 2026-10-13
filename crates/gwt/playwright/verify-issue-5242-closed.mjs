// Issue #5242 headed verification. Mutable state and GitHub traffic stay local.
// Usage: node crates/gwt/playwright/verify-issue-5242-closed.mjs [--skip-build] [Playwright arguments]
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, writeFile, access, symlink } from "node:fs/promises";
import { homedir, hostname, tmpdir } from "node:os";
import { dirname, join, resolve, delimiter } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const windows = process.platform === "win32";
const suffix = windows ? ".exe" : "";
const gwt = join(root, "target/debug/gwt" + suffix);
const gwtd = join(root, "target/debug/gwtd" + suffix);
const args = process.argv.slice(2);
const skipBuild = args.includes("--skip-build");
const playwrightArgs = args.filter(argument => argument !== "--skip-build");
const owned = [];
let checkHome;
let gui;
let daemon;
let browserSession;
let cleanupPromise;
const exists = async path => access(path).then(() => true, () => false);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));

function start(file, arguments_, env = process.env, cwd = root, input = "", stream = false) {
  const child = spawn(file, arguments_, { cwd, env, windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
  const record = { child, stdout: "", stderr: "", finished: false, error: null };
  owned.push(record); // Capture this exact owned PID before waiting for readiness.
  record.closed = new Promise(resolve => {
    child.once("error", error => { record.error = error; });
    child.once("close", code => { record.finished = true; resolve(code); });
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
    if (record.error || code !== 0) throw new Error(`${record.error ?? `Process exited ${code}`}\n${record.stdout}\n${record.stderr}`);
    return record.stdout;
  } finally { clearTimeout(timer); }
}

async function cleanup() {
  cleanupPromise ??= (async () => {
    browserSession?.close();
    const errors = [];
    for (const record of [...owned].reverse()) {
      if (record.finished || !record.child.pid) continue;
      if (windows) {
        // Only this harness's still-owned process tree, never a name/pattern.
        const killer = spawn("taskkill.exe", ["/PID", String(record.child.pid), "/T", "/F"], { windowsHide: true, stdio: "ignore" });
        await new Promise(resolve => { killer.once("error", resolve); killer.once("close", resolve); });
      } else record.child.kill("SIGTERM");
      let timer;
      const stopped = await Promise.race([record.closed.then(() => true), new Promise(resolve => {
        timer = setTimeout(() => resolve(false), 10_000);
      })]);
      clearTimeout(timer);
      if (!stopped) errors.push(`Owned PID ${record.child.pid} did not exit`);
    }
    if (gui && checkHome) {
      await writeFile(join(checkHome, "gwt.stdout.log"), gui.stdout);
      await writeFile(join(checkHome, "gwt.stderr.log"), gui.stderr);
    }
    if (daemon && checkHome) {
      await writeFile(join(checkHome, "gwtd.stdout.log"), daemon.stdout);
      await writeFile(join(checkHome, "gwtd.stderr.log"), daemon.stderr);
    }
    if (checkHome) console.log(`Owned processes stopped; evidence retained in ${checkHome}`);
    if (errors.length) throw new Error(errors.join("\n"));
  })();
  return cleanupPromise;
}

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.once(signal, () => {
  void cleanup().finally(() => process.exit(1));
});

const fixtureSource = String.raw`
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const home = path.dirname(__dirname), store = path.join(home, 'github-state.json');
const load = () => JSON.parse(fs.readFileSync(store, 'utf8'));
const save = state => { fs.writeFileSync(store + '.tmp', JSON.stringify(state)); fs.renameSync(store + '.tmp', store); };
const touch = (state, issue) => { issue.updatedAt = new Date(Date.UTC(2026, 9, 10) + ++state.revision * 1000).toISOString(); };
const rest = issue => ({...issue, state: issue.state.toLowerCase(), updated_at: issue.updatedAt});
const connection = nodes => ({nodes, pageInfo:{hasNextPage:false,endCursor:null}});
const graph = issue => issue && ({...issue, labels:connection(issue.labels), comments:connection(issue.comments.map(comment => ({...comment,databaseId:comment.id,updatedAt:comment.updated_at}))), timelineItems:connection([])});
if (process.argv[2] === '--gh') {
  const args = process.argv.slice(3), text = args.join(' '), state = load();
  fs.appendFileSync(path.join(home, 'gh-argv.jsonl'), JSON.stringify(args) + '\n');
  if (args[0] === '--version') { console.log('gh version 2.80.0'); process.exit(0); }
  if (args[0] === 'auth') { if (args[1] === 'token') console.log('fixture-only'); process.exit(0); }
  if (args.includes('--include')) process.stdout.write('HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n');
  if (args[0] === 'api' && text.includes('/issues?')) {
    const rows = state.issues.filter(issue => issue.state === 'OPEN' && (!text.includes('labels=gwt-queued') || issue.labels.some(label => label.name === 'gwt-queued')));
    console.log(JSON.stringify(text.includes('page=1') ? rows.map(rest) : [])); process.exit(0);
  }
  if (args[0] === 'issue' && args[1] === 'list') { console.log(JSON.stringify(state.issues)); process.exit(0); }
  if (args[0] === 'issue' && args[1] === 'view') { console.log(JSON.stringify(state.issues.find(issue => String(issue.number) === args[2]))); process.exit(0); }
  if (args[0] === 'api' && args[1] === 'graphql') { console.log(JSON.stringify({data:{repository:{issues:connection(state.issues.map(graph)),issue:{timelineItems:connection([])}}}})); process.exit(0); }
  if ((args[0] === 'api' && text.includes('/pulls?')) || (args[0] === 'pr' && args[1] === 'list')) { console.log('[]'); process.exit(0); }
  console.error('Unsupported fixture gh: ' + text); process.exit(64);
}
const server = http.createServer(async (request, response) => {
  let text = ''; for await (const chunk of request) text += chunk;
  const payload = text ? JSON.parse(text) : {}, state = load();
  fs.appendFileSync(path.join(home, 'http-requests.jsonl'), JSON.stringify({method:request.method,url:request.url,payload}) + '\n');
  let result, status = 200;
  const match = request.url.match(/\/issues\/(\d+)(.*)/), issue = state.issues.find(row => row.number === Number(match?.[1] ?? payload.variables?.number));
  if (request.url === '/graphql') result = {data:{repository:{issue:graph(issue)},rateLimit:{cost:1,remaining:5000,resetAt:'2099-01-01T00:00:00Z'}}};
  else if (match && issue && request.method === 'GET') result = rest(issue);
  else if (match && issue && request.method === 'PATCH' && !match[2]) {
    if (payload.state) issue.state = payload.state.toUpperCase();
    if (payload.state_reason) issue.state_reason = payload.state_reason;
    if (payload.labels) issue.labels = payload.labels.map(name => ({name}));
    touch(state, issue); save(state); result = rest(issue);
  } else if (match && issue && request.method === 'DELETE' && match[2].startsWith('/labels/')) {
    issue.labels = issue.labels.filter(label => label.name !== decodeURIComponent(match[2].slice(8)));
    touch(state, issue); save(state); result = issue.labels;
  } else if (match && issue && request.method === 'POST' && match[2] === '/labels') {
    for (const name of payload.labels) if (!issue.labels.some(label => label.name === name)) issue.labels.push({name});
    touch(state, issue); save(state); result = issue.labels;
  } else if (match && issue && request.method === 'POST' && match[2] === '/comments') {
    touch(state, issue); result = {id:++state.comment_id,body:payload.body,updated_at:issue.updatedAt};
    issue.comments.push(result); save(state);
  } else { status = 500; result = {message:'Unsupported fixture request'}; }
  response.writeHead(status, {'Content-Type':'application/json'}); response.end(JSON.stringify(result));
});
server.listen(0, '127.0.0.1', () => fs.writeFileSync(path.join(home, 'github-url'), 'http://127.0.0.1:' + server.address().port));
`;

try {
  if (!skipBuild) await finish(start("cargo", ["build", "-p", "gwt", "--bin", "gwt", "--bin", "gwtd"], process.env, root, "", true), 1_800_000);
  if (!await exists(gwt) || !await exists(gwtd)) throw new Error("Build this checkout's gwt and gwtd first.");
  const version = (await readFile(join(root, "scripts/playwright-version.txt"), "utf8")).trim();
  const deps = process.env.GWT_PLAYWRIGHT_DEPS_DIR || join(tmpdir(), `gwt-playwright-${version}`);
  const playwright = join(deps, "node_modules/@playwright/test/cli.js");
  if (!await exists(playwright)) {
    await mkdir(deps, { recursive: true });
    await writeFile(join(deps, "package.json"), JSON.stringify({ private: true, dependencies: { "@playwright/test": version } }));
    // npm ships next to node on Windows; use its JavaScript entrypoint directly.
    const npm = windows ? join(dirname(process.execPath), "node_modules/npm/bin/npm-cli.js") : "/usr/share/nodejs/npm/bin/npm-cli.js";
    await finish(start(process.execPath, [npm, "install", "--silent", "--no-audit", "--no-fund", "--package-lock=false"], process.env, deps), 180_000);
  }
  checkHome = await mkdtemp(join(tmpdir(), "gwt-fresh-home.5242-"));
  const project = join(checkHome, "project"), bin = join(checkHome, "bin");
  await Promise.all([project, bin, join(checkHome, ".gwt"), join(project, ".codex")].map(path => mkdir(path, { recursive: true })));
  const originalHome = process.env.HOME, originalProfile = process.env.USERPROFILE;
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(name) && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(name.toUpperCase())));
  const pathKey = Object.keys(env).find(name => name.toUpperCase() === "PATH") ?? "PATH";
  Object.assign(env, { HOME: checkHome, USERPROFILE: checkHome, GWT_TEST_GH: join(bin, windows ? "gh.cmd" : "gh"), GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_HOOK_BIN: "gwtd" });
  env[pathKey] = bin + delimiter + (env[pathKey] ?? "");
  const runtime = join(originalProfile || originalHome || homedir(), ".gwt/runtime");
  if (await exists(runtime)) await symlink(runtime, join(checkHome, ".gwt/runtime"), windows ? "junction" : "dir");
  const fixtureScript = join(bin, "github.cjs");
  await writeFile(fixtureScript, fixtureSource);
  await writeFile(env.GWT_TEST_GH, windows ? `@echo off\r\n"${process.execPath}" "${fixtureScript}" --gh %*\r\n` : `#!/bin/sh\nexec '${process.execPath}' '${fixtureScript}' --gh "$@"\n`, { mode: 0o755 });
  const issues = [524201, 524202].map(number => ({ number, title: `Closed retirement fixture #${number}`, body: "Isolated Issue #5242 browser fixture", state: "OPEN", labels: ["bug", "auto-improve", "gwt-queued"].map(name => ({ name })), updatedAt: "2026-10-10T00:00:00Z", url: `https://github.com/fixture/closed-live/issues/${number}`, comments: [] }));
  await writeFile(join(checkHome, "github-state.json"), JSON.stringify({ revision: 0, comment_id: 0, issues }));
  for (const arguments_ of [["init", "-q"], ["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-q", "-m", "fixture"], ["remote", "add", "origin", "https://github.com/fixture/closed-live.git"]]) await finish(start("git", arguments_, env, project), 10_000);
  async function waitFor(predicate, child, timeout, message) {
    const deadline = Date.now() + timeout;
    while (!await predicate()) {
      if (child.finished || Date.now() >= deadline) throw new Error(`${message}\n${child.stderr}`);
      await delay(100);
    }
  }
  const api = start(process.execPath, [fixtureScript], env, project);
  await waitFor(() => exists(join(checkHome, "github-url")), api, 15_000, "Fixture GitHub server failed to start");
  const endpoint = await readFile(join(checkHome, "github-url"), "utf8");
  Object.assign(env, { GWT_OWNER_GITHUB_TEST_MODE: "loopback-v1", GWT_OWNER_GITHUB_REST_BASE: endpoint, GWT_OWNER_GITHUB_GRAPHQL_URL: `${endpoint}/graphql`, GWT_OWNER_GITHUB_TOKEN: "fixture-only" });
  async function envelope(operation, params = {}, outer = false) {
    const response = JSON.parse(await finish(start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation, params }))));
    if (!response.ok) throw new Error(`${operation} failed: ${JSON.stringify(response)}`);
    if (outer) return response;
    if (["daemon.status", "hook.register_codex_managed_hook_trust"].includes(operation)) return response.output;
    return JSON.parse(response.output);
  }
  const repoHash = (await envelope("issue.monitor.status", {}, true)).project_store.hash;
  if (!repoHash) throw new Error("Isolated project_store.hash was not resolved");
  const prefs = join(checkHome, ".gwt/projects", repoHash, "project-state");
  await mkdir(prefs, { recursive: true });
  await writeFile(join(prefs, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(prefs, "issue-monitor.json"), JSON.stringify({ enabled: false, autonomous_mode: false, max_active_agents: 4, max_active_agents_mode: "manual", auto_close_merged_issues: false, auto_apply_updates: false, terminal_queue_auto_refill: false, priority_order: [], terminal_queues: { [hostname().toLowerCase()]: { entries: issues.map(issue => ({ number: issue.number, queued_at: issue.updatedAt, queued_by: "fixture" })) } }, launched_issues: issues.map(issue => ({ issue_number: issue.number, window_id: `fixture-${issue.number}` })) }));
  await writeFile(join(checkHome, ".gwt/session.json"), JSON.stringify({ tabs: [{ id: "closed-fixture", title: "Closed fixture", project_root: project, kind: "git" }], active_tab_id: "closed-fixture", recent_projects: [] }));
  await writeFile(join(project, ".codex/hooks.json"), "{}");
  async function assertHooks(repair) {
    const params = { expected_hook_bin: "gwtd", runtime_state_path: join(checkHome, ".gwt/missing-runtime.json") };
    if (repair) {
      await envelope("hook.doctor", { ...params, repair: true });
      await envelope("hook.register_codex_managed_hook_trust", { project_root: project, codex_config: join(checkHome, ".codex/config.toml"), codex_hook_discovery: "both" });
    }
    const health = await envelope("hook.health", params);
    const blocking = health.issues.filter(issue => !/^managed hook binary missing: .* uses gwtd$/.test(issue) && !/^managed hook failure: .*state=fail-open( |$)/.test(issue));
    if (health.status === "inactive" || blocking.length) throw new Error(`Hook audit failed: ${JSON.stringify(health)}`);
  }
  await assertHooks(true);
  daemon = start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }));
  await waitFor(async () => new RegExp(`^running pid=${daemon.child.pid} .*probe=ok`).test(await envelope("daemon.status")), daemon, 30_000, "Isolated daemon failed to become ready");
  env.GWT_BROWSER_URL_FILE = join(checkHome, "url");
  gui = start(gwt, ["--no-tray", "--no-open"], env, project);
  console.log(`Fresh gwt PID: ${gui.child.pid}; isolated HOME: ${checkHome}`);
  await waitFor(() => exists(env.GWT_BROWSER_URL_FILE), gui, 90_000, "Fresh gwt did not publish its URL");
  const url = (await readFile(env.GWT_BROWSER_URL_FILE, "utf8")).trim();
  if ((await fetch(url, { method: "HEAD", signal: AbortSignal.timeout(10_000) })).status !== 200) throw new Error("Fresh URL failed its readiness check");
  // #5219: transient gwt exits after its last browser disconnects. Retain one
  // real Hub connection across Playwright's separate dark/light contexts.
  browserSession = new WebSocket(new URL("/ws", url).toString().replace(/^http/, "ws"));
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Fixture browser session failed to connect")), 10_000);
    browserSession.addEventListener("open", () => { clearTimeout(timer); browserSession.send(JSON.stringify({ kind: "frontend_ready" })); resolve(); }, { once: true });
    browserSession.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Fixture browser session failed to connect")); }, { once: true });
  });
  await assertHooks(false);
  await writeFile(join(checkHome, "issue-5242-isolated.json"), JSON.stringify({ project, repo_hash: repoHash, gwtd, fake_gh: env.GWT_TEST_GH, endpoint }));
  const testEnv = { ...env, HOME: originalHome || originalProfile || homedir(), USERPROFILE: originalProfile || homedir(), NODE_PATH: join(deps, "node_modules"), GWT_PLAYWRIGHT_BASE_URL: url, GWT_PLAYWRIGHT_CHECK_HOME: checkHome, GWT_PLAYWRIGHT_PROJECT_ROOT: project };
  // Preserve canonical reporter evidence only for the test child, not the app.
  if (process.env.GWT_HEADED_E2E_REPORT) testEnv.GWT_HEADED_E2E_REPORT = process.env.GWT_HEADED_E2E_REPORT;
  const arguments_ = [playwright, "test", "--config", join(root, "crates/gwt/playwright/playwright.config.ts"), "issue-monitor-closed-live.spec.ts", "--workers=1", "--output", join(checkHome, "playwright"), ...playwrightArgs];
  if (!arguments_.includes("--headed")) arguments_.push("--headed");
  await finish(start(process.execPath, arguments_, testEnv, project, "", true), 600_000);
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally { await cleanup(); }
