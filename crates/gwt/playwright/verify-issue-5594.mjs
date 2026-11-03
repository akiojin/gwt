// Issue #5594: checkout assets, isolated HOME, real headed dark/light Chromium.
// Build first: cargo build -p gwt --bin gwt --bin gwtd
// Usage: node crates/gwt/playwright/verify-issue-5594.mjs [Playwright arguments]
import { spawn, spawnSync } from "node:child_process";
import { access, mkdir, mkdtemp, readFile, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const suffix = process.platform === "win32" ? ".exe" : "";
const gwt = join(root, `target/debug/gwt${suffix}`), gwtd = join(root, `target/debug/gwtd${suffix}`);
const owned = [];
const exists = path => access(path).then(() => true, () => false);
let evidence, home, gui, daemon, hub, cleaning;

function start(file, args, env, input = "", stream = false) {
  const child = spawn(file, args, { cwd: root, env, windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
  const record = { child, file, stdout: "", stderr: "", finished: false };
  owned.push(record); // Own the exact child PID before waiting for startup.
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

async function envelope(operation, params, env) {
  const result = JSON.parse(await finish(start(gwtd, [], env, JSON.stringify({ schema_version: 1, operation, params }))));
  if (!result.ok) throw new Error(`${operation} failed: ${JSON.stringify(result)}`);
  return result;
}
async function operation(name, params, env) {
  return JSON.parse((await envelope(name, params, env)).output);
}

// browser-check-process-cleanup-begin
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
      await writeFile(join(evidence, "cleanup.json"), JSON.stringify({ failures,
        processes: owned.map(({ child, file, finished }) => ({ pid: child.pid, file, finished })) }, null, 2));
      console.log(`Issue #5594 evidence retained: ${evidence}`);
    }
    if (failures.length) throw new Error(failures.join("\n"));
  })();
  return cleaning;
}
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.once(signal, () => {
  void cleanup().finally(() => process.exit(1));
});
// browser-check-process-cleanup-end

try {
  if (!await exists(gwt) || !await exists(gwtd)) throw new Error("Build this checkout first: cargo build -p gwt --bin gwt --bin gwtd");
  const checkout = (await finish(start("git", ["rev-parse", "--show-toplevel"], process.env))).trim();
  if (resolve(checkout) !== root) throw new Error("Unexpected launch checkout");
  evidence = await mkdtemp(join(root, "target/gwt-issue-5594-"));
  home = await mkdtemp(join(tmpdir(), "gwt-issue-5594-home-"));
  await mkdir(join(home, ".gwt"), { recursive: true });
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
    && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())));
  Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
    GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_DISABLE_BACKGROUND_INDEX: "1" });
  const runtime = join(process.env.HOME || homedir(), ".gwt/runtime");
  if (await exists(runtime)) await symlink(runtime, join(home, ".gwt/runtime"), process.platform === "win32" ? "junction" : "dir");
  const hash = (await envelope("issue.monitor.status", {}, env)).project_store?.hash;
  if (!hash) throw new Error("Missing canonical project_store.hash");
  const preferences = join(home, ".gwt/projects", hash, "project-state");
  await mkdir(preferences, { recursive: true });
  await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ enabled: false, autonomous_mode: false,
    max_active_agents: 1, priority_order: [], terminal_queue_auto_refill: false }));
  await writeFile(join(home, ".gwt/config.toml"), "auto_apply_updates = false\n");
  await writeFile(join(home, ".gwt/session.json"), JSON.stringify({ tabs: [{ id: "supported-agent-versions", title: "Supported agent versions", project_root: root, kind: "git" }], active_tab_id: "supported-agent-versions", recent_projects: [] }));

  // Keep shared managed hooks on their stable authority; the GUI alone uses
  // checkout binaries. Never persist a target/debug/gwtd fallback in hooks.
  const auditEnv = { ...process.env };
  delete auditEnv.GWT_BIN_PATH;
  const disposable = /(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(?:debug|release)[/\\]gwtd(?:\.exe)?(?:[^a-z0-9_.-]|$)/i;
  let hookBin = process.env.GWT_HOOK_BIN || "";
  if (disposable.test(hookBin) || (hookBin && hookBin !== "gwtd" && !await exists(hookBin))) hookBin = "";
  if (!hookBin) {
    delete auditEnv.GWT_HOOK_BIN;
    hookBin = (await operation("hook.doctor", {}, auditEnv)).managed_hook_bin;
  }
  if (!hookBin || disposable.test(hookBin)) throw new Error("No stable managed-hook fallback resolved");
  env.GWT_HOOK_BIN = auditEnv.GWT_HOOK_BIN = hookBin;
  const lookup = process.platform === "win32" ? ["where.exe", ["gwtd"]] : ["/bin/sh", ["-c", "command -v gwtd"]];
  const allowMissingLogical = hookBin === "gwtd" && spawnSync(...lookup, { env: auditEnv, windowsHide: true }).status !== 0;
  const hookParams = { expected_hook_bin: hookBin, runtime_state_path: join(home, ".gwt/browser-check-missing-runtime-state.json") };
  async function audit(repair) {
    const result = await operation(repair ? "hook.doctor" : "hook.health", repair ? { ...hookParams, repair: true } : hookParams, auditEnv);
    const health = repair ? result.health : result;
    await writeFile(join(evidence, repair ? "hook-repair.json" : "hook-audit.json"), JSON.stringify(health, null, 2));
    const blocking = health.issues.filter(issue => !(allowMissingLogical && /^managed hook binary missing: .* uses gwtd$/.test(issue))
      && !/^managed hook failure: .* state=fail-open( |$)/.test(issue));
    if (health.status === "inactive" || blocking.length) throw new Error(`Hook convergence failed: ${JSON.stringify(health)}`);
  }
  await audit(true);
  daemon = start(gwtd, [], env, JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }));
  await waitFor(async () => new RegExp(`^running pid=${daemon.child.pid} .*probe=ok`).test((await envelope("daemon.status", {}, env)).output), daemon, 30_000, "Fresh daemon not ready");
  env.GWT_BROWSER_URL_FILE = join(home, "url");
  gui = start(gwt, ["--no-tray", "--no-open"], env);
  console.log(`Fresh gwt PID: ${gui.child.pid}; isolated HOME: ${home}`);
  let url;
  await waitFor(async () => {
    try { url = (await readFile(env.GWT_BROWSER_URL_FILE, "utf8")).trim(); return ["http:", "https:"].includes(new URL(url).protocol); }
    catch { return false; }
  }, gui, 90_000, "Fresh gwt did not publish its URL");
  await finish(start("curl", ["-fsS", "-I", url], env), 15_000);
  if (/another tray-resident gwt instance is already running/i.test(gui.stdout + gui.stderr)) throw new Error("Fresh instance isolation failed");
  hub = new WebSocket(new URL("/ws", url).toString().replace(/^http/, "ws"));
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("Fresh Hub keepalive timed out")), 10_000);
    hub.addEventListener("open", () => { clearTimeout(timer); hub.send(JSON.stringify({ kind: "frontend_ready" })); resolve(); }, { once: true });
    hub.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Fresh Hub keepalive failed")); }, { once: true });
  });
  await audit(false);
  await writeFile(join(evidence, "launch.json"), JSON.stringify({ checkout: root, gwt, gwtd, gui_pid: gui.child.pid, home, url, hook_bin: hookBin,
    branch: (await finish(start("git", ["branch", "--show-current"], process.env))).trim(),
    sha: (await finish(start("git", ["rev-parse", "HEAD"], process.env))).trim() }, null, 2));
  const version = (await readFile(join(root, "scripts/playwright-version.txt"), "utf8")).trim();
  const cache = process.platform === "win32" ? join(process.env.LOCALAPPDATA ?? join(homedir(), "AppData/Local"), "Temp") : tmpdir();
  const deps = process.env.GWT_PLAYWRIGHT_NODE_MODULES ?? join(process.env.GWT_PLAYWRIGHT_DEPS_DIR ?? join(cache, `gwt-playwright-${version}`), "node_modules");
  const cli = join(deps, "@playwright/test/cli.js");
  if (!await exists(cli)) throw new Error(`Install pinned Playwright ${version} at ${deps}`);
  const testEnv = { ...process.env, NODE_PATH: deps, GWT_PLAYWRIGHT_BASE_URL: url,
    GWT_PLAYWRIGHT_CHECKOUT_ROOT: root, GWT_PLAYWRIGHT_CHECK_HOME: home, GWT_PLAYWRIGHT_PROJECT_ROOT: root };
  const args = [cli, "test", "--config", join(root, "crates/gwt/playwright/playwright.config.ts"), "settings-supported-agents-versions.spec.ts",
    "--output", join(evidence, "playwright"), "--trace=on", "--workers=1", "--retries=0", ...process.argv.slice(2)];
  if (!args.includes("--headed")) args.push("--headed");
  await finish(start(process.execPath, args, testEnv, "", true), 600_000);
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  try { await cleanup(); } catch (error) { console.error(error); process.exitCode = 1; }
}
