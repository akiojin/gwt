/** An ordinary private managed source supplies its own before/post proof. */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFile, realpath, unlink } from "node:fs/promises";
import { join, resolve } from "node:path";
import { expect, type Browser, type BrowserContext, type TestInfo } from "@playwright/test";
import { startIssueQueueLaunchFixture } from "./issue-queue-launch";
import { writeFixtureNodeCommand } from "./exact-relaunch";
import { gotoLiveGwt } from "./live-gwt";

const sha = (bytes: Buffer | string) => createHash("sha256").update(bytes).digest("hex");

export async function startManagedBrowserSource(browser: Browser, info: TestInfo) {
  const checkout = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
  let fixture: Awaited<ReturnType<typeof startIssueQueueLaunchFixture>> | undefined;
  let context: BrowserContext | undefined;
  let stopResult: Promise<void> | undefined;
  const sockets: WebSocket[] = [];
  const errors: string[] = [];
  const stop = () => stopResult ??= (async () => {
    const failures: unknown[] = [];
    for (const socket of sockets) {
      try { socket.close(); } catch (error) { failures.push(error); }
    }
    try { await context?.close(); } catch (error) { failures.push(error); }
    try { await fixture?.stop(); } catch (error) { failures.push(error); }
    try { expect(errors, "managed source console/page errors including cleanup").toEqual([]); }
    catch (error) { failures.push(error); }
    if (failures.length) throw new AggregateError(failures, "managed source cleanup failed:\n" + failures.map(String).join("\n").slice(0, 16384));
  })();
  async function connect(url: string) {
    const socket = new WebSocket(url);
    sockets.push(socket);
    const messages: any[] = [];
    socket.addEventListener("message", event => {
      if (typeof event.data !== "string") return;
      try { messages.push(JSON.parse(event.data)); } catch { /* PTY bytes are separate frames. */ }
    });
    await new Promise<void>((done, reject) => {
      socket.addEventListener("open", () => done(), { once: true });
      socket.addEventListener("error", () => reject(new Error("managed source control connection failed")), { once: true });
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
        expect(provider).toContain(marker);
        const actor = String.raw`
  if (line.trim() === 'GWT_FIXTURE_BROWSER_PREFLIGHT') {
    const cp = require('node:child_process');
    const crypto = require('node:crypto');
    const sourceRoot = process.cwd();
    const codeRoot = __CHECKOUT__;
    const workspaceRoot = __PROJECT__;
    const observation = { source_pid:process.pid, source_session_id:process.env.GWT_SESSION_ID };
    try {
      // The fixture provider follows its ordinary input callback through the
      // generated native hook command, as it already does for SessionStart.
      const hooksPath = [sourceRoot,workspaceRoot].map(root=>path.join(root,'.codex/hooks.json')).find(p=>fs.existsSync(p));
      const hooks = JSON.parse(fs.readFileSync(hooksPath,'utf8'));
      const input = JSON.stringify({session_id:native,hook_event_name:'UserPromptSubmit',cwd:sourceRoot,
        transcript_path:rollout,prompt:line});
      let hookCommands = 0;
      for (const group of hooks.hooks.UserPromptSubmit ?? []) for (const hook of group.hooks ?? []) {
        if (hook.type !== 'command') continue;
        const result = cp.spawnSync('/bin/sh',['-c',hook.command],{cwd:sourceRoot,env:process.env,input,encoding:'utf8',timeout:60000});
        const blocked = /"decision"\s*:\s*"block"/.test(result.stdout);
        observation.hook = {status:result.status,signal:result.signal,blocked,
          error:result.error ? {name:result.error.name,code:result.error.code ?? null} : null};
        if (result.error || result.status !== 0 || blocked) throw new Error('normal provider hook refused');
        hookCommands++;
      }
      if (!hookCommands) throw new Error('normal provider hook missing');
      const runtime = JSON.parse(fs.readFileSync(process.env.GWT_SESSION_RUNTIME_PATH,'utf8'));
      observation.outcome = runtime.last_completed_hook_outcome;
      observation.completed_at = runtime.last_completed_hook_event_at;
      observation.source_root = fs.realpathSync(sourceRoot);
      observation.incarnation = runtime.runtime_incarnation;
      observation.runtime_binary = fs.realpathSync(process.env.GWT_BIN_PATH);
      observation.runtime_binary_sha256 = crypto.createHash('sha256').update(fs.readFileSync(process.env.GWT_BIN_PATH)).digest('hex');
      const block = (name,reference) => {
        const text = fs.readFileSync(path.join(codeRoot,'.claude/skills/browser-check/references',reference+'.md'),'utf8');
        return text.split('# browser-check-'+name+'-begin')[1].split('# browser-check-'+name+'-end')[0]
          .split('\n').map(line=>line.startsWith('     ')?line.slice(5):line).join('\n');
      };
      const script = 'set -e\n' + block('hook-authority','launch') + '\n'
        + block('hook-repair','launch') + '\n' + block('hook-audit','readiness')
        + '\njq -n --argjson before "$HOOK_DOCTOR_HEALTH_JSON" --argjson after "$HOOK_HEALTH_JSON" \'{before:$before.preflight,after:$after.preflight}\'';
      const checked = cp.spawnSync('/bin/bash',['-c',script],{cwd:sourceRoot,env:{...process.env,
        REPO_ROOT:sourceRoot,CHECK_HOME:process.env.HOME,CHECKOUT_GWTD:process.env.GWT_BIN_PATH},encoding:'utf8',timeout:60000});
      observation.consumer = {status:checked.status,signal:checked.signal,
        error:checked.error ? {name:checked.error.name,code:checked.error.code ?? null} : null};
      if (checked.error || checked.status !== 0) {
        // Preserve this existing native refusal; no additional Doctor/probe runs.
        const stdout = checked.stdout ?? '', stderr = checked.stderr ?? '';
        Object.assign(observation.consumer, {stdout:stdout.slice(0,16384),stderr:stderr.slice(0,16384),
          stdout_truncated:stdout.length > 16384,stderr_truncated:stderr.length > 16384});
        throw new Error('canonical consumer refused');
      }
      Object.assign(observation, JSON.parse(checked.stdout));
    } catch(error) { observation.error = error.message; }
    fs.writeFileSync(path.join(process.env.HOME,'browser-preflight-'+process.env.GWT_SESSION_ID+'.json'),JSON.stringify(observation));
    console.log('GWT_BROWSER_PREFLIGHT_RECORDED');
    return;
  }
`.replace("__CHECKOUT__", JSON.stringify(checkout)).replace("__PROJECT__", JSON.stringify(project));
        const actual = provider.replace(marker, actor + marker);
        await writeFixtureNodeCommand(bin, "codex", actual);
        execFileSync(process.execPath, ["--check", providerPath]);
        expect(await readFile(providerPath, "utf8")).toBe(actual);
        await info.attach("managed-source-provider", { body: JSON.stringify({ sha256: sha(actual) }), contentType: "application/json" });
      },
    });
    context = await browser.newContext();
    const page = await context.newPage();
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    const hub = await connect(new URL("/ws", fixture.url).toString().replace(/^http/, "ws"));
    await expect.poll(() => hub.messages.findLast(message => message.kind === "hub_state")?.hub.projects[0]?.project_key).toBeTruthy();
    const key = hub.messages.findLast(message => message.kind === "hub_state").hub.projects[0].project_key;
    const control = await connect(new URL(`/ws?repo_hash=${key}`, fixture.url).toString().replace(/^http/, "ws"));
    control.send({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] });
    await expect.poll(async () => (await fixture!.launches()).length, { timeout: 90_000 }).toBe(1);
    const source = (await fixture.launches())[0];
    expect(source.gwt_session_id).toBeTruthy();
    await expect.poll(async () => JSON.parse((await fixture!.monitorPreferencesBytes()).toString())
      .launch_confirmations?.[String(fixture!.issueNumber)]?.window_id, { timeout: 90_000 }).toBeTruthy();
    const windows = () => (control.messages.findLast(message => message.kind === "workspace_state")?.workspace.tabs ?? [])
      .flatMap((tab: any) => tab.workspace.windows);
    await expect.poll(() => windows().find((window: any) => window.session_id === source.gwt_session_id)?.id).toBeTruthy();
    const windowId = windows().find((window: any) => window.session_id === source.gwt_session_id).id;
    const resultPath = join(fixture.home, `browser-preflight-${source.gwt_session_id}.json`);
    const binary = await realpath(join(checkout, "target/debug/gwtd"));
    const binarySha = sha(await readFile(binary));
    let before: any;
    async function audit(phase: "before" | "post") {
      if (phase === "post") {
        await info.attach("managed-source-prior-observation", { body: await readFile(resultPath), contentType: "application/json" });
        // Only this helper's observation is removed; native authority is untouched.
        await unlink(resultPath);
      }
      control.send({ kind: "terminal_input", id: windowId, data: "GWT_FIXTURE_BROWSER_PREFLIGHT\r" });
      let receipt: any;
      await expect.poll(async () => {
        try { receipt = JSON.parse(await readFile(resultPath, "utf8")); return receipt; }
        catch { return null; } // A partial write is not an observation.
      }, { timeout: 90_000 }).toBeTruthy();
      await info.attach(`managed-source-${phase}`, { body: JSON.stringify(receipt), contentType: "application/json" });
      expect(receipt.error, `managed source ${phase} callback`).toBeUndefined();
      expect(receipt.source_pid).toBe(source.pid);
      expect(receipt.source_session_id).toBe(source.gwt_session_id);
      expect(receipt.incarnation).toBeGreaterThan(0);
      expect(receipt.source_root).toBe(await realpath(source.cwd));
      expect(receipt.completed_at).toEqual(expect.any(String));
      expect(Number.isFinite(Date.parse(receipt.completed_at))).toBe(true);
      expect(receipt.outcome).toBe("normal");
      expect(receipt.runtime_binary).toBe(binary);
      expect(receipt.runtime_binary_sha256).toBe(binarySha);
      for (const preflight of [receipt.before, receipt.after]) {
        expect(preflight).toMatchObject({ schema_version: 1, result: "proceed", context: "current",
          normal_completion: "normal", convergence: "converged", reasons: [] });
        expect(Array.isArray(preflight.diagnostics)).toBe(true);
        for (const diagnostic of preflight.diagnostics) {
          expect(typeof diagnostic.id).toBe("string");
          expect(diagnostic.id.length).toBeGreaterThan(0);
          expect(["recovered", "fail_open", "foreign"]).toContain(diagnostic.classification);
        }
      }
      if (phase === "post") {
        for (const field of ["source_pid", "source_session_id", "incarnation", "source_root", "runtime_binary", "runtime_binary_sha256"])
          expect(receipt[field]).toBe(before[field]);
        expect([receipt.completed_at, receipt.outcome]).not.toEqual([before.completed_at, before.outcome]);
      } else { before = receipt; }
      expect(errors, `managed source ${phase} console/page errors`).toEqual([]);
    }
    // Bind the bootstrap's served asset, canonical mirrors and actual binaries.
    const served = await page.request.get(new URL("/styles/tokens.css", fixture.url).toString());
    expect(served.ok()).toBe(true);
    expect(sha(await served.body())).toBe(sha(await readFile(join(checkout, "crates/gwt/web/styles/tokens.css"))));
    const mirrors: Record<string, string> = {};
    for (const name of ["launch", "readiness"]) {
      const canonical = await readFile(join(checkout, `.claude/skills/browser-check/references/${name}.md`));
      expect(await readFile(join(checkout, `.codex/skills/browser-check/references/${name}.md`))).toEqual(canonical);
      mirrors[name] = sha(canonical);
    }
    await info.attach("managed-source-checkout-binding", { body: JSON.stringify({
      head: execFileSync("git", ["rev-parse", "HEAD"], { cwd: checkout, encoding: "utf8" }).trim(),
      gwt_sha256: sha(await readFile(join(checkout, "target/debug/gwt"))), gwtd_sha256: binarySha,
      served_tokens_sha256: sha(await served.body()), canonical_mirrors: mirrors,
    }), contentType: "application/json" });
    await audit("before");
    return { postAudit: () => audit("post"), stop };
  } catch (error) {
    try { await stop(); }
    catch (cleanupError) { throw new AggregateError([error, cleanupError], "managed source setup and cleanup failed:\n" + [error, cleanupError].map(String).join("\n").slice(0, 16384)); }
    throw error;
  }
}
