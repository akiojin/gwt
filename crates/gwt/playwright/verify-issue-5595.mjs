// Issue #5595: checkout assets, isolated HOME, real headed dark/light Chromium.
// Build first: cargo build -p gwt --bin gwt --bin gwtd
// Usage: node crates/gwt/playwright/verify-issue-5595.mjs [Playwright arguments]
import { spawn } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const suffix = process.platform === "win32" ? ".exe" : "";
const gwt = join(root, `target/debug/gwt${suffix}`), gwtd = join(root, `target/debug/gwtd${suffix}`);
const owned = [];
const exists = path => access(path).then(() => true, () => false);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let evidence, home, gui, daemon, hub, cleanupPromise;

function start(file, args, env = process.env, cwd = root, input = "", stream = false) {
  const child = spawn(file, args, { cwd, env, windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
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

async function finish(record, timeout = 60_000) {
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

// browser-check-process-cleanup-begin
async function cleanup() {
  cleanupPromise ??= (async () => {
    hub?.close();
    const failures = [];
    for (const record of [...owned].reverse()) {
      if (record.finished || !record.child.pid) continue;
      record.child.kill("SIGTERM");
      const stopped = await Promise.race([record.closed.then(() => true), delay(10_000).then(() => false)]);
      if (!stopped) {
        record.child.kill("SIGKILL");
        await Promise.race([record.closed, delay(10_000).then(() => failures.push(`Owned PID ${record.child.pid} remains`))]);
      }
    }
    if (evidence) {
      for (const [name, record] of [["gwt", gui], ["gwtd", daemon]]) if (record) {
        await writeFile(join(evidence, `${name}.stdout.log`), record.stdout);
        await writeFile(join(evidence, `${name}.stderr.log`), record.stderr);
      }
      await writeFile(join(evidence, "cleanup.json"), JSON.stringify({ failures,
        processes: owned.map(record => ({ pid: record.child.pid, file: record.file, finished: record.finished })) }, null, 2));
      console.log(`Issue #5595 evidence retained: ${evidence}`);
    }
    if (failures.length) throw new Error(failures.join("\n"));
  })();
  return cleanupPromise;
}
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.once(signal, () => {
  void cleanup().finally(() => process.exit(1));
});
// browser-check-process-cleanup-end

try {
  if (!await exists(gwt) || !await exists(gwtd)) throw new Error("Build this checkout first: cargo build -p gwt --bin gwt --bin gwtd");
  if (resolve((await finish(start("git", ["rev-parse", "--show-toplevel"]))).trim()) !== root) throw new Error("Unexpected launch checkout");
  evidence = await mkdtemp(join(root, "target/gwt-issue-5595-"));
  home = await mkdtemp(join(tmpdir(), "gwt-issue-5595-home-"));
  const project = join(home, "project");
  await Promise.all([project, join(project, ".codex"), join(home, ".gwt"), join(home, ".codex")].map(path => mkdir(path, { recursive: true })));
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
    && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())));
  Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
    GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_DISABLE_BACKGROUND_INDEX: "1" });
  const runtime = join(process.env.HOME || homedir(), ".gwt/runtime");
  if (await exists(runtime)) await symlink(runtime, join(home, ".gwt/runtime"), process.platform === "win32" ? "junction" : "dir");
  await finish(start("git", ["init", "-q", "-b", "fixture-main", "--separate-git-dir", join(home, "project.git")], env, project));
  await finish(start("git", ["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-q", "-m", "fixture"], env, project));
  async function envelope(operation, params = {}, outer = false) {
    const response = JSON.parse(await finish(start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation, params }))));
    if (!response.ok) throw new Error(`${operation} failed: ${JSON.stringify(response)}`);
    if (outer) return response;
    return ["daemon.status", "hook.register_codex_managed_hook_trust"].includes(operation) ? response.output : JSON.parse(response.output);
  }
  // browser-check-hook-authority-begin
  const disposable = /(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(?:debug|release)[/\\]gwtd(?:\.exe)?(?:[^a-z0-9_.-]|$)/i;
  let hookBin = process.env.GWT_HOOK_BIN || "";
  if (disposable.test(hookBin) || (hookBin && hookBin !== "gwtd" && !await exists(hookBin))) hookBin = "";
  if (!hookBin) hookBin = (await envelope("hook.doctor")).managed_hook_bin;
  if (!hookBin || disposable.test(hookBin)) throw new Error("No stable managed-hook fallback resolved");
  env.GWT_HOOK_BIN = hookBin;
  // browser-check-hook-authority-end
  const hash = (await envelope("issue.monitor.status", {}, true)).project_store.hash;
  if (!hash) throw new Error("Missing canonical project_store.hash");
  const preferences = join(home, ".gwt/projects", hash, "project-state");
  await mkdir(preferences, { recursive: true });
  await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ enabled: false, autonomous_mode: false, priority_order: [], terminal_queue_auto_refill: false }));
  await writeFile(join(home, ".gwt/config.toml"), "auto_apply_updates = false\n");
  await writeFile(join(home, ".gwt/session.json"), JSON.stringify({ tabs: [{ id: "surface-fixture", title: "Surface fixture", project_root: project, kind: "git" }], active_tab_id: "surface-fixture", recent_projects: [] }));
  await writeFile(join(project, ".codex/hooks.json"), "{}");
  async function audit(repair) {
    const params = { expected_hook_bin: hookBin, runtime_state_path: join(home, ".gwt/browser-check-missing-runtime-state.json") };
    if (repair) {
      await envelope("hook.doctor", { ...params, repair: true });
      await envelope("hook.register_codex_managed_hook_trust", { project_root: project, codex_config: join(home, ".codex/config.toml"), codex_hook_discovery: "both" });
    }
    const health = await envelope("hook.health", params);
    await writeFile(join(evidence, repair ? "hook-repair.json" : "hook-audit.json"), JSON.stringify(health, null, 2));
    const blocking = health.issues.filter(issue => !/^managed hook failure: .*state=fail-open( |$)/.test(issue));
    if (health.status === "inactive" || blocking.length) throw new Error(`Hook convergence failed: ${JSON.stringify(health)}`);
  }
  // browser-check-hook-repair-begin
  await audit(true);
  // browser-check-hook-repair-end
  daemon = start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }));
  await waitFor(async () => new RegExp(`^running pid=${daemon.child.pid} .*probe=ok`).test(await envelope("daemon.status")), daemon, 30_000, "Fresh daemon not ready");
  env.GWT_BROWSER_URL_FILE = join(home, "url");
  // browser-check-launch-begin
  gui = start(gwt, ["--no-tray", "--no-open"], env, project);
  console.log(`Fresh gwt PID: ${gui.child.pid}; isolated HOME: ${home}`);
  // browser-check-launch-end
  let url;
  await waitFor(async () => {
    try { url = (await readFile(env.GWT_BROWSER_URL_FILE, "utf8")).trim(); return ["http:", "https:"].includes(new URL(url).protocol); }
    catch { return false; }
  }, gui, 90_000, "Fresh gwt did not publish its URL");
  await finish(start("curl", ["-fsS", "-I", url]), 15_000);
  if (/another tray-resident gwt instance is already running/i.test(gui.stdout + gui.stderr)) throw new Error("Fresh instance isolation failed");
  hub = new WebSocket(new URL("/ws", url).toString().replace(/^http/, "ws"));
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Fresh Hub keepalive timed out")), 10_000);
    hub.addEventListener("open", () => { clearTimeout(timer); hub.send(JSON.stringify({ kind: "frontend_ready" })); resolve(); }, { once: true });
    hub.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Fresh Hub keepalive failed")); }, { once: true });
  });
  // browser-check-hook-audit-begin
  await audit(false);
  // browser-check-hook-audit-end
  await writeFile(join(evidence, "launch.json"), JSON.stringify({ checkout: root, gwt, gwtd, gui_pid: gui.child.pid, home, project, url, hook_bin: hookBin,
    branch: (await finish(start("git", ["branch", "--show-current"]))).trim(),
    sha: (await finish(start("git", ["rev-parse", "HEAD"]))).trim() }, null, 2));
  const version = (await readFile(join(root, "scripts/playwright-version.txt"), "utf8")).trim();
  const deps = process.env.GWT_PLAYWRIGHT_DEPS_DIR || join(tmpdir(), `gwt-playwright-${version}`);
  const playwright = join(deps, "node_modules/@playwright/test/cli.js");
  if (!await exists(playwright)) throw new Error(`Install pinned Playwright ${version} at ${deps}`);
  const testEnv = { ...process.env, NODE_PATH: join(deps, "node_modules"), GWT_PLAYWRIGHT_BASE_URL: url,
    GWT_PLAYWRIGHT_CHECKOUT_ROOT: root, GWT_PLAYWRIGHT_CHECK_HOME: home, GWT_PLAYWRIGHT_PROJECT_ROOT: project };
  const forwarded = process.argv.slice(2);
  const completeSpecs = ["surface-rail.spec.ts", "agent-focus-cycle.spec.ts", "agent-window-scroll.spec.ts", "terminal-input-disconnect.spec.ts",
    "file-drop-attachments.spec.ts", "window-interrupted-state.spec.ts", "project-open-surfaces.spec.ts", "agent-title-badge.spec.ts"];
  const testArgs = [playwright, "test", "--config", join(root, "crates/gwt/playwright/playwright.config.ts"),
    ...completeSpecs, "issue-preview.spec.ts", "kanban.spec.ts", "issue-queue-kanban.spec.ts",
    ...(!forwarded.some(argument => argument === "--grep" || argument.startsWith("--grep="))
      ? ["--grep", completeSpecs.map(name => name.replaceAll(".", "\\.")).join("|")
        + "|Windowize opens the mirrored agent in Agents|Windowize from the split view opens Agents"
        + "|Issue #5395:|Issue auto refresh reads only|an unfocused Issue window"] : []),
    "--output", join(evidence, "playwright"), "--trace=on", "--workers=1", "--retries=0", ...forwarded];
  if (!testArgs.includes("--headed")) testArgs.push("--headed");
  // Probe before restart without the canonical reporter: only the complete
  // post-restart matrix writes GWT_HEADED_E2E_REPORT, exactly once.
  const beforeArgs = [];
  for (let index = 0; index < testArgs.length; index += 1) {
    if (["--grep", "--output", "--reporter"].includes(testArgs[index])) { index += 1; continue; }
    if (/^--(?:grep|output|reporter)=/.test(testArgs[index])) continue;
    beforeArgs.push(testArgs[index]);
  }
  beforeArgs.push("--grep", "fixed .*has no destructive canvas chrome", "--output", join(evidence, "before-restart-playwright"));
  const beforeEnv = { ...testEnv };
  delete beforeEnv.GWT_HEADED_E2E_REPORT;
  await finish(start(process.execPath, beforeArgs, beforeEnv, root, "", true), 180_000);
  // Issue #4777 AC15: no old chrome may return after a real GUI restart.
  const previousPid = gui.child.pid;
  hub.close();
  gui.child.kill("SIGTERM");
  await Promise.race([gui.closed, delay(10_000).then(() => {
    throw new Error(`Owned GUI PID ${previousPid} did not stop before restart`);
  })]);
  await writeFile(join(evidence, "gwt-before-restart.stdout.log"), gui.stdout);
  await writeFile(join(evidence, "gwt-before-restart.stderr.log"), gui.stderr);
  await writeFile(env.GWT_BROWSER_URL_FILE, "");
  gui = start(gwt, ["--no-tray", "--no-open"], env, project);
  console.log(`Restarted fresh gwt PID: ${gui.child.pid}; previous PID: ${previousPid}`);
  await waitFor(async () => {
    try { url = (await readFile(env.GWT_BROWSER_URL_FILE, "utf8")).trim(); return ["http:", "https:"].includes(new URL(url).protocol); }
    catch { return false; }
  }, gui, 90_000, "Restarted gwt did not publish its URL");
  await finish(start("curl", ["-fsS", "-I", url]), 15_000);
  await audit(false);
  if (/another tray-resident gwt instance is already running/i.test(gui.stdout + gui.stderr)) throw new Error("Restarted instance isolation failed");
  hub = new WebSocket(new URL("/ws", url).toString().replace(/^http/, "ws"));
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Restarted Hub keepalive timed out")), 10_000);
    hub.addEventListener("open", () => { clearTimeout(timer); hub.send(JSON.stringify({ kind: "frontend_ready" })); resolve(); }, { once: true });
    hub.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Restarted Hub keepalive failed")); }, { once: true });
  });
  testEnv.GWT_PLAYWRIGHT_BASE_URL = url;
  await writeFile(join(evidence, "restart.json"), JSON.stringify({ previous_pid: previousPid,
    previous_finished: owned.find(record => record.child.pid === previousPid).finished,
    gui_pid: gui.child.pid, home, url, gwt }, null, 2));
  await finish(start(process.execPath, testArgs, testEnv, root, "", true), 1_800_000);
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  try { await cleanup(); } catch (error) { console.error(error); process.exitCode = 1; }
}
