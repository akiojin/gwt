// Issue #5321: checkout assets, fresh HOME, real headed Chromium.
// Fixtures: [--suites queue,preview,kanban,rail] [--before] [Playwright arguments]
// Backend: --live --suites <candidates|labels|tiers|capacity> --project=chromium-<dark|light> [Playwright arguments]
// Both modes accept --skip-build after the checkout binaries are freshly built.
import { spawn } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const gwt = join(root, "target/debug/gwt");
const gwtd = join(root, "target/debug/gwtd");
const args = process.argv.slice(2);
const before = args.includes("--before");
const live = args.includes("--live");
const suiteFiles = live ? {
  candidates: "issue-monitor-candidate-pool-live.spec.ts", labels: "issue-monitor-labels-live.spec.ts",
  tiers: "issue-monitor-auto-tiers-live.spec.ts", capacity: "issue-monitor-capacity-live.spec.ts",
} : { queue: "issue-queue-kanban.spec.ts", preview: "issue-preview.spec.ts", kanban: "kanban.spec.ts", rail: "surface-rail.spec.ts" };
const playwrightArgs = [];
let requestedSuites;
for (let index = 0; index < args.length; index++) {
  const argument = args[index];
  if (["--skip-build", "--before", "--live"].includes(argument)) continue;
  if (argument === "--suites") requestedSuites = args[++index];
  else if (argument.startsWith("--suites=")) requestedSuites = argument.slice("--suites=".length);
  else playwrightArgs.push(argument);
}
const suites = requestedSuites?.split(",").map(value => value.trim()).filter(Boolean)
  ?? (before ? ["queue"] : Object.keys(suiteFiles));
const owned = [];
const exists = async path => access(path).then(() => true, () => false);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let evidence, home, gui, daemon, github, hub, cleanupPromise;

function start(file, arguments_, env = process.env, cwd = root, input = "", stream = false) {
  const child = spawn(file, arguments_, { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  const record = { child, file, stdout: "", stderr: "", finished: false };
  owned.push(record); // Record the exact owned PID before readiness and auditing.
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

async function cleanup() {
  cleanupPromise ??= (async () => {
    hub?.close();
    const failures = [];
    for (const record of [...owned].reverse()) {
      if (record.finished || !record.child.pid) continue;
      record.child.kill("SIGTERM");
      let timer;
      const stopped = await Promise.race([record.closed.then(() => true), new Promise(resolve => {
        timer = setTimeout(() => resolve(false), 10_000);
      })]);
      clearTimeout(timer);
      if (!stopped) {
        record.child.kill("SIGKILL");
        await Promise.race([record.closed, delay(10_000).then(() => failures.push(`Owned PID ${record.child.pid} remains`))]);
      }
    }
    if (home) for (const [name, record] of [["gwt", gui], ["gwtd", daemon], ["github-fixture", github]]) {
      if (!record) continue;
      // Canonical verification removes its child TMPDIR, including HOME.
      await writeFile(join(evidence, `${name}.stdout.log`), record.stdout);
      await writeFile(join(evidence, `${name}.stderr.log`), record.stderr);
    }
    if (evidence) {
      await writeFile(join(evidence, "cleanup.json"), JSON.stringify({ failures,
        processes: owned.map(record => ({ pid: record.child.pid, file: record.file, finished: record.finished })) }, null, 2));
      console.log(`Issue #5321 ${live ? "LIVE" : before ? "BEFORE" : "AFTER"} evidence retained: ${evidence}`);
    }
    if (failures.length) throw new Error(failures.join("\n"));
  })();
  return cleanupPromise;
}
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.once(signal, () => {
  void cleanup().finally(() => process.exit(1));
});

// Same paged gh/loopback transport as verify-issue-5242-closed.mjs, read-only.
// Unknown commands fail locally instead of falling through to installed gh.
const githubFixtureSource = String.raw`
const fs = require("node:fs"), http = require("node:http"), path = require("node:path");
const home = path.dirname(__dirname);
const issues = JSON.parse(fs.readFileSync(path.join(home, "github-issues.json"), "utf8"));
const connection = nodes => ({ nodes, totalCount: nodes.length, pageInfo: { hasNextPage: false, endCursor: null } });
const rest = issue => ({ ...issue, state: issue.state.toLowerCase(), updated_at: issue.updatedAt, html_url: issue.url });
const graph = issue => issue && ({ ...issue, labels: connection(issue.labels), comments: connection([]), timelineItems: connection([]) });
const graphql = number => ({ data: { repository: { issues: connection(issues.map(graph)),
  issue: graph(issues.find(issue => issue.number === Number(number))) },
  rateLimit: { cost: 1, remaining: 5000, resetAt: "2099-01-01T00:00:00Z", nodeCount: 1 } } });
function api(endpoint) {
  const url = new URL(endpoint, "https://fixture.invalid/");
  const match = url.pathname.match(/\/issues(?:\/(\d+))?(.*)$/);
  if (match && match[1]) {
    if (match[2]) return [];
    return rest(issues.find(issue => issue.number === Number(match[1])));
  }
  if (match) {
    const label = url.searchParams.get("labels");
    return Number(url.searchParams.get("page") || 1) === 1
      ? issues.filter(issue => !label || issue.labels.some(value => value.name === label)).map(rest) : [];
  }
  if (/\/(pulls|commits|events)(\?|$|\/)/.test(url.pathname)) return [];
  if (url.pathname === "/rate_limit") return { resources: {
    core: { limit: 5000, remaining: 5000, reset: 4102444800 }, graphql: { limit: 5000, remaining: 5000, reset: 4102444800 } } };
  throw new Error("Unsupported fixture endpoint: " + endpoint);
}
if (process.argv[2] === "--gh") {
  const args = process.argv.slice(3);
  fs.appendFileSync(path.join(home, "gh-argv.jsonl"), JSON.stringify(args) + "\n");
  let result;
  if (args[0] === "--version") { console.log("gh version 2.80.0"); process.exit(0); }
  if (args[0] === "auth") { if (args[1] === "token") console.log("fixture-only"); process.exit(0); }
  if (args[0] === "issue" && args[1] === "list") result = issues;
  else if (args[0] === "issue" && args[1] === "view") result = issues.find(issue => issue.number === Number(args[2]));
  else if (args[0] === "pr" && args[1] === "list") result = [];
  else if (args[0] === "repo" && args[1] === "view") result = { nameWithOwner: "fixture/issues-live", defaultBranchRef: { name: "fixture-main" } };
  else if (args[0] === "api") {
    if (args.includes("--method") || args.includes("-X")) throw new Error("Fixture gh mutations are disabled");
    result = args[1] === "graphql" ? graphql(args.find(value => value.startsWith("number="))?.slice(7)) : api(args[1]);
  } else throw new Error("Unsupported fixture gh: " + args.join(" "));
  if (args.includes("--include")) process.stdout.write("HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n");
  console.log(JSON.stringify(result));
} else {
  const server = http.createServer(async (request, response) => {
    try {
      let text = ""; for await (const chunk of request) text += chunk;
      const payload = text ? JSON.parse(text) : {};
      fs.appendFileSync(path.join(home, "github-requests.jsonl"), JSON.stringify({ method: request.method, url: request.url, payload }) + "\n");
      if (request.method !== "GET" && !(request.method === "POST" && request.url === "/graphql" && !/\bmutation\b/.test(payload.query || ""))) {
        throw new Error("Fixture GitHub mutations are disabled");
      }
      const result = request.url === "/graphql" ? graphql(payload.variables?.number) : api(request.url);
      response.writeHead(200, { "Content-Type": "application/json" }); response.end(JSON.stringify(result));
    } catch (error) {
      response.writeHead(500, { "Content-Type": "application/json" }); response.end(JSON.stringify({ message: String(error) }));
    }
  });
  server.listen(0, "127.0.0.1", () => fs.writeFileSync(path.join(home, "github-url"), "http://127.0.0.1:" + server.address().port));
}
`;

try {
  if (live && !playwrightArgs.some(value => value === "--project" || value.startsWith("--project="))) throw new Error("Live themes use separate fresh projects: add --project=chromium-dark or --project=chromium-light");
  if (live && suites.length !== 1) throw new Error("Run each live suite in its own fresh project: --live --suites candidates|labels|tiers|capacity");
  if (live && before) throw new Error("--before captures the queue fixture; it cannot be combined with --live");
  if (!suites.length || suites.some(suite => !suiteFiles[suite])) throw new Error(`Use --suites ${Object.keys(suiteFiles).join(",")}`);
  if (process.platform === "win32") throw new Error("This browser-check runner requires POSIX executables");
  if (!args.includes("--skip-build")) await finish(start("cargo", ["build", "-p", "gwt", "--bin", "gwt", "--bin", "gwtd"], process.env, root, "", true), 1_800_000);
  if (!await exists(gwt) || !await exists(gwtd)) throw new Error("Build this checkout's gwt and gwtd first.");
  evidence = await mkdtemp(join(root, `target/gwt-issue-5321-${live ? "live" : before ? "before" : "after"}-`));
  // Keep HOME outside the checkout: recursive tool globs must not follow its
  // runtime inputs or project binaries back through the evidence directory.
  home = await mkdtemp(join(tmpdir(), "gwt-issue-5321-home-"));
  const project = join(home, "project");
  await Promise.all([project, join(project, ".codex"), join(home, ".gwt"), join(home, ".codex")].map(path => mkdir(path, { recursive: true })));
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
    && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())));
  // No user credentials or provider state enter either disposable instance.
  Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
    GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_DISABLE_BACKGROUND_INDEX: "1" });
  const runtime = join(process.env.HOME || homedir(), ".gwt/runtime");
  if (await exists(runtime)) await symlink(runtime, join(home, ".gwt/runtime"));
  // A separate Git directory gives this fixture a .git marker file, so gwt
  // opens it directly without an unrelated Normal-layout migration drawer.
  await finish(start("git", ["init", "-q", "-b", "fixture-main", "--separate-git-dir", join(home, "project.git")], env, project), 10_000);
  await finish(start("git", ["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-q", "-m", "fixture"], env, project), 10_000);
  let fixtureIssues;
  if (live) {
    const bin = join(home, "bin");
    await mkdir(bin);
    const script = join(bin, "github.cjs");
    const gh = join(bin, "gh");
    await writeFile(script, githubFixtureSource);
    const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
    await writeFile(gh, `#!/bin/sh\nexec ${quote(process.execPath)} ${quote(script)} --gh "$@"\n`, { mode: 0o755 });
    fixtureIssues = [[532101, ["agent:fixture", "gwt-queued"]], [532102, ["agent:elsewhere"]], [532103, []]].map(([number, labels]) => ({
      number, title: `Isolated Issues settings fixture #${number}`, body: "Offline Issue Monitor admission fixture.",
      state: "OPEN", labels: labels.map(name => ({ name })), updatedAt: "2026-10-13T00:00:00Z",
      url: `https://github.com/fixture/issues-live/issues/${number}`, comments: [],
    }));
    await writeFile(join(home, "github-issues.json"), JSON.stringify(fixtureIssues));
    Object.assign(env, { PATH: bin + delimiter + (env.PATH || ""), GWT_TEST_GH: gh, GWT_TEST_GH_SANDBOX: "1" });
    await mkdir(join(project, "target/debug"), { recursive: true });
    await symlink(gwtd, join(project, "target/debug/gwtd"));
    // A bounded real build artifact exercises capacity measurement without
    // recursively linking the checkout's complete target tree.
    await writeFile(join(project, "target/fixture-output.bin"), Buffer.alloc(1024 * 1024));
    await finish(start("git", ["remote", "add", "origin", "https://github.com/fixture/issues-live.git"], env, project), 10_000);
    // Preserve a parseable repository identity while refusing every Git network transport.
    await finish(start("git", ["config", "protocol.allow", "never"], env, project), 10_000);
    github = start(process.execPath, [script], env, project);
    await waitFor(() => exists(join(home, "github-url")), github, 15_000, "Offline GitHub fixture not ready");
    const endpoint = (await readFile(join(home, "github-url"), "utf8")).trim();
    Object.assign(env, { GWT_OWNER_GITHUB_TEST_MODE: "loopback-v1", GWT_OWNER_GITHUB_REST_BASE: endpoint,
      GWT_OWNER_GITHUB_GRAPHQL_URL: `${endpoint}/graphql`, GWT_OWNER_GITHUB_TOKEN: "fixture-only" });
  }

  const disposable = /(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(?:debug|release)[/\\]gwtd(?:\.exe)?(?:[^a-z0-9_.-]|$)/i;
  let hookBin = process.env.GWT_HOOK_BIN || "";
  if (disposable.test(hookBin) || (hookBin && hookBin !== "gwtd" && !await exists(hookBin))) hookBin = "";
  if (!hookBin) {
    const candidate = (await finish(start("/bin/sh", ["-c", "command -v gwtd || true"]))).trim();
    if (candidate && !disposable.test(candidate)) hookBin = "gwtd";
  }
  if (!hookBin && await exists("/Applications/GWT.app/Contents/MacOS/gwtd")) hookBin = "/Applications/GWT.app/Contents/MacOS/gwtd";
  hookBin ||= "gwtd";
  env.GWT_HOOK_BIN = hookBin;
  async function envelope(operation, params = {}, outer = false) {
    const response = JSON.parse(await finish(start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation, params }))));
    if (!response.ok) throw new Error(`${operation} failed: ${JSON.stringify(response)}`);
    if (outer) return response;
    return ["daemon.status", "hook.register_codex_managed_hook_trust"].includes(operation) ? response.output : JSON.parse(response.output);
  }
  const hash = (await envelope("issue.monitor.status", {}, true)).project_store.hash;
  if (!hash) throw new Error("Missing canonical project_store.hash");
  const preferences = join(home, ".gwt/projects", hash, "project-state");
  await mkdir(preferences, { recursive: true });
  await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ enabled: false, autonomous_mode: false,
    max_active_agents: 1, launch_auto: true, priority_order: [], terminal_queue_auto_refill: false, auto_apply_updates: false }));
  await writeFile(join(home, ".gwt/config.toml"), "auto_apply_updates = false\n");
  await writeFile(join(home, ".gwt/session.json"), JSON.stringify({ tabs: [{ id: "queue-fixture", title: "Queue fixture", project_root: project, kind: "git" }], active_tab_id: "queue-fixture", recent_projects: [] }));
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
  await audit(true);
  daemon = start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }));
  await waitFor(async () => new RegExp(`^running pid=${daemon.child.pid} .*probe=ok`).test(await envelope("daemon.status")), daemon, 30_000, "Fresh daemon not ready");
  env.GWT_BROWSER_URL_FILE = join(home, "url");
  gui = start(gwt, ["--no-tray", "--no-open"], env, project);
  console.log(`Fresh gwt PID: ${gui.child.pid}; isolated HOME: ${home}`);
  let url;
  await waitFor(async () => {
    try {
      const candidate = (await readFile(env.GWT_BROWSER_URL_FILE, "utf8")).trim();
      if (!["http:", "https:"].includes(new URL(candidate).protocol)) return false;
      url = candidate;
      return true;
    } catch { return false; }
  }, gui, 90_000, "Fresh gwt did not publish a valid URL");
  await finish(start("curl", ["-fsS", "-I", url]), 15_000);
  if (/another tray-resident gwt instance is already running/i.test(gui.stdout + gui.stderr)) throw new Error("Fresh instance isolation failed");
  // Retain a real Hub connection between the separate dark/light contexts.
  hub = new WebSocket(new URL("/ws", url).toString().replace(/^http/, "ws"));
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Fresh browser session did not connect")), 10_000);
    hub.addEventListener("open", () => { clearTimeout(timer); hub.send(JSON.stringify({ kind: "frontend_ready" })); resolve(); }, { once: true });
    hub.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Fresh browser session failed")); }, { once: true });
  });
  await audit(false);
  if (live) {
    const marker = { project, repo_hash: hash, url, gwtd, checkout: root };
    for (const issue of [3620, 4774]) await writeFile(join(home, `issue-${issue}-isolated.json`), JSON.stringify(marker));
    await writeFile(join(home, "issue-4158-isolated.json"), JSON.stringify({ ...marker,
      allowed_label: "agent:fixture", expected_admitted_issues: [fixtureIssues[0].number],
      expected_excluded_issues: fixtureIssues.slice(1).map(issue => issue.number) }));
  }
  await writeFile(join(evidence, "launch.json"), JSON.stringify({ phase: before ? "before" : "after",
    checkout: root, branch: (await finish(start("git", ["branch", "--show-current"]))).trim(),
    sha: (await finish(start("git", ["rev-parse", "HEAD"]))).trim(),
    gwt, gwtd, gui_pid: gui.child.pid, home, project, url, hook_bin: hookBin, live, suites }, null, 2));
  const version = (await readFile(join(root, "scripts/playwright-version.txt"), "utf8")).trim();
  const deps = process.env.GWT_PLAYWRIGHT_DEPS_DIR || `/tmp/gwt-playwright-${version}`;
  const playwright = join(deps, "node_modules/@playwright/test/cli.js");
  if (!await exists(playwright)) throw new Error(`Playwright ${version} is required at ${deps}; run the existing Playwright bootstrap first`);
  const testEnv = { ...env, HOME: process.env.HOME || homedir(), USERPROFILE: process.env.USERPROFILE || homedir(),
    NODE_PATH: join(deps, "node_modules"), GWT_PLAYWRIGHT_BASE_URL: url,
    GWT_PLAYWRIGHT_CHECK_HOME: home, GWT_PLAYWRIGHT_PROJECT_ROOT: project, GWT_PLAYWRIGHT_CHECKOUT_ROOT: root,
    GWT_ISSUE_5321_PHASE: before ? "before" : "after", GWT_ISSUE_5321_EVIDENCE: evidence };
  if (process.env.GWT_HEADED_E2E_REPORT) testEnv.GWT_HEADED_E2E_REPORT = process.env.GWT_HEADED_E2E_REPORT;
  const testArgs = [playwright, "test", "--config", join(root, "crates/gwt/playwright/playwright.config.ts"),
    ...suites.map(suite => suiteFiles[suite]), "--output", join(evidence, "playwright"),
    ...(before && !playwrightArgs.some(argument => argument === "--grep" || argument.startsWith("--grep=")) ? ["--grep", "Issue #5321"] : []),
    ...(!playwrightArgs.some(argument => argument === "--trace" || argument.startsWith("--trace=")) ? ["--trace=on"] : []),
    ...playwrightArgs, "--workers=1"];
  if (!testArgs.includes("--headed")) testArgs.push("--headed");
  await finish(start(process.execPath, testArgs, testEnv, root, "", true), live ? 3_600_000 : 1_200_000);
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally { await cleanup(); }
