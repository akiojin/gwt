import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import { createWriteStream } from "node:fs";
import { access, copyFile, mkdir, mkdtemp, readFile, readdir, realpath, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import type { TestInfo } from "@playwright/test";

// Rust's bare-command search on Windows requires a PE executable for git/gh;
// the provider itself uses the production-supported .cmd launch route.
export async function writeFixtureNodeCommand(bin: string, name: string, source: string, native = false) {
  const windows = process.platform === "win32";
  const script = join(bin, windows ? `${name}.cjs` : name);
  // Issue #5028: close writable fixture FDs in a child before any PTY spawn.
  const written = spawnSync(process.execPath, ["-e",
    "const fs=require('node:fs');fs.writeFileSync(process.argv[1],fs.readFileSync(0));fs.chmodSync(process.argv[1],0o755);",
    script], { input: source, encoding: "utf8", windowsHide: true });
  if (written.error) throw written.error;
  if (written.status !== 0) throw new Error(`Fixture executable writer failed: ${written.stderr}`);
  if (!windows) return script;
  if (!native) {
    const command = join(bin, `${name}.cmd`);
    await writeFile(command, `@echo off\r\n"${process.execPath}" "%~dp0${name}.cjs" %*\r\n`);
    return command;
  }
  const adapter = join(bin, "fixture-node.exe");
  if (!await access(adapter).then(() => true, () => false)) {
    const rust = join(bin, "fixture-node.rs");
    await writeFile(rust, `fn main() {
    let script = std::env::current_exe().unwrap().with_extension("cjs");
    let status = std::process::Command::new(${JSON.stringify(process.execPath)})
        .arg(script).args(std::env::args_os().skip(1)).status().expect("fixture Node command");
    std::process::exit(status.code().unwrap_or(1));
}\n`);
    const built = spawnSync("rustc", ["--crate-name", "fixture_node", "-C", "debuginfo=0", rust, "-o", adapter],
      { encoding: "utf8", windowsHide: true });
    if (built.error) throw built.error;
    if (built.status !== 0) throw new Error(`Fixture native adapter failed: ${built.stderr}`);
  }
  const command = join(bin, `${name}.exe`);
  await copyFile(adapter, command);
  return command;
}

export type ExactRelaunchInvocation = {
  pid: number;
  argv: string[];
  cwd: string;
  gwt_session_id: string | null;
  provider_session_id: string;
  session_id_argument: string | null;
  resume_session_id: string | null;
  at: string;
};

/** Real checkout/backend/PTY; only the provider executable is a recorder.
 * Never fabricates durable gwt Sessions, execution ledgers, or Work records.
 * The optional source actor uses the provider's ordinary authenticated commands.
 * Each invocation owns a fresh HOME, retained as evidence after cleanup.
 */
export type ExactRelaunchSetup = {
  freshNativeIds?: boolean;
  terminalConvergence?: boolean;
  switching?: { sourceIssueNumber: number; positive: boolean };
  prepare?: (context: { home: string; bin: string; project: string; env: NodeJS.ProcessEnv }) => Promise<void>;
  monitorPreferences?: Record<string, unknown>;
  beforeStart?: (context: { home: string; project: string; env: NodeJS.ProcessEnv; preferences: string }) => Promise<void>;
};

export async function startExactRelaunchFixture(testInfo: TestInfo, setup: ExactRelaunchSetup = {}) {
  const windows = process.platform === "win32";
  const suffix = windows ? ".exe" : "";
  const root = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
  const gwt = join(root, `target/debug/gwt${suffix}`);
  const gwtd = join(root, `target/debug/gwtd${suffix}`);
  await Promise.all([access(gwt), access(gwtd)]);
  const home = await realpath(await mkdtemp(join(tmpdir(), "gwt-exact-relaunch-")));
  const bin = join(home, "bin");
  const state = join(home, ".gwt");
  const argvLog = join(home, "provider-argv.jsonl");
  const nativeId = randomUUID();
  const env: NodeJS.ProcessEnv = { ...process.env };
  // No ambient parent Session, capability, Git selection, or credentials may
  // enter this independent instance, including its initial Git operations.
  for (const key of Object.keys(env)) {
    if (/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(key)
      || ["GH_TOKEN", "GITHUB_TOKEN", "OPENAI_API_KEY", "ANTHROPIC_API_KEY"].includes(key.toUpperCase())
      || key.toUpperCase() === "PATH") delete env[key];
  }
  Object.assign(env, {
    HOME: home, USERPROFILE: home, CODEX_HOME: join(home, ".codex"),
    PATH: [bin, process.env.PATH ?? process.env.Path ?? ""].join(delimiter),
    GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1",
    GWT_HOOK_BIN: "gwtd", GWT_PROJECT_ROOT: join(home, "project"),
    GWT_DISABLE_BACKGROUND_INDEX: "1",
  });
  await Promise.all([mkdir(bin), mkdir(state), mkdir(join(home, ".codex"))]);
  await writeFile(argvLog, "");
  // The project is a throwaway repository, not this checkout: startup restore
  // refuses a worktree whose HEAD already landed on origin/develop, and the
  // launch materializes hooks into the project, which must not touch the checkout.
  const project = join(home, "project");
  const branch = "main";
  await mkdir(project);
  const git = (...args: string[]) => spawnSync("git", args, { cwd: project, env, encoding: "utf8", windowsHide: true });
  for (const args of [["init", "-q", "-b", branch], ["-c", "user.name=gwt", "-c", "user.email=gwt@example.invalid",
    "commit", "-q", "--allow-empty", "-m", "exact relaunch fixture"]]) {
    const result = git(...args);
    if (result.status !== 0) throw new Error(`Fixture project git ${args[0]} failed: ${result.stderr}`);
  }

  // This actor follows the Host's instruction with its own ambient bridge.
  // The runner receives observations, never the launch capability or an ACK handle.
  const switchActor = setup.switching ? `const switching=${JSON.stringify(setup.switching)};\n` + String.raw`
const cp = require('node:child_process');
const sourceDirectory = process.cwd();
const idleDirectory = path.join(process.env.HOME, 'provider-idle', process.env.GWT_SESSION_ID);
const actorLog = path.join(process.env.HOME, 'switch-actor.jsonl');
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
const actorEvent = (stage, extra = {}) => fs.appendFileSync(actorLog, JSON.stringify({stage,
  pid:process.pid,gwt_session_id:process.env.GWT_SESSION_ID,provider_session_id:native,cwd:process.cwd(),...extra})+'\n');
const observedRuntime = () => {
  const bytes = fs.readFileSync(process.env.GWT_SESSION_RUNTIME_PATH);
  const runtime = JSON.parse(bytes);
  return {runtime_status:runtime.status,current_hook_event:runtime.current_hook_event ?? null,
    runtime_sha256:require('node:crypto').createHash('sha256').update(bytes).digest('hex')};
};
const sourceEnvelope = (operation, params = {}) => {
  const result = cp.spawnSync(process.env.GWT_BIN_PATH, [], {cwd:sourceDirectory,env:process.env,
    encoding:'utf8',timeout:60000,input:JSON.stringify({schema_version:1,operation,params})});
  let envelope;
  try { envelope = JSON.parse(result.stdout); } catch {}
  if (result.status !== 0 || !envelope?.ok) {
    actorEvent('operation_refused', {operation,exit_status:result.status});
    throw new Error('Source operation refused: '+operation);
  }
  return envelope;
};
const sourceOperation = (operation, params = {}) => {
  const output = sourceEnvelope(operation,params).output;
  if (typeof output !== 'string') return output;
  try { return JSON.parse(output); } catch { return output; }
};
const git = (...gitArgs) => {
  const result = cp.spawnSync('git',gitArgs,{cwd:sourceDirectory,env:process.env,encoding:'utf8',timeout:60000});
  if (result.status !== 0) throw new Error('Source git failed: '+gitArgs[0]);
  return result.stdout.trim();
};
const nativeHook = (event, prompt) => {
  startupEvent('native_hook_begin',{event});
  const hooks = JSON.parse(fs.readFileSync(path.join(sourceDirectory,'.codex','hooks.json'),'utf8'));
  const input = JSON.stringify({session_id:native,hook_event_name:event,cwd:sourceDirectory,
    transcript_path:rollout,prompt,stop_hook_active:false});
  for (const group of hooks.hooks?.[event] ?? []) for (const hook of group.hooks ?? []) {
    if (hook.type !== 'command') continue;
    const result = cp.spawnSync('/bin/sh',['-c',hook.command],{cwd:sourceDirectory,env:process.env,
      input,encoding:'utf8',timeout:60000});
    actorEvent('native_hook_result',{event,exit_status:result.status,signal:result.signal,
      session_lease_contended:/Session lease is held by another gwt operation/.test(result.stderr ?? '')});
    if (result.status !== 0 || /"decision"\s*:\s*"block"/.test(result.stdout)) {
      throw new Error('Native hook refused: '+event);
    }
  }
  actorEvent('native_hook',{event});
  startupEvent('native_hook_returned',{event});
};
const observe = async (read, description) => {
  const deadline = Date.now()+60000;
  while (Date.now()<deadline) {
    const value = read();
    if (value) return value;
    await pause(100);
  }
  throw new Error('Source observation timed out: '+description);
};
const actorReady = (async () => {
  startupEvent('execution_status_begin');
  const owner = sourceOperation('execution.status').owner_number;
  startupEvent('execution_status_returned');
  nativeHook('UserPromptSubmit',args.at(-1));
  if (owner === switching.sourceIssueNumber && !fs.existsSync(path.join(sourceDirectory,'source-in-progress.txt'))) {
    fs.writeFileSync(path.join(sourceDirectory,'source-in-progress.txt'),'Unfinished source implementation awaiting its next step.\n');
    git('add','--','source-in-progress.txt');
    git('-c','user.name=Switch fixture','-c','user.email=switch@example.invalid','commit','-m','fixture: unfinished source progress');
  }
  sourceOperation('issue.monitor.wait',{number:owner,reason:'Awaiting fixture operator',resume_condition:'Operator requests checkpoint or resumes current work'});
  nativeHook('Stop');
  const normal = observedRuntime();
  actorEvent('normal_n1',normal);
  if (normal.current_hook_event?.event !== 'Stop' || normal.current_hook_event?.outcome !== 'normal')
    throw new Error('Genuine initial Stop did not establish current Normal observation');
  let treeChild;
  if (switching.positive && owner === switching.sourceIssueNumber) {
    fs.mkdirSync(idleDirectory,{recursive:true});
    treeChild = cp.spawn(process.execPath,['-e','setInterval(()=>{},1000)','GWT_SWITCH_TREE_CHILD',process.env.HOME],
      {cwd:idleDirectory,env:{PATH:process.env.PATH,HOME:process.env.HOME},stdio:'ignore'}).pid;
  }
  actorEvent('idle',{owner_number:owner,tree_child_pid:treeChild});
})().catch(error=>{actorEvent('actor_failed',{message:error.message});throw error;});
// Prevent an unhandled-rejection exit while stdin is waiting for the real Host instruction.
actorReady.catch(()=>{});
const checkpointed = new Set();
const checkpoint = async line => {
  await actorReady;
  const operation = /GWT checkpoint request ([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/.exec(line)?.[1];
  if (!operation || checkpointed.has(operation)) return;
  checkpointed.add(operation);
  process.chdir(sourceDirectory);
  actorEvent('checkpoint_before_prompt',{operation,...observedRuntime()});
  nativeHook('UserPromptSubmit',line);
  actorEvent('checkpoint_activity',{operation,...observedRuntime()});
  const intent = sourceOperation('issue.monitor.switch.read').intents.find(value=>value.operation_id===operation);
  if (!intent || intent.stage !== 'checkpointing') {
    sourceOperation('issue.monitor.wait',{number:switching.sourceIssueNumber,clear:true});
    nativeHook('Stop');
    actorEvent('checkpoint_cancelled',{operation});
    return;
  }
  actorEvent('checkpoint_received',{operation});
  sourceOperation('board.post',{kind:'handoff',owners:[String(switching.sourceIssueNumber)],
    body:'Saved current source checkpoint '+operation+'\nNext action: resume the unfinished source implementation.'});
  const entry = sourceOperation('board.show',{all:true}).board.entries.find(value=>
    value.origin_session_id===process.env.GWT_SESSION_ID && value.body.split(/\s+/).includes(operation));
  if (!entry) throw new Error('Own checkpoint Board entry missing');
  const hash = sourceEnvelope('issue.monitor.status').project_store.hash;
  const worksPath = path.join(process.env.HOME,'.gwt','projects',hash,'project-state','works.json');
  const work = await observe(()=>JSON.parse(fs.readFileSync(worksPath,'utf8')).work_items.find(value=>
    value.events.some(event=>event.agent_session_id===process.env.GWT_SESSION_ID)
    && value.board_refs.includes(entry.id)),'canonical Board link');
  sourceOperation('workspace.update',{status:'idle',summary:'Saved source checkpoint '+operation,
    progress_summary:'Saved the actual source and immutable Work shards',next_action:'Resume the unfinished source implementation'});
  fs.writeFileSync(path.join(sourceDirectory,'checkpoint-source.txt'),'Saved source checkpoint '+operation+'\n');
  git('add','--all');
  git('-c','user.name=Switch fixture','-c','user.email=switch@example.invalid','commit','-m','fixture: saved source checkpoint '+operation);
  git('push','-u','origin','HEAD');
  const head = git('rev-parse','HEAD');
  sourceOperation('issue.monitor.switch.ack',{operation_id:operation,head,work_id:work.id,board_entry_id:entry.id});
  actorEvent('checkpoint_acked',{operation,head,work_id:work.id,board_entry_id:entry.id});
  sourceOperation('issue.monitor.wait',{number:switching.sourceIssueNumber,reason:'Saved checkpoint awaits switch',resume_condition:'Switch cancelled or saved work resumed'});
  if (switching.positive) {
    actorEvent('awaiting_normal_n2',{operation,...observedRuntime()});
    await observe(()=> {
      const release = path.join(process.env.HOME,'normal-n2-'+operation+'.json');
      return fs.existsSync(release) && JSON.parse(fs.readFileSync(release,'utf8')).operation === operation;
    },'genuine checkpoint Stop release');
  }
  // Record cwd before Stop publishes Idle: the Host may then exit this actor.
  actorEvent('checkpoint_idle',{operation});
  actorEvent('normal_n2_started',{operation});
  nativeHook('Stop');
  actorEvent('normal_n2_hook_returned',{operation});
};
` : "";

  const terminalActor = setup.terminalConvergence ? String.raw`
const terminalOperation = (operation, params = {}) => {
  const result = require('node:child_process').spawnSync(process.env.GWT_BIN_PATH, [], {
    cwd:process.cwd(),env:process.env,encoding:'utf8',timeout:60000,windowsHide:true,
    input:JSON.stringify({schema_version:1,operation,params}),
  });
  const envelope = JSON.parse(result.stdout || '{}');
  if (result.status !== 0 || !envelope.ok) throw new Error(operation + ': ' + result.stdout + result.stderr);
  return envelope;
};
const terminalHook = (event, prompt) => {
  const hooks = JSON.parse(fs.readFileSync(path.join(process.cwd(), '.codex/hooks.json'), 'utf8'));
  const input = JSON.stringify({session_id:native,hook_event_name:event,cwd:process.cwd(),
    transcript_path:rollout,prompt,stop_hook_active:false});
  for (const group of hooks.hooks?.[event] ?? []) for (const hook of group.hooks ?? []) {
    if (hook.type !== 'command') continue;
    const windows = process.platform === 'win32';
    const result = require('node:child_process').spawnSync(windows ? (process.env.ComSpec || 'cmd.exe') : '/bin/sh',
      windows ? ['/d','/s','/c',hook.command] : ['-c',hook.command],
      {input,encoding:'utf8',timeout:60000,windowsHide:true,windowsVerbatimArguments:windows});
    if (result.status !== 0 || /"decision"\s*:\s*"block"/.test(result.stdout || ''))
      throw new Error(event + ' refused: ' + result.stdout + result.stderr);
  }
};
const prepareTerminalWarning = () => {
  terminalHook('UserPromptSubmit', 'Prepare the closed owner terminal convergence fixture');
  const cp = require('node:child_process');
  const git = (...args) => {
    const result = cp.spawnSync('git', args, {cwd:process.cwd(),env:process.env,encoding:'utf8',windowsHide:true});
    if (result.status !== 0) throw new Error('fixture git failed: ' + result.stderr);
    return result.stdout.trim();
  };
  terminalOperation('pane.name');
  terminalOperation('workspace.update', {purpose:'Closed owner convergence fixture'});
  const branch = git('branch', '--show-current');
  // The remote is this fixture's private bare repository. Preserve its source;
  // point only this branch's bookkeeping readback at an absent remote ref.
  git('config', 'branch.' + branch + '.remote', 'origin');
  git('config', 'branch.' + branch + '.merge', 'refs/heads/terminal-warning-missing');
  terminalOperation('workspace.update', {status:'done',summary:'Delivered owner has zero source changes'});
  fs.writeFileSync(path.join(process.env.HOME, 'terminal-warning-prepared-' + process.env.GWT_SESSION_ID + '.json'), '{}');
};
const completeTerminalWarning = () => {
  terminalOperation('execution.no_action', {reason:'The private integration base already contains this owner source'});
  const diagnosis = JSON.parse(terminalOperation('execution.status').output);
  const hash = terminalOperation('issue.monitor.status').project_store.hash;
  let normalized = fs.realpathSync(process.cwd()).replaceAll('\\', '/').replace(/\/$/, '');
  if (process.platform === 'win32') normalized = normalized.toLowerCase();
  const crypto = require('node:crypto');
  const key = crypto.createHash('sha256').update(normalized).digest('hex').slice(0,16);
  const trusted = path.join(process.env.HOME, '.gwt/projects', hash, 'trusted', key);
  const paths = ['execution-control.json', 'work-event-settlement.json', 'execution-no-action.json']
    .map(name => path.join(trusted, name));
  const shards = path.join(process.cwd(), '.gwt/work/events');
  for (const bucket of fs.readdirSync(shards)) {
    if (!/^[0-9a-f]{2}$/.test(bucket)) continue;
    for (const name of fs.readdirSync(path.join(shards, bucket)))
      if (/^[0-9a-f]{64}\.jsonl$/.test(name)) paths.push(path.join(shards, bucket, name));
  }
  // Stop canonically refreshes the receipt. Only the later observer close is
  // required to preserve these files, so capture its baseline after Stop.
  terminalHook('Stop');
  const hashes = Object.fromEntries(paths.map(file =>
    [file, crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')]));
  fs.writeFileSync(path.join(process.env.HOME, 'terminal-warning-' + process.env.GWT_SESSION_ID + '.json'),
    JSON.stringify({diagnosis,hashes}));
  console.log('GWT_FIXTURE_TERMINAL_WARNING_READY');
};
const terminalActorFailure = error => {
  fs.writeFileSync(path.join(process.env.HOME, 'terminal-warning-failed-' + process.env.GWT_SESSION_ID + '.json'),
    JSON.stringify({error:error.message}));
  console.error('GWT_FIXTURE_TERMINAL_WARNING_FAILED ' + error.message);
};
` : "";
  // The fake provider owns only native conversation metadata. In particular,
  // its native conversation UUID is independent from GWT_SESSION_ID.
  const startupRecorder = setup.switching
    ? `const startupEvent = (stage, extra = {}) => fs.appendFileSync(${JSON.stringify(join(home, "switch-startup.jsonl"))}, JSON.stringify({stage,pid:process.pid,gwt_session_id:process.env.GWT_SESSION_ID,home_matches_fixture:process.env.HOME===${JSON.stringify(home)},codex_home_matches_fixture:process.env.CODEX_HOME===${JSON.stringify(join(home, ".codex"))},...extra})+'\\n');`
    : "const startupEvent = () => {};";
  const provider = `#!${process.execPath}
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');
const args = process.argv.slice(2);
if (args.includes('--version') || args.includes('-V')) {
  console.log('codex-cli 0.116.0'); process.exit(0);
}
// Configuration preflight is not an interactive provider session.
const configOptions = ['--config','-c','--profile','-p','--enable','--disable'];
let configEnd = 0;
while (configEnd < args.length - 2) {
  if (configOptions.includes(args[configEnd])) configEnd += 2;
  else if (/^--(?:config|profile|enable|disable)=/.test(args[configEnd])) configEnd += 1;
  else break;
}
if (configEnd === args.length - 2 && args.at(-2) === 'features' && args.at(-1) === 'list') {
  process.exit(0);
}
const option = name => {
  const index = args.indexOf(name);
  return index < 0 ? (args.find(a => a.startsWith(name + '='))?.slice(name.length + 1) ?? null)
    : (args[index + 1] ?? null);
};
const resumeAt = args.indexOf('resume');
const resumed = resumeAt < 0 ? null : (args.slice(resumeAt + 1).find(a => /^[0-9a-f]{8}-[0-9a-f-]{27}$/i.test(a)) ?? null);
const requested = option('--session-id');
const native = resumed || requested || ${setup.freshNativeIds ? "require('node:crypto').randomUUID()" : JSON.stringify(nativeId)};
const at = new Date().toISOString();
const directory = path.join(process.env.HOME, '.codex', 'sessions', ...at.slice(0,10).split('-'));
fs.mkdirSync(directory, {recursive:true});
const rollout = path.join(directory, 'rollout-' + at.replaceAll(':','-') + '-' + native + '.jsonl');
fs.writeFileSync(rollout, JSON.stringify({timestamp:at,type:'session_meta',payload:{id:native,timestamp:at,cwd:process.cwd(),originator:'codex_cli_rs',cli_version:'0.116.0',source:'cli',model_provider:'openai'}}) + '\\n');
fs.appendFileSync(${JSON.stringify(argvLog)}, JSON.stringify({pid:process.pid,argv:args,cwd:process.cwd(),gwt_session_id:process.env.GWT_SESSION_ID ?? null,provider_session_id:native,session_id_argument:requested,resume_session_id:resumed,at}) + '\\n');
// Like the real provider, report the native conversation to gwt through the
// SessionStart hooks materialized in the worktree; this is how gwt learns the
// exact resume handle (it never derives it from rollout files).
${startupRecorder}
startupEvent('session_start_begin');
try {
  const hooks = JSON.parse(fs.readFileSync(path.join(process.cwd(), '.codex', 'hooks.json'), 'utf8'));
  const input = JSON.stringify({session_id:native,hook_event_name:'SessionStart',source:resumed ? 'resume' : 'startup',cwd:process.cwd(),transcript_path:rollout,model:option('--model')});
  for (const group of hooks.hooks?.SessionStart ?? []) for (const hook of group.hooks ?? []) {
    if (hook.type !== 'command') continue;
    const windows = process.platform === 'win32';
    startupEvent('session_start_hook_begin');
    const result = require('node:child_process').spawnSync(windows ? (process.env.ComSpec || 'cmd.exe') : '/bin/sh',
      windows ? ['/d','/s','/c',hook.command] : ['-c',hook.command],
      {input, stdio:['pipe','ignore','inherit'], timeout:60000, windowsHide:true, windowsVerbatimArguments:windows});
    startupEvent('session_start_hook_returned',{exit_status:result.status,signal:result.signal,error_code:result.error?.code ?? null});
    if (result.error || result.status !== 0) throw new Error('SessionStart hook failed: ' + (result.error?.message || result.status));
  }
} catch (error) { startupEvent('session_start_failed'); console.error('GWT_FAKE_PROVIDER_HOOK_ERROR ' + error.message); }
startupEvent('session_start_returned');
${switchActor}
${terminalActor}
console.log('GWT_FAKE_PROVIDER_READY ' + native);
readline.createInterface({input:process.stdin}).on('line', line => {
  ${setup.switching ? "if (line.includes('GWT checkpoint request ')) { checkpoint(line).catch(error=>actorEvent('actor_failed',{message:error.message})); return; }" : ""}
  if (line.trim() === 'GWT_FIXTURE_EXIT') process.exit(0);
  ${setup.terminalConvergence ? "if (line.trim() === 'GWT_FIXTURE_PREPARE_TERMINAL_WARNING') { try { prepareTerminalWarning(); } catch (error) { terminalActorFailure(error); } return; }" : ""}
  ${setup.terminalConvergence ? "if (line.trim() === 'GWT_FIXTURE_TERMINAL_WARNING') { try { completeTerminalWarning(); } catch (error) { terminalActorFailure(error); } return; }" : ""}
  if (line.trim() === 'GWT_FIXTURE_DONE') {
    const result = require('node:child_process').spawnSync(process.env.GWT_BIN_PATH, [], {
      cwd:process.cwd(), env:process.env, encoding:'utf8', timeout:30000, windowsHide:true,
      input:JSON.stringify({schema_version:1,operation:'workspace.update',params:{status:'done'}}),
    });
    let done = false;
    try { done = result.status === 0 && JSON.parse(result.stdout).ok; } catch {}
    console.log(done ? 'GWT_FAKE_PROVIDER_DONE' : 'GWT_FAKE_PROVIDER_DONE_FAILED');
    return;
  }
  if (line.trim() === 'GWT_FIXTURE_BLOCK') {
    // Exercise the provider's real authenticated execution boundary. Never
    // copy its launch capability into the test runner or print the envelope.
    const result = require('node:child_process').spawnSync(process.env.GWT_BIN_PATH, [], {
      cwd:process.cwd(), env:process.env, encoding:'utf8', timeout:30000, windowsHide:true,
      input:JSON.stringify({schema_version:1,operation:'execution.blocked',params:{reason:'Fixture execution intentionally blocked'}}),
    });
    let blocked = false;
    try { blocked = result.status === 0 && JSON.parse(result.stdout).ok; } catch {}
    console.log(blocked ? 'GWT_FAKE_PROVIDER_BLOCKED' : 'GWT_FAKE_PROVIDER_BLOCK_FAILED');
    return;
  }
  console.log('GWT_FAKE_PROVIDER_READY ' + native);
});
`;
  await writeFixtureNodeCommand(bin, "codex", provider);
  // Hooks resolve `gwtd` through PATH when GWT_BIN_PATH is absent; it must be
  // the checkout build under test, never an installed binary.
  if (windows) await copyFile(gwtd, join(bin, `gwtd${suffix}`));
  else await symlink(gwtd, join(bin, "gwtd"));
  // An accidentally selected package runner must never contact a registry or
  // bypass the fixture's provider. Installed Codex is the intended test route.
  for (const runner of ["bunx", "npx", "npm"]) {
    await writeFixtureNodeCommand(bin, runner,
      `#!${process.execPath}\nconsole.error('Exact relaunch fixture requires installed Codex');process.exit(64);\n`);
  }
  try {
    const runtime = join(homedir(), ".gwt/runtime");
    await access(runtime);
    await symlink(runtime, join(state, "runtime"), windows ? "junction" : "dir");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  await setup.prepare?.({ home, bin, project, env });
  const status = spawnSync(gwtd, [], {
    cwd: project, env, encoding: "utf8", timeout: 30_000, windowsHide: true,
    input: JSON.stringify({ schema_version: 1, operation: "issue.monitor.status", params: {} }),
  });
  if (status.status !== 0) throw new Error(`Fixture project lookup failed: ${status.stderr}`);
  const envelope = JSON.parse(status.stdout);
  if (!envelope.ok) throw new Error("Fixture issue.monitor.status refused");
  // The envelope reports the operation's project store beside `output`.
  const hash = envelope.project_store?.hash;
  if (typeof hash !== "string" || !/^[a-zA-Z0-9_-]+$/.test(hash)) throw new Error("Missing canonical project_store.hash");
  const preferences = join(state, "projects", hash, "project-state");
  await mkdir(preferences, { recursive: true });
  await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
  await writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [], ...setup.monitorPreferences }));
  await writeFile(join(state, "session.json"), JSON.stringify({
    tabs: [{ id: "exact-relaunch", title: "exact-relaunch", project_root: project, kind: "git" }],
    active_tab_id: "exact-relaunch", recent_projects: [],
  }));

  await setup.beforeStart?.({ home, project, env, preferences });

  let child: ChildProcess | undefined;
  let incarnation = 0;
  let url = "";
  async function shutdownEvents() {
    const directory = join(state, "logs");
    const names = await readdir(directory).catch((error: NodeJS.ErrnoException) => {
      if (error.code === "ENOENT") return [];
      throw error;
    });
    const records = [];
    for (const name of names.filter(name => name.startsWith("gwt.log.")).sort()) {
      for (const line of (await readFile(join(directory, name), "utf8")).split("\n").filter(Boolean)) {
        const event = JSON.parse(line);
        if (event.target === "gwt::shutdown") records.push(event);
      }
    }
    return records;
  }
  async function launches(): Promise<ExactRelaunchInvocation[]> {
    return (await readFile(argvLog, "utf8")).split("\n").filter(Boolean).map(line => JSON.parse(line));
  }
  function processCommand(pid: number) {
    const result = windows
      ? spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
        `[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); (Get-CimInstance Win32_Process -Filter 'ProcessId = ${pid}').CommandLine`],
      { encoding: "utf8", windowsHide: true })
      : spawnSync("ps", ["-p", String(pid), "-o", "command="], { encoding: "utf8" });
    if (result.error) throw result.error;
    if (result.status !== 0 && (windows || result.status !== 1)) throw new Error(`Cannot inspect owned fixture PID ${pid}: ${result.stderr}`);
    return result.stdout.trim();
  }
  function processBirth(pid: number) {
    const result = windows
      ? spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command",
        `(Get-Process -Id ${pid} -ErrorAction Stop).StartTime.ToUniversalTime().Ticks`],
      { encoding: "utf8", windowsHide: true })
      : spawnSync("ps", ["-o", "lstart=", "-p", String(pid)], { encoding: "utf8" });
    if (result.error) throw result.error;
    return result.status === 0 ? result.stdout.trim() : "";
  }
  async function captureGuiOwner() {
    const directory = join(state, "projects", hash, "runtime/daemon");
    for (const name of await readdir(directory)) {
      if (!name.endsWith(".json")) continue;
      const endpointFile = join(directory, name), bytes = await readFile(endpointFile, "utf8");
      const endpoint = JSON.parse(bytes);
      if (endpoint.scope?.repo_hash !== hash || !endpoint.project_transport_url
        || await realpath(endpoint.scope.project_root) !== await realpath(project)) continue;
      if (!Number.isInteger(endpoint.pid) || endpoint.pid <= 0) throw new Error("Invalid fixture owner PID");
      const birth = processBirth(endpoint.pid);
      const response = await fetch(`${endpoint.project_transport_url}/private/project/command`, {
        method: "POST", headers: { Authorization: `Bearer ${endpoint.auth_token}`, "Content-Type": "application/json" },
        body: JSON.stringify({ project_key: hash, command: { type: "health" } }), signal: AbortSignal.timeout(15_000),
      });
      const health = await response.json();
      if (!response.ok || !birth || processBirth(endpoint.pid) !== birth
        || health.pid !== endpoint.pid || health.project_key !== hash
        || health.startup_intent !== "gui_managed" || !(health.gui_presence_count >= 1)
        || !(health.host_started_at > 0) || typeof health.host_instance_id !== "string" || !health.host_instance_id || health.draining
        || await readFile(endpointFile, "utf8") !== bytes) throw new Error("Fixture GUI owner identity changed before restart");
      return { pid: endpoint.pid as number, birth, host_started_at: health.host_started_at,
        host_instance_id: health.host_instance_id, project_key: hash,
        startup_intent: health.startup_intent, gui_presence_count: health.gui_presence_count };
    }
    throw new Error("Fixture has no authenticated GUI-managed owner to drain");
  }
  async function stopGui(signalFree = false) {
    let timedOut = false;
    if (child?.pid && child.exitCode === null && child.signalCode === null) {
      const current = child;
      if (!signalFree) current.kill("SIGTERM");
      const deadline = Date.now() + 8_000;
      while (current.exitCode === null && current.signalCode === null && Date.now() < deadline) await delay(100);
      if (current.exitCode === null && current.signalCode === null) {
        timedOut = true;
        current.kill("SIGKILL");
        const forcedDeadline = Date.now() + 8_000;
        while (current.exitCode === null && current.signalCode === null && Date.now() < forcedDeadline) await delay(100);
      }
    }
    if (child?.pid && processCommand(child.pid)) {
      throw new Error(`Fixture gwt process ${child.pid} remained after shutdown`);
    }
    if (signalFree && timedOut) throw new Error("Fixture signal-free GUI shutdown timed out");
  }
  async function stop(coldOwnerDrain = false, disconnectBrowser?: () => Promise<unknown>) {
    const failures: unknown[] = [];
    let owner: Awaited<ReturnType<typeof captureGuiOwner>> | undefined;
    let shutdownOffset = 0;
    if (coldOwnerDrain) {
      try {
        owner = await captureGuiOwner();
        shutdownOffset = (await shutdownEvents()).length;
      } catch (error) { failures.push(error); }
    }
    // Capture the live identity before disconnect can start its natural drain.
    try { await disconnectBrowser?.(); } catch (error) { failures.push(error); }
    const ownedPid = child?.pid;
    try { await stopGui(coldOwnerDrain); } catch (error) { failures.push(error); }
    if (coldOwnerDrain) {
      try {
        const events = (await shutdownEvents()).slice(shutdownOffset);
        const proof = {
          pid: ownedPid, exit_code: child?.exitCode, signal: child?.signalCode,
          shutdown_requested: events.some(event => event.fields?.message === "gui shutdown requested"
            && event.fields?.reason === "quit app"),
          cleanup_completed: events.some(event => event.fields?.message === "gui shutdown cleanup completed"
            && event.fields?.reason === "quit app"),
          backstop: events.some(event => event.fields?.message === "gui shutdown timed out; forcing process exit"),
          events,
        };
        await testInfo.attach("fixture-cold-gui-shutdown", { body: JSON.stringify(proof), contentType: "application/json" });
        if (proof.exit_code !== 0 || proof.signal !== null || !proof.shutdown_requested
          || !proof.cleanup_completed || proof.backstop) failures.push(new Error("Fixture cold GUI shutdown lacks normal-exit proof"));
      } catch (error) { failures.push(error); }
    }
    let drainError: Error | undefined;
    if (owner) {
      // Cold restore starts after the real GUI lifetime drain. Killing providers
      // during reconnect grace would instead leave intentional error diagnostics.
      const deadline = Date.now() + 45_000;
      while (processBirth(owner.pid) === owner.birth && Date.now() < deadline) await delay(100);
      const exited = processBirth(owner.pid) !== owner.birth;
      if (!exited) drainError = new Error(`Fixture owner ${owner.pid} did not drain after GUI shutdown`);
    }
    // PTYs can create their own process groups; clean only recorded fixture
    // providers still carrying our unique argv-recorder path in their command.
    const providerPaths = [join(bin, windows ? "codex.cjs" : "codex"), join(bin, windows ? "claude.cjs" : "claude")];
    const providerPids: number[] = [];
    for (const launch of await launches()) {
      const command = processCommand(launch.pid);
      if (providerPaths.some(path => command.includes(path))) {
        providerPids.push(launch.pid);
        try { process.kill(launch.pid, "SIGTERM"); } catch { /* exited */ }
      }
    }
    const providerCleanup = await Promise.allSettled(providerPids.map(async pid => {
      const deadline = Date.now() + 5_000;
      while (Date.now() < deadline) {
        const command = processCommand(pid);
        if (!providerPaths.some(path => command.includes(path))) return;
        await delay(100);
      }
      throw new Error(`Fixture provider ${pid} remained after shutdown`);
    }));
    for (const result of providerCleanup) if (result.status === "rejected") failures.push(result.reason);
    // Cleanup only descendants recorded by this actor and carrying its unique HOME.
    const actorEvents = (await readFile(join(home, "switch-actor.jsonl"), "utf8").catch(() => ""))
      .split("\n").filter(Boolean).map(line => JSON.parse(line));
    for (const event of actorEvents) {
      if (!event.tree_child_pid) continue;
      const command = spawnSync("ps", ["-p", String(event.tree_child_pid), "-o", "command="], { encoding: "utf8" });
      if (command.stdout.includes("GWT_SWITCH_TREE_CHILD") && command.stdout.includes(home)) {
        try { process.kill(event.tree_child_pid, "SIGTERM"); } catch { /* exited */ }
      }
    }
    if (ownedPid) {
      const remaining = processCommand(ownedPid);
      try {
        await testInfo.attach("fixture-process-cleanup", { body: JSON.stringify({ pid: ownedPid, exited: !remaining }), contentType: "application/json" });
      } catch (error) { failures.push(error); }
      if (remaining) failures.push(new Error(`Fixture gwt process ${ownedPid} remained after shutdown`));
    }
    child = undefined;
    if (owner) {
      try {
        await testInfo.attach("fixture-cold-owner-drain", {
          body: JSON.stringify({ ...owner, exited: !drainError }), contentType: "application/json",
        });
      } catch (error) { failures.push(error); }
    }
    if (drainError) failures.push(drainError);
    if (failures.length === 1) throw failures[0];
    if (failures.length) throw new AggregateError(failures, failures.map(String).join("; "));
  }
  async function start() {
    const urlFile = join(home, `url-${++incarnation}.txt`);
    const logFile = join(home, `gwt-${incarnation}.log`);
    const stream = createWriteStream(logFile);
    try {
      child = spawn(gwt, ["--no-tray", "--no-open"], {
        cwd: project, env: { ...env, GWT_BROWSER_URL_FILE: urlFile }, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
      });
      const current = child;
      let failure: Error | undefined;
      current.on("error", error => { failure = error; });
      current.stdout?.pipe(stream, { end: false });
      current.stderr?.pipe(stream, { end: false });
      current.on("close", () => stream.end());
      const deadline = Date.now() + 60_000;
      while (Date.now() < deadline) {
        if (failure) throw failure;
        if (current.exitCode !== null || current.signalCode !== null) throw new Error(`Fresh gwt exited; see ${logFile}`);
        let readyUrl: string | undefined;
        try {
          const candidate = (await readFile(urlFile, "utf8")).trim();
          if (candidate && (await fetch(candidate, { method: "HEAD", signal: AbortSignal.timeout(2_000) })).ok) {
            readyUrl = candidate;
          }
        } catch { /* Fresh process has not published its URL yet. */ }
        if (readyUrl) {
          url = readyUrl;
          await testInfo.attach("exact-relaunch-fixture", {
            body: JSON.stringify({ home, argvLog, checkout: root, project, url, pid: current.pid }), contentType: "application/json",
          });
          return url;
        }
        await delay(100);
      }
      throw new Error(`Fresh gwt readiness timed out; see ${logFile}`);
    } catch (error) {
      await stop();
      throw error;
    }
  }
  await start();
  return { get url() { return url; }, get ownedPid() { return child?.pid; }, home, project, branch, argvLog, launches, start, stopGui, stop,
    async restart(options: { coldOwnerDrain?: boolean; disconnectBrowser?: () => Promise<unknown> } = {}) {
      await stop(options.coldOwnerDrain, options.disconnectBrowser); return start();
    } };
}
