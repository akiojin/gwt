#!/usr/bin/env node
// Run after building this checkout and acquiring the verification lease.
// Provide a fresh HOME containing the session/workspace fixture to restore:
//   node scripts/measure-startup.mjs --home <isolated-home> --out <report.json>
// --expected-restored N rejects accidentally empty restore fixtures. Repeat
// Use --expect-terminal when a fresh PM starts without restored windows.
// Prepare runtime/credential symlinks and session.json per browser-check first;
// this script preserves the supplied restore fixture rather than reseeding it.
// with --theme light and dark. The browser is opened before the gwt process;
// browser installation/launch time is outside the gwt cold-start budget.
// GWT_PLAYWRIGHT_NODE_MODULES selects the pinned deps from run-visual-tests.sh.

import { spawn, spawnSync } from "node:child_process";
import { access, mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import { constants } from "node:fs";
import { createRequire } from "node:module";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { setTimeout as delay } from "node:timers/promises";

const { values } = parseArgs({ options: {
  home: { type: "string" }, out: { type: "string" }, project: { type: "string" },
  theme: { type: "string", default: "dark" }, "expected-restored": { type: "string" },
  "timeout-seconds": { type: "string", default: "60" },
  "expect-terminal": { type: "boolean", default: false },
} });
if (!values.home || !values.out || !["dark", "light"].includes(values.theme)) {
  throw new Error("Required: --home <isolated-home> --out <report.json> [--theme dark|light]");
}
const checkHome = path.resolve(values.home);
if (checkHome.toLowerCase() === path.resolve(homedir()).toLowerCase()) {
  throw new Error("The measurement HOME must be isolated from the user's HOME");
}
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const project = path.resolve(values.project ?? root);
const out = path.resolve(values.out);
const suffix = process.platform === "win32" ? ".exe" : "";
const gwt = path.join(root, "target", "debug", `gwt${suffix}`);
const gwtd = path.join(root, "target", "debug", `gwtd${suffix}`);
const require = createRequire(import.meta.url);
const { chromium } = require(process.env.GWT_PLAYWRIGHT_NODE_MODULES
  ? path.join(process.env.GWT_PLAYWRIGHT_NODE_MODULES, "playwright") : "playwright");
const timeoutMs = Number(values["timeout-seconds"]) * 1000;
if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw new Error("Invalid timeout");
await mkdir(path.dirname(out), { recursive: true });
await mkdir(path.join(checkHome, ".gwt"), { recursive: true });
if ((await realpath(checkHome)).toLowerCase() === (await realpath(homedir())).toLowerCase()) {
  throw new Error("Measurement HOME resolves to the user's HOME");
}
await access(gwt, constants.X_OK);
await access(gwtd, constants.X_OK);
const expectedCount = values["expected-restored"];
if (expectedCount !== undefined && (!Number.isSafeInteger(Number(expectedCount)) || Number(expectedCount) < 0)) {
  throw new Error("--expected-restored must be a non-negative integer");
}
const urlFile = path.join(checkHome, ".gwt", `startup-url-${Date.now()}.txt`);
const env = { ...process.env, HOME: checkHome, USERPROFILE: checkHome,
  GWT_BROWSER_URL_FILE: urlFile, GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1" };
const hookBin = env.GWT_HOOK_BIN;
env.GWT_HOOK_BIN = hookBin && !/[\\/]target[\\/]/i.test(hookBin) ? hookBin : "gwtd";
if (env.GWT_HOOK_BIN !== "gwtd") {
  try { await access(env.GWT_HOOK_BIN, constants.X_OK); } catch { env.GWT_HOOK_BIN = "gwtd"; }
}
for (const key of Object.keys(env)) {
  if (/^GWT_(BIN_PATH|REPO_HASH|WORKTREE_HASH|SESSION_|PANE_|HOOK_FORWARD_|AUTONOMOUS_)/.test(key)) delete env[key];
}
env.GWT_PROJECT_ROOT = project;

function operation(name, params, operationEnv = env, cwd = project) {
  const result = spawnSync(gwtd, [], { cwd, env: operationEnv, windowsHide: true, encoding: "utf8",
    timeout: 30_000, input: JSON.stringify({ schema_version: 1, operation: name, params }) });
  if (result.status !== 0) throw new Error(`${name}: ${result.error?.message || result.stderr || `exit ${result.status}`}`);
  let envelope;
  try { envelope = JSON.parse(result.stdout); } catch { throw new Error(`${name}: invalid JSON response`); }
  if (!envelope.ok) throw new Error(`${name}: ${JSON.stringify(envelope.error)}`);
  return typeof envelope.output === "string" ? JSON.parse(envelope.output) : envelope.output;
}

// Audit the launch checkout using the real config HOME, as browser-check does.
const auditEnv = { ...process.env, GWT_HOOK_BIN: env.GWT_HOOK_BIN };
delete auditEnv.GWT_BIN_PATH;
const hookParams = { expected_hook_bin: env.GWT_HOOK_BIN,
  runtime_state_path: path.join(checkHome, ".gwt", "browser-check-missing-runtime-state.json") };
const locator = spawnSync(process.platform === "win32" ? "where.exe" : "which", ["gwtd"],
  { env: auditEnv, windowsHide: true, encoding: "utf8" });
const allowMissingLogical = env.GWT_HOOK_BIN === "gwtd" && locator.status !== 0;
function assertHookHealth(health) {
  const issues = health?.issues?.filter((issue) => !(allowMissingLogical
    && issue.startsWith("managed hook binary missing: ") && issue.endsWith(" uses gwtd")));
  if (!health || health.status === "inactive" || !Array.isArray(issues) || issues.length) {
    throw new Error(`Hook convergence failed: ${JSON.stringify(health)}`);
  }
}
try {
  assertHookHealth(operation("hook.doctor", { ...hookParams, repair: true }, auditEnv, root).health);
} catch (error) {
  await writeFile(out, JSON.stringify({ passed: false, stage: "hook_preflight", errors: [error.message] }, null, 2) + "\n");
  throw error;
}

const browser = await chromium.launch({ headless: false,
  ...(process.env.GWT_PLAYWRIGHT_CHROMIUM_CHANNEL ? { channel: process.env.GWT_PLAYWRIGHT_CHROMIUM_CHANNEL } : {}) });
const page = await browser.newPage({ colorScheme: values.theme, viewport: { width: 1440, height: 1000 } });
await page.addInitScript((theme) => localStorage.setItem("gwt:ui:theme", theme), values.theme);
const errors = [];
page.on("pageerror", (error) => errors.push(error.message));
page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
const launchedAt = new Date().toISOString();
const child = spawn(gwt, ["--no-tray", "--no-open"], {
  cwd: project, env, windowsHide: true, detached: process.platform !== "win32",
  stdio: ["ignore", "pipe", "pipe"],
});
let log = "";
let launchError;
child.on("error", (error) => { launchError = error; });
child.stdout.on("data", (data) => { log += data; });
child.stderr.on("data", (data) => { log += data; });

try {
  const deadline = Date.now() + timeoutMs;
  let url;
  while (Date.now() < deadline) {
    if (launchError) throw launchError;
    if (child.exitCode !== null) throw new Error(`gwt exited ${child.exitCode}: ${log}`);
    try { url = (await readFile(urlFile, "utf8")).trim(); } catch { /* not published yet */ }
    if (url) break;
    await delay(25);
  }
  if (!url || !(await fetch(url, { method: "HEAD" })).ok) throw new Error("Fresh gwt URL was not ready");
  await page.goto(url, { waitUntil: "domcontentloaded", timeout: timeoutMs });
  let startup;
  while (Date.now() < deadline) {
    if (launchError) throw launchError;
    if (child.exitCode !== null) throw new Error(`gwt exited ${child.exitCode}`);
    startup = operation("perf.startup", {}).startup;
    if (startup && Date.parse(startup.process_started_at) < Date.parse(launchedAt)) startup = undefined;
    const phases = startup?.phases ?? [];
    if (phases.some((p) => p.phase === "first_frame") && phases.some((p) => p.phase === "shell_interactive")
      && phases.some((p) => p.phase === "restore_drain")
      && (!(values["expect-terminal"] || startup.restored_window_count > 0)
        || phases.some((p) => p.phase === "terminal_interactive"))) break;
    await delay(200);
  }
  const hasShell = startup?.phases.some((p) => p.phase === "shell_interactive");
  const hasTerminal = startup?.phases.some((p) => p.phase === "terminal_interactive");
  const countMatches = expectedCount === undefined || startup?.restored_window_count === Number(expectedCount);
  const hasDrain = startup?.phases.some((p) => p.phase === "restore_drain");
  const needsTerminal = values["expect-terminal"] || (startup?.restored_window_count ?? 0) > 0;
  if (!hasShell || !hasDrain || (needsTerminal && !hasTerminal)) errors.push("Startup readiness deadline expired: required milestones missing");
  if (!countMatches) errors.push(`Restored count mismatch: expected ${expectedCount}, observed ${startup?.restored_window_count}`);
  if (startup?.first_frame_within_budget === false) errors.push("First frame exceeded startup budget");
  assertHookHealth(operation("hook.health", hookParams, auditEnv, root));
  const passed = startup?.first_frame_within_budget === true && hasShell && hasDrain
    && (!needsTerminal || hasTerminal) && countMatches && errors.length === 0;
  await page.screenshot({ path: `${out}.png`, fullPage: true });
  const report = { passed, launched_at: launchedAt, checkout: root, project, home: checkHome,
    pid: child.pid, theme: values.theme, expected_restored: expectedCount ?? null, errors, startup };
  await writeFile(out, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify(report, null, 2));
  if (!passed) process.exitCode = 1;
} catch (error) {
  const report = { passed: false, launched_at: launchedAt, checkout: root, project, home: checkHome,
    pid: child.pid, theme: values.theme, errors: [...errors, error.message] };
  await writeFile(out, JSON.stringify(report, null, 2) + "\n");
  console.error(JSON.stringify(report, null, 2));
  process.exitCode = 1;
} finally {
  if (child.pid && child.exitCode === null) {
    if (process.platform === "win32") spawnSync("taskkill", ["/PID", String(child.pid), "/T", "/F"], { windowsHide: true });
    else { try { process.kill(-child.pid, "SIGTERM"); } catch { /* already stopped */ } }
  }
  await browser.close();
  await writeFile(`${out}.log`, log);
}
