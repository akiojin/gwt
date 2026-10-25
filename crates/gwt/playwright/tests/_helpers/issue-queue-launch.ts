import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";
import { createServer } from "node:http";
import { existsSync } from "node:fs";
import { createHash, randomUUID } from "node:crypto";
import { chmod, mkdir, readFile, readdir, realpath, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import type { TestInfo } from "@playwright/test";
import { startExactRelaunchFixture } from "./exact-relaunch";

/** Real monitor, claims, materialization and PTY; GitHub and provider are local recorders. */
export async function startIssueQueueLaunchFixture(info: TestInfo) {
  const comments: { id: number; body: string; updated_at: string }[] = [];
  const requests: { method: string; url: string }[] = [];
  const issue = {
    number: 43, title: "Queue launch fixture", state: "OPEN",
    body: "## Acceptance Criteria\n- [ ] AC-1: Record the provider handoff.",
    labels: [{ name: "bug" }, { name: "auto-improve" }],
    url: "https://github.com/fixture/queue/issues/43",
    updatedAt: new Date().toISOString(),
  };
  const server = createServer(async (request, response) => {
    requests.push({ method: request.method || "", url: request.url || "" });
    let body = "";
    for await (const chunk of request) body += chunk;
    const payload = body ? JSON.parse(body) : {};
    let result: unknown;
    if (request.url === "/graphql" && request.method === "POST") {
      result = { data: { repository: { issue: { ...issue,
        labels: { nodes: issue.labels },
        comments: { nodes: comments.map(comment => ({ databaseId: comment.id,
          body: comment.body, updatedAt: comment.updated_at })),
          pageInfo: { hasNextPage: false, endCursor: null } },
        timelineItems: { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } },
      } } } };
    } else if (request.url === "/repos/fixture/queue/issues/43/comments" && request.method === "POST") {
      const comment = { id: comments.length + 1, body: payload.body, updated_at: new Date().toISOString() };
      comments.push(comment);
      result = comment;
    } else if (/^\/repos\/fixture\/queue\/issues\/comments\/\d+$/.test(request.url || "") && request.method === "PATCH") {
      const comment = comments.find(entry => entry.id === Number(request.url!.split("/").at(-1)));
      if (!comment) { response.writeHead(404); response.end("{}"); return; }
      comment.body = payload.body;
      comment.updated_at = new Date().toISOString();
      result = comment;
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
  const hookAudits: unknown[] = [];
  function auditHooks(cwd: string, repair: boolean) {
    const operation = repair ? "hook.doctor" : "hook.health";
    const result = spawnSync(checkoutGwtd, [], { cwd, env: auditEnv, encoding: "utf8", timeout: 30_000,
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
    }
  }
  let fixture;
  try {
    fixture = await startExactRelaunchFixture(info, {
      monitorPreferences: {
        enabled: true, autonomous_mode: false, auto_close_merged_issues: false,
        auto_apply_updates: false, terminal_queue_auto_refill: false,
        launch_profile: { agent_id: "codex", version: "installed", runtime_target: "Host", skip_permissions: true },
      },
      async beforeStart({ project, env, preferences }) {
        projectState = preferences;
        projectLogs = join(preferences, "..", "logs");
        daemon = spawn(checkoutGwtd, [], { cwd: project, env, stdio: ["pipe", "ignore", "pipe"] });
        let daemonErrors = "";
        daemon.stderr?.on("data", chunk => { daemonErrors += chunk; });
        daemon.stdin?.end(JSON.stringify({ schema_version: 1, operation: "daemon.start", params: {} }) + "\n");
        const until = Date.now() + 30_000;
        while (Date.now() < until) {
          if (daemon.exitCode !== null || daemon.signalCode !== null) throw new Error(`Fixture daemon exited: ${daemonErrors}`);
          const status = spawnSync(checkoutGwtd, [], { cwd: project, env, encoding: "utf8", timeout: 5_000,
            input: JSON.stringify({ schema_version: 1, operation: "daemon.status", params: {} }) });
          if (status.status === 0 && status.stdout.includes(String(daemon.pid))) return;
          await delay(100);
        }
        throw new Error(`Fixture daemon readiness timed out: ${daemonErrors}`);
      },
      async prepare({ home, bin, project, env }) {
        const candidates = [process.env.GWT_HOOK_BIN, process.env.GWT_BIN_PATH,
          spawnSync("which", ["gwtd"], { encoding: "utf8" }).stdout.trim(),
          "/Applications/GWT.app/Contents/MacOS/gwtd"];
        stableHook = candidates.find(path => path && path.startsWith("/") && existsSync(path)
          && !/[/\\]target[/\\].*[/\\](?:debug|release)[/\\]gwtd/.test(path)
          && !/[/\\]target[/\\](?:debug|release)[/\\]gwtd/.test(path)) || "";
        if (!stableHook) throw new Error("No stable installed gwtd hook fallback available");
        env.GWT_HOOK_BIN = stableHook;
        auditEnv = env;
        checkoutGwtd = join(bin, "gwtd");
        const realGit = spawnSync("which", ["git"], { encoding: "utf8" }).stdout.trim();
        const git = (...args: string[]) => {
          const result = spawnSync(realGit, args, { cwd: project, env, encoding: "utf8" });
          if (result.status !== 0) throw new Error(`Fixture git failed: ${result.stderr}`);
        };
        const remote = join(home, "origin.git");
        git("init", "--bare", "-q", remote);
        git("remote", "add", "origin", remote);
        git("push", "-q", "-u", "origin", "main");
        git("--git-dir", remote, "symbolic-ref", "HEAD", "refs/heads/main");
        git("symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main");
        const gitWrapper = `#!${process.execPath}\nconst cp=require('node:child_process');\nconst a=process.argv.slice(2);\nif(a.slice(-3).join(' ')==='remote get-url origin'){console.log('https://github.com/fixture/queue.git');process.exit(0);}\nconst r=cp.spawnSync(${JSON.stringify(realGit)},a,{stdio:'inherit'});process.exit(r.status??1);\n`;
        await writeFile(join(bin, "git"), gitWrapper);
        await chmod(join(bin, "git"), 0o755);
        await writeFile(join(home, ".codex", "auth.json"), JSON.stringify({ OPENAI_API_KEY: "fixture-only" }));
        const ghLog = join(home, "gh-argv.jsonl");
        await writeFile(ghLog, "");
        const gh = `#!${process.execPath}
const fs=require('node:fs');const a=process.argv.slice(2);const text=a.join(' ');
fs.appendFileSync(${JSON.stringify(ghLog)},JSON.stringify(a)+'\\n');
const issue=${JSON.stringify(issue)};
if(a[0]==='--version'){console.log('gh version 2.80.0');process.exit(0);}
if(a[0]==='auth'){if(a[1]==='token')console.log('fixture-only');process.exit(0);}
if(a.includes('--include'))process.stdout.write('HTTP/2.0 200 OK\\r\\n\\r\\n');
if(a[0]==='api' && text.includes('/issues?')){console.log(JSON.stringify(/[?&]page=1(?:&| |$)/.test(text)?[issue]:[]));process.exit(0);}
if(a[0]==='api' && text.includes('/pulls?')){console.log('[]');process.exit(0);}
if(a[0]==='issue'&&a[1]==='view'){console.log(JSON.stringify({...issue,comments:[]}));process.exit(0);}
if(a[0]==='api'&&a[1]==='graphql'){console.log(JSON.stringify({data:{repository:{issue:{timelineItems:{nodes:[],pageInfo:{hasNextPage:false,endCursor:null}}}}}}));process.exit(0);}
if(a[0]==='pr'&&a[1]==='list'){console.log('[]');process.exit(0);}
console.error('Unimplemented fixture gh: '+text);process.exit(64);
`;
        await writeFile(join(bin, "gh"), gh);
        await chmod(join(bin, "gh"), 0o755);
        Object.assign(env, {
          RUST_LOG: "info",
          GWT_OWNER_GITHUB_TEST_MODE: "loopback-v1",
          GWT_OWNER_GITHUB_REST_BASE: endpoint,
          GWT_OWNER_GITHUB_GRAPHQL_URL: `${endpoint}/graphql`,
          GWT_OWNER_GITHUB_TOKEN: "fixture-only",
          GWT_TEST_GH: join(bin, "gh"),
        });
        auditHooks(project, true);
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
    const result = spawnSync("git", ["diff", "--cached", "--binary"], { cwd, env: auditEnv });
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
    const staged = spawnSync("git", ["add", "--", stagedPath], { cwd: worktree, env: auditEnv });
    if (staged.status !== 0) throw new Error("Fixture stage failed");
    return { predecessorId: predecessor.id, successorId, orphan, worktree,
      branch: start.execution_container.branch, stagedDiff: stagedDiff(worktree) };
  }
  function readOperation(operation: "execution.status" | "perf.startup", cwd: string) {
    const result = spawnSync(checkoutGwtd, [], { cwd, env: auditEnv, encoding: "utf8", timeout: 30_000,
      input: JSON.stringify({ schema_version: 1, operation,
        params: operation === "execution.status" ? { issue: issue.number } : {} }),
    });
    if (result.status !== 0) throw new Error(`Fixture ${operation} failed`);
    const envelope = JSON.parse(result.stdout);
    if (!envelope.ok) throw new Error(`Fixture ${operation} refused`);
    return typeof envelope.output === "string" ? JSON.parse(envelope.output) : envelope.output;
  }
  function scheduleOperation(
    operation: "issue.monitor.schedule.read" | "issue.monitor.schedule.reassign",
    params: Record<string, unknown> = {},
  ) {
    const result = spawnSync(checkoutGwtd, [], { cwd: active.project, env: auditEnv, encoding: "utf8", timeout: 30_000,
      input: JSON.stringify({ schema_version: 1, operation, params: { ...params, project_root: active.project } }),
    });
    if (result.error) throw result.error;
    if (!result.stdout.trim()) throw new Error(`Fixture ${operation} returned no envelope: ${result.stderr}`);
    return JSON.parse(result.stdout);
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
    issueNumber: issue.number, requests, comments, hookAudits, restoreSummary,
    workItems, eventShardPath, stagedDiff, seedActiveOrphan,
    scheduleOperation,
    monitorPreferencesBytes: () => readFile(join(projectState, "issue-monitor.json")),
    auditHooks: (cwd: string) => auditHooks(cwd, false),
    executionStatus: (cwd: string) => readOperation("execution.status", cwd),
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
