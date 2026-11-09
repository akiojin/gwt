import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import { createWriteStream } from "node:fs";
import { access, copyFile, mkdir, mkdtemp, readdir, readFile, realpath, symlink, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import type { Page, TestInfo } from "@playwright/test";

export async function readProcessState(pid: number): Promise<{ birth: number; parent: number } | null> {
  if (process.platform === "linux") {
    let raw: string;
    try { raw = await readFile(`/proc/${pid}/stat`, "utf8"); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error; }
    const end = raw.lastIndexOf(")");
    if (end < 0) throw new Error(`Cannot parse PID ${pid} stat`);
    const fields = raw.slice(end + 1).trim().split(/\s+/);
    const parent = Number(fields[1]), birth = Number(fields[19]);
    if (!Number.isSafeInteger(parent) || !Number.isSafeInteger(birth) || birth <= 0) throw new Error(`Unknown PID ${pid} identity`);
    return { parent, birth };
  }
  const result = spawnSync("ps", ["-p", String(pid), "-o", "lstart=", "-o", "ppid="], { encoding: "utf8", timeout: 5_000 });
  if (result.error) throw result.error;
  if (result.status === 1 && !result.stdout.trim() && !result.stderr.trim()) return null;
  const match = result.status === 0 ? result.stdout.trim().match(/^(.*?)\s+(\d+)$/) : null;
  const birth = match ? Math.floor(Date.parse(match[1]) / 1000) : NaN, parent = match ? Number(match[2]) : NaN;
  if (!Number.isSafeInteger(birth) || birth <= 0 || !Number.isSafeInteger(parent)) throw new Error(`Unknown PID ${pid} identity`);
  return { birth, parent };
}

// Unknown activity is significant: only a definitely foreign project may be skipped.
export function selectLatestOwnActivity(candidates: any[], projectKey: string): any | null {
  for (let index = candidates.length - 1; index >= 0; index -= 1) {
    const activity = candidates[index];
    if (typeof activity?.project_key === "string" && /^[a-f0-9]{16}$/.test(activity.project_key) && activity.project_key !== projectKey) continue;
    return activity ?? null;
  }
  return null;
}

export async function readLatestOwnActivity(page: Page, expectedProjectKey?: string) {
  const { projectKey, candidates } = await page.evaluate(() => ({
    projectKey: location.pathname.split("/")[2],
    candidates: ((window as any).__gwtPlaywrightMessages ?? []).flatMap(({ payload }: any) => {
      if (payload?.kind === "local_agent_activity") return [payload.activity ?? null];
      if (payload?.projection && Object.hasOwn(payload.projection, "local_agent_activity"))
        return [payload.projection.local_agent_activity ?? null];
      return [];
    }),
  }));
  const snapshot = selectLatestOwnActivity(candidates, projectKey);
  if (!/^[a-f0-9]{16}$/.test(projectKey) || (expectedProjectKey !== undefined && projectKey !== expectedProjectKey)
    || snapshot?.project_key !== projectKey || !/^\d+:\d+$/.test(snapshot?.host_id ?? "")
    || !Number.isFinite(Date.parse(snapshot?.observed_at)) || !Array.isArray(snapshot?.agents) || !snapshot.agents.length)
    throw new Error("Latest own project activity is unavailable or unknown");
  return snapshot;
}

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
  page?: Page;
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
  const hooks = JSON.parse(fs.readFileSync(path.join(sourceDirectory,'.codex','hooks.json'),'utf8'));
  const input = JSON.stringify({session_id:native,hook_event_name:event,cwd:sourceDirectory,
    transcript_path:rollout,prompt,stop_hook_active:false});
  for (const group of hooks.hooks?.[event] ?? []) for (const hook of group.hooks ?? []) {
    if (hook.type !== 'command') continue;
    const result = cp.spawnSync('/bin/sh',['-c',hook.command],{cwd:sourceDirectory,env:process.env,
      input,encoding:'utf8',timeout:60000});
    if (result.status !== 0 || /"decision"\s*:\s*"block"/.test(result.stdout)) {
      throw new Error('Native hook refused: '+event);
    }
  }
  actorEvent('native_hook',{event});
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
  const owner = sourceOperation('execution.status').owner_number;
  nativeHook('UserPromptSubmit',args.at(-1));
  if (owner === switching.sourceIssueNumber && !fs.existsSync(path.join(sourceDirectory,'source-in-progress.txt'))) {
    fs.writeFileSync(path.join(sourceDirectory,'source-in-progress.txt'),'Unfinished source implementation awaiting its next step.\n');
    git('add','--','source-in-progress.txt');
    git('-c','user.name=Switch fixture','-c','user.email=switch@example.invalid','commit','-m','fixture: unfinished source progress');
  }
  sourceOperation('issue.monitor.wait',{number:owner,reason:'Awaiting fixture operator',resume_condition:'Operator requests checkpoint or resumes current work'});
  nativeHook('Stop');
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
  nativeHook('UserPromptSubmit',line);
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
  // Record cwd before Stop publishes Idle: the Host may then exit this actor.
  actorEvent('checkpoint_idle',{operation});
  nativeHook('Stop');
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
try {
  const hooks = JSON.parse(fs.readFileSync(path.join(process.cwd(), '.codex', 'hooks.json'), 'utf8'));
  const input = JSON.stringify({session_id:native,hook_event_name:'SessionStart',source:resumed ? 'resume' : 'startup',cwd:process.cwd(),transcript_path:rollout,model:option('--model')});
  for (const group of hooks.hooks?.SessionStart ?? []) for (const hook of group.hooks ?? []) {
    if (hook.type !== 'command') continue;
    const windows = process.platform === 'win32';
    const result = require('node:child_process').spawnSync(windows ? (process.env.ComSpec || 'cmd.exe') : '/bin/sh',
      windows ? ['/d','/s','/c',hook.command] : ['-c',hook.command],
      {input, stdio:['pipe','ignore','inherit'], timeout:60000, windowsHide:true, windowsVerbatimArguments:windows});
    if (result.error || result.status !== 0) throw new Error('SessionStart hook failed: ' + (result.error?.message || result.status));
  }
} catch (error) { console.error('GWT_FAKE_PROVIDER_HOOK_ERROR ' + error.message); process.exit(1); }
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
  let coldCleanupFailed = false, protectGuiOwner = false;
  let incarnation = 0;
  let url = "";
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
  const processCwd = async (pid: number) => {
    if (process.platform === "linux") return realpath(`/proc/${pid}/cwd`);
    const result = spawnSync("lsof", ["-a", "-p", String(pid), "-d", "cwd", "-Fn"], { encoding: "utf8", timeout: 5_000 });
    if (result.error) throw result.error;
    const paths = result.stdout.split("\n").filter(line => line.startsWith("n")).map(line => line.slice(1));
    if (result.status !== 0 || paths.length !== 1 || !paths[0]) throw new Error(`Unknown PID ${pid} cwd`);
    return realpath(paths[0]);
  };
  async function stopGui() {
    if (!protectGuiOwner && child?.pid && child.exitCode === null && child.signalCode === null) {
      const current = child;
      current.kill("SIGTERM");
      const deadline = Date.now() + 8_000;
      while (current.exitCode === null && current.signalCode === null && Date.now() < deadline) await delay(100);
      if (current.exitCode === null && current.signalCode === null) {
        current.kill("SIGKILL");
        const forcedDeadline = Date.now() + 8_000;
        while (current.exitCode === null && current.signalCode === null && Date.now() < forcedDeadline) await delay(100);
      }
    }
    if (child?.pid && processCommand(child.pid)) {
      throw new Error(`Fixture gwt process ${child.pid} remained after shutdown`);
    }
  }
  async function stop(afterGuiStop?: () => Promise<void>) {
    let browserError: Error | undefined;
    let afterGuiStopError: Error | undefined;
    if (setup.page && url) {
      try {
        const origin = new URL(url).origin;
        const pages = setup.page.context().pages().filter(page => !page.isClosed() && new URL(page.url()).origin === origin);
        const unloaded = await Promise.allSettled(pages.map(page => page.goto("about:blank")));
        if (unloaded.some(result => result.status === "rejected")) throw new Error("Fixture browser unload failed");
      } catch {
        browserError = new Error("Fixture browser unload failed");
      }
    }
    const ownedGui = child;
    const ownedPid = ownedGui?.pid;
    const ownerObservationSupported = process.platform === "darwin" || process.platform === "linux";
    const providerPaths = [join(bin, windows ? "codex.cjs" : "codex"), join(bin, windows ? "claude.cjs" : "claude")];
    let owner: { pid: number; birth: number; instance: string; startup_intent: "gui_managed" | "standalone"; natural: boolean | null } | null = null;
    let ownerCaptureFailed = false;
    let ownerError: Error | undefined;
    if (ownedPid && ownerObservationSupported && !coldCleanupFailed && !protectGuiOwner) {
      try {
        const descriptorDirectory = join(state, "projects", hash, "runtime", "daemon");
        const projectRoot = await realpath(project);
        let descriptorFiles: string[] = [];
        try {
          descriptorFiles = await readdir(descriptorDirectory);
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        }
        let descriptor, descriptorFile: string | undefined;
        for (const file of descriptorFiles.filter(file => file.endsWith(".json"))) {
          const candidate = JSON.parse(await readFile(join(descriptorDirectory, file), "utf8"));
          if (typeof candidate?.scope?.project_root !== "string") throw new Error("Fixture owner descriptor scope unavailable");
          if (await realpath(candidate.scope.project_root) !== projectRoot) continue;
          if (descriptor !== undefined) throw new Error("Fixture owner descriptor is ambiguous");
          descriptor = candidate;
          descriptorFile = file;
        }
        if (descriptor === undefined
          && (await launches()).some(launch => providerPaths.some(path => processCommand(launch.pid).includes(path))))
          throw new Error("Fixture owner descriptor unavailable while owned recorder remains");
        if (descriptor !== undefined) {
          if (!Number.isSafeInteger(descriptor.pid) || descriptor.pid <= 0
            || !["gui_managed", "standalone"].includes(descriptor.startup_intent) || descriptor.scope?.repo_hash !== hash
            || typeof descriptor.scope.worktree_hash !== "string"
            || descriptorFile !== `${descriptor.scope.worktree_hash}.json` || descriptor.scope.target !== "host")
            throw new Error("Fixture owner descriptor identity mismatch");
          if (descriptor.pid === ownedPid) {
            protectGuiOwner = true;
            throw new Error("Fixture owner identity overlaps GUI");
          }
          const observed = await readProcessState(descriptor.pid);
          if (observed) {
            const executable = process.platform === "linux"
              ? await realpath(`/proc/${descriptor.pid}/exe`)
              : spawnSync("ps", ["-p", String(descriptor.pid), "-o", "comm="], { encoding: "utf8", timeout: 5_000 });
            const executablePath = typeof executable === "string" ? executable
              : !executable.error && executable.status === 0 ? await realpath(executable.stdout.trim()) : null;
            if (executablePath !== await realpath(gwtd) || await processCwd(descriptor.pid) !== await realpath(project))
              throw new Error("Fixture owner executable or cwd mismatch");
            const transport = new URL(descriptor.project_transport_url);
            if (transport.protocol !== "http:" || !["127.0.0.1", "[::1]"].includes(transport.hostname)
              || transport.username || transport.password || transport.pathname !== "/" || transport.search || transport.hash
              || typeof descriptor.auth_token !== "string" || !descriptor.auth_token)
              throw new Error("Fixture owner private transport unavailable");
            const response = await fetch(new URL("/private/project/command", transport), {
              method: "POST", redirect: "error", signal: AbortSignal.timeout(5_000),
              headers: { Authorization: `Bearer ${descriptor.auth_token}`, "Content-Type": "application/json" },
              body: JSON.stringify({ project_key: hash, command: { type: "health" } }),
            });
            if (!response.ok) throw new Error("Fixture owner health unavailable");
            const health = await response.json();
            if (health.pid !== descriptor.pid || health.project_key !== hash || !["gui_managed", "standalone"].includes(health.startup_intent)
              || health.host_started_at !== observed.birth || typeof health.host_instance_id !== "string" || !health.host_instance_id
              || (await readProcessState(descriptor.pid))?.birth !== observed.birth)
              throw new Error("Fixture owner physical identity changed");
            owner = { pid: descriptor.pid, birth: observed.birth, instance: health.host_instance_id,
              startup_intent: health.startup_intent, natural: health.startup_intent === "gui_managed" ? false : null };
          }
        }
      } catch {
        // Never include descriptor or HTTP error content: it can contain the private bearer.
        ownerCaptureFailed = true;
        ownerError = new Error("Fixture owner capture failed");
      }
    }
    try {
      if (!protectGuiOwner && child?.pid && child.exitCode === null && child.signalCode === null) {
        const current = child;
        current.kill("SIGTERM");
        const deadline = Date.now() + 8_000;
        while (current.exitCode === null && current.signalCode === null && Date.now() < deadline) await delay(100);
        if (current.exitCode === null && current.signalCode === null) {
          current.kill("SIGKILL");
          const forcedDeadline = Date.now() + 8_000;
          while (current.exitCode === null && current.signalCode === null && Date.now() < forcedDeadline) await delay(100);
        }
      }
      if (afterGuiStop) {
        try {
          if (!ownedGui?.pid || child !== ownedGui || (ownedGui.exitCode === null && ownedGui.signalCode === null))
            throw new Error("Fixture GUI exit is unconfirmed before Host drain");
          await afterGuiStop();
        } catch (error) {
          afterGuiStopError = error instanceof Error ? error : new Error("Fixture post-GUI shutdown callback failed");
        }
      }
      if (owner?.startup_intent === "gui_managed") {
        try {
          const deadline = performance.now() + 30_000;
          while (performance.now() < deadline) {
            const current = await readProcessState(owner.pid);
            if (performance.now() >= deadline) break;
            if (!current || current.birth !== owner.birth) { owner.natural = true; break; }
            await delay(100);
          }
          if (!owner.natural) throw new Error("Fixture owner natural reap not observed before deadline");
        } catch {
          ownerError = new Error("Fixture owner natural reap not observed before deadline");
        }
      }
      // PTYs can create their own process groups; clean only recorded fixture
      // providers still carrying our unique argv-recorder path in their command.
      const providerPids: number[] = [];
      const providerPreFallback: unknown[] = [];
      for (const launch of coldCleanupFailed ? [] : await launches()) {
        const command = processCommand(launch.pid);
        const privatePathMatch = providerPaths.some(path => command.includes(path));
        // Diagnostic only: inspection failure is UNKNOWN and never changes the
        // existing exact-provider fallback or its cleanup authority.
        if (!windows) {
          try {
            const result = spawnSync("ps", ["-p", String(launch.pid), "-o", "lstart=", "-o", "ppid="], { encoding: "utf8" });
            const match = result.status === 0 && !result.error ? result.stdout.trim().match(/^(.*?)\s+(\d+)$/) : null;
            providerPreFallback.push({ at: new Date().toISOString(), pid: launch.pid, session_id: launch.gwt_session_id,
              private_path_match: privatePathMatch, birth: match?.[1] ?? null, parent_pid: match ? Number(match[2]) : null,
              state: match ? "observed" : result.status === 1 && !result.error && !result.stdout.trim() && !result.stderr.trim() ? "absent" : "unknown" });
          } catch { providerPreFallback.push({ at: new Date().toISOString(), pid: launch.pid, state: "unknown" }); }
        } else providerPreFallback.push({ at: new Date().toISOString(), pid: launch.pid, state: "unknown_unsupported" });
        if (privatePathMatch) {
          providerPids.push(launch.pid);
          try { process.kill(launch.pid, "SIGTERM"); } catch { /* exited */ }
        }
      }
      await Promise.all(providerPids.map(async pid => {
        const deadline = Date.now() + 5_000;
        while (Date.now() < deadline) {
          const command = processCommand(pid);
          if (!providerPaths.some(path => command.includes(path))) return;
          await delay(100);
        }
        throw new Error(`Fixture provider ${pid} remained after shutdown`);
      }));
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
        await testInfo.attach("fixture-process-cleanup", { body: JSON.stringify({ pid: ownedPid, exited: !remaining,
          owner, ...(ownerObservationSupported ? {} : { owner_observation: { status: "unsupported_platform", platform: process.platform, natural: null } }),
          ...(ownerCaptureFailed ? { owner_capture_failed: true } : {}), provider_pre_fallback: providerPreFallback }), contentType: "application/json" });
        if (remaining) throw new Error(`Fixture gwt process ${ownedPid} remained after shutdown`);
      }
      child = undefined;
    } catch (error) {
      const errors = [browserError, ownerError, afterGuiStopError].filter((failure): failure is Error => failure !== undefined);
      if (!errors.length) throw error;
      throw new AggregateError([...errors, new Error("Fixture physical cleanup failed")], "Fixture cleanup failed");
    }
    if (afterGuiStopError) throw new AggregateError(
      [browserError, ownerError, afterGuiStopError].filter((failure): failure is Error => failure !== undefined),
      "Fixture post-GUI shutdown cleanup failed");
    if (browserError && ownerError) throw new AggregateError([browserError, ownerError], "Fixture browser and owner cleanup failed");
    if (browserError) throw browserError;
    if (ownerError) throw ownerError;
  }
  type ColdCensus = { project_key: string; host_id: string; observed_at: string;
    agents: { window_id: string; session_id: string; runtime_incarnation: number }[] };
  type ColdWitness = { session: string; childPid: number; childBirth: number; hostPid: number; hostBirth: number; incarnation: number; identity: string; window: string; cwd: string; providerPath: string };
  async function coldRestart(census: ColdCensus) {
    if (!["darwin", "linux"].includes(process.platform)) throw new Error("Cold owner observation unsupported on this platform");
    const errors: unknown[] = [], witnesses: ColdWitness[] = [];
    const ownedPid = child?.pid;
    const providerPaths = [join(bin, "codex"), join(bin, "claude")];
    const privateChild = async (witness: ColdWitness) => {
      const current = await readProcessState(witness.childPid);
      if (!current || current.birth !== witness.childBirth) return false;
      if (!processCommand(witness.childPid).includes(witness.providerPath))
        throw new Error(`Owned recorder ${witness.childPid} path changed`);
      return true;
    };
    let natural = false;
    const qualifyCohort = async (capture: boolean): Promise<ColdWitness[]> => {
      const qualified: ColdWitness[] = [];
      if (!ownedPid || !census || !Array.isArray(census.agents) || !census.agents.length || !Number.isFinite(Date.parse(census.observed_at)))
        throw new Error("Cold owner census unavailable");
      const host = /^(\d+):(\d+)$/.exec(census.host_id || "");
      if (!host || census.project_key !== hash) throw new Error("Cold owner host identity unavailable");
      const hostPid = Number(host[1]), hostBirth = Number(host[2]);
      if (!Number.isSafeInteger(hostPid) || hostPid <= 0 || !Number.isSafeInteger(hostBirth) || hostBirth <= 0)
        throw new Error("Cold owner host identity invalid");
      if (hostPid === ownedPid) {
        protectGuiOwner = true;
        throw new Error("Cold owner identity overlaps GUI; refusing to signal it");
      }
      const actualHost = await readProcessState(hostPid);
      if (!actualHost || actualHost.birth !== hostBirth) throw new Error("Cold owner birth changed");
      for (const launch of await launches()) {
        const matches = census.agents.filter(agent => agent.session_id === launch.gwt_session_id);
        const actualChild = await readProcessState(launch.pid);
        if (!actualChild) {
          if (matches.length) throw new Error("Current census recorder child is absent or unknown");
          continue;
        }
        const command = processCommand(launch.pid);
        const providerPath = providerPaths.find(path => command.includes(path));
        if (!providerPath) throw new Error("Live recorded child lacks private recorder path");
        if (matches.length !== 1 || !matches[0].window_id || !Number.isSafeInteger(matches[0].runtime_incarnation) || matches[0].runtime_incarnation <= 0)
          throw new Error("Live recorded child lacks exact current census");
        const session = launch.gwt_session_id;
        if (!session || !/^[0-9a-f-]{36}$/i.test(session)) throw new Error("Invalid owned Session id");
        const sidecar = JSON.parse(await readFile(join(home, ".gwt", "sessions", "runtime", String(hostPid), `${session}.json`), "utf8"));
        if (sidecar.child_pid !== launch.pid || sidecar.child_started_at !== actualChild.birth || sidecar.host_started_at !== hostBirth
          || sidecar.runtime_incarnation !== matches[0].runtime_incarnation || actualChild.parent !== hostPid)
          throw new Error("Cold owner runtime identity mismatch");
        if (Object.hasOwn(sidecar, "execution_identity")
          && (!sidecar.execution_identity || sidecar.execution_identity.session_id !== session
            || sidecar.execution_identity.execution_binding?.session_id !== session))
          throw new Error("Cold owner execution identity mismatch");
        const cwd = await processCwd(launch.pid), expectedCwd = await realpath(launch.cwd);
        if (cwd !== expectedCwd || !(cwd === home || cwd.startsWith(home + "/"))) throw new Error("Cold owner child cwd mismatch");
        if (qualified.some(witness => witness.session === session || witness.childPid === launch.pid))
          throw new Error("Duplicate cold owner recorder identity");
        const witness = { session, childPid: launch.pid, childBirth: actualChild.birth, hostPid, hostBirth,
          incarnation: matches[0].runtime_incarnation, identity: JSON.stringify(sidecar.execution_identity ?? null),
          window: matches[0].window_id, cwd, providerPath };
        qualified.push(witness);
        if (capture) witnesses.push(witness);
      }
      if (!qualified.length) throw new Error("Cold owner has no qualified live witness");
      return qualified;
    };
    try {
      await qualifyCohort(true);
      const current = await qualifyCohort(false);
      if (JSON.stringify(current) !== JSON.stringify(witnesses)) throw new Error("Cold owner cohort changed before GUI shutdown");
    } catch (error) { errors.push(error); }

    // Always attempt each cleanup independently; no observer failure may skip
    // the GUI receipt or leave the handle assigned to this fixture.
    try {
      if (!protectGuiOwner && child?.pid && child.exitCode === null && child.signalCode === null) {
        const current = child;
        current.kill("SIGTERM");
        const deadline = Date.now() + 8_000;
        while (current.exitCode === null && current.signalCode === null && Date.now() < deadline) await delay(100);
        if (current.exitCode === null && current.signalCode === null) {
          current.kill("SIGKILL");
          const forced = Date.now() + 8_000;
          while (current.exitCode === null && current.signalCode === null && Date.now() < forced) await delay(100);
        }
        if (current.exitCode === null && current.signalCode === null) throw new Error(`Fixture gwt process ${current.pid} remained after shutdown`);
      }
    } catch (error) { errors.push(error); }
    if (!errors.length) {
      try {
        const deadline = performance.now() + 30_000;
        while (performance.now() < deadline) {
          const observations = await Promise.all(witnesses.map(async witness => ({ host: await readProcessState(witness.hostPid), child: await readProcessState(witness.childPid), witness })));
          if (performance.now() >= deadline) throw new Error("Cold owner natural reap not observed before deadline");
          if (observations.every(({ host, child, witness }) => (!host || host.birth !== witness.hostBirth)
            && (!child || child.birth !== witness.childBirth))) { natural = true; break; }
          await delay(100);
        }
        if (!natural) throw new Error("Cold owner natural reap not observed before deadline");
      } catch (error) { errors.push(error); }
    }
    const providerPreFallback: unknown[] = [];
    for (const witness of witnesses) {
      try {
        const current = await readProcessState(witness.childPid);
        const same = current?.birth === witness.childBirth;
        providerPreFallback.push({ at: new Date().toISOString(), pid: witness.childPid, session_id: witness.session,
          state: !current ? "absent" : same ? "observed" : "replaced", birth: current?.birth ?? null,
          parent_pid: current?.parent ?? null, private_path_match: same ? await privateChild(witness) : false });
      } catch (error) {
        providerPreFallback.push({ at: new Date().toISOString(), pid: witness.childPid, state: "unknown" });
        errors.push(error);
      }
    }
    if (!natural) {
      for (const witness of witnesses) {
        try {
          if (await privateChild(witness)) {
            process.kill(witness.childPid, "SIGTERM");
            const deadline = performance.now() + 5_000;
            while (performance.now() < deadline && await privateChild(witness)) await delay(100);
            if (await privateChild(witness)) throw new Error(`Owned recorder ${witness.childPid} remained after cleanup`);
          }
        } catch (error) { errors.push(error); }
      }
    }
    try {
      const actorEvents = (await readFile(join(home, "switch-actor.jsonl"), "utf8").catch(() => ""))
        .split("\n").filter(Boolean).map(line => JSON.parse(line));
      for (const event of actorEvents) {
        if (!event.tree_child_pid) continue;
        try {
          const command = spawnSync("ps", ["-p", String(event.tree_child_pid), "-o", "command="], { encoding: "utf8" });
          if (command.error) throw command.error;
          if (command.stdout.includes("GWT_SWITCH_TREE_CHILD") && command.stdout.includes(home)) {
            try { process.kill(event.tree_child_pid, "SIGTERM"); } catch { /* exited */ }
          }
        } catch (error) { errors.push(error); }
      }
    } catch (error) { errors.push(error); }
    try {
      if (ownedPid) {
        let exited = false;
        try {
          exited = !processCommand(ownedPid);
          if (!exited) errors.push(new Error(`Fixture gwt process ${ownedPid} remained after shutdown`));
        } catch (error) { errors.push(error); }
        try {
          await testInfo.attach("fixture-process-cleanup", { body: JSON.stringify({ pid: ownedPid, exited,
            provider_pre_fallback: providerPreFallback, cold_owner: { natural, witnesses: witnesses.map(({ identity: _identity, window: _window, cwd: _cwd, providerPath: _providerPath, ...witness }) => witness) } }), contentType: "application/json" });
        } catch (error) { errors.push(error); }
      } else errors.push(new Error("Cold GUI PID unavailable"));
    } finally {
      if (child && (child.exitCode !== null || child.signalCode !== null)) child = undefined;
    }
    if (errors.length) {
      // A later ordinary finally must not signal an unqualified old child.
      coldCleanupFailed = true;
      throw new AggregateError(errors, "Cold owner restart or cleanup failed");
    }
    return start();
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
    async restart(options?: { coldOwner: ColdCensus }) { if (options) return coldRestart(options.coldOwner); await stop(); return start(); } };
}
