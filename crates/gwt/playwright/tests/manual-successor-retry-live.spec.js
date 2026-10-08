// Issue #4964: a new manual launch after Aborted must create a new attempt.
// Opt in after building this checkout's gwt/gwtd. No production HOME is used.
const { test, expect } = require('@playwright/test');
const { spawn, spawnSync } = require('node:child_process');
const { access, copyFile, mkdir, mkdtemp, readFile, readdir, stat, symlink, writeFile } = require('node:fs/promises');
const { createWriteStream, realpathSync } = require('node:fs');
const { homedir, tmpdir } = require('node:os');
const { delimiter, join, resolve } = require('node:path');
const { setTimeout: delay } = require('node:timers/promises');
const { gotoLiveGwt, openLiveLaunchWizardForBranch, sendLiveGwtEvent } = require('./_helpers/live-gwt.ts');

const ROOT = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
const OWNER = 4964;
const BRANCH = `work/issue-${OWNER}`;
const OPT_IN = process.env.GWT_PLAYWRIGHT_MANUAL_SUCCESSOR_RETRY === '1';
const WINDOWS = process.platform === 'win32';
const EXE = WINDOWS ? '.exe' : '';
let fixtureBinaries;

async function compileFixtures() {
  const deps = join(ROOT, 'target/debug/deps');
  const output = await mkdtemp(join(tmpdir(), 'gwt-manual-successor-bin-'));
  const files = await readdir(deps);
  async function artifact(crate) {
    const candidates = await Promise.all(files.filter(name => name.startsWith(`lib${crate}-`) && name.endsWith('.rlib'))
      .map(async name => ({ path: join(deps, name), modified: (await stat(join(deps, name))).mtimeMs })));
    candidates.sort((left, right) => right.modified - left.modified);
    if (!candidates.length) throw new Error(`Build this checkout's gwt/gwtd first; missing ${crate} rlib in ${deps}`);
    return candidates[0].path;
  }
  const gwtLibrary = await artifact('gwt');
  // Test/feature builds can leave several serde identities in deps. Use the
  // exact dependency fingerprint of the gwt library, rather than its mtime.
  const gwtHash = gwtLibrary.match(/libgwt-([a-f0-9]+)\.rlib$/)[1];
  const gwtMetadata = await readFile(join(ROOT, 'target/debug/.fingerprint', `gwt-${gwtHash}`, 'lib-gwt.json'), 'utf8');
  const jsonFingerprint = BigInt(gwtMetadata.match(/\[\d+,"serde_json",false,(\d+)\]/)[1]);
  let jsonLibrary;
  for (const name of files.filter(name => /^libserde_json-[a-f0-9]+\.rlib$/.test(name))) {
    const hash = name.match(/libserde_json-([a-f0-9]+)\.rlib$/)[1];
    const fingerprint = (await readFile(join(ROOT, 'target/debug/.fingerprint', `serde_json-${hash}`, 'lib-serde_json'), 'utf8')).trim();
    if (BigInt(`0x${fingerprint.match(/../g).reverse().join('')}`) === jsonFingerprint) jsonLibrary = join(deps, name);
  }
  if (!jsonLibrary) throw new Error(`Missing exact serde_json dependency for ${gwtLibrary}; build gwt/gwtd first`);
  const nativeDirectories = new Set();
  for (const entry of await readdir(join(ROOT, 'target/debug/build'))) {
    const path = join(ROOT, 'target/debug/build', entry, 'out');
    try { if ((await stat(path)).isDirectory()) nativeDirectories.add(path); } catch {}
    // Cargo also records registry-native libraries (notably windows targets).
    const metadata = await readFile(join(ROOT, 'target/debug/build', entry, 'output'), 'utf8').catch(() => '');
    for (const line of metadata.split(/\r?\n/)) {
      const search = line.match(/^cargo(?:::|:)rustc-link-search=(?:native=)?(.+)$/);
      if (search) nativeDirectories.add(search[1]);
    }
  }
  const nativePaths = [...nativeDirectories].flatMap(path => ['-L', `native=${path}`]);
  const cargoConfig = await readFile(join(ROOT, '.cargo/config.toml'), 'utf8');
  const linker = cargoConfig.match(/\[target\.x86_64-pc-windows-msvc\][\s\S]*?linker\s*=\s*"([^"]+)"/)?.[1];
  const result = {};
  for (const name of ['seed', 'provider']) {
    const binary = join(output, `${name}${EXE}`);
    const args = ['--edition=2021', '--crate-name', `manual_successor_${name}`,
      join(ROOT, 'crates/gwt/playwright/fixtures', `manual-successor-${name}.rs`),
      '-L', `dependency=${deps}`, ...nativePaths, '--extern', `serde_json=${jsonLibrary}`];
    if (WINDOWS && linker) args.push('-C', `linker=${linker}`);
    if (name === 'seed') args.push('--extern', `gwt=${gwtLibrary}`);
    args.push('-o', binary);
    const run = spawnSync('rustc', args, { cwd: ROOT, encoding: 'utf8', windowsHide: true, timeout: 120000 });
    if (run.status !== 0) throw new Error(`Fixture ${name} compilation failed: ${run.stdout}\n${run.stderr}`);
    result[name] = binary;
  }
  return result;
}

function exactProviderProcess(pid, expectedPath) {
  const run = WINDOWS ? spawnSync('pwsh', ['-NoProfile', '-Command',
    `$fixtureProcess = Get-Process -Id ${Number(pid)} -ErrorAction SilentlyContinue; if ($fixtureProcess) { $fixtureProcess.Path }`],
  { encoding: 'utf8', windowsHide: true })
    : spawnSync('ps', ['-p', String(Number(pid)), '-o', 'comm='], { encoding: 'utf8' });
  if (!run.stdout.trim() && !run.stderr.trim()) return false;
  if (run.status !== 0) throw new Error(`Provider process inspection failed: ${run.stderr}`);
  const path = run.stdout.trim();
  // macOS reports a reaped executable as <defunct> until its parent waits.
  if (!path || (!WINDOWS && path === '<defunct>')) return false;
  const actual = realpathSync.native(path);
  const expected = realpathSync.native(expectedPath);
  return WINDOWS ? actual.toLowerCase() === expected.toLowerCase() : actual === expected;
}

async function startFixture(info) {
  const home = await mkdtemp(join(tmpdir(), 'gwt-manual-successor-'));
  const project = join(home, 'project');
  const bin = join(home, 'bin');
  const state = join(home, '.gwt');
  const gwt = join(ROOT, `target/debug/gwt${EXE}`);
  const gwtd = join(ROOT, `target/debug/gwtd${EXE}`);
  await Promise.all([mkdir(project), mkdir(bin), mkdir(state), mkdir(join(home, '.codex')), access(gwt), access(gwtd)]);
  await Promise.all(['codex', 'gh'].map(name => copyFile(fixtureBinaries.provider, join(bin, `${name}${EXE}`))));
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (key.startsWith('GWT_') || key.startsWith('CODEX_') || key.startsWith('CLAUDE_')
      || ['GH_TOKEN', 'GITHUB_TOKEN', 'OPENAI_API_KEY', 'ANTHROPIC_API_KEY'].includes(key)) delete env[key];
  }
  Object.assign(env, { HOME: home, USERPROFILE: home, CODEX_HOME: join(home, '.codex'),
    PATH: `${bin}${delimiter}${process.env.PATH}`, GIT_TERMINAL_PROMPT: '0', GH_PROMPT_DISABLED: '1',
    GWT_HOOK_BIN: 'gwtd', GWT_PROJECT_ROOT: project, GWT_TEST_GH: join(bin, `gh${EXE}`) });
  const remote = join(home, 'origin.git');
  // This disposable fixture project does not create a branch/worktree in ROOT.
  for (const args of [['init', '-q', '-b', BRANCH],
    ['-c', 'user.name=fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '--allow-empty', '-q', '-m', 'fixture'],
    ['init', '--bare', '-q', remote], ['remote', 'add', 'origin', remote], ['push', '-q', 'origin', 'HEAD:develop'],
    ['symbolic-ref', 'refs/remotes/origin/HEAD', 'refs/remotes/origin/develop']]) {
    const run = spawnSync('git', args, { cwd: project, env, encoding: 'utf8', windowsHide: true });
    if (run.status !== 0) throw new Error(`Throwaway project setup failed: ${run.stderr}`);
  }
  await writeFile(join(home, '.codex/auth.json'), JSON.stringify({ OPENAI_API_KEY: 'fixture-only' }));
  try {
    const runtime = join(homedir(), '.gwt/runtime');
    await access(runtime);
    await symlink(runtime, join(state, 'runtime'), WINDOWS ? 'junction' : 'dir');
  } catch (error) { if (error.code !== 'ENOENT') throw error; }
  function rpc(operation, params = {}) {
    const run = spawnSync(gwtd, [], { cwd: project, env, encoding: 'utf8', timeout: 30000, windowsHide: true,
      input: JSON.stringify({ schema_version: 1, operation, params }) });
    if (run.status !== 0) throw new Error(`Isolated ${operation} failed: ${run.stdout} ${run.stderr}`);
    const envelope = JSON.parse(run.stdout);
    if (!envelope.ok) throw new Error(`Isolated ${operation} refused: ${run.stdout}`);
    return envelope;
  }
  function inspect(action = 'inspect') {
    const run = spawnSync(fixtureBinaries.seed, [action, project, String(OWNER)],
      { cwd: project, env, encoding: 'utf8', timeout: 30000, windowsHide: true });
    if (run.status !== 0) throw new Error(`Fixture ${action} failed: ${run.stderr}`);
    return JSON.parse(run.stdout);
  }
  inspect('seed');
  const hash = rpc('issue.monitor.status').project_store.hash;
  const preferences = join(state, 'projects', hash, 'project-state');
  await mkdir(preferences, { recursive: true });
  await writeFile(join(preferences, 'pm.json'), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(preferences, 'issue-monitor.json'), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] }));
  await writeFile(join(state, 'session.json'), JSON.stringify({
    tabs: [{ id: 'fixture', title: 'fixture', project_root: project, kind: 'git' }], active_tab_id: 'fixture', recent_projects: [],
  }));
  await mkdir(join(project, '.codex'));
  await writeFile(join(project, '.codex/hooks.json'), '{}');
  const audits = [];
  function hooks(repair) {
    const operation = repair ? 'hook.doctor' : 'hook.health';
    let envelope = rpc(operation, { ...(repair ? { repair: true } : {}),
      expected_hook_bin: 'gwtd', runtime_state_path: join(state, 'missing-runtime.json') });
    if (repair) {
      audits.push({ operation, result: envelope.output });
      const trust = rpc('hook.register_codex_managed_hook_trust', {
        project_root: project, codex_config: join(home, '.codex/config.toml'), codex_hook_discovery: 'both',
      });
      audits.push({ operation: 'hook.register_codex_managed_hook_trust', result: trust.output });
      envelope = rpc('hook.health', { expected_hook_bin: 'gwtd', runtime_state_path: join(state, 'missing-runtime.json') });
    }
    const value = typeof envelope.output === 'string' ? JSON.parse(envelope.output) : envelope.output;
    const health = value;
    const blocking = (health.issues || []).filter(issue => !(issue.startsWith('managed hook binary missing: ') && issue.endsWith(' uses gwtd'))
      && !(issue.startsWith('managed hook failure: ') && /state=fail-open(?: |$)/.test(issue)));
    if (health.status === 'inactive' || blocking.length) throw new Error(`Hook audit failed: ${JSON.stringify(health)}`);
    audits.push({ operation: 'hook.health', health });
  }
  hooks(true);
  const logPath = join(home, 'gwt.log');
  const urlPath = join(home, 'url.txt');
  const stream = createWriteStream(logPath);
  const host = spawn(gwt, ['--no-tray', '--no-open'], { cwd: project,
    env: { ...env, GWT_BROWSER_URL_FILE: urlPath }, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
  host.stdout.pipe(stream, { end: false });
  host.stderr.pipe(stream, { end: false });
  host.on('close', () => stream.end());
  const launches = async () => (await readFile(join(home, 'provider-launches.jsonl'), 'utf8').catch(() => ''))
    .split('\n').filter(Boolean).map(line => JSON.parse(line));
  async function stop() {
    if (host.exitCode === null && host.signalCode === null) host.kill('SIGTERM');
    for (const entry of await launches()) {
      if (exactProviderProcess(entry.pid, join(bin, `codex${EXE}`))) {
        try { process.kill(entry.pid, 'SIGTERM'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
      }
    }
    await expect.poll(() => host.exitCode !== null || host.signalCode !== null, { timeout: 10000 }).toBe(true);
    await info.attach('gwt-startup-log', { path: logPath, contentType: 'text/plain' });
    await info.attach('hook-audits', { body: JSON.stringify(audits), contentType: 'application/json' });
  }
  try {
    let url = '';
    const deadline = Date.now() + 60000;
    while (Date.now() < deadline) {
      if (host.exitCode !== null) throw new Error(`Fresh gwt exited: ${await readFile(logPath, 'utf8')}`);
      try {
        const candidate = (await readFile(urlPath, 'utf8')).trim();
        if (candidate && (await fetch(candidate, { method: 'HEAD' })).ok) { url = candidate; break; }
      } catch {}
      await delay(100);
    }
    if (!url) throw new Error(`Fresh gwt readiness timed out: ${logPath}`);
    hooks(false);
    await info.attach('isolated-fixture', { body: JSON.stringify({ home, project, url, logPath, host_pid: host.pid, checkout: ROOT }),
      contentType: 'application/json' });
    return { home, project, url, inspect, launches, stop,
      providerAlive: pid => exactProviderProcess(pid, join(bin, `codex${EXE}`)),
      ready: session => writeFile(join(home, `ready-${session}`), 'ready') };
  } catch (error) { await stop(); throw error; }
}

async function wizardState(page, after) {
  return (await page.waitForFunction(after => {
    const message = window.__gwtPlaywrightMessages?.findLast(entry => entry.sequence > after && entry.payload.kind === 'launch_wizard_state'
      && (!entry.payload.wizard || (!entry.payload.wizard.is_hydrating && !entry.payload.wizard.runtime_resolution_pending
        && !entry.payload.wizard.launch_materialization_pending)));
    return message ? { wizard: message.payload.wizard } : null;
  }, after, { timeout: 30000 })).jsonValue();
}
async function cursor(page) { return page.evaluate(() => window.__gwtPlaywrightMessageSequence || 0); }
async function action(page, action) {
  const after = await cursor(page);
  await sendLiveGwtEvent(page, { kind: 'launch_wizard_action', action, bounds: { x: 100, y: 100, width: 700, height: 500 } });
  return wizardState(page, after);
}
async function launch(page) {
  const after = await cursor(page);
  await openLiveLaunchWizardForBranch(page, BRANCH);
  await wizardState(page, after);
  await action(page, { kind: 'set_launch_path', path: 'manual_setup' });
  await action(page, { kind: 'set_agent', agent_id: 'codex' });
  await action(page, { kind: 'set_execution_mode', mode: 'normal' });
  await action(page, { kind: 'set_linked_issue', issue_number: OWNER });
  let state = await action(page, { kind: 'set_skip_permissions', enabled: true });
  for (let step = 0; step < 12 && state.wizard; step++) {
    expect(state.wizard.error).toBeFalsy();
    if (state.wizard.selected_runtime_target !== 'host' && state.wizard.runtime_target_options?.some(option => option.value === 'host')) {
      state = await action(page, { kind: 'set_runtime_target', target: 'Host' });
    } else {
      expect(state.wizard.primary_action_enabled, state.wizard.primary_action_disabled_reason).toBe(true);
      state = await action(page, { kind: 'submit' });
    }
  }
  expect(state.wizard).toBeNull();
}
async function windows(page) {
  return page.evaluate(() => {
    const state = window.__gwtPlaywrightMessages?.findLast(entry => entry.payload.kind === 'workspace_state');
    return (state?.payload.workspace.tabs || []).flatMap(tab => tab.workspace.windows).filter(entry => entry.agent_id === 'codex');
  });
}
const latestAttempt = state => state.ledger.continuation_attempts.at(-1);

test.describe('Manual successor retry (isolated checkout)', () => {
  test.skip(!OPT_IN || !['win32', 'darwin'].includes(process.platform), 'Set GWT_PLAYWRIGHT_MANUAL_SUCCESSOR_RETRY=1 on macOS/Windows after building gwt/gwtd');
  test.setTimeout(240000);
  test.beforeAll(async () => { fixtureBinaries = await compileFixtures(); });

  test('close Prepared candidate and launch a new same-owner successor through Ready', async ({ page }, info) => {
    const fixture = await startFixture(info);
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    try {
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      const theme = info.project.name.includes('light') ? 'light' : 'dark';
      await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
      const predecessor = fixture.inspect();
      await launch(page);
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 60000 }).toBe(1);
      const first = (await fixture.launches())[0];
      await expect.poll(async () => (await windows(page)).find(window => window.session_id === first.session_id), { timeout: 60000 }).toBeTruthy();
      const pane = (await windows(page)).find(window => window.session_id === first.session_id);
      const firstAttempt = latestAttempt(fixture.inspect());
      expect(firstAttempt.status).toBe('prepared');
      expect(fixture.inspect().binding).toEqual(predecessor.binding);
      await info.attach(`prepared-${theme}`, { body: await page.screenshot(), contentType: 'image/png' });
      await sendLiveGwtEvent(page, { kind: 'close_window', id: pane.id });
      await expect.poll(async () => (await windows(page)).some(window => window.id === pane.id), { timeout: 10000 }).toBe(false);
      await expect.poll(() => latestAttempt(fixture.inspect()).status, { timeout: 30000 }).toBe('aborted');
      const aborted = fixture.inspect();
      expect(aborted.binding).toEqual(predecessor.binding);
      expect(aborted.ledger.generations).toHaveLength(1);
      for (const path of [join(fixture.home, '.gwt/sessions', `${first.session_id}.toml`),
        join(fixture.home, '.gwt/sessions/execution-launch-recovery', `${first.session_id}.json`)]) {
        await expect.poll(async () => { try { await access(path); return true; } catch { return false; } }, { timeout: 30000 }).toBe(false);
      }
      await expect.poll(() => fixture.providerAlive(first.pid), { timeout: 10000 }).toBe(false);

      await launch(page);
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 60000 }).toBe(2);
      const second = (await fixture.launches())[1];
      const nextAttempt = latestAttempt(fixture.inspect());
      expect(nextAttempt.status).toBe('prepared');
      expect(nextAttempt.request.operation_id).not.toBe(firstAttempt.request.operation_id);
      expect(nextAttempt.candidate_generation_id).not.toBe(firstAttempt.candidate_generation_id);
      expect(second.session_id).not.toBe(first.session_id);
      expect(nextAttempt.request.initial_session_id).toBe(second.session_id);
      expect(fixture.inspect().binding).toEqual(predecessor.binding);
      await expect.poll(async () => (await windows(page)).find(window => window.session_id === second.session_id), { timeout: 60000 }).toBeTruthy();
      await info.attach(`relaunched-prepared-${theme}`, { body: await page.screenshot(), contentType: 'image/png' });
      await fixture.ready(second.session_id);
      await expect.poll(() => {
        try {
          const state = fixture.inspect();
          return { status: latestAttempt(state).status, current_generation_id: state.ledger.current_generation_id,
            binding_generation_id: state.binding?.generation_id, record_status: state.record?.status, primary_session_id: state.record?.primary_session_id };
        } catch (error) {
          // Publication may briefly expose the new ledger before its pointer.
          if (!error.message.startsWith('Fixture inspect failed: ')
            || !error.message.includes('execution generation pointer/projection is stale, missing, or mismatched')) throw error;
          return { pending_read_error: error.message };
        }
      }, { timeout: 60000 }).toEqual({ status: 'activated', current_generation_id: nextAttempt.candidate_generation_id,
        binding_generation_id: nextAttempt.candidate_generation_id, record_status: 'active', primary_session_id: second.session_id });
      const activated = fixture.inspect();
      expect(activated.ledger.generations).toHaveLength(2);
      expect(activated.ledger.continuation_attempts.map(attempt => attempt.status)).toEqual(['prepared', 'aborted', 'prepared', 'activated']);
      await expect.poll(async () => {
        try {
          return JSON.parse(await readFile(join(fixture.home, `hook-${second.session_id}.json`), 'utf8')).success;
        } catch (error) {
          if (error.code === 'ENOENT' || error instanceof SyntaxError) return false;
          throw error;
        }
      }, { timeout: 30000 }).toBe(true);
      await info.attach(`active-${theme}`, { body: await page.screenshot(), contentType: 'image/png' });
      await info.attach('final-ledger', { body: JSON.stringify(activated), contentType: 'application/json' });
      expect(errors, 'real backend console/page errors').toEqual([]);
    } finally { await fixture.stop(); }
  });
});
