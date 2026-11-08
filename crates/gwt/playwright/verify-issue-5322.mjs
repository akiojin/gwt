// Issue #5322: isolated checkout servers, two independent projects, headed Chromium.
// Build first: cargo build -p gwt --bin gwt --bin gwtd
// Usage: node crates/gwt/playwright/verify-issue-5322.mjs [Playwright arguments]
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { access, mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const gwt = join(root, "target/debug/gwt"), gwtd = join(root, "target/debug/gwtd");
const owned = [], backends = [];
const exists = async path => access(path).then(() => true, () => false);
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
let evidence, cleanupPromise;

function start(file, args, env = process.env, cwd = root, input = "", stream = false) {
  const child = spawn(file, args, { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
  const record = { child, file, stdout: "", stderr: "", finished: false };
  owned.push(record); // Record this exact child immediately; never stop by process name.
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
      for (const [name, record] of [["gwt", backend.gui], ["gwtd", backend.daemon]]) {
        if (!record) continue;
        await writeFile(join(backend.home, `${name}.stdout.log`), record.stdout);
        await writeFile(join(backend.home, `${name}.stderr.log`), record.stderr);
      }
    }
    if (evidence) {
      await writeFile(join(evidence, "cleanup.json"), JSON.stringify({ failures,
        processes: owned.map(record => ({ pid: record.child.pid, file: record.file, finished: record.finished, signal: record.signal })),
      }, null, 2));
      console.log(`Issue #5322 evidence retained: ${evidence}`);
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
  if (process.platform === "win32") throw new Error("This browser-check owner requires POSIX processes.");
  if (!await exists(gwt) || !await exists(gwtd)) throw new Error("Build this checkout first: cargo build -p gwt --bin gwt --bin gwtd");
  const checkout = (await finish(start("git", ["rev-parse", "--show-toplevel"]))).trim();
  if (checkout !== root) throw new Error(`Unexpected launch checkout: ${checkout}`);
  const branch = (await finish(start("git", ["branch", "--show-current"]))).trim();
  // Canonical verify.run removes its TMPDIR; retain screenshots and owner logs.
  evidence = await mkdtemp(join(root, "target/gwt-issue-5322-"));

  // browser-check-hook-authority-begin
  const disposable = /(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(?:debug|release)[/\\]gwtd(?:\.exe)?(?:[^a-z0-9_.-]|$)/i;
  let hookBin = process.env.GWT_HOOK_BIN || "";
  if (disposable.test(hookBin) || (hookBin && hookBin !== "gwtd" && !await exists(hookBin))) hookBin = "";
  const pathHook = (await finish(start("/bin/sh", ["-c", "command -v gwtd || true"]))).trim();
  if (!hookBin && pathHook && !disposable.test(pathHook)) hookBin = "gwtd";
  if (!hookBin && await exists("/Applications/GWT.app/Contents/MacOS/gwtd")) hookBin = "/Applications/GWT.app/Contents/MacOS/gwtd";
  hookBin ||= "gwtd";
  const allowMissingLogical = hookBin === "gwtd" && !pathHook;
  // browser-check-hook-authority-end
  await finish(start("/bin/sh", ["-c", "command -v jq >/dev/null 2>&1 || { echo 'browser-check preflight failed: jq is required to parse hook.doctor evidence; install jq (for example: brew install jq) and rerun' >&2; exit 1; }"]));

  for (const theme of ["dark", "light"]) {
    const home = await mkdtemp(join(evidence, `${theme}-home-`));
    const backend = { theme, home, projects: [] };
    backends.push(backend);
    await mkdir(join(home, ".gwt"), { recursive: true });
    await mkdir(join(home, ".codex"), { recursive: true });
    // Coordinator and runtime writes must remain in this fixture's HOME.
    await mkdir(join(home, ".gwt/runtime"), { recursive: true });
    const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
      && !["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())));
    Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
      GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_HOOK_BIN: hookBin, GWT_DISABLE_BACKGROUND_INDEX: "1" });
    // No provider credentials or real project state are needed by Settings.
    await writeFile(join(home, ".gwt/config.toml"), "auto_apply_updates = false\n");

    for (const name of ["A", "B"]) {
      const project = join(home, `project-${name}`);
      await mkdir(join(project, ".codex"), { recursive: true });
      await writeFile(join(project, "compose.yaml"), "services:\n  fixture-service:\n    image: busybox:latest\n");
      await finish(start("git", ["init", "-q", "-b", "fixture-main"], env, project));
      await finish(start("git", ["-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-q", "-m", "fixture"], env, project));
      async function envelope(operation, params = {}) {
        const result = JSON.parse(await finish(start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation, params }))));
        if (!result.ok) throw new Error(`${operation} failed: ${JSON.stringify(result)}`);
        return result;
      }
      const hash = (await envelope("issue.monitor.status")).project_store.hash;
      if (!hash) throw new Error("Missing canonical project_store.hash");
      // browser-check-agent-seed-begin
      const preferences = join(home, ".gwt/projects", hash, "project-state");
      await mkdir(preferences, { recursive: true });
      await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
      const profiles = [{ agent_id: "codex", model: name === "A" ? "gpt-6-astra" : "project-b-model", reasoning: "high",
        version: "1.2.3", session_mode: "Normal", runtime_target: "Host", docker_lifecycle_intent: "Connect",
        skip_permissions: false, fast_mode: false, prefer_for: ["type:fix"] },
      { agent_id: "claude", model: "sonnet", reasoning: "medium", session_mode: "Normal", runtime_target: "Host" }];
      const members = profiles.map(profile => {
        profile.agent_member_id = randomUUID();
        return { agent_member_id: profile.agent_member_id, agent_id: profile.agent_id,
          name: `Settings ${name} ${profile.agent_id}`, enabled: true, created_at: new Date().toISOString() };
      });
      const preferenceFile = join(preferences, "issue-monitor.json");
      await writeFile(preferenceFile, JSON.stringify({ enabled: false, autonomous_mode: false,
        max_active_agents: members.length, max_active_agents_mode: "manual", priority_order: [],
        agent_roster: { schema_version: 1, revision: 1, members },
        terminal_queue_auto_refill: false, terminal_queue_auto_refill_limit: 3,
        allowed_labels: name === "A" ? ["auto-merge"] : ["project-b"], launch_profiles: profiles, launch_profile: profiles[0] }));
      // browser-check-agent-seed-end
      backend.projects.push({ root: project, preferences: preferenceFile, hash });
      await writeFile(join(project, ".codex/hooks.json"), "{}");
      async function audit(repair) {
        const params = { expected_hook_bin: hookBin, runtime_state_path: join(home, ".gwt/browser-check-missing-runtime-state.json") };
        const operation = repair ? "hook.doctor" : "hook.health";
        const result = await envelope(operation, repair ? { ...params, repair: true } : params);
        let health = JSON.parse(result.output);
        if (repair) {
          await writeFile(join(home, `project-${name}-hook-doctor.json`), JSON.stringify(health, null, 2));
          // This fixture creates Codex hooks in a fresh config, so register
          // those exact generated commands before enforcing health.
          await envelope("hook.register_codex_managed_hook_trust", {
            project_root: project, codex_config: join(home, ".codex/config.toml"), codex_hook_discovery: "both",
          });
          health = JSON.parse((await envelope("hook.health", params)).output);
        }
        await writeFile(join(home, `project-${name}-${repair ? "hook-repair" : "hook-audit"}.json`), JSON.stringify(health, null, 2));
        const blocking = health.issues.filter(issue => !(allowMissingLogical && /^managed hook binary missing: .* uses gwtd$/.test(issue))
          && !/^managed hook failure: .* state=fail-open( |$)/.test(issue));
        if (health.status === "inactive" || blocking.length) throw new Error(`Hook convergence failed: ${JSON.stringify(health)}`);
      }
      // browser-check-hook-repair-begin
      await audit(true);
      // browser-check-hook-repair-end
      backend.projects.at(-1).audit = audit;
    }
    const project = backend.projects[0].root;
    env.GWT_PROJECT_ROOT = project;
    await writeFile(join(home, ".gwt/session.json"), JSON.stringify({
      tabs: backend.projects.map((project, index) => ({ id: `project-${index}`, title: `Settings project ${index ? "B" : "A"}`, project_root: project.root, kind: "git" })),
      active_tab_id: "project-0", recent_projects: [],
    }));
    backend.daemon = start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }));
    await waitFor(async () => {
      const result = JSON.parse(await finish(start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation: "daemon.status", params: {} }))));
      return result.ok && new RegExp(`^running pid=${backend.daemon.child.pid} .*probe=ok`).test(result.output);
    }, backend.daemon, 30_000, "Isolated daemon did not become ready");
    env.GWT_BROWSER_URL_FILE = join(home, "url");
    // browser-check-launch-begin
    backend.gui = start(gwt, ["--no-tray", "--no-open"], env, project);
    console.log(`Fresh ${theme} gwt PID: ${backend.gui.child.pid}; HOME: ${home}`);
    // browser-check-launch-end
    await waitFor(async () => (await readFile(env.GWT_BROWSER_URL_FILE, "utf8").catch(() => "")).trim(), backend.gui, 90_000, "Fresh checkout did not publish its URL");
    backend.url = (await readFile(env.GWT_BROWSER_URL_FILE, "utf8")).trim();
    await finish(start("curl", ["-fsS", "-I", backend.url], env));
    if (/another tray-resident gwt instance is already running/i.test(backend.gui.stdout + backend.gui.stderr)) throw new Error("Fresh instance isolation failed");
    // Keep this fresh Hub alive while the browser reloads its project pages.
    backend.hub = new WebSocket(new URL("/ws", backend.url).toString().replace(/^http/, "ws"));
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("Hub keepalive failed")), 10_000);
      backend.hub.addEventListener("open", () => { clearTimeout(timer); backend.hub.send(JSON.stringify({ kind: "frontend_ready" })); resolve(); }, { once: true });
      backend.hub.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Hub keepalive failed")); }, { once: true });
    });
    // browser-check-hook-audit-begin
    for (const project of backend.projects) await project.audit(false);
    // browser-check-hook-audit-end
  }
  const metadata = Object.fromEntries(backends.map(backend => [backend.theme, {
    url: backend.url, home: backend.home, pid: backend.gui.child.pid,
    projects: backend.projects.map(({ root, preferences, hash }) => ({ root, preferences, hash })),
  }]));
  await writeFile(join(evidence, "isolated-backends.json"), JSON.stringify({ checkout: root, branch, gwt, gwtd, hook_bin: hookBin,
    fixture: { migration_drawer: "Closed through its frontend renderer only; backend migration_pending is preserved, and no worktree, Start Work or Launch action runs." },
    backends: metadata }, null, 2));
  const testEnv = { ...process.env, GWT_ISSUE_5322_BACKENDS: JSON.stringify(metadata),
    GWT_L3_SCREENSHOT_DIR: evidence, GWT_PLAYWRIGHT_CHECKOUT_ROOT: root };
  const args = [join(root, "scripts/run-visual-tests.sh"), "settings-issues-live.spec.ts", "--workers=1", "--retries=0",
    "--trace=on", "--output", join(evidence, "playwright"), ...process.argv.slice(2)];
  if (!args.includes("--headed")) args.push("--headed");
  await finish(start("bash", args, testEnv, root, "", true), 600_000);
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  try { await cleanup(); } catch (error) { console.error(error); process.exitCode = 1; }
}
