#!/usr/bin/env node
// #5379: five real custom agents, 8 KiB/second each. Reports traffic is excluded.
// node scripts/verify-issue-5379.mjs --seconds 1800 --label before --binary target/issue-5379/baseline/gwt --out target/issue-5379/before.json --project=chromium-dark
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { access, mkdir, mkdtemp, readFile, readdir, realpath, symlink, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SPEC = 'crates/gwt/playwright/tests/runtime-output-batch-live.spec.ts';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const exists = file => access(file).then(() => true, () => false);

export function parseOptions(args) {
  const options = { seconds: 30, label: 'after', binary: path.join(ROOT, 'target/debug/gwt'), out: '', playwrightArgs: [] };
  for (let index = 0; index < args.length; index++) {
    const argument = args[index];
    const match = argument.match(/^--(seconds|label|binary|out)(?:=(.*))?$/);
    if (match) {
      const value = match[2] ?? args[++index];
      if (!value || value.startsWith('--')) throw new Error(`${match[1]} needs a value`);
      options[match[1]] = match[1] === 'seconds' ? Number(value) : value;
    } else if (argument !== '--') options.playwrightArgs.push(argument);
  }
  if (!Number.isInteger(options.seconds) || options.seconds < 1 || options.seconds > 1800) throw new Error('seconds must be an integer from 1 to 1800');
  if (!['before', 'after'].includes(options.label)) throw new Error('label must be before or after');
  return options;
}

export function summarizeTiming(records, start, end) {
  const dispatch = () => ({ count: 0, stall_count: 0, at_least_100ms: 0, max_ms: 0 });
  const result = { all_dispatch: dispatch(), runtime_output: dispatch(), hook_health_worker: { count: 0, at_least_100ms: 0, max_ms: 0 } };
  for (const record of records) {
    const at = Date.parse(record.timestamp);
    if (at < start || at >= end || !Number.isFinite(at) || record.target !== 'gwt.frontend.timing') continue;
    const fields = record.fields ?? {}, ms = fields.elapsed_ms;
    if (!Number.isFinite(ms)) continue;
    if (fields.stage === 'work_rows_hook_health_excluding_project') {
      const summary = result.hook_health_worker;
      summary.count++; summary.max_ms = Math.max(summary.max_ms, ms); summary.at_least_100ms += Number(ms >= 100);
    }
    const stalled = fields.message === `${fields.event} blocked the GUI event loop for ${ms}ms`;
    if (!fields.event || (!stalled && fields.message !== 'GUI event loop dispatch handled')) continue;
    for (const summary of [result.all_dispatch, ...(fields.event.startsWith('RuntimeOutput') ? [result.runtime_output] : [])]) {
      summary.count++; summary.stall_count += Number(stalled); summary.at_least_100ms += Number(ms >= 100);
      summary.max_ms = Math.max(summary.max_ms, ms);
    }
  }
  return result;
}

// The #5016 fixed producer, without its unrelated CPU/RSS/wire comparison matrix.
const PRODUCER = String.raw`const fs = require('node:fs'), path = require('node:path');
const index = Number(process.argv[2]), directory = process.env.GWT_LOOP_AGENT_DIR;
const record = { index, pid: process.pid, sequence: 0, bytes: 0, complete: false };
const save = () => { const file = path.join(directory,index+'.json'); fs.writeFileSync(file+'.tmp',JSON.stringify(record)); fs.renameSync(file+'.tmp',file); };
const sleep = ms => new Promise(resolve => setTimeout(resolve,ms));
(async () => {
  save(); while (!fs.existsSync(process.env.GWT_LOOP_BARRIER)) await sleep(100);
  const { start, seconds } = JSON.parse(fs.readFileSync(process.env.GWT_LOOP_BARRIER,'utf8'));
  for (let sequence=1; sequence<=seconds; sequence++) {
    await sleep(Math.max(0,start+(sequence-1)*1000-Date.now()));
    const line = prefix => prefix+'x'.repeat(510-prefix.length)+'\r\n';
    const marker = 'GWT_LOOP_5379_'+index+'_'+String(sequence).padStart(4,'0');
    const output = line('fixed task ').repeat(15)+line(marker+' ');
    if (!process.stdout.write(output)) await new Promise(resolve => process.stdout.once('drain',resolve));
    record.sequence=sequence; record.bytes+=Buffer.byteLength(output); save();
  }
  await sleep(Math.max(0,start+seconds*1000-Date.now())); record.complete=true; save();
})().catch(error => { console.error(error); process.exitCode=1; });
`;

async function run(options) {
  if (!options.out) throw new Error('--out is required');
  const binary = await realpath(path.resolve(ROOT, options.binary));
  const target = await realpath(path.join(ROOT, 'target'));
  if (!binary.startsWith(target + path.sep) || path.basename(binary) !== 'gwt') throw new Error('--binary must be this checkout target/**/gwt');
  const gwtd = path.join(path.dirname(binary), 'gwtd');
  await access(gwtd);
  const evidenceRoot = path.join(ROOT, 'target/issue-5379');
  await mkdir(evidenceRoot, { recursive: true });
  const evidence = await mkdtemp(path.join(evidenceRoot, `${options.label}-`));
  const owned = [], manifests = {};
  let cleanupPromise, failure;
  function start(file, args, env, cwd, input = '', stream = false, group = false) {
    const child = spawn(file, args, { env, cwd, detached: group, stdio: ['pipe', 'pipe', 'pipe'] });
    const record = { child, group, stdout: '', stderr: '', done: false };
    owned.push(record);
    record.closed = new Promise(resolve => {
      child.once('error', error => { record.error = error; });
      child.once('close', code => { record.done = true; resolve(code); });
    });
    for (const channel of ['stdout', 'stderr']) child[channel].on('data', data => {
      if (stream) process[channel].write(data);
      else record[channel] = (record[channel] + data).slice(-1024 * 1024);
    });
    child.stdin.on('error', () => {}); child.stdin.end(input);
    return record;
  }
  async function finish(record, timeout = 60_000) {
    let timer;
    try {
      const code = await Promise.race([record.closed, new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`Owned PID ${record.child.pid} timed out`)), timeout);
      })]);
      if (record.error || code !== 0) throw new Error(`${record.error ?? `exit ${code}`}\n${record.stdout}\n${record.stderr}`);
      return record.stdout;
    } finally { clearTimeout(timer); }
  }
  const groupExists = pid => {
    const result = spawnSync('ps', ['-axo', 'pgid=,pid='], { encoding: 'utf8', timeout: 5000 });
    if (result.status !== 0) throw new Error('Owned Playwright group probe failed');
    return result.stdout.split('\n').some(line => Number(line.trim().split(/\s+/)[0]) === pid);
  };
  async function cleanup() {
    return cleanupPromise ??= (async () => {
      const failures = [];
      for (const record of [...owned].reverse()) {
        if (!record.child.pid) continue;
        for (const signal of ['SIGTERM', 'SIGKILL']) {
          if (record.group ? !groupExists(record.child.pid) : record.done) break;
          try { if (record.group) process.kill(-record.child.pid, signal); else record.child.kill(signal); }
          catch (error) { if (error.code !== 'ESRCH') failures.push(error.message); }
          const deadline = Date.now() + 10_000;
          while (Date.now() < deadline && (record.group ? groupExists(record.child.pid) : !record.done)) await delay(100);
        }
        if (record.group ? groupExists(record.child.pid) : !record.done) failures.push(`Owned PID ${record.child.pid} remains`);
      }
      // PTYs have their own process groups. Verify receipt identity before stopping one.
      for (const manifest of Object.values(manifests)) for (let index = 0; index < 5; index++) {
        let receipt;
        try { receipt = JSON.parse(await readFile(path.join(manifest.agentDir, `${index}.json`), 'utf8')); } catch { continue; }
        const probe = () => spawnSync('ps', ['-ww', '-p', String(receipt.pid), '-o', 'command='], { encoding: 'utf8', timeout: 5000 });
        const expected = `${process.execPath} ${manifest.producer} ${index}`;
        for (const signal of ['SIGTERM', 'SIGKILL']) {
          const identity = probe();
          if (identity.status === 1 && !identity.stdout.trim()) break;
          if (identity.status !== 0 || identity.stdout.trim() !== expected) { failures.push(`PTY ${receipt.pid} identity mismatch`); break; }
          try { process.kill(receipt.pid, signal); } catch (error) { if (error.code !== 'ESRCH') failures.push(error.message); }
          const deadline = Date.now() + 10_000;
          while (Date.now() < deadline && probe().status === 0) await delay(100);
        }
        if (probe().status === 0) failures.push(`Owned PTY ${receipt.pid} remains`);
      }
      await writeFile(path.join(evidence, 'cleanup.json'), JSON.stringify({ failures, owned: owned.map(r => ({ pid: r.child.pid, done: r.done, group: r.group })) }, null, 2));
      if (failures.length) throw new Error(failures.join('\n'));
    })();
  }
  const signals = ['SIGINT', 'SIGTERM', 'SIGHUP'].map(signal => [signal, () => { void cleanup().finally(() => process.exit(1)); }]);
  for (const [signal, handler] of signals) process.once(signal, handler);
  try {
    const projectArgs = [], forwarded = [];
    for (let i = 0; i < options.playwrightArgs.length; i++) {
      const arg = options.playwrightArgs[i];
      if (arg === '--project') projectArgs.push(options.playwrightArgs[++i]);
      else if (arg.startsWith('--project=')) projectArgs.push(arg.slice(10));
      else forwarded.push(arg);
    }
    const themes = projectArgs.length ? [...new Set(projectArgs.map(value => {
      if (!['chromium-dark', 'chromium-light'].includes(value)) throw new Error('Only chromium-dark/chromium-light projects are supported');
      return value.slice(9);
    }))] : ['dark', 'light'];
    for (const theme of themes) {
      const home = path.join(evidence, theme, 'home'), project = path.join(home, 'project');
      await mkdir(path.join(home, '.gwt'), { recursive: true });
      await mkdir(path.join(home, '.codex'), { recursive: true });
      await mkdir(path.join(project, '.codex'), { recursive: true });
      const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
        && !['GH_TOKEN', 'GITHUB_TOKEN', 'OPENAI_API_KEY', 'ANTHROPIC_API_KEY'].includes(key.toUpperCase())));
      Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: path.join(home, '.codex'),
        GIT_TERMINAL_PROMPT: '0', GH_PROMPT_DISABLED: '1', RUST_LOG: 'info,gwt.frontend.timing=debug', GWT_DISABLE_BACKGROUND_INDEX: '1' });
      const runtime = path.join(homedir(), '.gwt/runtime');
      if (await exists(runtime)) await symlink(runtime, path.join(home, '.gwt/runtime'));
      const git = args => finish(start('git', args, env, project));
      await git(['init', '-q', '-b', 'develop', '--separate-git-dir', path.join(home, 'project.git')]);
      await git(['-c', 'user.name=measurement', '-c', 'user.email=measurement@gwt.invalid', 'commit', '--allow-empty', '-q', '-m', 'Fixed workload']);
      await git(['clone', '--quiet', '--bare', '.', path.join(home, 'origin.git')]);
      await git(['remote', 'add', 'origin', path.join(home, 'origin.git')]);
      async function envelope(operation, params = {}) {
        const reply = JSON.parse(await finish(start(gwtd, [], env, project, JSON.stringify({ schema_version: 1, operation, params }))));
        if (!reply.ok) throw new Error(`${operation} refused: ${JSON.stringify(reply)}`);
        return reply;
      }
      const value = reply => typeof reply.output === 'string' ? JSON.parse(reply.output) : reply.output;
      const hookBin = value(await envelope('hook.doctor')).managed_hook_bin;
      if (!hookBin || /[/\\]target[/\\]/i.test(hookBin)) throw new Error('Hook fallback must be stable/public or logical gwtd');
      env.GWT_HOOK_BIN = hookBin;
      const hookParams = { expected_hook_bin: hookBin, runtime_state_path: path.join(home, '.gwt/missing-runtime.json') };
      await writeFile(path.join(project, '.codex/hooks.json'), '{}');
      await envelope('hook.doctor', { ...hookParams, repair: true });
      await envelope('hook.register_codex_managed_hook_trust', { project_root: project, codex_config: path.join(home, '.codex/config.toml'), codex_hook_discovery: 'both' });
      const audit = async phase => {
        const health = value(await envelope('hook.doctor', { ...hookParams, repair: false })).health;
        await writeFile(path.join(home, `hook-${phase}.json`), JSON.stringify(health, null, 2));
        if (!health || health.status === 'inactive' || !Array.isArray(health.issues) || health.issues.some(issue =>
          !/^managed hook failure: .*state=fail-open( |$)/.test(issue)
          && !(hookBin === 'gwtd' && /^managed hook binary missing: .* uses gwtd$/.test(issue)))) throw new Error(`Managed hook ${phase} audit failed`);
      };
      await audit('pre');
      const hash = (await envelope('issue.monitor.status')).project_store?.hash;
      if (!hash) throw new Error('Canonical project hash missing');
      const state = path.join(home, '.gwt/projects', hash, 'project-state');
      await mkdir(state, { recursive: true });
      await writeFile(path.join(state, 'pm.json'), JSON.stringify({ settings: { auto_start: false } }));
      await writeFile(path.join(state, 'issue-monitor.json'), JSON.stringify({ enabled: false, autonomous_mode: false, max_active_agents: 5, priority_order: [], auto_apply_updates: false }));
      await writeFile(path.join(home, '.gwt/session.json'), JSON.stringify({ tabs: [{ id: 'loop-load', title: 'Loop load', project_root: project, kind: 'git' }], active_tab_id: 'loop-load', recent_projects: [] }));
      const producer = path.join(home, 'producer.cjs'), agentDir = path.join(home, 'agents'), barrier = path.join(home, 'start.json');
      await mkdir(agentDir); await writeFile(producer, PRODUCER);
      await writeFile(path.join(home, '.gwt/config.toml'), 'auto_apply_updates = false\n' + Array.from({ length: 5 }, (_, i) =>
        `[tools.customCodingAgents.loop-load-${i}]\ndisplayName = "Loop load ${i}"\nagentType = "path"\ncommand = ${JSON.stringify(process.execPath)}\ndefaultArgs = [${JSON.stringify(producer)}, "${i}"]\n[tools.customCodingAgents.loop-load-${i}.env]\nGWT_LOOP_AGENT_DIR = ${JSON.stringify(agentDir)}\nGWT_LOOP_BARRIER = ${JSON.stringify(barrier)}\n`).join('\n'));
      const urlFile = path.join(home, 'url');
      env.GWT_BROWSER_URL_FILE = urlFile;
      const gui = start(binary, ['--no-tray', '--no-open'], env, project);
      const manifest = { theme, home, project, agentDir, producer, barrier, seconds: options.seconds, gui_pid: gui.child.pid,
        result: path.join(evidence, theme, 'measurement.json') };
      manifests[theme] = manifest;
      const deadline = Date.now() + 90_000;
      while (!manifest.url && Date.now() < deadline && !gui.done) {
        try {
          const url = (await readFile(urlFile, 'utf8')).trim();
          if ((await fetch(url, { method: 'HEAD', signal: AbortSignal.timeout(2000) })).ok) manifest.url = url;
        } catch { /* readiness */ }
        if (!manifest.url) await delay(100);
      }
      if (!manifest.url) throw new Error(`Fresh GUI ${gui.child.pid} did not become ready`);
      await audit('post');
      console.log(JSON.stringify({ phase: 'ready', theme, pid: gui.child.pid, home, binary }));
    }
    const manifestPath = path.join(evidence, 'manifest.json');
    await writeFile(manifestPath, JSON.stringify(manifests));
    const args = [path.join(ROOT, 'scripts/run-visual-tests.sh'), SPEC, '--headed', '--workers=1', '--retries=0',
      ...themes.map(theme => `--project=chromium-${theme}`), ...forwarded];
    await finish(start('bash', args, { ...process.env, GWT_LOOP_MANIFEST: manifestPath,
      GWT_PLAYWRIGHT_CHECKOUT_ROOT: ROOT }, ROOT, '', true, true), (options.seconds * themes.length + 600) * 1000);
  } catch (error) { failure = error; }
  finally {
    try { await cleanup(); } catch (error) { failure ??= error; }
    for (const [signal, handler] of signals) process.removeListener(signal, handler);
  }
  const runs = [];
  for (const manifest of Object.values(manifests)) {
    let measurement;
    try { measurement = JSON.parse(await readFile(manifest.result, 'utf8')); } catch { continue; }
    const records = [];
    const dirs = [path.join(manifest.home, '.gwt/logs')];
    for (const entry of await readdir(path.join(manifest.home, '.gwt/projects')).catch(() => [])) dirs.push(path.join(manifest.home, '.gwt/projects', entry, 'logs'));
    for (const directory of dirs) for (const file of await readdir(directory).catch(() => [])) if (/^gwt\.log/.test(file)) {
      for (const line of (await readFile(path.join(directory, file), 'utf8')).trim().split('\n')) if (line) records.push(JSON.parse(line));
    }
    measurement.timing = summarizeTiming(records, measurement.started_at_ms, measurement.ended_at_ms);
    if (!measurement.timing.runtime_output.count) failure ??= new Error('No native RuntimeOutput dispatch samples');
    runs.push(measurement);
  }
  const report = { schema_version: 1, label: options.label, seconds: options.seconds, agents: 5, bytes_per_second_per_agent: 8192,
    binary, binary_sha256: createHash('sha256').update(await readFile(binary)).digest('hex'),
    source_sha: spawnSync('git', ['rev-parse', 'HEAD'], { cwd: ROOT, encoding: 'utf8' }).stdout.trim(),
    workload_note: '#5016 five scripted agents / 8 KiB per second / 1800-second full run; Reports traffic intentionally excluded.',
    evidence, runs, complete: !failure && runs.length === Object.keys(manifests).length && runs.length > 0,
    acceptance_duration: options.seconds === 1800, UserVerificationResult: 'n/a (autonomous)', error: failure?.message ?? null };
  const sourceDiff = spawnSync('git', ['diff', '--binary', 'HEAD', '--', 'crates', 'scripts', 'Cargo.toml', 'Cargo.lock'], { cwd: ROOT, encoding: 'utf8' });
  if (sourceDiff.status !== 0) throw new Error('Source identity unavailable');
  report.source_dirty = sourceDiff.stdout.length > 0;
  report.source_diff_sha256 = createHash('sha256').update(sourceDiff.stdout).digest('hex');
  report.harness_sha256 = createHash('sha256').update(await readFile(fileURLToPath(import.meta.url))).update(await readFile(path.join(ROOT, SPEC))).digest('hex');
  await mkdir(path.dirname(path.resolve(options.out)), { recursive: true });
  await writeFile(options.out, JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ report: path.resolve(options.out), evidence, complete: report.complete }));
  if (failure) throw failure;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  run(parseOptions(process.argv.slice(2))).catch(error => { console.error(error.message); process.exitCode = 1; });
}
