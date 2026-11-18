import { spawnSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, withLiveGwtBackendLock } from "./_helpers/live-gwt";

test("gwtd opens a Project and places its Agent in the live Kanban", async ({ page }, info) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const checkHome = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  const gwtd = process.env.GWT_PLAYWRIGHT_GWTD;
  test.skip(!base || !checkHome || !gwtd, "requires browser-check isolated checkout HOME");
  test.setTimeout(180_000);

  await withLiveGwtBackendLock(base!, info, async () => {
    const root = await mkdtemp(join(checkHome!, "envelope-parity-"));
    const env = { ...process.env };
    for (const key of Object.keys(env)) {
      if (/^(GWT_|GIT_|CODEX_|CLAUDE_)/i.test(key)
        || /^(GH_TOKEN|GITHUB_TOKEN|OPENAI_API_KEY|ANTHROPIC_API_KEY)$/i.test(key)) delete env[key];
    }
    Object.assign(env, { HOME: checkHome, USERPROFILE: checkHome, GIT_TERMINAL_PROMPT: "0", GH_PROMPT_DISABLED: "1" });
    const runGit = (...args: string[]) => {
      const result = spawnSync("git", args, { cwd: root, env, encoding: "utf8", timeout: 30_000 });
      if (result.error) throw result.error;
      expect(result.status, result.stdout || result.stderr).toBe(0);
    };
    const envelope = (
      operation: string,
      params: Record<string, unknown> = {},
      caller: { cwd?: string; session?: string; refusal?: RegExp } = {},
    ) => {
      const result = spawnSync(gwtd!, [], {
        cwd: caller.cwd ?? root,
        env: caller.session ? { ...env, GWT_SESSION_ID: caller.session } : env,
        encoding: "utf8",
        input: JSON.stringify({ schema_version: 1, operation, params }),
        timeout: 60_000,
      });
      if (result.error) throw result.error;
      expect(result.status, result.stdout || result.stderr).toBe(caller.refusal ? 1 : 0);
      const reply = JSON.parse(result.stdout);
      if (caller.refusal) {
        expect(reply.ok).toBe(false);
        expect(reply.error).toMatch(caller.refusal);
      } else {
        expect(reply.ok, reply.error).toBe(true);
      }
      return reply;
    };

    // A gitfile models an existing worktree and avoids repository migration.
    runGit("init", "--quiet", "--initial-branch=develop", "--separate-git-dir", `${root}.git`, root);
    runGit("-c", "user.name=GWT acceptance", "-c", "user.email=acceptance@example.invalid",
      "commit", "--quiet", "--allow-empty", "-m", "chore: seed GUI envelope acceptance");
    const key = envelope("issue.monitor.status").project_store.hash;
    expect(key).toMatch(/^[0-9a-f]{16}$/);
    const storage = join(checkHome!, ".gwt", "projects", key);
    const preferences = join(storage, "project-state");
    await mkdir(preferences, { recursive: true });
    await Promise.all([
      writeFile(join(preferences, "pm.json"), JSON.stringify({ settings: { auto_start: false } })),
      writeFile(join(preferences, "issue-monitor.json"), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] })),
      writeFile(join(storage, "workspace.json"), JSON.stringify({
        windows: [
          // The Agent lacks a session or provider identity and stays stopped.
          { id: "parity-agent", title: "Parity Agent", preset: "agent", z_index: 1, status: "stopped" },
          { id: "parity-board", title: "Parity Kanban", preset: "agent_kanban", z_index: 2, status: "idle" },
        ],
        next_z_index: 3,
      })),
    ]);

    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => {
      if (message.type() === "error") errors.push(message.text());
    });
    const theme = info.project.name.endsWith("-light") ? "light" : "dark";
    const projectPath = `/p/${key}`;
    await page.goto(base!);
    await expect(page.locator("[data-hub]")).toBeVisible();
    const projectLink = page.locator(`[data-hub-list="open"] a[href="${projectPath}"]`);
    await expect(projectLink).toHaveCount(0);

    const opened = envelope("project.open", { path: root });
    const payload = opened.data ?? JSON.parse(opened.output);
    expect(payload).toMatchObject({ project_key: key, url_path: projectPath });
    await expect(projectLink).toBeVisible({ timeout: 30_000 });
    await gotoLiveGwt(page, new URL(projectPath, base!).href, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const windows = await page.waitForFunction(projectKey => {
      const state = (window as any).__gwtPlaywrightMessages?.findLast(
        (entry: any) => entry.payload.kind === "workspace_state",
      )?.payload.workspace;
      const rows: Array<{ id: string; preset: string }> | undefined = state?.tabs.find(
        (tab: { project_key: string }) => tab.project_key === projectKey,
      )?.workspace.windows;
      const agent = rows?.find(row => row.preset === "agent" && row.id.endsWith("::parity-agent"));
      const board = rows?.find(row => row.preset === "agent_kanban" && row.id.endsWith("::parity-board"));
      return agent && board ? { agent: agent.id, board: board.id } : null;
    }, key).then(value => value.jsonValue());
    if (!windows) throw new Error("The restored Agent and Kanban board never became available");

    const lane = page.locator(`.agent-kanban-lane[data-board-id="${windows.board}"][data-lane-id="active"]`);
    await expect(lane).toBeVisible();
    const card = lane.locator(`.agent-kanban-card[data-window-id="${windows.agent}"]`);
    await expect(card).toHaveCount(0);
    const placement = { id: windows.agent, board_id: windows.board, lane_id: "active", order: 0 };
    const untrusted = await page.request.post(new URL("/internal/projects/panes/place", base!).href, {
      data: { ...placement, project_key: key },
    });
    expect(untrusted.status()).toBe(401);
    const foreign = await mkdtemp(join(checkHome!, "foreign-envelope-"));
    runGit("-C", foreign, "init", "--quiet", "--initial-branch=develop");
    envelope("pane.layout.place", placement, { cwd: foreign, refusal: /current project/ });
    envelope("pane.layout.place", placement, { session: randomUUID(), refusal: /registered PM/ });
    await expect(card).toHaveCount(0);
    const placed = envelope("pane.layout.place", placement);
    expect(placed.data ?? JSON.parse(placed.output)).toMatchObject({ placed: true });
    await expect(card).toBeVisible();
    await expect(card).toContainText("Parity Agent");
    expect(errors).toEqual([]);
  });
});
