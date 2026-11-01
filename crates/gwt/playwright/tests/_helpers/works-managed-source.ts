/** T4: a private native source proves its own Normal before the separate UI target starts. */
import { createHash } from "node:crypto";
import { execFileSync, spawn, spawnSync, type ChildProcess } from "node:child_process";
import { createWriteStream } from "node:fs";
import { mkdir, mkdtemp, readFile, realpath, stat, symlink, unlink, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { expect, type Browser, type BrowserContext, type TestInfo } from "@playwright/test";
import { startIssueQueueLaunchFixture } from "./issue-queue-launch";
import { writeFixtureNodeCommand } from "./exact-relaunch";
import { gotoLiveGwt } from "./live-gwt";

const WAIT = 30_000;
const assets = ["app.js", "works-surface.js", "terminal-viewport-reflow.js", "terminal-text-preview.js", "styles/tokens.css"];
const sha = (bytes: Buffer | string) => createHash("sha256").update(bytes).digest("hex");

// Read only the captured PID. Recorder timestamps and kill(pid, 0) are not birth identity.
function processBirth(pid: number): string | null {
  const result = spawnSync("ps", ["-p", String(pid), "-o", "lstart=", "-o", "stat="],
    { encoding: "utf8", timeout: WAIT, env: { ...process.env, LC_ALL: "C", TZ: "UTC" } });
  if (result.error) throw new Error("Works fixture process identity inspection failed");
  if (result.status === 1) return null;
  if (result.status !== 0) throw new Error("Works fixture process identity inspection refused");
  const fields = result.stdout.trim().split(/\s+/);
  if (fields.length < 6 || /^[ZX]/.test(fields[5])) return null;
  return fields.slice(0, 5).join(" ");
}

export async function startWorksManagedSource(browser: Browser, info: TestInfo) {
  const checkout = await realpath(resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd()));
  const gwt = await realpath(join(checkout, "target/debug/gwt"));
  const gwtd = await realpath(join(checkout, "target/debug/gwtd"));
  const binaries = { gwt: sha(await readFile(gwt)), gwtd: sha(await readFile(gwtd)) };
  const sourceAssets = Object.fromEntries(await Promise.all(assets.map(async name =>
    [name, sha(await readFile(join(checkout, "crates/gwt/web", name)))])));
  const mirrors: Record<string, string> = {};
  for (const name of ["launch", "readiness"]) {
    const bytes = await readFile(join(checkout, `.claude/skills/browser-check/references/${name}.md`));
    expect(await readFile(join(checkout, `.codex/skills/browser-check/references/${name}.md`))).toEqual(bytes);
    mirrors[name] = sha(bytes);
  }
  const head = execFileSync("git", ["rev-parse", "HEAD"], { cwd: checkout, encoding: "utf8", timeout: WAIT }).trim();
  const merge = spawnSync("git", ["rev-parse", "--verify", "-q", "MERGE_HEAD"],
    { cwd: checkout, encoding: "utf8", timeout: WAIT });
  if (merge.error || (merge.status !== 0 && merge.status !== 1)) throw new Error("Works merge binding inspection failed");
  const treeBinding = Object.freeze({ merge_head: merge.status === 0 ? merge.stdout.trim() : null,
    diff_head_binary_sha256: sha(execFileSync("git", ["diff", "HEAD", "--binary"],
      { cwd: checkout, timeout: WAIT, maxBuffer: 32 * 1024 * 1024 })),
    helper_sha256: sha(await readFile(join(checkout, "crates/gwt/playwright/tests/_helpers/works-managed-source.ts"))) });
  let fixture: Awaited<ReturnType<typeof startIssueQueueLaunchFixture>> | undefined;
  let context: BrowserContext | undefined;
  let target: ChildProcess | undefined;
  let targetHome: string | undefined;
  let targetBirth: string | null = null;
  let sourceGuiPid = 0;
  let sourceGuiBirth: string | null = null;
  let sourcePid = 0;
  let sourceBirth: string | null = null;
  let stopped: Promise<void> | undefined;
  const sockets: WebSocket[] = [];
  const errors: string[] = [];
  let observationSequence = 0;

  const stop = () => stopped ??= (async () => {
    const failures: unknown[] = [];
    if (target) {
      try {
        const owned = target;
        const stillOwned = () => targetBirth ? processBirth(owned.pid!) === targetBirth
          : owned.exitCode === null && owned.signalCode === null;
        if (owned.pid && stillOwned()) {
          owned.kill("SIGTERM");
          try { await expect.poll(stillOwned, { timeout: 10_000 }).toBe(false); }
          catch {
            if (stillOwned()) owned.kill("SIGKILL");
            await expect.poll(stillOwned, { timeout: 10_000 }).toBe(false);
          }
        }
        const evidence = JSON.stringify({ pid: owned.pid, birth: targetBirth, exited: !owned.pid || !stillOwned() });
        await writeFile(info.outputPath("works-target-cleanup.json"), evidence);
        await info.attach("works-target-cleanup", { body: evidence, contentType: "application/json" });
      } catch (error) { failures.push(error); }
    }
    for (const socket of sockets) {
      try { socket.close(); } catch (error) { failures.push(error); }
    }
    try { await context?.close(); } catch (error) { failures.push(error); }
    try { await fixture?.stop(); } catch (error) { failures.push(error); }
    try {
      const sourceActual = sourcePid ? processBirth(sourcePid) : null;
      const guiActual = sourceGuiPid ? processBirth(sourceGuiPid) : null;
      const sourceGone = sourceActual === null || sourceActual !== sourceBirth;
      const guiGone = guiActual === null || guiActual !== sourceGuiBirth;
      const evidence = JSON.stringify({ source_pid: sourcePid, source_birth: sourceBirth,
        source_exited: sourceGone, gui_exited: guiGone });
      await writeFile(info.outputPath("works-source-cleanup.json"), evidence);
      await info.attach("works-source-cleanup", { body: evidence, contentType: "application/json" });
      expect(sourceGone).toBe(true);
      expect(guiGone).toBe(true);
      expect(errors, "source console/page errors including cleanup").toEqual([]);
    } catch (error) { failures.push(error); }
    if (failures.length) throw new AggregateError(failures, "Works owned fixture cleanup failed");
  })();

  async function connect(url: string) {
    const socket = new WebSocket(url);
    sockets.push(socket);
    const messages: any[] = [];
    socket.addEventListener("message", event => {
      if (typeof event.data !== "string") return;
      try { messages.push(JSON.parse(event.data)); } catch { /* Native PTY frames are not JSON. */ }
    });
    await new Promise<void>((done, reject) => {
      const timer = setTimeout(() => reject(new Error("Works source control connection timed out")), WAIT);
      socket.addEventListener("open", () => { clearTimeout(timer); done(); }, { once: true });
      socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("Works source control connection failed")); }, { once: true });
    });
    socket.send(JSON.stringify({ kind: "frontend_ready" }));
    return { messages, send: (payload: unknown) => socket.send(JSON.stringify(payload)) };
  }

  try {
    fixture = await startIssueQueueLaunchFixture(info, {
      async prepare({ bin, project }) {
        const providerPath = join(bin, process.platform === "win32" ? "codex.cjs" : "codex");
        const provider = await readFile(providerPath, "utf8");
        const marker = "  if (line.trim() === 'GWT_FIXTURE_EXIT')";
        expect(provider.split(marker)).toHaveLength(2);
        const callback = String.raw`
  if (line.trim() === 'GWT_FIXTURE_WORKS_PREFLIGHT') {
    const cp = require('node:child_process');
    const crypto = require('node:crypto');
    const sourceRoot = fs.realpathSync(process.cwd());
    const codeRoot = __CHECKOUT__;
    const observation = { source_pid:process.pid, source_session_id:process.env.GWT_SESSION_ID, source_root:sourceRoot };
    let stage = 'generated_hook';
    try {
      const hooksPath = [sourceRoot,__PROJECT__].map(root=>path.join(root,'.codex/hooks.json')).find(file=>fs.existsSync(file));
      const hooks = JSON.parse(fs.readFileSync(hooksPath,'utf8'));
      const input = JSON.stringify({session_id:native,hook_event_name:'UserPromptSubmit',cwd:sourceRoot,transcript_path:rollout,prompt:line});
      let count = 0;
      for (const group of hooks.hooks?.UserPromptSubmit ?? []) for (const hook of group.hooks ?? []) {
        if (hook.type !== 'command') continue;
        const result = cp.spawnSync('/bin/sh',['-c',hook.command],{cwd:sourceRoot,env:process.env,input,encoding:'utf8',timeout:30000});
        observation.hook = {status:result.status,signal:result.signal,blocked:/"decision"\s*:\s*"block"/.test(result.stdout ?? ''),error_code:result.error?.code ?? null};
        if (result.error || result.status !== 0 || observation.hook.blocked) throw new Error('generated_hook_refused');
        count++;
      }
      if (!count) throw new Error('generated_hook_missing');
      stage = 'native_completion';
      const runtime = JSON.parse(fs.readFileSync(process.env.GWT_SESSION_RUNTIME_PATH,'utf8'));
      observation.completed_at = runtime.last_completed_hook_event_at;
      observation.outcome = runtime.last_completed_hook_outcome;
      observation.incarnation = runtime.runtime_incarnation;
      observation.runtime_binary = fs.realpathSync(process.env.GWT_BIN_PATH);
      observation.runtime_binary_sha256 = crypto.createHash('sha256').update(fs.readFileSync(process.env.GWT_BIN_PATH)).digest('hex');
      stage = 'canonical_consumer';
      const block = (name,reference) => {
        const text = fs.readFileSync(path.join(codeRoot,'.claude/skills/browser-check/references',reference+'.md'),'utf8');
        const parts = text.split('# browser-check-'+name+'-begin');
        if (parts.length !== 2 || !parts[1].includes('# browser-check-'+name+'-end')) throw new Error('canonical_block_missing');
        return parts[1].split('# browser-check-'+name+'-end')[0].split('\n').map(line=>line.startsWith('     ')?line.slice(5):line).join('\n');
      };
      // EXIT observes native typed refusals too; it does not alter their status or authority.
      const report = 'HOOK_DOCTOR_HEALTH_JSON=null\nHOOK_HEALTH_JSON=null\nCHECK_HOOK_BIN=""\n'
        + 'report_preflight() { jq -cn --argjson before "$HOOK_DOCTOR_HEALTH_JSON" --argjson after "$HOOK_HEALTH_JSON" --arg fallback "$CHECK_HOOK_BIN" \'def safe: if type == "object" then {schema_version,result,context,normal_completion,convergence,reasons,diagnostics:(.diagnostics | if type == "array" then map({id,classification}) else . end)} else . end; {before:($before.preflight|safe),after:($after.preflight|safe),hook_bin:$fallback}\'; }\ntrap report_preflight EXIT\n';
      const script = 'set -e\n' + report + block('hook-authority','launch') + '\n' + block('hook-repair','launch') + '\n' + block('hook-audit','readiness');
      const checked = cp.spawnSync('/bin/bash',['-c',script],{cwd:sourceRoot,env:{...process.env,REPO_ROOT:sourceRoot,CHECK_HOME:process.env.HOME,CHECKOUT_GWTD:process.env.GWT_BIN_PATH},encoding:'utf8',timeout:30000});
      observation.consumer = {status:checked.status,signal:checked.signal,error_code:checked.error?.code ?? null};
      try { Object.assign(observation,JSON.parse((checked.stdout ?? '').trim().split(/\r?\n/).at(-1))); } catch {}
      if (checked.error || checked.status !== 0) throw new Error('canonical_consumer_refused');
    } catch(error) { observation.failure_stage = stage; observation.failed = true; }
    fs.writeFileSync(path.join(process.env.HOME,'works-preflight-'+process.env.GWT_SESSION_ID+'.json'),JSON.stringify(observation));
    console.log('GWT_WORKS_PREFLIGHT_RECORDED');
    return;
  }
`.replace("__CHECKOUT__", JSON.stringify(checkout)).replace("__PROJECT__", JSON.stringify(project));
        const actual = provider.replace(marker, callback + marker);
        await writeFixtureNodeCommand(bin, "codex", actual);
        execFileSync(process.execPath, ["--check", providerPath], { timeout: WAIT });
        expect(await readFile(providerPath, "utf8")).toBe(actual);
        await info.attach("works-source-provider", { body: JSON.stringify({ sha256: sha(actual) }), contentType: "application/json" });
      },
    });
    sourceGuiPid = fixture.ownedPid!;
    sourceGuiBirth = processBirth(sourceGuiPid);
    expect(sourceGuiBirth).not.toBeNull();
    context = await browser.newContext();
    const page = await context.newPage();
    page.on("pageerror", () => errors.push("source_page_error"));
    page.on("console", message => { if (message.type() === "error") errors.push("source_console_error"); });
    page.setDefaultTimeout(WAIT);
    page.setDefaultNavigationTimeout(WAIT);
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    const hub = await connect(new URL("/ws", fixture.url).toString().replace(/^http/, "ws"));
    await expect.poll(() => hub.messages.findLast(message => message.kind === "hub_state")?.hub.projects[0]?.project_key,
      { timeout: WAIT }).toBeTruthy();
    const key = hub.messages.findLast(message => message.kind === "hub_state").hub.projects[0].project_key;
    const control = await connect(new URL(`/ws?repo_hash=${key}`, fixture.url).toString().replace(/^http/, "ws"));
    control.send({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] });
    await expect.poll(async () => (await fixture!.launches()).length, { timeout: WAIT }).toBe(1);
    const source = (await fixture.launches())[0];
    expect(source.gwt_session_id).toBeTruthy();
    sourcePid = source.pid;
    sourceBirth = processBirth(sourcePid);
    expect(sourceBirth).not.toBeNull();
    const sourceRoot = await realpath(source.cwd);
    const windows = () => (control.messages.findLast(message => message.kind === "workspace_state")?.workspace.tabs ?? [])
      .flatMap((tab: any) => tab.workspace.windows);
    async function readPaneBinding() {
      // GUI input aliases and durable physical pane IDs join through Session.
      const guiRows = windows().filter((window: any) => window.session_id === source.gwt_session_id
        && ["running", "idle"].includes(window.status));
      expect(guiRows.length, "one live GUI alias for the source Session").toBe(1);
      expect(typeof guiRows[0].id).toBe("string");
      expect(guiRows[0].id.length).toBeGreaterThan(0);
      const workspace = JSON.parse(await readFile(join(fixture!.home, ".gwt/projects", key, "workspace.json"), "utf8"));
      const physicalRows = workspace.windows.filter((row: any) => row.session_id === source.gwt_session_id);
      expect(physicalRows.length, "one durable physical pane for the source Session").toBe(1);
      expect(physicalRows[0].preset).toBe("agent");
      expect(physicalRows[0].linked_issue_number).toBe(fixture!.issueNumber);
      expect(typeof physicalRows[0].id).toBe("string");
      expect(physicalRows[0].id.length).toBeGreaterThan(0);
      const physicalWindowId = `${key}::${physicalRows[0].id}`;
      const prefs = JSON.parse((await fixture!.monitorPreferencesBytes()).toString());
      const confirmation = prefs.launch_confirmations?.[String(fixture!.issueNumber)];
      expect(confirmation?.window_id).toBe(physicalWindowId);
      expect(typeof confirmation.claim_id).toBe("string");
      expect(confirmation.claim_id.length).toBeGreaterThan(0);
      expect(typeof confirmation.delivery_id).toBe("string");
      expect(confirmation.delivery_id.length).toBeGreaterThan(0);
      expect(prefs.launch_bindings?.[physicalWindowId]).toBe(fixture!.issueNumber);
      expect(prefs.launched_issues.filter((entry: any) => entry.issue_number === fixture!.issueNumber))
        .toEqual([{ issue_number: fixture!.issueNumber, window_id: physicalWindowId }]);
      return { project_key: key, session_id: source.gwt_session_id, issue_number: fixture!.issueNumber,
        gui_window_id: guiRows[0].id as string, physical_window_id: physicalWindowId };
    }
    let paneBinding: Awaited<ReturnType<typeof readPaneBinding>>;
    await expect(async () => { paneBinding = await readPaneBinding(); }).toPass({ timeout: WAIT });
    const windowId = paneBinding!.gui_window_id;

    const resultPath = join(fixture.home, `works-preflight-${source.gwt_session_id}.json`);
    let initial: any;
    let previous: any;
    let featureBefore: any;
    async function observe(phase: "launch" | "before" | "post") {
      expect(stopped, "source must remain owned until post").toBeUndefined();
      const birthBefore = { source: processBirth(sourcePid), gui: processBirth(sourceGuiPid) };
      expect(birthBefore.source).toBe(sourceBirth);
      expect(birthBefore.gui).toBe(sourceGuiBirth);
      expect((await fixture!.launches()).length).toBe(1);
      expect(windows().find((window: any) => window.id === windowId)?.session_id).toBe(source.gwt_session_id);
      expect(await readPaneBinding()).toEqual(paneBinding!);
      if (previous) await unlink(resultPath); // Observation only: never native authority/runtime/ledger.
      control.send({ kind: "terminal_input", id: windowId, data: "GWT_FIXTURE_WORKS_PREFLIGHT\r" });
      let receipt: any;
      await expect.poll(async () => {
        try { receipt = JSON.parse(await readFile(resultPath, "utf8")); return receipt; }
        catch { return null; } // A partial observation is not evidence.
      }, { timeout: WAIT }).toBeTruthy();
      const birthAfter = { source: processBirth(sourcePid), gui: processBirth(sourceGuiPid) };
      const evidence = JSON.stringify({ ...receipt, birth_before: birthBefore, birth_after: birthAfter,
        pane_binding: paneBinding!, tree_binding: treeBinding });
      // beforeAll's TestInfo can outlive its first trace; retain every safe
      // observation independently before the private fixture is removed.
      await writeFile(info.outputPath(`works-source-${++observationSequence}-${phase}.json`), evidence);
      await info.attach(`works-source-${phase}`, { body: evidence, contentType: "application/json" });
      expect(receipt.failed, `source ${phase} native protocol`).toBeUndefined();
      expect(receipt.source_pid).toBe(sourcePid);
      expect(receipt.source_session_id).toBe(source.gwt_session_id);
      expect(receipt.source_root).toBe(sourceRoot);
      expect(receipt.incarnation).toBeGreaterThan(0);
      expect(receipt.outcome).toBe("normal");
      expect(typeof receipt.completed_at).toBe("string");
      expect(Number.isFinite(Date.parse(receipt.completed_at))).toBe(true);
      expect(receipt.runtime_binary).toBe(gwtd);
      expect(receipt.runtime_binary_sha256).toBe(binaries.gwtd);
      for (const preflight of [receipt.before, receipt.after]) {
        expect(preflight).toMatchObject({ schema_version: 1, result: "proceed", context: "current",
          normal_completion: "normal", convergence: "converged", reasons: [] });
        expect(preflight.reasons).toEqual([]);
        expect(Array.isArray(preflight.diagnostics)).toBe(true);
        for (const diagnostic of preflight.diagnostics) {
          expect(typeof diagnostic.id).toBe("string");
          expect(diagnostic.id.length).toBeGreaterThan(0);
          expect(["recovered", "fail_open", "foreign"]).toContain(diagnostic.classification);
        }
      }
      if (initial) expect(receipt.incarnation).toBe(initial.incarnation);
      if (previous) expect([receipt.completed_at, receipt.outcome]).not.toEqual([previous.completed_at, previous.outcome]);
      expect(birthAfter.source).toBe(sourceBirth);
      expect(birthAfter.gui).toBe(sourceGuiBirth);
      expect(await readPaneBinding()).toEqual(paneBinding!);
      expect(sha(await readFile(gwt))).toBe(binaries.gwt);
      expect(sha(await readFile(gwtd))).toBe(binaries.gwtd);
      expect(errors).toEqual([]);
      initial ??= receipt;
      previous = receipt;
      return receipt;
    }
    const launchProof = await observe("launch");
    const fallback = launchProof.hook_bin;
    expect(typeof fallback).toBe("string");
    expect(fallback.length).toBeGreaterThan(0);
    expect(fallback).not.toMatch(/(^|[/\\])target[/\\](?:[^/\\]+[/\\])*(?:debug|release)[/\\]gwtd/i);

    // The static target is separate; only public hook configuration crosses this boundary.
    targetHome = await mkdtemp(join(checkout, "target/issue-4777-prechange/works-fresh-home."));
    await mkdir(join(targetHome, ".gwt"), { recursive: true });
    try {
      const runtime = join(homedir(), ".gwt/runtime");
      if ((await stat(runtime)).isDirectory()) await symlink(runtime, join(targetHome, ".gwt/runtime"));
    }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
    const env: NodeJS.ProcessEnv = { ...process.env };
    for (const name of Object.keys(env)) {
      if (/^(GWT_|CODEX_|CLAUDE_|GIT_)/i.test(name) || /^(GH_TOKEN|GITHUB_TOKEN|OPENAI_API_KEY|ANTHROPIC_API_KEY)$/i.test(name)) delete env[name];
    }
    const urlFile = join(targetHome, "browser.url");
    Object.assign(env, { HOME: targetHome, USERPROFILE: targetHome, CODEX_HOME: join(targetHome, ".codex"),
      GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1", GWT_HOOK_BIN: fallback, GWT_BROWSER_URL_FILE: urlFile });
    const scope = spawnSync(gwtd, [], { cwd: checkout, env, encoding: "utf8", timeout: WAIT,
      input: JSON.stringify({ schema_version: 1, operation: "issue.monitor.status", params: {} }) });
    expect(scope.error, "target scope observation").toBeUndefined();
    expect(scope.status).toBe(0);
    const envelope = JSON.parse(scope.stdout);
    expect(envelope.ok).toBe(true);
    const hash = envelope.project_store?.hash;
    expect(hash).toMatch(/^[a-zA-Z0-9_-]+$/);
    const projectState = join(targetHome, ".gwt/projects", hash, "project-state");
    await mkdir(projectState, { recursive: true });
    await writeFile(join(projectState, "pm.json"), JSON.stringify({ settings: { auto_start: false } }));
    await writeFile(join(projectState, "issue-monitor.json"), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] }));
    await writeFile(join(targetHome, ".gwt/session.json"), JSON.stringify({ tabs: [{ id: "works-check", title: "Works check",
      project_root: checkout, kind: "git" }], active_tab_id: "works-check", recent_projects: [] }));
    const log = createWriteStream(join(targetHome, "startup.log"));
    target = spawn(gwt, ["--no-tray", "--no-open"], { cwd: checkout, env, stdio: ["ignore", "pipe", "pipe"] });
    let spawnError = false;
    target.once("error", () => { spawnError = true; });
    target.stdout?.pipe(log, { end: false });
    target.stderr?.pipe(log, { end: false });
    target.once("close", () => log.end());
    expect(target.pid, "owned static target PID").toBeGreaterThan(0);
    targetBirth = processBirth(target.pid!);
    expect(targetBirth).not.toBeNull();
    let url = "";
    await expect.poll(async () => {
      expect(spawnError).toBe(false);
      expect(processBirth(target!.pid!)).toBe(targetBirth);
      try { url = (await readFile(urlFile, "utf8")).trim(); } catch { return false; }
      if (!url) return false;
      try { return (await context!.request.head(url, { timeout: WAIT })).ok(); } catch { return false; }
    }, { timeout: WAIT }).toBe(true);
    for (const base of [fixture.url, url]) for (const asset of assets) {
      const response = await context.request.get(new URL(`/${asset}`, base).toString(), { timeout: WAIT });
      expect(response.ok()).toBe(true);
      expect(sha(await response.body())).toBe(sourceAssets[asset]);
    }
    const binding = Object.freeze({ checkout, head, tree_binding: treeBinding,
      binaries: Object.freeze(binaries), assets: Object.freeze(sourceAssets),
      canonical_mirrors: Object.freeze(mirrors), pane_binding: paneBinding!, source_pid: sourcePid, source_birth: sourceBirth,
      source_session_id: source.gwt_session_id, source_root: sourceRoot, incarnation: launchProof.incarnation,
      source_gui_pid: sourceGuiPid, source_gui_birth: sourceGuiBirth,
      target_home: targetHome, target_pid: target.pid!, target_birth: targetBirth });
    await info.attach("works-source-target-binding", { body: JSON.stringify(binding), contentType: "application/json" });
    return { url, sourceUrl: fixture.url, binding,
      async audit(phase: "before" | "post") {
        if (phase === "before") expect(featureBefore, "previous feature requires its post proof").toBeUndefined();
        else expect(featureBefore, "post proof requires this feature's before").toBeTruthy();
        expect(processBirth(target!.pid!)).toBe(targetBirth);
        const receipt = await observe(phase);
        if (phase === "before") featureBefore = receipt;
        else {
          expect([receipt.completed_at, receipt.outcome]).not.toEqual([featureBefore.completed_at, featureBefore.outcome]);
          featureBefore = undefined;
        }
      }, stop };
  } catch (error) {
    try { await stop(); }
    catch (cleanupError) { throw new AggregateError([error, cleanupError], "Works source/target setup and cleanup failed"); }
    throw error;
  }
}
