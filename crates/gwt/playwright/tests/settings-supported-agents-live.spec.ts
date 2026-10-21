/** SPEC #1921 / Issue #5017: real detection, fixture installers and live PTY safety. */
import { expect, test, type Locator, type Page, type TestInfo } from "@playwright/test";
import { readFile, unlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import {
  acquireLiveGwtBackendLock,
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  openLiveLaunchWizardForBranch,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

type Backend = { url: string; home: string; project: string; pid: number };
const BACKENDS: Record<string, Backend> = JSON.parse(process.env.GWT_L3_BACKENDS ?? "{}");
const MISSING_AGENT = process.env.GWT_L3_MISSING_AGENT ?? "agy";
const themeFor = (info: TestInfo) => info.project.name.endsWith("light") ? "light" : "dark";
const backendFor = (info: TestInfo): Backend => BACKENDS[themeFor(info)] ?? {
  url: process.env.GWT_PLAYWRIGHT_BASE_URL ?? "",
  home: process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "",
  project: process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? process.cwd(),
  pid: Number(process.env.GWT_PLAYWRIGHT_CHECK_PID),
};
const lines = async <T,>(path: string): Promise<T[]> => (await readFile(path, "utf8").catch(() => ""))
  .split("\n").filter(Boolean).map(line => JSON.parse(line));
const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };

async function cursor(page: Page): Promise<number> {
  return page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence || 0);
}

async function maintenance(page: Page, after: number, id: string, success: boolean) {
  return (await page.waitForFunction(({ after, id, success }) => {
    return (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
      entry.sequence > after && entry.payload.kind === "supported_agent_maintenance"
      && entry.payload.agent_id === id && !entry.payload.pending && entry.payload.success === success)?.payload;
  }, { after, id, success }, { timeout: 30_000 })).jsonValue();
}

async function catalog(page: Page, after = 0) {
  return (await page.waitForFunction(after => (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
    entry.sequence > after && entry.payload.kind === "supported_agent_list")?.payload,
  after, { timeout: 30_000 })).jsonValue();
}

async function screenshot(settings: Locator, info: TestInfo, name: string) {
  const theme = themeFor(info);
  await test.step("records the real rendered Operator theme", async () => {
    await expect(settings.page().locator("html")).toHaveAttribute("data-theme", theme);
    await info.attach(`${name}-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
    if (process.env.GWT_L3_SCREENSHOT_DIR) await settings.screenshot({
      path: join(process.env.GWT_L3_SCREENSHOT_DIR, `${name}-${theme}.png`),
    });
  });
}

test.describe.serial("Supported Agents Settings (isolated live backend)", () => {
  test.skip((!process.env.GWT_PLAYWRIGHT_BASE_URL && !process.env.GWT_L3_BACKENDS)
    || !process.env.GWT_L3_AGENT_FIXTURES, "requires checkout/fresh HOME with version and installer fixtures");
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(120_000);
  let errors: string[];
  let release: (() => Promise<void>) | undefined;
  let settings: Locator;
  let panel: Locator;

  test.beforeEach(async ({ page }, info) => {
    const backend = backendFor(info);
    errors = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    release = await acquireLiveGwtBackendLock(backend.url, info);
    await gotoLiveGwt(page, backend.url, { enableTestBridge: true });
    await openLiveGwtProject(page, backend.project);
    const migration = page.locator("#migration-modal.open");
    if (await migration.count()) {
      await migration.getByRole("button", { name: "Migrate", exact: true }).click();
      await expect(migration).toHaveCount(0, { timeout: 60_000 });
      backend.project = String(await (await page.waitForFunction(() =>
        (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.payload.kind === "migration_done")?.payload.branch_worktree_path,
      )).jsonValue());
      await openLiveGwtProject(page, backend.project);
    }
    await page.evaluate(() => document.dispatchEvent(
      new CustomEvent("settings:open", { detail: { target: "supported-agents" } }),
    ));
    settings = page.locator('.workspace-window[data-preset="settings"]').last();
    await expect(settings).toBeVisible();
    await expect(settings.getByRole("tab", { name: "Supported Agents", exact: true }))
      .toHaveAttribute("aria-selected", "true");
    panel = settings.locator('[data-settings-panel="supported-agents"]');
    await expect(panel).toBeVisible();
    const catalogAfter = await cursor(page);
    await sendLiveGwtEvent(page, { kind: "list_supported_agents" });
    await catalog(page, catalogAfter);
    await expect(panel.locator("tbody [data-agent-id]")).toHaveCount(8);
    await expect(panel.locator(".settings-help")).toContainText("At the next app startup");
    await expect(panel.locator(".settings-help")).toContainText("installed agents with a verified update route");
    await expect(panel.locator(".settings-help")).toContainText("Nothing is installed automatically");
  });

  test.afterEach(async ({ page }, info) => {
    try {
      await info.attach("supported-agents-live-evidence", { body: JSON.stringify({
        theme: themeFor(info), backend: backendFor(info), errors,
        messages: await page.evaluate(() => (window as any).__gwtPlaywrightMessages ?? []),
        npm: await lines(join(backendFor(info).home, "npm-argv.jsonl")),
        gh: await lines(join(backendFor(info).home, "gh-argv.jsonl")),
        curl: await lines(join(backendFor(info).home, "curl-argv.jsonl")),
        native_updates: await lines(join(backendFor(info).home, "native-update-argv.jsonl")),
        rejected: await lines(join(backendFor(info).home, "rejected-operations.jsonl")),
        metadata: await lines(join(backendFor(info).home, "metadata-events.jsonl")),
        providers: await lines(join(backendFor(info).home, "provider-argv.jsonl")),
      }, null, 2), contentType: "application/json" });
      expect(errors, "console and page errors across the complete live flow").toEqual([]);
    } finally { await release?.(); release = undefined; }
  });

  test("shows the full catalog and distinguishes unknown from uninstalled versions", async ({ page }, info) => {
    const claude = panel.locator('[data-agent-id="claude"]');
    await expect(claude).toContainText("Installed");
    await expect(claude).toContainText("2.1.0");
    await expect(claude).toContainText(/channel/i);
    await expect(claude.getByRole("button", { name: /^Update / })).toHaveCount(0);
    const hermes = panel.locator('[data-agent-id="hermes"]');
    await expect(hermes).toContainText("Installed");
    await expect(hermes).toContainText(/channel/i);
    await expect(hermes.getByRole("button", { name: /^Update / })).toHaveCount(0);
    const copilot = panel.locator('[data-agent-id="gh"]');
    await expect(copilot).toContainText("Installed");
    await expect(copilot).toContainText("1.0.0");
    await expect(copilot).toContainText(/distribution cannot be verified.*built-in CLI or an extension/i);
    await expect(copilot.getByRole("button", { name: /^Update / })).toHaveCount(0);
    const unknown = panel.locator('[data-agent-id="codex"]');
    await expect(unknown).toContainText("Installed");
    await expect(unknown).toContainText("Unknown (version unavailable)");
    if (process.env.GWT_L3_BACKENDS) await expect(unknown).toContainText("Fixture Codex metadata unavailable");
    const missing = panel.locator(`[data-agent-id="${MISSING_AGENT}"]`);
    await expect(missing).toContainText("Not installed");
    await expect(missing).not.toContainText("Unknown");
    const state = await catalog(page);
    expect(state.agents.map((agent: any) => agent.id)).toEqual([
      "claude", "codex", "grok", "agy", "opencode", "openclaw", "hermes", "gh",
    ]);
    if (process.env.GWT_L3_BACKENDS) {
      const metadata = await lines<{ id: string; stage: string }>(join(backendFor(info).home, "metadata-events.jsonl"));
      const codexStarted = metadata.findIndex(event => event.id === "codex" && event.stage === "started");
      const openclawFinished = metadata.findIndex(event => event.id === "openclaw" && event.stage === "finished");
      expect(codexStarted, "Codex metadata begins in the concurrent catalog batch").toBeGreaterThanOrEqual(0);
      expect(openclawFinished, "OpenClaw completes after observing Codex start").toBeGreaterThan(codexStarted);
      expect(state.agents.find((agent: any) => agent.id === "claude").available_version).toBeNull();
      expect(state.agents.find((agent: any) => agent.id === "codex").update_check_error)
        .toContain("Fixture Codex metadata unavailable");
      expect(state.agents.find((agent: any) => agent.id === "grok").available_version).toBe("2.0.0");
      expect(state.agents.find((agent: any) => agent.id === "agy").available_version).toBe("1.3.5");
      expect(state.agents.find((agent: any) => agent.id === "gh").available_version).toBeNull();
    }
    await settings.getByRole("tab", { name: "System", exact: true }).click();
    await expect(panel).toBeHidden();
    await settings.getByRole("tab", { name: "Supported Agents", exact: true }).click();
    await expect(panel.locator("tbody [data-agent-id]")).toHaveCount(8);
    await screenshot(settings, info, "supported-agents");
  });

  test("refreshes Antigravity metadata, hides failed checks and updates its detected installation", async ({ page }, info) => {
    test.skip(!process.env.GWT_L3_BACKENDS, "requires the bounded official-distribution fixtures");
    const backend = backendFor(info), row = panel.locator('[data-agent-id="agy"]');
    const unavailable = join(backend.home, "non-npm-metadata-unavailable");
    const refresh = async () => {
      const after = await cursor(page);
      await sendLiveGwtEvent(page, { kind: "list_supported_agents" });
      return catalog(page, after);
    };
    await expect(row.getByRole("button", { name: "Update Antigravity CLI", exact: true })).toBeVisible();
    await writeFile(unavailable, "unavailable");
    try {
      const failed = (await refresh()).agents.find((agent: any) => agent.id === "agy");
      expect(failed.installed_version).toContain("1.0.0");
      expect(failed.available_version).toBeNull();
      expect(failed.update_available).toBe(false);
      expect(failed.update_check_error).toContain("Fixture Antigravity metadata unavailable");
      await expect(row).toContainText("Fixture Antigravity metadata unavailable");
      await expect(row.getByRole("button", { name: /^Update / })).toHaveCount(0);
      await expect(panel.locator('[data-agent-id="grok"]')).toContainText("2.0.0");
      await screenshot(settings, info, "supported-agents-metadata-failed");
    } finally { await unlink(unavailable); }
    const recovered = (await refresh()).agents.find((agent: any) => agent.id === "agy");
    expect(recovered.available_version).toBe("1.3.5");
    expect(recovered.update_available).toBe(true);
    const after = await cursor(page);
    await row.getByRole("button", { name: "Update Antigravity CLI", exact: true }).click();
    const result = await maintenance(page, after, "agy", true);
    expect(result.before_version).toContain("1.0.0");
    expect(result.after_version).toContain("1.3.5");
    const updated = (await refresh()).agents.find((agent: any) => agent.id === "agy");
    expect(updated.installed_version).toContain("1.3.5");
    expect(updated.available_version).toBe("1.3.5");
    expect(updated.update_available).toBe(false);
    await expect(row).toContainText("Latest");
    await expect(row.getByRole("button", { name: /^Update / })).toHaveCount(0);
    expect((await lines<string[]>(join(backend.home, "gh-argv.jsonl")))
      .every(args => ["--version", "copilot --version",
        "pr list --json number,title,headRefName,state --state all --limit 999"].includes(args.join(" "))),
    "GitHub Copilot detection cannot invoke metadata or an unverified updater").toBe(true);
    expect(await lines(join(backend.home, "native-update-argv.jsonl")))
      .toEqual([{ command: "agy", args: ["update"] }]);
    expect(await lines(join(backend.home, "rejected-operations.jsonl"))).toEqual([]);
    await screenshot(settings, info, "supported-agents-native-updated");
  });

  test("reports an actionable install failure, retries and immediately detects the installed CLI", async ({ page }, info) => {
    test.skip(!process.env.GWT_L3_BACKENDS, "requires the bounded Issue #5017 installer fixture");
    const backend = backendFor(info), row = panel.locator('[data-agent-id="opencode"]');
    const installs = async () => (await lines<string[]>(join(backend.home, "npm-argv.jsonl")))
      .filter(args => args[0] === "install" && args[2].startsWith("opencode-ai"));
    expect(await installs()).toHaveLength(0);
    let after = await cursor(page);
    await row.getByRole("button", { name: "Install OpenCode", exact: true }).click();
    const failure = await maintenance(page, after, "opencode", false);
    expect(failure.message).toMatch(/permission denied/i);
    await expect(row.getByRole("alert")).toContainText(/permissions.*retry/i);
    await expect(row).toContainText("Not installed");
    expect(await installs()).toHaveLength(1);
    after = await cursor(page);
    await row.getByRole("button", { name: "Install OpenCode", exact: true }).click();
    const success = await maintenance(page, after, "opencode", true);
    expect(success.before_version).toBeNull();
    expect(success.after_version).toContain("1.0.0");
    await expect(row).toContainText("Installed");
    await expect(row).toContainText("1.0.0");
    await expect(row).not.toContainText("Not installed");
    expect(await installs()).toHaveLength(2);
    await screenshot(settings, info, "supported-agents-installed");
    // AC-8: maintenance invalidates detection for the existing launch path;
    // no app restart and no fallback to offering still-missing built-ins.
    const launch = await openLiveLaunchWizardForBranch(page, "fixture-main");
    try {
      const wizard = page.locator("#wizard-modal");
      const configure = wizard.getByRole("button", { name: /^Configure and start/ });
      const agents = wizard.getByLabel("Agent", { exact: true });
      if (!await agents.isVisible()) {
        await expect(configure).toBeVisible({ timeout: 60_000 });
        await configure.click();
      }
      await expect(agents).toBeVisible();
      await expect(agents.locator('option[value="openclaw"], .launch-segmented__option[data-value="openclaw"]')).toHaveCount(0);
      if (await agents.evaluate(node => node.tagName.toLowerCase()) === "select") {
        await agents.selectOption("opencode");
        await expect(agents).toHaveValue("opencode");
      } else {
        const option = agents.locator('.launch-segmented__option[data-value="opencode"]');
        await option.click();
        await expect(option).toHaveAttribute("aria-checked", "true");
      }
      await expect(wizard.getByLabel("Version", { exact: true })).toHaveCount(0);
      const summary = page.locator("#wizard-summary .wizard-summary-item");
      await expect(summary.filter({ has: page.locator(".wizard-summary-label", { hasText: /^Agent$/ }) })
        .locator(".wizard-summary-value")).toHaveText("OpenCode");
      await expect(summary.filter({ has: page.locator(".wizard-summary-label", { hasText: /^Version$/ }) })
        .locator(".wizard-summary-value")).toContainText("1.0.0");
    } finally {
      try { await clearLiveLaunchWizard(page); } finally { await launch.cleanup(); }
    }
  });

  test("protects a live agent, updates before/after versions and persists automatic updates", async ({ page }, info) => {
    test.skip(!process.env.GWT_L3_BACKENDS, "requires the bounded Issue #5017 provider and installer fixtures");
    const backend = backendFor(info), row = panel.locator('[data-agent-id="grok"]');
    const installerCount = async () => (await lines<string[]>(join(backend.home, "npm-argv.jsonl")))
      .filter(args => args[0] === "install").length;
    const launches = () => lines<{ pid: number; ppid: number }>(join(backend.home, "provider-argv.jsonl"));
    let windowId = "";
    await expect(row).toContainText("1.0.0");
    try {
      await sendLiveGwtEvent(page, { kind: "create_window", preset: "claude",
        bounds: { x: 80, y: 80, width: 700, height: 440 } });
      await expect.poll(async () => (await launches()).length, { timeout: 30_000 }).toBe(1);
      const provider = (await launches())[0];
      const pane = await (await page.waitForFunction(() => {
        const workspace = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.payload.kind === "workspace_state")?.payload.workspace;
        return workspace?.tabs.flatMap((tab: any) => tab.workspace.windows)
          .find((candidate: any) => candidate.preset === "claude" && candidate.session_id);
      }, undefined, { timeout: 30_000 })).jsonValue();
      windowId = pane.id;
      const runtimePath = join(backend.home, ".gwt/sessions/runtime", String(backend.pid), `${pane.session_id}.json`);
      const runtime = async () => JSON.parse(await readFile(runtimePath, "utf8"));
      // Windows Host launches keep the selected interactive shell as the PTY
      // child; the Node fixture is its child. POSIX launches exec the fixture.
      await expect.poll(async () => (await runtime()).child_pid)
        .toBe(process.platform === "win32" ? provider.ppid : provider.pid);
      const before = await runtime(), installsBefore = await installerCount();
      expect(alive(provider.pid), "the test fixture is an actual live PTY child").toBe(true);
      const after = await cursor(page);
      // Safety also applies to clients bypassing the disabled action button.
      await sendLiveGwtEvent(page, { kind: "maintain_supported_agent", agent_id: "grok", action: "update" });
      const refusal = await maintenance(page, after, "grok", false);
      expect(refusal.message).toMatch(/agent.*(?:running|live)|(?:close|stop).*agent/i);
      expect(await installerCount(), "refusal cannot invoke any installer").toBe(installsBefore);
      const unchanged = await runtime();
      expect({ child_pid: unchanged.child_pid, child_started_at: unchanged.child_started_at })
        .toEqual({ child_pid: before.child_pid, child_started_at: before.child_started_at });
      expect(alive(provider.pid)).toBe(true);
      await sendLiveGwtEvent(page, { kind: "terminal_input", id: windowId, data: "maintenance-safety-ack\r" });
      await expect.poll(() => page.evaluate(id => (window as any).__gwtTerminalTestApi?.bufferText(id) ?? "", windowId),
        { timeout: 15_000 }).toContain("GWT_MAINTENANCE_FIXTURE_ACK maintenance-safety-ack");
      const identity = await page.evaluate(id => {
        const workspace = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
          entry.payload.kind === "workspace_state")?.payload.workspace;
        return workspace.tabs.flatMap((tab: any) => tab.workspace.windows).find((candidate: any) => candidate.id === id)?.session_id;
      }, windowId);
      expect(identity).toBe(pane.session_id);
      await info.attach("live-agent-safety", { body: JSON.stringify({ provider, session_id: identity,
        before, after: unchanged, refusal, installer_count: installsBefore }, null, 2), contentType: "application/json" });
    } finally {
      if (windowId) {
        await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
        await expect(page.locator(`.workspace-window[data-id="${windowId}"]`)).toHaveCount(0);
        await expect.poll(async () => (await launches()).every(launch => !alive(launch.pid)), { timeout: 15_000 }).toBe(true);
      }
    }
    const after = await cursor(page);
    await row.getByRole("button", { name: "Update Grok Build", exact: true }).click();
    const result = await maintenance(page, after, "grok", true);
    expect(result.before_version).toContain("1.0.0");
    expect(result.after_version).toContain("2.0.0");
    await expect(row.getByRole("status")).toContainText(/1\.0\.0.*2\.0\.0/);
    const next = await cursor(page);
    await sendLiveGwtEvent(page, { kind: "list_supported_agents" });
    const updated = (await catalog(page, next)).agents.find((agent: any) => agent.id === "grok");
    expect(updated.installed_version).toContain("2.0.0");
    expect(updated.available_version).toBe("2.0.0");
    expect(updated.update_available).toBe(false);
    await expect(row).toContainText("Latest");
    const automatic = panel.getByRole("checkbox", { name: "Automatically update agents", exact: true });
    await expect(automatic).not.toBeChecked();
    await automatic.check();
    await expect.poll(async () => readFile(join(backend.home, ".gwt/config.toml"), "utf8"))
      .toMatch(/auto_update\s*=\s*true/);
    await page.reload();
    await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages?.some((entry: any) =>
      entry.payload.kind === "workspace_state"));
    await expect(settings).toBeVisible();
    await page.evaluate(() => document.dispatchEvent(
      new CustomEvent("settings:open", { detail: { target: "supported-agents" } }),
    ));
    const persistedAfter = await cursor(page);
    await sendLiveGwtEvent(page, { kind: "list_supported_agents" });
    const persisted = await catalog(page, persistedAfter);
    await expect(automatic).toBeChecked();
    expect(persisted.auto_update).toBe(true);
    await screenshot(settings, info, "supported-agents-updated");
  });
});
