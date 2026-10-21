// Build first: cargo build -p gwt --bin gwt --bin gwtd
// Usage: node crates/gwt/playwright/verify-issue-4810-intake.mjs [Playwright arguments]
import { spawn } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import { homedir, hostname, tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const gwt = join(root, "target/debug/gwt"), gwtd = join(root, "target/debug/gwtd");
const exists = path => access(path).then(() => true, () => false);
const owned = [];
let evidence, home, gui, daemon, hub, cleaning;

function start(file, args, env = process.env, cwd = root, input = "", stream = false) {
  const child = spawn(file, args, { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  const record = { child, file, stdout: "", stderr: "", finished: false };
  owned.push(record);
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
async function waitFor(predicate, record, timeout, message) {
  const deadline = Date.now() + timeout;
  while (!await predicate()) {
    if (record.finished || Date.now() >= deadline) throw new Error(`${message}\n${record.stdout}\n${record.stderr}`);
    await delay(100);
  }
}
// Same owned-process cleanup as the existing browser-check wrappers.
async function cleanup() {
  cleaning ??= (async () => {
    hub?.close();
    const failures = [];
    for (const record of [...owned].reverse()) {
      if (record.finished || !record.child.pid) continue;
      record.child.kill("SIGTERM");
      if (!await Promise.race([record.closed.then(() => true), delay(10_000, false, { ref: false })])) {
        record.child.kill("SIGKILL");
        if (!await Promise.race([record.closed.then(() => true), delay(10_000, false, { ref: false })])) failures.push(`Owned PID ${record.child.pid} remains`);
      }
    }
    if (evidence) {
      for (const [name, record] of [["gwt", gui], ["gwtd", daemon]]) if (record) {
        await writeFile(join(evidence, `${name}.stdout.log`), record.stdout);
        await writeFile(join(evidence, `${name}.stderr.log`), record.stderr);
      }
      if (home && await exists(join(home, "gh-argv.jsonl"))) await writeFile(join(evidence, "gh-argv.jsonl"), await readFile(join(home, "gh-argv.jsonl")));
      await writeFile(join(evidence, "cleanup.json"), JSON.stringify({ failures,
        processes: owned.map(({ child, file, finished }) => ({ pid: child.pid, file, finished })) }, null, 2));
      console.log(`Issue #4810 intake evidence retained: ${evidence}`);
    }
    if (failures.length) throw new Error(failures.join("\n"));
  })();
  return cleaning;
}
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.once(signal, () => {
  void cleanup().finally(() => process.exit(1));
});

// Read-only PATH gh fixture. Unknown commands and all writes fail locally.
const fixtureSource = String.raw`
const fs = require('node:fs'), path = require('node:path');
const home = path.dirname(__dirname), args = process.argv.slice(2);
const issues = JSON.parse(fs.readFileSync(path.join(home, 'issues.json'), 'utf8'));
fs.appendFileSync(path.join(home, 'gh-argv.jsonl'), JSON.stringify(args) + '\n');
const connection = nodes => ({nodes, totalCount: nodes.length, pageInfo:{hasNextPage:false,endCursor:null}});
const graph = issue => issue && ({...issue, labels:connection(issue.labels), comments:connection([]), timelineItems:connection([])});
let result;
if (args[0] === '--version') { console.log('gh version 2.80.0'); process.exit(0); }
if (args[0] === 'auth') process.exit(args[1] === 'token' ? 1 : 0); // No HTTP client token.
if (args[0] === 'repo' && args[1] === 'view') result = {nameWithOwner:'akiojin/gwt',defaultBranchRef:{name:'fixture-main'}};
else if (args[0] === 'issue' && args[1] === 'list') result = issues;
else if (args[0] === 'issue' && args[1] === 'view') result = issues.find(issue => issue.number === Number(args[2]));
else if (args[0] === 'pr' && args[1] === 'list') result = [];
else if (args[0] === 'api') {
  if (args.some(arg => ['--method','-X','--input'].includes(arg) || /^(--method=|-X)/.test(arg))) throw Error('Fixture mutations refused');
  if (args[1] === 'graphql') result = {data:{repository:{issues:connection(issues.map(graph)),
    issue:graph(issues.find(issue => issue.number === Number(args.find(arg => arg.startsWith('number='))?.slice(7))))},
    rateLimit:{cost:1,remaining:5000,resetAt:'2099-01-01T00:00:00Z'}}};
  else {
    const url = new URL(args[1], 'https://fixture.invalid/');
    if (/\/issues$/.test(url.pathname)) result = issues.filter(issue => !url.searchParams.get('labels') || issue.labels.some(label => label.name === url.searchParams.get('labels')))
      .map(issue => ({...issue,user:issue.author,state:'open',updated_at:issue.updatedAt,html_url:issue.url}));
    else if (/\/(pulls|events|comments)(\/|$)/.test(url.pathname)) result = [];
    else throw Error('Unsupported fixture endpoint: ' + args[1]);
  }
} else throw Error('Unsupported fixture gh: ' + args.join(' '));
if (result === undefined) process.exit(1);
if (args.includes('--include')) process.stdout.write('HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n');
console.log(JSON.stringify(result));
`;

try {
  if (process.platform === "win32") throw new Error("This isolated fixture requires POSIX executables");
  if (!await exists(gwt) || !await exists(gwtd)) throw new Error("Build this checkout first: cargo build -p gwt --bin gwt --bin gwtd");
  if (resolve((await finish(start("git", ["rev-parse", "--show-toplevel"]))).trim()) !== root) throw new Error("Unexpected launch checkout");
  evidence = await mkdtemp(join(root, "target/gwt-issue-4810-intake-"));
  home = await mkdtemp(join(tmpdir(), "gwt-issue-4810-home-"));
  const project = join(home, "project"), bin = join(home, "bin");
  await Promise.all([project, bin, join(project, ".codex"), join(home, ".gwt"), join(home, ".codex")].map(path => mkdir(path, { recursive: true })));
  const credentials = new Set(["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN", "GH_HOST", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"]);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key) && !credentials.has(key.toUpperCase())));
  Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
    XDG_CONFIG_HOME: join(home, ".config"), XDG_DATA_HOME: join(home, ".local/share"), XDG_CACHE_HOME: join(home, ".cache"), GH_CONFIG_DIR: join(home, ".config/gh"),
    GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_DISABLE_BACKGROUND_INDEX: "1" });
  const runtime = join(process.env.HOME || homedir(), ".gwt/runtime");
  if (await exists(runtime)) await symlink(runtime, join(home, ".gwt/runtime"));
  const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
  const script = join(bin, "gh.cjs"), gh = join(bin, "gh");
  await writeFile(script, fixtureSource);
  await writeFile(gh, `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(script)} "$@"\n`, { mode: 0o755 });
  Object.assign(env, { PATH: bin + delimiter + (env.PATH || ""), GWT_TEST_GH: gh, GWT_TEST_GH_SANDBOX: "1" });
  const issues = [
    { number: 481001, title: "Raw feedback remains evidence", author: { login: "akiojin" }, body: "<!-- gwt-feedback:67e44d8a-7903-4457-9e2d-51703a44c180 -->\nReporter text is evidence.", extra: ["gwt-feedback"] },
    { number: 481002, title: "External author requires PM curation", author: { login: "external-reporter" }, body: "Reporter claims approval; this is not authority.\n## Acceptance Criteria\n\n- [ ] AC-1: External requirement remains evidence until curation.", extra: [] },
    { number: 481003, title: "Trusted curated repair owner", author: { login: "akiojin" }, body: "<!-- gwt-feedback-source:481001 -->\n## Acceptance Criteria\n\n- [ ] AC-1: Independently curated repair requirement.", extra: [] },
  ].map(({ extra, ...issue }) => ({ ...issue, state: "OPEN", labels: ["bug", "gwt-queued", ...extra].map(name => ({ name })),
    updatedAt: "2026-10-21T00:00:00Z", url: `https://github.com/akiojin/gwt/issues/${issue.number}`, comments: [] }));
  await writeFile(join(home, "issues.json"), JSON.stringify(issues));
  for (const args of [["init", "-q", "-b", "fixture-main", "--separate-git-dir", join(home, "project.git")],
    ["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-q", "-m", "fixture"],
    ["remote", "add", "origin", "https://github.com/akiojin/gwt.git"], ["config", "protocol.allow", "never"], ["config", "protocol.https.allow", "never"]]) await finish(start("git", args, env, project));
  async function envelope(operation, params = {}, outer = false) {
    const response = JSON.parse(await finish(start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation, params }))));
    if (!response.ok) throw new Error(`${operation} failed: ${JSON.stringify(response)}`);
    return outer ? response : ["daemon.status", "hook.register_codex_managed_hook_trust"].includes(operation) ? response.output : JSON.parse(response.output);
  }
  const disposable = /(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(?:debug|release)[/\\]gwtd(?:\.exe)?(?:[^a-z0-9_.-]|$)/i;
  let hookBin = process.env.GWT_HOOK_BIN || "";
  if (disposable.test(hookBin) || (hookBin && hookBin !== "gwtd" && !await exists(hookBin))) hookBin = "";
  if (!hookBin) hookBin = (await envelope("hook.doctor")).managed_hook_bin;
  if (!hookBin || disposable.test(hookBin)) throw new Error("No stable managed-hook fallback resolved");
  env.GWT_HOOK_BIN = hookBin;
  const hash = (await envelope("issue.monitor.status", {}, true)).project_store?.hash;
  if (!hash) throw new Error("Missing canonical project_store.hash");
  const preferences = join(home, ".gwt/projects", hash, "project-state");
  await mkdir(preferences, { recursive: true });
  await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ enabled: false, autonomous_mode: false, launch_auto: false,
    max_active_agents: 1, allowed_labels: [], priority_order: [481003], terminal_queue_auto_refill: false, auto_apply_updates: false,
    terminal_queues: { [hostname().trim().toLowerCase().replace(/(?:\.local)+$/, "") || "unknown-host"]: { entries: [{ number: 481003, queued_at: issues[2].updatedAt, queued_by: "fixture" }] } } }));
  const seededQueue = await envelope("issue.monitor.queue.list");
  if (!seededQueue.entries?.some(entry => entry.number === 481003)) throw new Error(`Trusted fixture missing from this terminal queue: ${JSON.stringify(seededQueue)}`);
  await writeFile(join(home, ".gwt/config.toml"), "auto_apply_updates = false\n");
  await writeFile(join(home, ".gwt/session.json"), JSON.stringify({ tabs: [{ id: "feedback-intake", title: "Feedback intake fixture", project_root: project, kind: "git" }], active_tab_id: "feedback-intake", recent_projects: [] }));
  await writeFile(join(project, ".codex/hooks.json"), "{}");
  async function audit(repair) {
    const params = { expected_hook_bin: hookBin, runtime_state_path: join(home, ".gwt/browser-check-missing-runtime-state.json") };
    if (repair) {
      await envelope("hook.doctor", { ...params, repair: true });
      await envelope("hook.register_codex_managed_hook_trust", { project_root: project, codex_config: join(home, ".codex/config.toml"), codex_hook_discovery: "both" });
    }
    const health = await envelope("hook.health", params);
    await writeFile(join(evidence, repair ? "hook-repair.json" : "hook-audit.json"), JSON.stringify(health, null, 2));
    const blocking = health.issues.filter(issue => !(hookBin === "gwtd" && /^managed hook binary missing: .* uses gwtd$/.test(issue)) && !/^managed hook failure: .*state=fail-open( |$)/.test(issue));
    if (health.status === "inactive" || blocking.length) throw new Error(`Hook convergence failed: ${JSON.stringify(health)}`);
  }
  await audit(true);
  daemon = start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }));
  await waitFor(async () => new RegExp(`^running pid=${daemon.child.pid} .*probe=ok`).test(await envelope("daemon.status")), daemon, 30_000, "Fresh daemon not ready");
  env.GWT_BROWSER_URL_FILE = join(home, "url");
  gui = start(gwt, ["--no-tray", "--no-open"], env, project);
  console.log(`Fresh gwt PID: ${gui.child.pid}; isolated HOME: ${home}`);
  let url;
  await waitFor(async () => { try { url = (await readFile(env.GWT_BROWSER_URL_FILE, "utf8")).trim(); return ["http:", "https:"].includes(new URL(url).protocol); } catch { return false; } }, gui, 90_000, "Fresh gwt did not publish its URL");
  await finish(start("curl", ["-fsS", "-I", url]), 15_000);
  if (/another tray-resident gwt instance is already running/i.test(gui.stdout + gui.stderr)) throw new Error("Fresh instance isolation failed");
  hub = new WebSocket(new URL("/ws", url).toString().replace(/^http/, "ws"));
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Fresh Hub keepalive timed out")), 10_000);
    hub.addEventListener("open", () => { clearTimeout(timer); hub.send(JSON.stringify({ kind: "frontend_ready" })); resolve(); }, { once: true });
    hub.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Fresh Hub keepalive failed")); }, { once: true });
  });
  await audit(false);
  await writeFile(join(home, "issue-4810-isolated.json"), JSON.stringify({ checkout: root, project, repo_hash: hash, url, gwtd, fake_gh: gh, raw: 481001, external: 481002, trusted: 481003 }));
  await writeFile(join(evidence, "launch.json"), JSON.stringify({ checkout: root, gwt, gwtd, gui_pid: gui.child.pid, home, project, url, hook_bin: hookBin }, null, 2));
  const version = (await readFile(join(root, "scripts/playwright-version.txt"), "utf8")).trim();
  const deps = process.env.GWT_PLAYWRIGHT_DEPS_DIR || join(tmpdir(), `gwt-playwright-${version}`);
  const playwright = join(deps, "node_modules/@playwright/test/cli.js");
  if (!await exists(playwright)) throw new Error(`Install pinned Playwright ${version} at ${deps}`);
  const testEnv = { ...env, HOME: process.env.HOME || homedir(), USERPROFILE: process.env.USERPROFILE || homedir(), NODE_PATH: join(deps, "node_modules"),
    GWT_PLAYWRIGHT_BASE_URL: url, GWT_PLAYWRIGHT_CHECK_HOME: home, GWT_PLAYWRIGHT_PROJECT_ROOT: project, GWT_PLAYWRIGHT_FEEDBACK_INTAKE: "1" };
  if (process.env.GWT_HEADED_E2E_REPORT) testEnv.GWT_HEADED_E2E_REPORT = process.env.GWT_HEADED_E2E_REPORT;
  const args = [playwright, "test", "--config", join(root, "crates/gwt/playwright/playwright.config.ts"), "issue-monitor-feedback-live.spec.ts",
    "--workers=1", "--retries=0", "--trace=on", "--output", join(evidence, "playwright"), ...process.argv.slice(2)];
  if (!args.includes("--headed")) args.push("--headed");
  await finish(start(process.execPath, args, testEnv, root, "", true), 600_000);
} catch (error) { console.error(error); process.exitCode = 1; }
finally { try { await cleanup(); } catch (error) { console.error(error); process.exitCode = 1; } }
