/** Issue #6622: a normal isolated managed launch supplies its own source proof.
 * The live implementation owner and its old runtime are never used as a fixture.
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFile, realpath } from "node:fs/promises";
import { join, resolve } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { startIssueQueueLaunchFixture } from "./_helpers/issue-queue-launch";
import { writeFixtureNodeCommand } from "./_helpers/exact-relaunch";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const enabled = process.env.GWT_PLAYWRIGHT_BROWSER_PREFLIGHT === "1";
const checkout = resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT ?? process.cwd());
const sha = (bytes: Buffer | string) => createHash("sha256").update(bytes).digest("hex");

test.describe("managed browser preflight from the normal isolated source", () => {
  test.skip(!enabled, "requires the isolated Issue #6622 fixture");
  test.setTimeout(240_000);
  let fixture: Awaited<ReturnType<typeof startIssueQueueLaunchFixture>> | undefined;
  let receipt: any;
  let source: any;
  let bootstrap: Page | undefined;
  const errors: string[] = [];
  const sockets: WebSocket[] = [];
  const captureErrors = (page: Page) => {
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  };

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
      socket.addEventListener("error", () => reject(new Error("fixture control connection failed")), { once: true });
    });
    socket.send(JSON.stringify({ kind: "frontend_ready" }));
    return { messages, send: (payload: unknown) => socket.send(JSON.stringify(payload)) };
  }

  // This is native fixture setup, before the measured browser page is opened.
  // No Session/ECR/authority file or completion stamp is supplied by the runner.
  test.beforeAll(async ({ browser }, info) => {
    // Hooks have their own Playwright timeout; apply this scenario's existing
    // aggregate budget while retaining the bounded per-phase waits below.
    test.setTimeout(240_000);
    try {
      fixture = await startIssueQueueLaunchFixture(info, {
        async prepare({ home, bin, project }) {
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
        if (result.error || result.status !== 0 || /"decision"\s*:\s*"block"/.test(result.stdout)) throw new Error('normal provider hook refused');
        hookCommands++;
      }
      if (!hookCommands) throw new Error('normal provider hook missing');
      const runtime = JSON.parse(fs.readFileSync(process.env.GWT_SESSION_RUNTIME_PATH,'utf8'));
      observation.outcome = runtime.last_completed_hook_outcome;
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
      if (checked.error || checked.status !== 0) throw new Error('canonical consumer refused: '+checked.stderr);
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
          await info.attach("normal-source-provider", { body: JSON.stringify({ sha256: sha(actual) }), contentType: "application/json" });
        },
      });
      // Let the existing frontend render and acknowledge its own startup.
      // This bootstrap page is setup, separate from the measured page below.
      bootstrap = await (await browser.newContext()).newPage();
      captureErrors(bootstrap);
      await gotoLiveGwt(bootstrap, fixture.url, { enableTestBridge: true });
      const hub = await connect(new URL("/ws", fixture.url).toString().replace(/^http/, "ws"));
      await expect.poll(() => hub.messages.findLast(message => message.kind === "hub_state")?.hub.projects[0]?.project_key).toBeTruthy();
      const key = hub.messages.findLast(message => message.kind === "hub_state").hub.projects[0].project_key;
      const control = await connect(new URL(`/ws?repo_hash=${key}`, fixture.url).toString().replace(/^http/, "ws"));
      control.send({ kind: "issue_monitor_queue_push", issue_numbers: [fixture.issueNumber] });
      await expect.poll(async () => (await fixture!.launches()).length, { timeout: 90_000 }).toBe(1);
      source = (await fixture.launches())[0];
      await expect.poll(async () => JSON.parse((await fixture!.monitorPreferencesBytes()).toString())
        .launch_confirmations?.[String(fixture!.issueNumber)]?.window_id, { timeout: 90_000 }).toBeTruthy();
      const windows = () => (control.messages.findLast(message => message.kind === "workspace_state")?.workspace.tabs ?? [])
        .flatMap((tab: any) => tab.workspace.windows);
      await expect.poll(() => windows().find((window: any) => window.session_id === source.gwt_session_id)?.id).toBeTruthy();
      control.send({ kind: "terminal_input", id: windows().find((window: any) => window.session_id === source.gwt_session_id).id,
        data: "GWT_FIXTURE_BROWSER_PREFLIGHT\r" });
      const resultPath = join(fixture.home, `browser-preflight-${source.gwt_session_id}.json`);
      await expect.poll(async () => readFile(resultPath, "utf8").then(text => JSON.parse(text), () => null), { timeout: 90_000 }).toBeTruthy();
      receipt = JSON.parse(await readFile(resultPath, "utf8"));
      expect(receipt.error, JSON.stringify(receipt)).toBeUndefined();
      expect(receipt.source_pid).toBe(source.pid);
      expect(receipt.source_session_id).toBe(source.gwt_session_id);
      expect(receipt.incarnation).toBeGreaterThan(0);
      expect(receipt.outcome).toBe("normal");
      expect(receipt.runtime_binary).toBe(await realpath(join(checkout, "target/debug/gwtd")));
      expect(receipt.runtime_binary_sha256).toBe(sha(await readFile(join(checkout, "target/debug/gwtd"))));
      expect(receipt.before).toMatchObject({ result: "proceed", context: "current", normal_completion: "normal", convergence: "converged" });
      expect(receipt.after).toMatchObject({ result: "proceed", context: "current", normal_completion: "normal", convergence: "converged" });
      await info.attach("actual-source-preflight", { body: JSON.stringify(receipt), contentType: "application/json" });
      await bootstrap.context().close();
      bootstrap = undefined;
    } catch (error) {
      for (const socket of sockets) socket.close();
      await bootstrap?.context().close();
      if (fixture) await fixture.stop();
      fixture = undefined;
      throw error;
    }
  });

  test.afterAll(async () => {
    for (const socket of sockets) socket.close();
    await bootstrap?.context().close();
    if (fixture) await fixture.stop();
    expect(errors, "console/page errors including setup and cleanup").toEqual([]);
  });

  test("current source passes canonical consumer before real themed UI verification", async ({ page }, info) => {
    captureErrors(page);
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    await gotoLiveGwt(page, fixture!.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.locator(".project-bar")).toBeVisible();
    await expect(page.locator(".op-rail")).toBeVisible();
    const served = await page.request.get(new URL("/styles/tokens.css", fixture!.url).toString());
    expect(served.ok()).toBe(true);
    expect(sha(await served.body())).toBe(sha(await readFile(join(checkout, "crates/gwt/web/styles/tokens.css"))));
    for (const name of ["launch", "readiness"]) {
      const canonical = await readFile(join(checkout, `.claude/skills/browser-check/references/${name}.md`));
      expect(await readFile(join(checkout, `.codex/skills/browser-check/references/${name}.md`))).toEqual(canonical);
    }
    await page.screenshot({ path: info.outputPath(`${theme}-managed-browser-preflight.png`) });
    expect(errors, "console/page errors").toEqual([]);
    await info.attach("checkout-binding", { body: JSON.stringify({
      head: execFileSync("git", ["rev-parse", "HEAD"], { cwd: checkout, encoding: "utf8" }).trim(),
      gwt_sha256: sha(await readFile(join(checkout, "target/debug/gwt"))),
      gwtd_sha256: receipt.runtime_binary_sha256, served_tokens_sha256: sha(await served.body()), theme,
    }), contentType: "application/json" });
  });
});
