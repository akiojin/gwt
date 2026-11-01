import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { createHash, randomUUID } from "node:crypto";
import { mkdir, readFile, readdir, realpath, writeFile } from "node:fs/promises";
import { dirname, isAbsolute, join } from "node:path";
import type { TestInfo } from "@playwright/test";
import { startExactRelaunchFixture, writeFixtureNodeCommand, type ExactRelaunchSetup } from "./exact-relaunch";

/** Real monitor, claims, materialization and PTY; GitHub and provider are local recorders. */
export async function startIssueQueueLaunchFixture(info: TestInfo, options: {
  targetIssueNumber?: 44; switching?: boolean; positive?: boolean; terminalConvergence?: boolean; autonomousMode?: boolean;
  prepare?: ExactRelaunchSetup["prepare"];
} = {}) {
  const windows = process.platform === "win32";
  const comments: { id: number; body: string; updated_at: string }[] = [];
  const requests: { method: string; url: string }[] = [];
  const issue = {
    number: 43, title: "Queue launch fixture", state: "OPEN",
    body: "## Acceptance Criteria\n- [ ] AC-1: Record the provider handoff.",
    labels: [{ name: "bug" }, { name: "auto-improve" }],
    url: "https://github.com/fixture/queue/issues/43",
    updatedAt: new Date().toISOString(),
  };
  const issues = [issue, ...(options.targetIssueNumber ? [{ ...issue, number: options.targetIssueNumber,
    title: "Next planned switch fixture", url: `https://github.com/fixture/queue/issues/${options.targetIssueNumber}` }] : [])];
  const issueComments = new Map(issues.map(value => [value.number, value.number === issue.number ? comments : [] as typeof comments]));
  let nextCommentId = 1;
  const restIssue = (value: typeof issue) => ({ ...value, state: value.state.toLowerCase(), html_url: value.url,
    updated_at: value.updatedAt, user: { login: "fixture" } });
  const server = createServer(async (request, response) => {
    requests.push({ method: request.method || "", url: request.url || "" });
    let body = "";
    for await (const chunk of request) body += chunk;
    const payload = body ? JSON.parse(body) : {};
    let result: unknown;
    if (request.url === "/graphql" && request.method === "POST") {
      const number = Number(payload.variables?.number ?? /issue\s*\(\s*number:\s*(\d+)/.exec(payload.query || "")?.[1] ?? issue.number);
      const selected = issues.find(value => value.number === number);
      if (!selected) { response.writeHead(404); response.end("{}"); return; }
      result = { data: { repository: { issue: { ...selected,
        labels: { nodes: selected.labels },
        comments: { nodes: issueComments.get(number)!.map(comment => ({ databaseId: comment.id,
          body: comment.body, updatedAt: comment.updated_at })),
          pageInfo: { hasNextPage: false, endCursor: null } },
        timelineItems: { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } },
      } } } };
    } else if (/^\/repos\/fixture\/queue\/issues\/\d+\/comments$/.test(request.url || "") && request.method === "POST") {
      const number = Number(request.url!.split("/").at(-2));
      const selected = issueComments.get(number);
      if (!selected) { response.writeHead(404); response.end("{}"); return; }
      const comment = { id: nextCommentId++, body: payload.body, updated_at: new Date().toISOString() };
      selected.push(comment);
      result = comment;
    } else if (/^\/repos\/fixture\/queue\/issues\/comments\/\d+$/.test(request.url || "") && request.method === "PATCH") {
      const comment = [...issueComments.values()].flat().find(entry => entry.id === Number(request.url!.split("/").at(-1)));
      if (!comment) { response.writeHead(404); response.end("{}"); return; }
      comment.body = payload.body;
      comment.updated_at = new Date().toISOString();
      result = comment;
    } else if (options.terminalConvergence && request.method === "PATCH"
      && /^\/repos\/fixture\/queue\/issues\/\d+$/.test(request.url || "")) {
      const selected = issues.find(value => value.number === Number(request.url!.split("/").at(-1)));
      if (!selected) { response.writeHead(404); response.end("{}"); return; }
      selected.state = String(payload.state).toUpperCase();
      selected.updatedAt = new Date().toISOString();
      result = restIssue(selected);
    } else if (options.terminalConvergence && request.method === "DELETE"
      && /^\/repos\/fixture\/queue\/issues\/\d+\/labels\//.test(request.url || "")) {
      const parts = request.url!.split("/");
      const selected = issues.find(value => value.number === Number(parts[5]));
      if (!selected) { response.writeHead(404); response.end("{}"); return; }
      selected.labels = selected.labels.filter(label => label.name !== decodeURIComponent(parts[7]));
      selected.updatedAt = new Date().toISOString();
      result = selected.labels;
    } else if (request.method === "GET" && /^\/repos\/fixture\/queue\/issues(?:\?|$)/.test(request.url || "")) {
      result = issues.map(restIssue);
    } else if (request.method === "GET" && /^\/repos\/fixture\/queue\/issues\/\d+(?:\?|$)/.test(request.url || "")) {
      const number = Number(request.url!.split("/").at(-1)!.split("?")[0]);
      const selected = issues.find(value => value.number === number);
      if (!selected) { response.writeHead(404); response.end("{}"); return; }
      result = restIssue(selected);
    } else if (request.method === "GET" && /^\/repos\/fixture\/queue\/issues\/\d+\/comments(?:\?|$)/.test(request.url || "")) {
      const number = Number(request.url!.split("/").at(-2));
      result = (issueComments.get(number) || []).map(comment => ({ ...comment, user: { login: "fixture" }, created_at: comment.updated_at }));
    } else {
      response.writeHead(500); response.end(JSON.stringify({ message: "Unimplemented fixture request" })); return;
    }
    response.writeHead(200, { "Content-Type": "application/json" });
    response.end(JSON.stringify(result));
  });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Missing fixture server address");
  const endpoint = `http://127.0.0.1:${address.port}`;
  let auditEnv: NodeJS.ProcessEnv;
  let checkoutGwtd = "";
  let stableHook = "";
  let projectLogs = "";
  let projectState = "";
  let remotePath = "";
  let realGitPath = "";
  const hookAudits: unknown[] = [];
  function auditHooks(cwd: string, repair: boolean): { status: string; issues: string[] } {
    const operation = repair ? "hook.doctor" : "hook.health";
    const result = spawnSync(checkoutGwtd, [], { cwd, env: auditEnv, encoding: "utf8", timeout: 30_000, windowsHide: true,
      input: JSON.stringify({ schema_version: 1, operation, params: {
        ...(repair ? { repair: true } : {}), expected_hook_bin: stableHook,
        runtime_state_path: join(auditEnv.HOME!, ".gwt", "fixture-missing-runtime-state.json"),
      } }),
    });
    if (result.status !== 0) throw new Error(`${operation} failed: ${result.stdout} ${result.stderr}`);
    const envelope = JSON.parse(result.stdout);
    if (!envelope.ok) throw new Error(`${operation} refused: ${result.stdout}`);
    const output = typeof envelope.output === "string" ? JSON.parse(envelope.output) : envelope.output;
    const health = repair ? output.health : output;
    hookAudits.push({ operation, cwd, health });
    if (repair) {
      const registered = spawnSync(checkoutGwtd, [], { cwd, env: auditEnv, encoding: "utf8", timeout: 30_000, windowsHide: true,
        input: JSON.stringify({ schema_version: 1, operation: "hook.register_codex_managed_hook_trust", params: {
          project_root: cwd, codex_config: join(auditEnv.HOME!, ".codex/config.toml"), codex_hook_discovery: "both",
        } }),
      });
      if (registered.status !== 0 || !JSON.parse(registered.stdout).ok) throw new Error(`Fixture hook trust registration failed: ${registered.stderr}`);
      return auditHooks(cwd, false);
    }
    const issues = (health.issues ?? []).filter((issue: string) => !issue.startsWith("managed hook failure:") || !issue.includes("state=fail-open"));
    if (issues.length || (!repair && health.status === "inactive")) throw new Error(`Hook audit failed: ${JSON.stringify(health)}`);
    return health;
  }
  let daemon: ChildProcess | undefined;
  async function stopDaemon() {
    if (daemon && daemon.exitCode === null && daemon.signalCode === null) {
      daemon.kill("SIGTERM");
      const until = Date.now() + 5_000;
      while (daemon.exitCode === null && daemon.signalCode === null && Date.now() < until) await delay(100);
      if (daemon.exitCode === null && daemon.signalCode === null) daemon.kill("SIGKILL");
      const forcedUntil = Date.now() + 5_000;
      while (daemon.exitCode === null && daemon.signalCode === null && Date.now() < forcedUntil) await delay(100);
      if (daemon.exitCode === null && daemon.signalCode === null) throw new Error(`Owned fixture daemon ${daemon.pid} remained after shutdown`);
    }
  }
  let fixture;
  try {
    fixture = await startExactRelaunchFixture(info, {
      terminalConvergence: options.terminalConvergence,
      freshNativeIds: !!options.switching || !!options.terminalConvergence,
      switching: options.switching ? { sourceIssueNumber: issue.number,
        positive: !!options.positive && process.platform === "linux" && process.env.GWT_PLAYWRIGHT_SWITCH_POSITIVE === "1" } : undefined,
      monitorPreferences: {
        enabled: true, autonomous_mode: options.autonomousMode ?? false, auto_close_merged_issues: false,
        auto_apply_updates: false, terminal_queue_auto_refill: false,
        launch_profile: { agent_id: "codex", version: "installed", runtime_target: "Host", skip_permissions: true },
      },
      async beforeStart({ home, project, env, preferences }) {
        if (options.terminalConvergence)
          await writeFile(join(home, ".gwt/config.toml"), "[agent]\nterminal_close_grace_secs = 1\n");
        projectState = preferences;
        projectLogs = join(preferences, "..", "logs");
        daemon = spawn(checkoutGwtd, [], { cwd: project, env, windowsHide: true, stdio: ["pipe", "ignore", "pipe"] });
        let daemonErrors = "";
        daemon.stderr?.on("data", chunk => { daemonErrors += chunk; });
        daemon.stdin?.end(JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }) + "\n");
        const until = Date.now() + 30_000;
        while (Date.now() < until) {
          if (daemon.exitCode !== null || daemon.signalCode !== null) throw new Error(`Fixture daemon exited: ${daemonErrors}`);
          const status = spawnSync(checkoutGwtd, [], { cwd: project, env, encoding: "utf8", timeout: 5_000, windowsHide: true,
            input: JSON.stringify({ schema_version: 1, operation: "daemon.status", params: {} }) });
          if (status.status === 0 && status.stdout.includes(String(daemon.pid))) return;
          await delay(100);
        }
        throw new Error(`Fixture daemon readiness timed out: ${daemonErrors}`);
      },
      async prepare({ home, bin, project, env }) {
        auditEnv = env;
        checkoutGwtd = join(bin, windows ? "gwtd.exe" : "gwtd");
        const disposable = /(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(?:debug|release)[/\\]gwtd(?:\.exe)?(?:[^a-z0-9_.-]|$)/i;
        stableHook = process.env.GWT_HOOK_BIN || "";
        if (disposable.test(stableHook) || (stableHook && stableHook !== "gwtd" && (!isAbsolute(stableHook) || !existsSync(stableHook)))) stableHook = "";
        if (!stableHook) {
          delete env.GWT_HOOK_BIN;
          const resolved = spawnSync(checkoutGwtd, [], { cwd: project, env, encoding: "utf8", timeout: 30_000, windowsHide: true,
            input: JSON.stringify({ schema_version: 1, operation: "hook.doctor", params: {} }),
          });
          if (resolved.error) throw resolved.error;
          const envelope = resolved.stdout.trim() ? JSON.parse(resolved.stdout) : null;
          if (resolved.status !== 0 || !envelope?.ok) throw new Error(`Fixture hook fallback resolution failed: ${resolved.stderr}`);
          const output = typeof envelope.output === "string" ? JSON.parse(envelope.output) : envelope.output;
          stableHook = output.managed_hook_bin;
        }
        if (!stableHook || disposable.test(stableHook)) throw new Error("No stable managed-hook fallback resolved");
        env.GWT_HOOK_BIN = stableHook;
        const foundGit = spawnSync(windows ? "where.exe" : "which", ["git"], { encoding: "utf8", windowsHide: true });
        if (foundGit.error) throw foundGit.error;
        const realGit = foundGit.stdout.split(/\r?\n/).find(path => path && (!windows || /\.exe$/i.test(path)));
        if (!realGit) throw new Error("No native Git executable found for the fixture");
        realGitPath = realGit;
        const git = (...args: string[]) => {
          const result = spawnSync(realGit, args, { cwd: project, env, encoding: "utf8", windowsHide: true });
          if (result.status !== 0) throw new Error(`Fixture git failed: ${result.stderr}`);
        };
        const remote = join(home, "origin.git");
        remotePath = remote;
        if (options.terminalConvergence) {
          // Match this repository's treatment of generated local hook config.
          await writeFile(join(project, ".gitignore"), "/.codex/hooks.json\n");
          git("add", "--", ".gitignore");
          git("-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid",
            "commit", "-qm", "test: ignore generated local hook config");
        }
        git("init", "--bare", "-q", remote);
        git("remote", "add", "origin", remote);
        git("push", "-q", "-u", "origin", "main");
        git("--git-dir", remote, "symbolic-ref", "HEAD", "refs/heads/main");
        git("symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main");
        const gitWrapper = `#!${process.execPath}\nconst cp=require('node:child_process');\nconst a=process.argv.slice(2);\nif(a.slice(-3).join(' ')==='remote get-url origin'){console.log('https://github.com/fixture/queue.git');process.exit(0);}\nconst r=cp.spawnSync(${JSON.stringify(realGit)},a,{stdio:'inherit',windowsHide:true});process.exit(r.status??1);\n`;
        await writeFixtureNodeCommand(bin, "git", gitWrapper, true);
        await writeFile(join(home, ".codex", "auth.json"), JSON.stringify({ OPENAI_API_KEY: "fixture-only" }));
        const ghLog = join(home, "gh-argv.jsonl");
        await writeFile(ghLog, "");
        const gh = `#!${process.execPath}
const fs=require('node:fs');const a=process.argv.slice(2);const text=a.join(' ');
fs.appendFileSync(${JSON.stringify(ghLog)},JSON.stringify(a)+'\\n');
const issues=${JSON.stringify(issues)};
const selected=issues.find(issue=>issue.number===Number(a[2]))||issues[0];
if(a[0]==='--version'){console.log('gh version 2.80.0');process.exit(0);}
if(a[0]==='auth'){if(a[1]==='token')console.log('fixture-only');process.exit(0);}
if(a.includes('--include'))process.stdout.write('HTTP/2.0 200 OK\\r\\n\\r\\n');
if(a[0]==='api' && text.includes('/issues?')){console.log(JSON.stringify(/[?&]page=1(?:&| |$)/.test(text)?issues:[]));process.exit(0);}
if(a[0]==='api' && text.includes('/pulls?')){console.log('[]');process.exit(0);}
if(a[0]==='issue'&&a[1]==='view'){console.log(JSON.stringify({...selected,comments:[]}));process.exit(0);}
if(a[0]==='api'&&a[1]==='graphql'){console.log(JSON.stringify({data:{repository:{issue:{timelineItems:{nodes:[],pageInfo:{hasNextPage:false,endCursor:null}}}}}}));process.exit(0);}
if(a[0]==='pr'&&a[1]==='list'){console.log('[]');process.exit(0);}
console.error('Unimplemented fixture gh: '+text);process.exit(64);
`;
        const ghCommand = await writeFixtureNodeCommand(bin, "gh", gh, true);
        Object.assign(env, {
          ...(options.terminalConvergence ? { GWT_GITHUB_REPO: "fixture/queue" } : {}),
          RUST_LOG: "info",
          GWT_OWNER_GITHUB_TEST_MODE: "loopback-v1",
          GWT_OWNER_GITHUB_REST_BASE: endpoint,
          GWT_OWNER_GITHUB_GRAPHQL_URL: `${endpoint}/graphql`,
          GWT_OWNER_GITHUB_TOKEN: "fixture-only",
          GWT_TEST_GH: ghCommand,
        });
        await mkdir(join(project, ".codex"), { recursive: true });
        await writeFile(join(project, ".codex/hooks.json"), "{}");
        auditHooks(project, true);
        await options.prepare?.({ home, bin, project, env });
      },
    });
  } catch (error) {
    try { await stopDaemon(); }
    finally { await new Promise<void>(resolve => server.close(() => resolve())); }
    throw error;
  }
  const active = fixture;
  const workItems = async () => JSON.parse(await readFile(join(projectState, "works.json"), "utf8"));
  function eventShardPath(cwd: string, eventId: string) {
    const digest = createHash("sha256").update(eventId).digest("hex");
    return join(cwd, ".gwt", "work", "events", digest.slice(0, 2), `${digest}.jsonl`);
  }
  function stagedDiff(cwd: string) {
    const result = spawnSync("git", ["diff", "--cached", "--binary"], { cwd, env: auditEnv, windowsHide: true });
    if (result.status !== 0) throw new Error("Fixture staged diff failed");
    return result.stdout;
  }
  async function seedActiveOrphan(cwd: string, sessionId: string) {
    // Pause only the owned GUI; retain the loopback service and daemon for
    // restart. No Session, ECR, capability or home projection is fabricated.
    await active.stop();
    const worktree = await realpath(cwd);
    const candidates = (await workItems()).work_items.filter((work: any) =>
      work.events.some((event: any) => event.kind === "start" && event.agent_session_id === sessionId));
    if (candidates.length !== 1 || candidates[0].status_category !== "done") {
      throw new Error("Fixture requires one authenticated terminal predecessor");
    }
    const predecessor = candidates[0];
    const start = predecessor.events.find((event: any) => event.kind === "start" && event.agent_session_id === sessionId);
    if (start.owner !== `Issue #${issue.number}` || !start.execution_container?.branch
      || await realpath(start.execution_container.worktree_path) !== worktree) {
      throw new Error("Fixture predecessor has no exact owner/container proof");
    }
    const successorId = `work-successor-${createHash("sha256").update(predecessor.id).digest("hex")}`;
    const orphan = { ...start, id: randomUUID(), work_item_id: successorId,
      agent_session_id: `fixture-departed-orphan-${randomUUID()}`, related_work_item_id: null,
      execution_container: { ...start.execution_container, worktree_path: worktree }, updated_at: new Date().toISOString() };
    const shard = eventShardPath(worktree, orphan.id);
    await mkdir(dirname(shard), { recursive: true });
    await writeFile(shard, `${JSON.stringify(orphan)}\n`, { flag: "wx" });
    const stagedPath = "retained-staged-source.txt";
    await writeFile(join(worktree, stagedPath), "Active orphan recovery must retain these staged bytes.\n");
    const staged = spawnSync("git", ["add", "--", stagedPath], { cwd: worktree, env: auditEnv, windowsHide: true });
    if (staged.status !== 0) throw new Error("Fixture stage failed");
    return { predecessorId: predecessor.id, successorId, orphan, worktree,
      branch: start.execution_container.branch, stagedDiff: stagedDiff(worktree) };
  }
  function readOperation(operation: "execution.status" | "perf.startup", cwd: string, exact = false) {
    const result = spawnSync(checkoutGwtd, [], { cwd, env: auditEnv, encoding: "utf8", timeout: 30_000, windowsHide: true,
      input: JSON.stringify({ schema_version: 1, operation,
        params: operation === "execution.status" && !exact ? { issue: issue.number } : {} }),
    });
    if (result.status !== 0) throw new Error(`Fixture ${operation} failed`);
    const envelope = JSON.parse(result.stdout);
    if (!envelope.ok) throw new Error(`Fixture ${operation} refused`);
    return typeof envelope.output === "string" ? JSON.parse(envelope.output) : envelope.output;
  }
  function scheduleOperation(
    operation: "issue.monitor.schedule.read" | "issue.monitor.schedule.reassign" | "issue.monitor.requeue" | "issue.monitor.launch_now"
      | "issue.monitor.config.set" | "issue.monitor.status" | "issue.monitor.profiles.set",
    params: Record<string, unknown> = {},
    projectRoot = active.project,
  ) {
    const result = spawnSync(checkoutGwtd, [], { cwd: projectRoot, env: auditEnv, encoding: "utf8", timeout: 30_000, windowsHide: true,
      input: JSON.stringify({ schema_version: 1, operation, params: { ...params, project_root: projectRoot } }),
    });
    if (result.error) throw result.error;
    if (!result.stdout.trim()) throw new Error(`Fixture ${operation} returned no envelope: ${result.stderr}`);
    return JSON.parse(result.stdout);
  }
  async function closeIssue() {
    if (!options.terminalConvergence) throw new Error("Closing requires the isolated terminal fixture");
    // The loopback HTTP server runs on this worker. A synchronous child would
    // prevent it answering the canonical client's requests.
    const child = spawn(checkoutGwtd, [], {
      cwd: active.project, env: auditEnv, windowsHide: true,
    });
    let stdout = "", stderr = "", timedOut = false;
    child.stdout.on("data", data => { stdout += data; });
    child.stderr.on("data", data => { stderr += data; });
    const timer = setTimeout(() => { timedOut = true; child.kill(); }, 45_000);
    let code;
    try {
      child.stdin.end(JSON.stringify({ schema_version: 1, operation: "issue.close",
        params: { number: issue.number, reason: "completed" } }));
      code = await new Promise<number | null>((resolve, reject) => {
        child.once("error", reject);
        child.once("close", resolve);
      });
    } finally { clearTimeout(timer); }
    if (timedOut || code !== 0) throw new Error("Fixture issue.close failed: " + stdout + stderr);
    return JSON.parse(stdout);
  }
  async function restoreSummary(since: string) {
    for (const directory of [join(active.home, ".gwt", "logs"), projectLogs]) {
      for (const file of await readdir(directory).catch(() => [])) {
        if (!/\.log(?:\.|$)/.test(file)) continue;
        for (const line of (await readFile(join(directory, file), "utf8")).split("\n")) {
          let row;
          try { row = JSON.parse(line); } catch { continue; } // Retry an incomplete final write.
          if (row.target === "gwt.restore.admission" && row.fields?.scope === "startup restore"
            && row.fields.message === "session restore admission summary"
            && Date.parse(row.timestamp) >= Date.parse(since)) return row.fields;
        }
      }
    }
    return null;
  }
  return { ...active, get url() { return active.url; }, get ownedPid() { return active.ownedPid; },
    issueNumber: issue.number, targetIssueNumber: options.targetIssueNumber, requests, comments, hookAudits, restoreSummary,
    actorEvents: async () => (await readFile(join(active.home, "switch-actor.jsonl"), "utf8").catch(() => ""))
      .split("\n").filter(Boolean).map(line => JSON.parse(line)),
    processAlive: (pid: number) => {
      const result = spawnSync("ps", ["-p", String(pid), "-o", "stat="], { encoding: "utf8" });
      return result.status === 0 && !/^[ZX]/.test(result.stdout.trim());
    },
    launchIssues: async () => Promise.all((await active.launches()).map(async launch => {
      const record = await readFile(join(active.home, ".gwt", "sessions", `${launch.gwt_session_id}.toml`), "utf8");
      return Number(/^linked_issue_number\s*=\s*(\d+)/m.exec(record)?.[1]);
    })),
    savedSource: (cwd: string) => {
      const read = (...args: string[]) => {
        const result = spawnSync(realGitPath, args, { cwd, env: auditEnv, encoding: "utf8" });
        if (result.status !== 0) throw new Error(`Fixture source observation failed: ${result.stderr}`);
        return result.stdout.trim();
      };
      const branch = read("symbolic-ref", "--short", "HEAD");
      return { branch, head: read("rev-parse", "HEAD"),
        remoteHead: read("--git-dir", remotePath, "rev-parse", `refs/heads/${branch}`),
        porcelain: read("status", "--porcelain=v1", "--untracked-files=all") };
    },
    workItems, eventShardPath, stagedDiff, seedActiveOrphan,
    scheduleOperation,
    fixtureOperation: scheduleOperation,
    closeIssue,
    monitorPreferencesBytes: () => readFile(join(projectState, "issue-monitor.json")),
    auditHooks: (cwd: string) => auditHooks(cwd, false),
    executionStatus: (cwd: string) => readOperation("execution.status", cwd),
    terminalExecutionStatus: (cwd: string) => readOperation("execution.status", cwd, true),
    startupReport: () => readOperation("perf.startup", active.project).startup,
    async stop() {
      try { await active.stop(); }
      finally {
        try { await stopDaemon(); }
        finally { await new Promise<void>(resolve => server.close(() => resolve())); }
      }
    },
  };
}
