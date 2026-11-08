/* Issue #4975 / SPEC #3431: real PM launch and snapshot-only prompt hooks.
 * POSIX Chromium evidence does not verify native Windows startup latency. */
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, writeFileSync } from "node:fs";
import { mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { expect, test } from "@playwright/test";
import { startExactRelaunchFixture } from "./_helpers/exact-relaunch";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

test.describe("PM startup responsiveness (isolated checkout)", () => {
  test.skip(process.env.GWT_PLAYWRIGHT_EXACT_RELAUNCH !== "1", "requires rebuilt checkout binaries");
  test.skip(process.platform === "win32", "POSIX recorder; native Windows remains a separate gate");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("broken project Git configuration shows one diagnosis and recovers after repair", async ({ page }, info) => {
    const fixture = await startExactRelaunchFixture(info, {
      async prepare({ home, bin, project, env }) {
        env.LANG = "en_US.UTF-8";
        env.LC_ALL = "en_US.UTF-8";
        const git = (...args: string[]) => execFileSync("git", ["-C", project, ...args], { env, encoding: "utf8" });
        const origin = join(home, "origin.git");
        git("clone", "--bare", "--quiet", project, origin);
        git("remote", "add", "origin", origin);
        git("fetch", "--quiet", "origin");
        git("symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main");
        const gh = join(bin, "gh");
        writeFileSync(gh, "#!/bin/sh\nif [ \"$1 $2\" = 'auth token' ]; then exit 1; fi\nif [ \"$1\" = '--version' ]; then echo 'gh version 2.80.0'; else echo '[]'; fi\n");
        chmodSync(gh, 0o755);
      },
      async beforeStart({ preferences }) {
        await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: {
          auto_start: false, launch_profile: { agent_id: "codex" },
        } }));
      },
    });
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const config = join(fixture.project, ".git", "config");
    const healthy = await readFile(config, "utf8");
    try {
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
      await writeFile(config, `${healthy}\n[core\n`);
      await page.locator("#op-pm-entry").click();
      const notices = () => page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_toast" && payload.message?.includes("Git needs attention"))
        .map(({ payload }: any) => payload.message as string));
      await expect.poll(notices, { timeout: 30_000 }).toHaveLength(1);
      expect((await notices())[0]).toContain("git rev-parse --git-common-dir");
      expect((await notices())[0]).toContain("Git stderr:");
      await expect(page.locator('.workspace-window[data-pm="true"]')).toHaveCount(0);
      const pmStatusCount = () => page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "pm_status").length);
      const previousStatusCount = await pmStatusCount();
      await page.locator("#op-pm-entry").click();
      await expect.poll(pmStatusCount).toBeGreaterThan(previousStatusCount);
      expect(await notices()).toHaveLength(1);
      await writeFile(config, healthy);
      await page.locator("#op-pm-entry").click();
      await expect(page.locator('.workspace-window[data-pm="true"]')).toHaveCount(1, { timeout: 90_000 });
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      expect(await readFile(config, "utf8")).toBe(healthy);
      expect(errors).toEqual([]);
      await info.attach("recovered-pm", { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      await writeFile(config, healthy);
      await fixture.stop();
    }
  });

  test("788 unrelated registrations preserve canonical PM launch and a bounded prompt hook", async ({ page }, info) => {
    let preferences = "";
    let childEnv: NodeJS.ProcessEnv = {};
    const git = (cwd: string, ...args: string[]) => execFileSync("git", ["-C", cwd, ...args], {
      env: childEnv, encoding: "utf8", timeout: 30_000,
    }).trim();
    const fixture = await startExactRelaunchFixture(info, {
      async prepare({ home, bin, project, env }) {
        env.LANG = "en_US.UTF-8";
        env.LC_ALL = "en_US.UTF-8";
        childEnv = env;
        // Only local Git and this deterministic gh executable are available.
        const gh = join(bin, "gh");
        writeFileSync(gh, "#!/bin/sh\nif [ \"$1 $2\" = 'auth token' ]; then exit 1; fi\nif [ \"$1\" = '--version' ]; then echo 'gh version 2.80.0'; else echo '[]'; fi\n");
        chmodSync(gh, 0o755);
        const origin = join(home, "origin.git");
        git(project, "clone", "--bare", "--quiet", project, origin);
        git(project, "remote", "add", "origin", origin);
        git(project, "fetch", "--quiet", "origin");
        git(project, "symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main");
        const head = git(project, "rev-parse", "HEAD");
        const common = join(project, ".git");
        for (let index = 0; index < 788; index++) {
          const admin = join(common, "worktrees", `unrelated-${index}`);
          await mkdir(admin, { recursive: true });
          await Promise.all([
            writeFile(join(admin, "HEAD"), `${head}\n`),
            writeFile(join(admin, "commondir"), "../..\n"),
            writeFile(join(admin, "gitdir"), `${join(home, `missing-${index}`, ".git")}\n`),
          ]);
        }
        const malformed = join(common, "worktrees", "unrelated-malformed");
        await mkdir(malformed);
        await writeFile(join(malformed, "HEAD"), "not a valid HEAD\n");
        const codex = join(bin, "codex");
        const provider = await readFile(codex, "utf8");
        const marker = "  if (line.trim() === 'GWT_FIXTURE_EXIT') process.exit(0);";
        expect(provider).toContain(marker);
        // Run the generated hook inside the launched provider's real Session
        // environment. Profiling adds no substitute Session or execution state.
        const command = `
  if (line.trim() === 'GWT_FIXTURE_PROMPT') {
    const cp = require('node:child_process');
    const profile = path.join(process.env.HOME, 'prompt-profile.jsonl');
    const hooks = JSON.parse(fs.readFileSync(path.join(process.cwd(), '.codex/hooks.json'), 'utf8'));
    const input = JSON.stringify({session_id:native,hook_event_name:'UserPromptSubmit',cwd:process.cwd(),transcript_path:rollout,prompt:'Inspect the last recorded PM freshness'});
    const results = [];
    for (const group of hooks.hooks?.UserPromptSubmit ?? []) for (const hook of group.hooks ?? []) {
      if (hook.type !== 'command') continue;
      const result = cp.spawnSync('/bin/sh', ['-c', hook.command], {input,encoding:'utf8',timeout:30000,env:{...process.env,GWT_HOOK_PROFILE_PATH:profile}});
      results.push({command:hook.command,status:result.status,stdout:result.stdout,stderr:result.stderr,error:result.error?.message});
    }
    fs.writeFileSync(path.join(process.env.HOME, 'prompt-result.json'), JSON.stringify({session:process.env.GWT_SESSION_ID,bin:process.env.GWT_BIN_PATH,results,profile:fs.existsSync(profile) ? fs.readFileSync(profile,'utf8') : ''}));
  }
`;
        writeFileSync(codex, provider.replace(marker, `${marker}\n${command}`));
      },
      async beforeStart(context) {
        preferences = context.preferences;
        await writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: {
          auto_start: false, paused: true, launch_profile: { agent_id: "codex" },
        } }));
      },
    });
    const errors: string[] = [];
    const theme = info.project.name.includes("light") ? "light" : "dark";
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    try {
      await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(page.locator('.workspace-window[data-pm="true"]')).toHaveCount(0);
      await page.locator("#op-pm-entry").click();
      const pane = page.locator('.workspace-window[data-pm="true"]');
      await expect(pane).toHaveCount(1, { timeout: 90_000 });
      await expect(pane).toBeVisible();
      await expect.poll(async () => (await fixture.launches()).length, { timeout: 90_000 }).toBe(1);
      const launch = (await fixture.launches())[0];
      const prefs = async () => JSON.parse(await readFile(join(preferences, "pm.json"), "utf8"));
      await expect.poll(async () => (await prefs()).registration?.session_id).toBe(launch.gwt_session_id);
      expect((await prefs()).settings.auto_start).toBe(false);
      expect(JSON.parse(await readFile(join(preferences, "issue-monitor.json"), "utf8")).enabled).toBe(false);
      const registration = (await prefs()).registration;
      const worktree = join(dirname(preferences), "pm", "worktree");
      expect(await realpath(registration.worktree_path)).toBe(await realpath(worktree));
      expect(registration.agent_id).toBe("codex");
      const sessionPath = join(fixture.home, ".gwt/sessions", `${launch.gwt_session_id}.toml`);
      await expect.poll(() => readFile(sessionPath, "utf8"), { timeout: 30_000 })
        .toContain(`agent_session_id = "${launch.provider_session_id}"`);
      const session = await readFile(sessionPath, "utf8");
      expect(await realpath(session.match(/^worktree_path\s*=\s*"([^"]+)"/m)![1])).toBe(await realpath(worktree));
      const freshness = (await prefs()).worktree_freshness;
      expect(freshness.checked_at).toBeTruthy();
      const head = git(worktree, "rev-parse", "HEAD");
      git(fixture.project, "-c", "user.name=gwt", "-c", "user.email=gwt@example.invalid", "commit", "--quiet", "--allow-empty", "-m", "advance local fixture origin");
      git(fixture.project, "push", "--quiet", "origin", "main");
      expect(git(fixture.project, "rev-parse", "origin/main")).not.toBe(head);
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: await pane.getAttribute("data-id"), data: "GWT_FIXTURE_PROMPT\r" });
      const resultPath = join(fixture.home, "prompt-result.json");
      await expect.poll(() => readFile(resultPath, "utf8").catch(() => ""), { timeout: 30_000 }).not.toBe("");
      const result = JSON.parse(await readFile(resultPath, "utf8"));
      const profilePath = info.outputPath(`pm-prompt-${theme}.json`);
      await writeFile(profilePath, JSON.stringify(result, null, 2));
      await info.attach(`pm-prompt-${theme}`, { path: profilePath, contentType: "application/json" });
      const screenshotPath = info.outputPath(`pm-pane-${theme}.png`);
      await page.screenshot({ path: screenshotPath });
      await info.attach(`pm-pane-${theme}`, { path: screenshotPath, contentType: "image/png" });
      expect(result.session).toBe(launch.gwt_session_id);
      expect(await realpath(result.bin)).toBe(await realpath(join(resolve(process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!), "target/debug/gwtd")));
      expect(result.results.length).toBeGreaterThan(0);
      expect(result.results.every((hook: any) => hook.status === 0 && !hook.error)).toBe(true);
      expect(result.results.map((hook: any) => hook.stdout).join("\n")).toContain(`checked_at=${freshness.checked_at}`);
      expect(result.results.map((hook: any) => hook.stdout).join("\n")).toContain("last recorded freshness");
      const profile = result.profile.trim().split("\n").map((line: string) => JSON.parse(line));
      const handler = profile.find((row: any) => row.event === "UserPromptSubmit" && row.handler === "pm-loop-reset");
      expect(handler).toMatchObject({ status: "ok" });
      expect(handler.duration_ms).toBeLessThan(200);
      const total = profile.find((row: any) => row.event === "UserPromptSubmit" && row.handler === "event-total");
      expect(total).toMatchObject({ status: "ok" });
      expect(total.duration_ms).toBeLessThan(200);
      expect(git(worktree, "rev-parse", "HEAD")).toBe(head);
      await expect(page.locator("#canvas, #canvas-stage, #zoom-in-button")).toHaveCount(0);
      await expect(page.locator("#split-surfaces")).toBeVisible();
      expect(errors).toEqual([]);
    } finally {
      const ownedPid = fixture.ownedPid;
      await fixture.stop();
      const cleanup = spawnSync("ps", ["-p", String(ownedPid), "-o", "pid="], { encoding: "utf8" });
      const cleanupPath = info.outputPath(`pm-cleanup-${theme}.json`);
      await writeFile(cleanupPath, JSON.stringify({ owned_pid: ownedPid, ps_status: cleanup.status }));
      await info.attach(`pm-cleanup-${theme}`, { path: cleanupPath, contentType: "application/json" });
      expect(cleanup.status).toBe(1);
    }
  });
});
