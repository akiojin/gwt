#!/usr/bin/env node
// Issue #5502: Windows fresh-home PM Reports E2E and an idle worker CPU sample.
// Build this checkout first. Arguments after -- are forwarded to Playwright.
import { spawn, spawnSync } from 'node:child_process';
import { access, mkdir, mkdtemp, readFile, readdir, rename, stat, symlink, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { parseArgs } from 'node:util';

const separator = process.argv.indexOf('--');
const { values } = parseArgs({ args: process.argv.slice(2, separator < 0 ? undefined : separator), options: {
  label: { type: 'string', default: 'after' }, out: { type: 'string' },
  'measure-only': { type: 'boolean', default: false },
  spec: { type: 'string', multiple: true },
} });
const playwrightArgs = separator < 0 ? [] : process.argv.slice(separator + 1);
if (process.platform !== 'win32') throw new Error('This worker CPU harness requires Windows.');
if (!/^[a-zA-Z0-9_-]+$/.test(values.label)) throw new Error('--label must be a filename-safe label.');
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const gwt = path.join(root, 'target/debug/gwt.exe');
const gwtd = path.join(root, 'target/debug/gwtd.exe');
await Promise.all([access(gwt), access(gwtd)]);
const out = path.resolve(values.out ?? path.join(root, `target/issue-5502/${values.label}.json`));
await mkdir(path.dirname(out), { recursive: true });
const checkHome = await mkdtemp(path.join(tmpdir(), 'gwt-issue-5502-'));
const stateHome = path.join(checkHome, '.gwt');
await mkdir(stateHome);
try { await access(path.join(homedir(), '.gwt/runtime')); await symlink(path.join(homedir(), '.gwt/runtime'), path.join(stateHome, 'runtime'), 'junction'); } catch (error) { if (error.code !== 'ENOENT') throw error; }
const timestamp = '2026-10-11T00:00:00Z';
const session = 'pm-reports-fixture';
const fixture = { prefs_bytes: 524 * 1024, runtime_directories: 691, runtime_sidecars: 4047 };
let diagnosticBytes;
const childEnv = { ...process.env };
for (const key of Object.keys(childEnv)) if (/^(GWT_|CODEX_|CLAUDE_|GH_TOKEN$|GITHUB_TOKEN$|OPENAI_API_KEY$|ANTHROPIC_API_KEY$)/.test(key)) delete childEnv[key];
Object.assign(childEnv, { HOME: checkHome, USERPROFILE: checkHome, GIT_TERMINAL_PROMPT: '0', GH_PROMPT_DISABLED: '1' });
// Measure the Reports route independently of index provisioning and other
// frontend pollers. The subsequent E2E uses ordinary unfiltered connections.
childEnv.GWT_DISABLE_BACKGROUND_INDEX = '1';
childEnv.PIP_CACHE_DIR = path.join(checkHome, 'pip-cache');

function envelope(operation, params = {}, env = process.env) {
  const result = spawnSync(gwtd, [], { cwd: root, env, windowsHide: true, encoding: 'utf8', timeout: 60_000,
    input: JSON.stringify({ schema_version: 1, operation, params }) });
  if (result.status !== 0) throw new Error(`${operation}: ${result.error?.message ?? result.stderr ?? result.status}`);
  const response = JSON.parse(result.stdout);
  if (!response.ok) throw new Error(`${operation}: ${JSON.stringify(response.error)}`);
  return response;
}
function operation(name, params, env) { const response = envelope(name, params, env); return typeof response.output === 'string' ? JSON.parse(response.output) : response.output; }
const repoHash = envelope('issue.monitor.status').project_store?.hash;
if (!repoHash || !/^[a-zA-Z0-9_-]+$/.test(repoHash)) throw new Error('Missing canonical project_store.hash.');
const project = path.join(stateHome, 'projects', repoHash);
const state = path.join(project, 'project-state');
await mkdir(state, { recursive: true });
const json = async (file, value) => writeFile(file, JSON.stringify(value));
await json(path.join(stateHome, 'session.json'), { active_tab_id: 'issue-5502', tabs: [{ id: 'issue-5502', title: 'Issue 5502', project_root: root, kind: 'git' }], recent_projects: [] });
await json(path.join(state, 'pm.json'), { settings: { auto_start: false }, registration: { session_id: session, agent_id: 'codex', worktree_path: root, created_at: timestamp } });
const prefsPath = path.join(state, 'issue-monitor.json');
await json(prefsPath, { enabled: false, max_active_agents: 1, priority_order: [] });
await writeFile(path.join(stateHome, 'config.toml'), '[pr_inventory]\ncache_ttl_secs = 3600\nchecks_refresh_secs = 3600\n');
const observed = new Date().toISOString();
await json(path.join(project, 'pr-inventory-cache.json'), { fetched_at: observed, rows: [{ number: 5352, title: 'Release v9.117.2', url: 'https://github.com/akiojin/gwt/pull/5352', headRefName: 'develop', baseRefName: 'main', isDraft: false, updatedAt: observed, createdAt: observed, mergeable: 'UNKNOWN', mergeStateStatus: 'UNKNOWN', statusCheckRollup: [] }] });
await json(path.join(project, 'workspace.json'), { viewport: { x: 0, y: 0, zoom: 1 }, windows: [{ id: 'pm-reports', title: 'PM', preset: 'agent', geometry: { x: 30, y: 30, width: 1050, height: 720 }, z_index: 1, status: 'stopped', session_id: session, agent_id: 'codex', persist: true }], next_z_index: 2 });
const sessions = path.join(stateHome, 'sessions');
await mkdir(sessions);
const ledger = (id, directory, branch = 'historical-fixture') => `id = ${JSON.stringify(id)}\nworktree_path = ${JSON.stringify(directory.replaceAll('\\', '/'))}\nbranch = ${JSON.stringify(branch)}\nagent_id = { type = "Codex" }\nstatus = "Stopped"\ncreated_at = "${timestamp}"\nupdated_at = "${timestamp}"\nlast_activity_at = "${timestamp}"\n`;
await writeFile(path.join(sessions, `${session}.toml`), ledger(session, root, 'work/issue-5502'));
const runtimeRoot = path.join(sessions, 'runtime');
await mkdir(runtimeRoot);
async function seedRuntimeFixture() {
  for (let directory = 0, sidecar = 0; directory < fixture.runtime_directories; directory++) {
    const namespace = path.join(runtimeRoot, String(2_000_000_000 + directory));
    await mkdir(namespace, { recursive: true });
    const count = Math.floor(fixture.runtime_sidecars / fixture.runtime_directories) + (directory < fixture.runtime_sidecars % fixture.runtime_directories ? 1 : 0);
    const writes = [];
    for (let index = 0; index < count; index++, sidecar++) {
      const id = `historical-${sidecar}`;
      writes.push(json(path.join(namespace, `${id}.json`), { status: 'Stopped', updated_at: timestamp, last_activity_at: timestamp, host_started_at: 1, child_pid: 2_000_010_000 + sidecar, child_started_at: 1 }));
      writes.push(writeFile(path.join(sessions, `${id}.toml`), ledger(id, path.join(checkHome, 'foreign-project'))));
    }
    await Promise.all(writes);
  }
}
await json(path.join(checkHome, 'issue-5023-isolated.json'), { project: root, repo_hash: repoHash });
await writeFile(path.join(state, 'pm-reports.jsonl'), `${JSON.stringify({ id: 'persisted-before-restart', kind: 'decision', body: '# Persisted before restart\n\nThis report survives the application restart.', created_at: timestamp })}\n`);
async function assertFixture() {
  const directories = await readdir(runtimeRoot, { withFileTypes: true });
  let files = 0;
  let populated = 0;
  let empty = 0;
  for (const entry of directories) if (entry.isDirectory()) {
    const count = (await readdir(path.join(runtimeRoot, entry.name))).filter(name => name.endsWith('.json')).length;
    files += count;
    if (count) populated++; else empty++;
  }
  const prefs = JSON.parse(await readFile(prefsPath, 'utf8'));
  const payloadBytes = Buffer.byteLength(prefs.claim_diagnostics?.['999999999']?.last_failure ?? '');
  const actual = { prefs_bytes: (await stat(prefsPath)).size, diagnostic_payload_bytes: payloadBytes, runtime_directories: populated, runtime_sidecars: files, empty_runtime_directories: empty };
  // The GUI updates observed_at with a variable number of fractional digits.
  // Permit only that tiny envelope drift; the retained payload must be exact.
  if (payloadBytes !== diagnosticBytes || Math.abs(actual.prefs_bytes - fixture.prefs_bytes) > 32
    || actual.runtime_directories !== fixture.runtime_directories || actual.runtime_sidecars !== fixture.runtime_sidecars) throw new Error(`Fixture changed: ${JSON.stringify(actual)}`);
  return actual;
}
async function seedMeasurementFixture() {
  // Startup publishes its own runtime census and normalizes preferences. Seed
  // after that transaction, preserving the typed fields the GUI has written.
  await seedRuntimeFixture();
  const prefs = JSON.parse(await readFile(prefsPath, 'utf8'));
  const diagnostic = { state: 'unknown', claim_id: null, owner: null, expires_at: null,
    last_attempt_at: null, last_success_at: null, last_failure_at: timestamp, last_failure: '' };
  prefs.claim_diagnostics = { ...prefs.claim_diagnostics, '999999999': diagnostic };
  // This nonexistent Issue has no inbox/queue row. Its durable diagnostic is
  // valid prefs data, retained by serde, and never projected onto the PM card.
  diagnosticBytes = fixture.prefs_bytes - Buffer.byteLength(JSON.stringify(prefs, null, 2));
  diagnostic.last_failure = 'x'.repeat(diagnosticBytes);
  await writeFile(`${prefsPath}.fixture.tmp`, JSON.stringify(prefs, null, 2));
  await rename(`${prefsPath}.fixture.tmp`, prefsPath);
}

// Resolve and audit the stable hook-generation authority on the real HOME.
const localHook = /(^|[\\/])target[\\/](?:[^\\/]+[\\/])*(?:debug|release)[\\/]gwtd(?:\.exe)?$/i;
const auditEnv = { ...process.env };
delete auditEnv.GWT_BIN_PATH;
let hookBin = process.env.GWT_HOOK_BIN;
if (hookBin && (localHook.test(hookBin) || (hookBin !== 'gwtd' && !(await stat(hookBin).catch(() => null))?.isFile()))) hookBin = undefined;
if (!hookBin) { delete auditEnv.GWT_HOOK_BIN; hookBin = operation('hook.doctor', {}, auditEnv).managed_hook_bin; }
if (!hookBin || localHook.test(hookBin)) throw new Error('Managed hook resolver did not return a stable fallback.');
auditEnv.GWT_HOOK_BIN = hookBin;
childEnv.GWT_HOOK_BIN = hookBin;
const allowMissingLogical = hookBin === 'gwtd' && spawnSync('where.exe', ['gwtd'], { windowsHide: true }).status !== 0;
const hookParams = { expected_hook_bin: hookBin, runtime_state_path: path.join(stateHome, 'missing-runtime-state.json') };
function assertHookHealth(health) {
  const issues = health?.issues?.filter(issue => !(allowMissingLogical && issue.startsWith('managed hook binary missing: ') && issue.endsWith(' uses gwtd')) && !/^managed hook failure: .* state=fail-open( |$)/.test(issue));
  if (!health || health.status === 'inactive' || !Array.isArray(issues) || issues.length) throw new Error(`Hook convergence failed: ${JSON.stringify(health)}`);
}
assertHookHealth(operation('hook.doctor', { ...hookParams, repair: true }, auditEnv).health);
const version = (await readFile(path.join(root, 'scripts/playwright-version.txt'), 'utf8')).trim();
// verify.run replaces TEMP for each command; dependencies live in the stable
// per-user cache, while the fixture still belongs to the isolated TEMP above.
const dependencyCache = path.join(process.env.LOCALAPPDATA ?? path.join(homedir(), 'AppData/Local'), 'Temp', `gwt-playwright-${version}`);
const deps = process.env.GWT_PLAYWRIGHT_NODE_MODULES ?? path.join(process.env.GWT_PLAYWRIGHT_DEPS_DIR ?? dependencyCache, 'node_modules');
const require = createRequire(import.meta.url);
const { chromium } = require(path.join(deps, 'playwright'));
const cli = path.join(deps, '@playwright/test/cli.js');
await access(cli);
const resolver = path.join(checkHome, 'resolve-playwright.cjs');
await writeFile(resolver, `const Module = require('module'); const path = require('path'); const original = Module._resolveFilename; Module._resolveFilename = function(request, parent, ...rest) { if (request === '@playwright/test' || request.startsWith('@playwright/test/')) request = path.join(process.env.GWT_PLAYWRIGHT_NODE_MODULES, request); return original.call(this, request, parent, ...rest); };\n`);
const collector = path.join(checkHome, 'collect-worker-cpu.ps1');
await writeFile(collector, String.raw`param([int]$TargetProcessId, [string]$OutputPath)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class WorkerCpu5502 {
  [StructLayout(LayoutKind.Sequential)] struct FileTime { public uint Low; public uint High; public ulong Ticks { get { return ((ulong)High << 32) | Low; } } }
  class Worker { public IntPtr Handle; public int Id; public ulong InitialTicks; }
  public class Reading { public double CpuSeconds; public int Observed; public int Exited; public int DiscoveryFailures; public int[] ThreadIds; }
  static Dictionary<string, Worker> workers = new Dictionary<string, Worker>();
  static bool initialized;
  static int discoveryFailures;
  [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenThread(uint access, bool inherit, uint id);
  [DllImport("kernel32.dll")] static extern int GetThreadDescription(IntPtr thread, out IntPtr description);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetThreadTimes(IntPtr thread, out FileTime creation, out FileTime exit, out FileTime kernel, out FileTime user);
  [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr memory);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
  public static Reading Snapshot(int processId) {
    using (var process = Process.GetProcessById(processId)) {
      foreach (ProcessThread item in process.Threads) {
        IntPtr thread = OpenThread(0x0800, false, (uint)item.Id);
        if (thread == IntPtr.Zero) { discoveryFailures++; continue; }
        try {
          IntPtr text;
          int descriptionResult = GetThreadDescription(thread, out text);
          // A thread may exit between enumeration and its description read.
          if (descriptionResult < 0) { discoveryFailures++; continue; }
          string name;
          try { name = Marshal.PtrToStringUni(text); } finally { LocalFree(text); }
          if (name != "tokio-rt-worker") continue;
          FileTime created, exited, kernel, user;
          if (!GetThreadTimes(thread, out created, out exited, out kernel, out user)) throw new InvalidOperationException("GetThreadTimes failed");
          string identity = item.Id + ":" + created.Ticks;
          if (!workers.ContainsKey(identity)) {
            workers.Add(identity, new Worker { Handle = thread, Id = item.Id, InitialTicks = initialized ? 0 : kernel.Ticks + user.Ticks });
            thread = IntPtr.Zero;
          }
        } finally { if (thread != IntPtr.Zero) CloseHandle(thread); }
      }
    }
    initialized = true;
    var result = new Reading { Observed = workers.Count, DiscoveryFailures = discoveryFailures };
    var ids = new List<int>();
    foreach (var worker in workers.Values) {
      FileTime created, exited, kernel, user;
      if (!GetThreadTimes(worker.Handle, out created, out exited, out kernel, out user)) throw new InvalidOperationException("Retained worker GetThreadTimes failed");
      result.CpuSeconds += (kernel.Ticks + user.Ticks - worker.InitialTicks) / 10000000.0;
      if (exited.Ticks != 0) result.Exited++;
      ids.Add(worker.Id);
    }
    ids.Sort(); result.ThreadIds = ids.ToArray();
    return result;
  }
  public static void Close() { foreach (var worker in workers.Values) CloseHandle(worker.Handle); workers.Clear(); }
}
'@
$baseline = [WorkerCpu5502]::Snapshot($TargetProcessId)
if (!$baseline.Observed) { throw 'No tokio-rt-worker threads were observed.' }
$process = [Diagnostics.Process]::GetProcessById($TargetProcessId)
$process.Refresh()
$initialCpu = $process.TotalProcessorTime.TotalSeconds
$timer = [Diagnostics.Stopwatch]::StartNew()
$samples = @()
try {
  for ($index = 1; $index -le 6; $index++) {
    $remaining = [Math]::Max(0, [int]($index * 5000 - $timer.Elapsed.TotalMilliseconds))
    if ($remaining -gt 0) { Start-Sleep -Milliseconds $remaining }
    $current = [WorkerCpu5502]::Snapshot($TargetProcessId)
    $process.Refresh()
    $sample = [ordered]@{elapsed_seconds=$timer.Elapsed.TotalSeconds;worker_cpu_seconds=$current.CpuSeconds;process_cpu_seconds=($process.TotalProcessorTime.TotalSeconds-$initialCpu);worker_thread_ids=@($current.ThreadIds);worker_threads_observed=$current.Observed;exited_worker_threads=$current.Exited;discovery_failures=$current.DiscoveryFailures}
    $samples += $sample
    Write-Output ("CPU sample {0}/6: {1:N3}s worker CPU in {2:N3}s" -f $index,$current.CpuSeconds,$timer.Elapsed.TotalSeconds)
  }
  [ordered]@{method='Windows GetThreadDescription exact tokio-rt-worker + retained GetThreadTimes handles, kernel+user';duration_seconds=30;poll_seconds=5;worker_threads=$baseline.Observed;samples=$samples} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
} finally { [WorkerCpu5502]::Close() }
`);

const urlFile = path.join(checkHome, 'url');
childEnv.GWT_BROWSER_URL_FILE = urlFile;
let browser;
let anchor;
let child;
let logs = '';
let passed = false;
let e2ePassed = false;
let cpu;
let polling;
let fixtureBefore;
let fixtureAfter;
const errors = [];
const auxiliary = new Set();
let cleaning;
function stopOwned(processChild) {
  if (processChild?.pid && processChild.exitCode === null && processChild.signalCode === null) {
    spawnSync('taskkill.exe', ['/PID', String(processChild.pid), '/T', '/F'], { windowsHide: true, encoding: 'utf8' });
  }
}
async function cleanup() {
  if (cleaning) return cleaning;
  cleaning = (async () => {
    for (const processChild of auxiliary) stopOwned(processChild);
    if (browser) await browser.close().catch(error => errors.push(error.message));
    if (anchor) anchor.close();
    if (child?.pid && child.exitCode === null && child.signalCode === null) {
      const exited = new Promise(resolve => child.once('exit', resolve));
      stopOwned(child);
      await Promise.race([exited, delay(10_000, undefined, { ref: false })]);
      if (child.exitCode === null && child.signalCode === null) { errors.push(`Owned GUI PID ${child.pid} remained after cleanup.`); process.exitCode = 1; }
    }
  })();
  return cleaning;
}
for (const [signal, exitCode] of [['SIGINT', 130], ['SIGTERM', 143], ['SIGHUP', 129]]) {
  process.on(signal, () => { cleanup().finally(() => process.exit(exitCode)); });
}
async function run(binary, args, env = process.env) {
  const processChild = spawn(binary, args, { cwd: root, env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let errorOutput = '';
  processChild.stdout.on('data', data => { process.stdout.write(data); logs += data; });
  processChild.stderr.on('data', data => { process.stderr.write(data); logs += data; errorOutput += data; });
  auxiliary.add(processChild);
  try {
    await new Promise((resolve, reject) => { processChild.once('error', reject); processChild.once('exit', (code, signal) => code === 0 ? resolve() : reject(new Error(`${path.basename(binary)} exited ${code ?? signal}${errorOutput ? `: ${errorOutput.trim()}` : ''}`))); });
  } finally { auxiliary.delete(processChild); }
}
try {
  child = spawn(gwt, ['--no-tray', '--no-open'], { cwd: root, env: childEnv, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
  child.stdout.on('data', data => { logs += data; });
  child.stderr.on('data', data => { logs += data; });
  let launchError;
  child.on('error', error => { launchError = error; });
  console.log(`Owned gwt PID: ${child.pid}; isolated HOME: ${checkHome}`);
  let url;
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    if (launchError) throw launchError;
    if (child.exitCode !== null) throw new Error(`Fresh gwt exited ${child.exitCode}: ${logs}`);
    try { const candidate = (await readFile(urlFile, 'utf8')).trim(); if (candidate && (await fetch(candidate, { method: 'HEAD', signal: AbortSignal.timeout(2000) })).ok) { url = candidate; break; } } catch { /* URL has not become ready yet. */ }
    await delay(100);
  }
  if (!url || /another tray-resident gwt instance is already running/i.test(logs)) throw new Error('Fresh checkout URL never became ready or isolation failed.');
  anchor = new WebSocket(new URL('/ws', url).href.replace(/^http/, 'ws'));
  await new Promise((resolve, reject) => { anchor.addEventListener('open', resolve, { once: true }); anchor.addEventListener('error', reject, { once: true }); });
  assertHookHealth(operation('hook.health', hookParams, auditEnv));
  browser = await chromium.launch({ headless: false, ...(process.env.GWT_PLAYWRIGHT_CHROMIUM_CHANNEL ? { channel: process.env.GWT_PLAYWRIGHT_CHROMIUM_CHANNEL } : {}) });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, colorScheme: 'dark' });
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  await page.addInitScript(() => { localStorage.setItem('gwt:ui:theme', 'dark'); sessionStorage.setItem('gwt:ui:briefing', '1'); });
  await page.goto(url);
  const key = await page.evaluate(base => new Promise((resolve, reject) => {
    const socket = new WebSocket(new URL('/ws', base).href.replace(/^http/, 'ws'));
    const timer = setTimeout(() => { socket.close(); reject(new Error('Hub catalog timeout')); }, 30_000);
    socket.addEventListener('open', () => socket.send(JSON.stringify({ kind: 'frontend_ready' })));
    socket.addEventListener('message', event => { const message = JSON.parse(String(event.data)); if (message.kind === 'hub_state' && message.hub.projects[0]?.project_key) { clearTimeout(timer); socket.close(); resolve(message.hub.projects[0].project_key); } });
  }), url);
  await page.goto(new URL(`/p/${key}`, url).href);
  await page.addStyleTag({ content: '#op-briefing,#preset-modal { display:none !important; pointer-events:none !important; }' });
  await page.getByRole('log', { name: 'PM reports' }).waitFor({ state: 'visible', timeout: 30_000 });
  await page.evaluate(() => {
    const counters = { forwarded_reports: 0, other_requests: {} };
    window.__pmReportsMeasurement = counters;
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      if (typeof data === 'string') {
        try {
          const message = JSON.parse(data);
          if (message.kind !== 'load_pm_reports') {
            counters.other_requests[message.kind] = (counters.other_requests[message.kind] ?? 0) + 1;
            return;
          }
          counters.forwarded_reports++;
        } catch { /* Non-JSON transport frames retain their normal path. */ }
      }
      return send.call(this, data);
    };
  });
  await delay(5000);
  console.log('Reports pane ready; reseeding the fixed measurement fixture.');
  await seedMeasurementFixture();
  await delay(5000);
  fixtureBefore = await assertFixture();
  console.log(`Measurement fixture: ${JSON.stringify(fixtureBefore)}`);
  await page.evaluate(() => { window.__pmReportsMeasurement.forwarded_reports = 0; window.__pmReportsMeasurement.other_requests = {}; });
  const cpuPath = path.join(checkHome, 'cpu.json');
  await run('pwsh', ['-NoProfile', '-NonInteractive', '-File', collector, '-TargetProcessId', String(child.pid), '-OutputPath', cpuPath]);
  cpu = JSON.parse((await readFile(cpuPath, 'utf8')).replace(/^\uFEFF/, ''));
  fixtureAfter = await assertFixture();
  polling = await page.evaluate(() => window.__pmReportsMeasurement);
  if (polling.forwarded_reports < 5) throw new Error('The measurement did not exercise the real five-second Reports poll.');
  await page.screenshot({ path: `${out}.png`, fullPage: true });
  await browser.close(); browser = undefined;
  if (errors.length) throw new Error(`Browser errors: ${errors.join('; ')}`);
  if (!values['measure-only']) {
    const runnerEnv = { ...process.env, GWT_PLAYWRIGHT_BASE_URL: url, GWT_PLAYWRIGHT_CHECK_HOME: checkHome,
      GWT_PLAYWRIGHT_PROJECT_ROOT: root, GWT_PLAYWRIGHT_CHECKOUT_ROOT: root, GWT_PLAYWRIGHT_PROJECT_KEY: repoHash,
      GWT_PLAYWRIGHT_NODE_MODULES: deps, NODE_OPTIONS: `--require ${JSON.stringify(resolver.replaceAll('\\', '/'))} ${process.env.NODE_OPTIONS ?? ''}` };
    await run(process.execPath, [cli, 'test', '--config', path.join(root, 'crates/gwt/playwright/playwright.config.ts'), '--workers=1', '--headed', '--output', path.join(checkHome, 'playwright'), 'pm-chat-live.spec.ts', ...(values.spec ?? []), ...playwrightArgs], runnerEnv);
    e2ePassed = true;
  }
  passed = true;
} catch (error) {
  errors.push(error.message);
  console.error(error.message);
  process.exitCode = 1;
} finally {
  await cleanup();
  const sha = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8', windowsHide: true }).stdout.trim();
  const binary = await stat(gwt);
  const report = { passed: passed && errors.length === 0, label: values.label, sha, binary: { path: gwt, bytes: binary.size, modified_at: binary.mtime.toISOString() }, check_home: checkHome,
    owned_pid: child?.pid, fixture, fixture_before: fixtureBefore, fixture_after: fixtureAfter, theme: 'dark', background_index_disabled: true, polling, cpu, e2e: values['measure-only'] ? 'not requested' : e2ePassed ? 'passed (chromium-dark and chromium-light)' : 'failed or not reached', errors };
  await writeFile(out, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(`${out}.log`, logs);
  console.log(`Evidence: ${out}; owned process cleanup finished.`);
}
