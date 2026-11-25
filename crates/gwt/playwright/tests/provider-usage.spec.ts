/* Issue #7648 (SPEC #7007 T-030) — deterministic browser contract for the
 * Execution environments screen: the top bar opens it, every environment's
 * state is visible, and usage, reset times and limit state are shown once
 * per account there and nowhere else (the SPEC #2970 status-strip summary
 * and its hover popover are gone; Settings keeps only a link).
 * Playwright runs this spec in both Chromium dark and light projects.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const liveUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;

async function installUsageTheme(page: any, theme: string) {
  await page.addInitScript(({ theme, appOrigin }) => {
    // Init scripts can also run in the opaque document before app navigation.
    if (location.origin === appOrigin) localStorage.setItem("gwt:ui:theme", theme);
  }, { theme, appOrigin: new URL(liveUrl || APP_URL).origin });
}

test.describe("Execution environments", () => {
  test.use({ viewport: { width: 1440, height: 900 } });
  test.beforeEach(async ({ page }, info) => {
    await installUsageTheme(page,
      info.project.name.includes("light") ? "light" : "dark");
  });

  async function open(page: any, withSettings = false, withPools = false) {
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page, withSettings, false, withPools);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);
  }
  const surface = (page: any) => page.locator(".exec-env");

  test("the top bar opens the screen and every environment's state is visible (AC-1)", async ({ page }, info) => {
    const errors = captureUsageErrors(page);
    await open(page);
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
    const entry = page.locator(".project-bar").getByRole("button", { name: "Environments" });
    await expect(entry).not.toHaveAttribute("aria-current", "page");
    await entry.click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Execution environments" })).toBeVisible();
    await expect(entry).toHaveAttribute("aria-current", "page");

    const cards = surface(page).locator("[data-environment]");
    await expect(cards).toHaveCount(2);
    const codex = surface(page).locator('[data-environment="codex"]');
    await expect(codex).toContainText("Available");
    await expect(codex).toContainText("Using Codex 1 · 1 account in pool · Via Codex");
    await expect(codex).toContainText("96% used · resets");
    await expect(codex.locator(".exec-env__usage")).toHaveAttribute("data-severity", "danger");
    await expect(codex.locator("time")).toHaveAttribute("datetime", "2030-01-02T03:04:05Z");
    const claude = surface(page).locator('[data-environment="claude"]');
    await expect(claude).toContainText("80% used");
    await expect(claude.locator(".exec-env__usage")).toHaveAttribute("data-severity", "warning");

    // A limit reported by the backend reaches the environment row as it is.
    await emitProviderUsage(page, [{ provider: "codex", plan: "pro", limit_reached: true,
      windows: [{ kind: "weekly", used_percent: 100, resets_at: "2030-01-06T03:04:05Z" }], state: { kind: "ok" } }]);
    // An environment is a definition: it stays when its account stops reporting usage.
    await expect(cards).toHaveCount(2);
    await expect(claude).toHaveAttribute("data-availability", "unknown");
    await expect(codex).toHaveAttribute("data-availability", "limit");
    await expect(codex.locator(".exec-env__chip")).toHaveText("Limit reached");
    await expect(codex).toContainText("100% used · resets");

    const overflow = await page.evaluate(() =>
      document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);
    expect(overflow, "no horizontal page overflow").toBe(false);
    await page.screenshot({ path: info.outputPath("execution-environments.png"), fullPage: true });
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("every account opens its detail with usage, reset and limit state (AC-2)", async ({ page }, info) => {
    const errors = captureUsageErrors(page);
    await open(page);
    await page.locator('.app-head [data-destination="environments"]').click();
    await surface(page).getByRole("button", { name: "Accounts", exact: true }).click();
    const heading = surface(page).getByRole("heading", { level: 1, name: "Accounts" });
    await expect(heading).toBeFocused();
    const rows = surface(page).locator("[data-account]");
    await expect(rows).toHaveCount(2);
    for (const row of await rows.all()) await expect(row.getByRole("button", { name: /^Details for the / })).toBeVisible();
    await expect(surface(page).locator('[data-account="codex"]')).toContainText("Codex · pro · Signed in · Up to date");
    await page.screenshot({ path: info.outputPath("accounts.png"), fullPage: true });

    await surface(page).getByRole("button", { name: "Details for the Codex 1 account" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Codex 1 account" })).toBeFocused();
    const card = surface(page).locator('.op-usage-card[data-provider="codex"]');
    await expect(card).toHaveCount(1);
    const windows = card.locator(".op-usage-win");
    await expect(windows.nth(0)).toContainText(/5-hour\s*96%/);
    await expect(windows.nth(0).locator(".op-usage-win__reset")).not.toBeEmpty();
    await expect(windows.nth(0).locator(".op-usage-bar__fill")).toHaveAttribute("data-severity", "danger");
    await expect(windows.nth(1)).toContainText(/Weekly\s*29%/);
    await expect(surface(page).getByRole("checkbox")).toHaveCount(0);

    // The open detail follows the next snapshot: limit state and sessions.
    await emitProviderUsage(page, [{ provider: "codex", plan: "pro", limit_reached: true,
      windows: [{ kind: "weekly", used_percent: 100, resets_at: "2030-01-06T03:04:05Z" }], state: { kind: "ok" } }],
    [{ session_id: "sess-codex-context", provider: "codex", model: "gpt-6-astra", input_tokens: 100000,
      output_tokens: 1000, total_tokens: 101000, context_used_tokens: 100000, context_limit_tokens: 400000,
      context_left_pct: 75, eligible: true, state: { kind: "ok" } }]);
    await expect(card.locator(".op-usage-card__limit")).toHaveText("Limit reached");
    await expect(card.locator(".op-usage-win")).toContainText(/Weekly\s*100%/);
    const session = card.locator(".op-usage-sess__row");
    await expect(session).toHaveCount(1);
    await expect(session.locator(".op-usage-sess__model")).toHaveText("gpt-6-astra");
    await expect(session.locator(".op-usage-sess__ctx")).toHaveText("75% left");
    await page.screenshot({ path: info.outputPath("account-detail.png"), fullPage: true });

    await surface(page).getByRole("button", { name: "← Accounts" }).click();
    await expect(surface(page).locator('[data-account="codex"] .exec-env__chip--limit')).toHaveText("Limit reached");
    await surface(page).getByRole("button", { name: "← Execution environments" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Execution environments" })).toBeFocused();
    expect(errors, "zero console/page errors").toEqual([]);
  });

  // Issue #7841 AC-1/AC-2/AC-3: one row per account, grouped by pool, each
  // with its own usage, sign-in and limit state; a tool without an official
  // usage source says so and keeps the last limit gwt saw.
  test("accounts are listed per pool with their own usage, sign-in and limits (Issue #7841)", async ({ page }, info) => {
    const errors = captureUsageErrors(page);
    await open(page, false, true);
    await page.locator('.app-head [data-destination="environments"]').click();
    const claudeEnvironment = surface(page).locator('[data-environment="claude"]');
    await expect(claudeEnvironment).toContainText("Using Claude Code 1 · 2 accounts in pool · Via Claude Code");
    await emitProviderUsage(page, [
      { provider: "codex", pool_account_id: "codex", plan: "pro",
        windows: [{ kind: "weekly", used_percent: 29, resets_at: "2030-01-06T03:04:05Z" }], state: { kind: "ok" } },
      { provider: "claude_code", pool_account_id: "claude", plan: "max",
        windows: [{ kind: "weekly", used_percent: 80, resets_at: "2030-01-07T03:04:05Z" }], state: { kind: "ok" } },
      { provider: "claude_code", pool_account_id: "claude-2", account_label: "second@example.test", plan: "pro", limit_reached: true,
        windows: [{ kind: "five_hour", used_percent: 100, resets_at: "2030-01-02T05:00:00Z" }], state: { kind: "ok" } },
      { provider: "grok_build", pool_account_id: "grok", windows: [], limit_reached: false,
        state: { kind: "unknown", reason: "no official usage source" },
        last_limit: { detected_at: "2026-11-24T02:00:00Z", resets_at: "2099-11-24T06:00:00Z" } },
    ]);
    await surface(page).getByRole("button", { name: "Accounts", exact: true }).click();
    const pools = surface(page).locator("[data-pool]");
    await expect(pools).toHaveCount(3);
    const claudePool = surface(page).locator('[data-pool="claude"]');
    await expect(claudePool).toContainText("2 accounts in pool, in priority order");
    const second = claudePool.locator('[data-account="claude-2"]');
    await expect(second).toContainText("Claude Code · second@example.test · pro · Not signed in");
    await expect(second).toContainText("100% used · resets");
    await expect(second.locator(".exec-env__chip--limit")).toHaveText("Limit reached");
    await expect(claudePool.locator('[data-account="claude"]')).toContainText("In use");
    const grok = surface(page).locator('[data-account="grok"]');
    await expect(grok).toContainText("Usage unknown (no official usage source)");
    await expect(grok.locator("[data-last-limit] time").first()).toHaveAttribute("datetime", "2026-11-24T02:00:00Z");
    await expect(grok.locator(".exec-env__chip--limit")).toHaveText("Limit reached");
    const overflow = await page.evaluate(() =>
      document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);
    expect(overflow, "no horizontal page overflow").toBe(false);
    await page.screenshot({ path: info.outputPath("account-pools.png"), fullPage: true });

    await surface(page).getByRole("button", { name: "Details for the Claude Code 2 account" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Claude Code 2 account" })).toBeFocused();
    await expect(surface(page).locator('.op-usage-card[data-provider="claude_code"] .op-usage-win')).toContainText(/5-hour\s*100%/);
    await surface(page).getByRole("button", { name: "← Accounts" }).click();
    await surface(page).getByRole("button", { name: "Details for the Grok Build 1 account" }).click();
    await expect(surface(page).locator(".op-usage-card")).toHaveCount(0);
    await expect(surface(page)).toContainText("Usage unknown (no official usage source)");
    await expect(surface(page)).toContainText("Last limit hit");
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("Claude usage collection lives in its account detail and follows snapshots", async ({ page }) => {
    const errors = captureUsageErrors(page);
    await open(page);
    await page.locator('.app-head [data-destination="environments"]').click();
    await surface(page).getByRole("button", { name: "Open the Claude Code 1 account" }).click();
    const checkbox = surface(page).getByRole("checkbox", { name: "Show Claude Code account usage" });
    await expect(checkbox).toBeChecked();
    await checkbox.focus();
    await emitProviderUsage(page, [{ provider: "claude_code", windows: [], state: { kind: "disabled" } }]);
    await expect(checkbox).not.toBeChecked();
    await expect(checkbox).toBeFocused();
    await expect(surface(page).locator(".op-usage-card__reason")).toHaveText("Usage collection is off");
    await emitProviderUsage(page, [{ provider: "claude_code", windows: [], state: { kind: "ok" } }]);
    await expect(checkbox).toBeChecked();
    await expect(checkbox).toBeFocused();
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("usage is shown nowhere else: no status-strip summary, Settings only links to Accounts (AC-3)", async ({ page }, info) => {
    const errors = captureUsageErrors(page);
    await open(page, true);
    await expect(page.locator(".op-status-strip")).toBeVisible();
    await expect(page.locator("#op-strip-usage")).toHaveCount(0);
    await expect(page.locator(".op-status-strip")).not.toContainText(/USAGE|% used|\d+%\s*$/);
    await expect(page.locator("#provider-usage-popover")).toHaveCount(0);

    await page.getByRole("tab", { name: "Usage & Limits" }).click();
    const panel = page.locator('[data-settings-panel="usage"]');
    await expect(panel.getByRole("checkbox")).toHaveCount(0);
    await expect(panel.locator(".op-usage-card")).toHaveCount(0);
    await page.screenshot({ path: info.outputPath("settings-usage-link.png"), fullPage: true });
    await panel.getByRole("button", { name: "Open Accounts" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Accounts" })).toBeVisible();
    await expect(surface(page).locator("[data-account]")).toHaveCount(2);

    // Tool management stays in Settings; the screen only links to it.
    await surface(page).getByRole("button", { name: "Tool management" }).click();
    await expect(page.getByRole("tab", { name: "Supported Agents" })).toHaveAttribute("aria-selected", "true");
    expect(errors, "zero console/page errors").toEqual([]);
  });
});

/* Issue #7741 (SPEC #7007 items 77-82) — the environments are definitions
 * that can be registered through a tool, edited and deleted, and a project
 * chooses the ones it uses in priority order. Runs in Chromium dark and light.
 */
test.describe("Execution environment definitions", () => {
  test.use({ viewport: { width: 1440, height: 900 } });
  test.beforeEach(async ({ page }, info) => {
    await installUsageTheme(page, info.project.name.includes("light") ? "light" : "dark");
  });
  const surface = (page: any) => page.locator(".exec-env");
  const sent = (page: any, kind: string) =>
    page.evaluate((kind: string) => (window as any).__sentMessages.filter((message: any) => message.kind === kind), kind);
  async function open(page: any, withSettings = false, withPools = false) {
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page, withSettings, false, withPools);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);
  }

  test("an environment is registered through a tool, shown read-only, and deleted with its projects named (AC-1, AC-3, AC-4)", async ({ page }, info) => {
    test.skip(Boolean(liveUrl), "uses the deterministic fixture backend");
    const errors = captureUsageErrors(page);
    await open(page);
    await page.locator('.app-head [data-destination="environments"]').click();
    const cards = surface(page).locator("[data-environment]");
    await expect(cards).toHaveCount(2);

    // An account's own environment came with the account: Delete is there, disabled, with the reason.
    const own = surface(page).locator('[data-environment="claude"]');
    await expect(own).toHaveAttribute("data-kind", "set");
    const ownDelete = own.getByRole("button", { name: "Delete Claude Code 1" });
    await expect(ownDelete).toBeDisabled();
    await expect(own).toContainText("It is deleted when the account is deleted.");
    await expect(own).toContainText("Used by Usage Fixture");

    await surface(page).getByRole("button", { name: "Add an execution environment" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Add an execution environment" })).toBeFocused();
    const tool = surface(page).getByLabel("Tool to go through");
    await expect(tool).toHaveValue("hermes");
    await expect(tool.locator('option[value="opencode"]')).toHaveAttribute("disabled", "");
    await expect(surface(page).locator('[data-blocked-tool="opencode"]')).toContainText("OpenCode is not installed");
    await expect(surface(page).getByLabel("Name")).toHaveValue("Claude Code 1 (via Hermes Agent)");
    await page.screenshot({ path: info.outputPath("environment-add.png"), fullPage: true });
    await surface(page).getByRole("button", { name: "Register" }).click();

    await expect(surface(page).getByRole("heading", { level: 1, name: "Execution environments" })).toBeFocused();
    await expect(cards).toHaveCount(3);
    const through = surface(page).locator('[data-environment="claude:hermes"]');
    await expect(through).toHaveAttribute("data-kind", "via");
    await expect(through).toContainText("Account Claude Code 1 · Via Hermes Agent");
    await expect(through).toContainText("Not on any project's list");
    expect(await sent(page, "add_execution_environment")).toEqual([
      { kind: "add_execution_environment", account_id: "claude", via_tool: "hermes", name: "Claude Code 1 (via Hermes Agent)" },
    ]);

    // Its runtime options are the tool's own: shown, never editable.
    await through.getByRole("button", { name: "Details for Claude Code 1 (via Hermes Agent)" }).click();
    await expect(surface(page).getByRole("checkbox")).toHaveCount(0);
    const settings = surface(page).getByRole("region", { name: "Current settings" });
    await expect(settings).toContainText("zai");
    await expect(settings).toContainText("glm-4.6");
    await expect(settings).toContainText("reads these from its own settings file");
    await page.screenshot({ path: info.outputPath("environment-read-only.png"), fullPage: true });

    // Once a project lists it, the confirmation names that project.
    await surface(page).getByRole("button", { name: "← Execution environments" }).click();
    await page.evaluate(() => (window as any).__setProjectEnvironments(["codex", "claude:hermes"]));
    await expect(through).toContainText("Used by Usage Fixture");
    await through.getByRole("button", { name: "Delete Claude Code 1 (via Hermes Agent)" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Delete this execution environment?" })).toBeFocused();
    await expect(surface(page).locator('[data-project="Usage Fixture"]')).toContainText("priority 2");
    await expect(surface(page)).toContainText("The Claude Code 1 account and its other environments stay.");
    await page.screenshot({ path: info.outputPath("environment-delete.png"), fullPage: true });
    await surface(page).getByRole("button", { name: "Delete", exact: true }).click();
    await expect(cards).toHaveCount(2);
    await expect(through).toHaveCount(0);
    const overflow = await page.evaluate(() =>
      document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);
    expect(overflow, "no horizontal page overflow").toBe(false);
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("runtime options of an account's own environment are saved as they change (AC-3)", async ({ page }, info) => {
    test.skip(Boolean(liveUrl), "uses the deterministic fixture backend");
    const errors = captureUsageErrors(page);
    await open(page);
    await page.locator('.app-head [data-destination="environments"]').click();
    await surface(page).getByRole("button", { name: "Edit Claude Code 1" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Edit Claude Code 1" })).toBeFocused();
    await expect(surface(page)).toContainText("take effect from the next session");

    const fast = surface(page).getByRole("checkbox", { name: "Fast mode" });
    await fast.click();
    await expect(surface(page).getByRole("status")).toHaveText("Saved");
    await expect(fast).toBeChecked();
    const model = surface(page).getByLabel("Model");
    await model.fill("sonnet");
    await model.press("Enter");
    await expect(surface(page).getByRole("status")).toHaveText("Saved");
    expect(await sent(page, "update_execution_environment")).toEqual([
      { kind: "update_execution_environment", environment_id: "claude", runtime: { model: null, skip_permissions: false, fast_mode: true } },
      { kind: "update_execution_environment", environment_id: "claude", runtime: { model: "sonnet", skip_permissions: false, fast_mode: true } },
    ]);
    await expect(surface(page).getByRole("button", { name: "Delete", exact: true })).toBeDisabled();
    await page.screenshot({ path: info.outputPath("environment-edit.png"), fullPage: true });

    // The saved values are what the screen shows when it is opened again.
    await surface(page).getByRole("button", { name: "← Execution environments" }).click();
    await surface(page).getByRole("button", { name: "Edit Claude Code 1" }).click();
    await expect(surface(page).getByLabel("Model")).toHaveValue("sonnet");
    await expect(surface(page).getByRole("checkbox", { name: "Fast mode" })).toBeChecked();
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("a project reorders, removes and adds its available execution environments (AC-2)", async ({ page }, info) => {
    test.skip(Boolean(liveUrl), "uses the deterministic fixture backend");
    const errors = captureUsageErrors(page);
    await open(page, true);
    await page.locator('[data-settings-tab="issues"]').click();
    const list = page.locator("[data-settings-environments]");
    const names = list.locator(".project-environments__name");
    await expect(names).toHaveText(["1. Codex 1", "2. Claude Code 1"]);
    await expect(list).toContainText("This is the order the project launches with today.");
    await expect(list.getByRole("button", { name: "Move up Codex 1" })).toBeDisabled();
    await page.screenshot({ path: info.outputPath("project-environments.png"), fullPage: true });

    await list.getByRole("button", { name: "Move down Codex 1" }).click();
    await expect(names).toHaveText(["1. Claude Code 1", "2. Codex 1"]);
    await expect(list).not.toContainText("launches with today");

    await list.getByRole("button", { name: "Remove Codex 1" }).click();
    await expect(names).toHaveText(["1. Claude Code 1"]);
    // The last one cannot be removed, and the row says why.
    await expect(list.getByRole("button", { name: "Remove Claude Code 1" })).toBeDisabled();
    await expect(list).toContainText("Keep at least one.");

    await expect(list.getByLabel("Add an execution environment")).toHaveValue("codex");
    await list.getByRole("button", { name: "Add to the end" }).click();
    await expect(names).toHaveText(["1. Claude Code 1", "2. Codex 1"]);
    await expect(list).toContainText("Every execution environment is already on the list.");
    expect(await sent(page, "set_project_execution_environments")).toEqual([
      { kind: "set_project_execution_environments", environment_ids: ["claude", "codex"] },
      { kind: "set_project_execution_environments", environment_ids: ["claude"] },
      { kind: "set_project_execution_environments", environment_ids: ["claude", "codex"] },
    ]);

    await list.getByRole("button", { name: "Manage execution environments" }).click();
    await expect(page.locator(".exec-env").getByRole("heading", { level: 1, name: "Execution environments" })).toBeVisible();
    expect(errors, "zero console/page errors").toEqual([]);
  });

  // Issue #7648 AC-6: the open project's priority list is edited on the
  // Execution environments screen itself.
  test("the screen edits the open project's priority list (Issue #7648 AC-6)", async ({ page }, info) => {
    test.skip(Boolean(liveUrl), "uses the deterministic fixture backend");
    const errors = captureUsageErrors(page);
    await open(page);
    await page.locator('.app-head [data-destination="environments"]').click();
    const list = surface(page).locator('[data-environment-list="project"]');
    const names = list.locator(".project-environments__name");
    await expect(names).toHaveText(["1. Codex 1", "2. Claude Code 1"]);
    await expect(list.getByRole("button", { name: "Manage execution environments" })).toHaveCount(0);
    await list.getByRole("button", { name: "Move down Codex 1" }).click();
    await expect(names).toHaveText(["1. Claude Code 1", "2. Codex 1"]);
    // The environment rows follow the saved order.
    await expect(surface(page).locator('[data-environment="codex"]')).toContainText("Used by Usage Fixture");
    await list.getByRole("button", { name: "Remove Codex 1" }).click();
    await expect(names).toHaveText(["1. Claude Code 1"]);
    await expect(surface(page).locator('[data-environment="codex"]')).toContainText("Not on any project's list");
    expect(await sent(page, "set_project_execution_environments")).toEqual([
      { kind: "set_project_execution_environments", environment_ids: ["claude", "codex"] },
      { kind: "set_project_execution_environments", environment_ids: ["claude"] },
    ]);
    await page.screenshot({ path: info.outputPath("environments-priority.png"), fullPage: true });
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("the application settings keep the Project Manager's own list, and its last entry stays (Issue #7732 AC-2)", async ({ page }, info) => {
    test.skip(Boolean(liveUrl), "uses the deterministic fixture backend");
    const errors = captureUsageErrors(page);
    await open(page, true);
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
    await page.locator('[data-settings-tab="system"]').click();
    const list = page.locator("[data-settings-pm-environments]");
    await expect(list.getByRole("group", { name: "Project Manager execution environments" })).toBeVisible();
    const names = list.locator(".project-environments__name");
    await expect(names).toHaveText(["1. Claude Code 1", "2. Codex 1"]);
    await expect(list).toContainText("made from the accounts registered on this machine");
    await expect(list).toContainText("separate from each project's list");
    await list.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath("pm-environments.png"), fullPage: true });

    await list.getByRole("button", { name: "Move down Claude Code 1" }).click();
    await expect(names).toHaveText(["1. Codex 1", "2. Claude Code 1"]);
    await expect(list).not.toContainText("made from the accounts");

    await list.getByRole("button", { name: "Remove Claude Code 1" }).click();
    await expect(names).toHaveText(["1. Codex 1"]);
    // The last one cannot be removed, and the row says why.
    const removeLast = list.getByRole("button", { name: "Remove Codex 1" });
    await expect(removeLast).toBeDisabled();
    await expect(list).toContainText("Keep at least one. The Project Manager cannot start a session");
    await page.screenshot({ path: info.outputPath("pm-environments-last.png"), fullPage: true });

    await list.getByRole("button", { name: "Add to the end" }).click();
    await expect(names).toHaveText(["1. Codex 1", "2. Claude Code 1"]);
    expect(await sent(page, "set_pm_execution_environments")).toEqual([
      { kind: "set_pm_execution_environments", environment_ids: ["codex", "claude"] },
      { kind: "set_pm_execution_environments", environment_ids: ["codex"] },
      { kind: "set_pm_execution_environments", environment_ids: ["codex", "claude"] },
    ]);
    // The project's own list is a different list and did not move.
    await page.locator('[data-settings-tab="issues"]').click();
    await expect(page.locator("[data-settings-environments] .project-environments__name"))
      .toHaveText(["1. Codex 1", "2. Claude Code 1"]);
    expect(await sent(page, "set_project_execution_environments")).toEqual([]);
    expect(errors, "zero console/page errors").toEqual([]);
  });
});

function captureUsageErrors(page: any): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error: Error) => errors.push(error.message));
  page.on("console", (message: any) => { if (message.type() === "error") errors.push(message.text()); });
  return errors;
}

async function emitProviderUsage(page: any, accounts: unknown[], sessions: unknown[] = []): Promise<void> {
  await page.evaluate(({ accounts, sessions }) => {
    (window as any).__emitProviderUsage(accounts, sessions);
  }, { accounts, sessions });
}

async function installProviderUsageBackend(page: any, withSettings = false, withAgents = false, withPools = false): Promise<void> {
  await page.addInitScript(({ withSettings, withAgents, withPools }) => {
    try {
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    } catch {
      /* no-op */
    }

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Usage Fixture",
            project_root: "/fixture",
            project_key: location.pathname.split("/")[2],
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: withSettings ? [{ id: "tab-1::settings", title: "Settings", preset: "settings",
                geometry: { x: 40, y: 40, width: 1100, height: 750 }, z_index: 1,
                status: "running", persist: true, minimized: false, maximized: false }] :
                withAgents ? ["fable", "opus", "codex"].map((model, index) => ({
                  id: `usage-${model}`, title: model, preset: "agent", agent_name: model,
                  agent_id: model === "codex" ? "codex" : "claude", session_id: `usage-${model}-session`,
                  geometry: { x: 100, y: 100, width: 720, height: 400 }, geometry_revision: 0,
                  z_index: index + 1, status: "idle", persist: true, minimized: false, maximized: false,
                })) : [],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };
    const usageSnapshot = {
      kind: "provider_usage",
      accounts: [
        {
          provider: "codex",
          plan: "pro",
          windows: [
            {
              kind: "five_hour",
              used_percent: 96,
              resets_at: "2030-01-02T03:04:05Z",
            },
            {
              kind: "weekly",
              used_percent: 29,
              resets_at: "2030-01-06T03:04:05Z",
            },
          ],
          state: { kind: "ok" },
        },
        {
          provider: "claude_code",
          plan: "max",
          windows: [
            {
              kind: "weekly",
              used_percent: 80,
              resets_at: "2030-01-07T03:04:05Z",
            },
          ],
          state: { kind: "ok" },
        },
      ],
      // Issue #3862 — per-session tokens / context ride the same snapshot and
      // must be readable from the same popover as the account windows.
      sessions: [
        {
          session_id: "sess-claude-fable",
          provider: "claude_code",
          model: "claude-fable-5-1",
          input_tokens: 1000000,
          output_tokens: 234567,
          total_tokens: 1234567,
          context_used_tokens: 420000,
          context_limit_tokens: 1000000,
          context_left_pct: 58,
          limit_reached: false,
          eligible: true,
          state: { kind: "ok" },
        },
        {
          session_id: "sess-claude-sonnet",
          provider: "claude_code",
          model: "claude-sonnet-5",
          input_tokens: 20000,
          output_tokens: 4000,
          total_tokens: 24000,
          context_used_tokens: 176000,
          context_limit_tokens: 200000,
          context_left_pct: 12,
          limit_reached: false,
          eligible: true,
          state: { kind: "ok" },
        },
      ],
      consumption: [],
    };

    // Issue #7741: the execution environment definitions and this project's
    // priority list, kept the way the backend keeps them.
    const TOOL_NAMES: Record<string, string> = {
      claude: "Claude Code", codex: "Codex", hermes: "Hermes Agent", openclaw: "OpenClaw", opencode: "OpenCode",
    };
    const environmentState = {
      accounts: [
        { id: "claude", tool: "claude", tool_name: "Claude Code", name: "Claude Code 1", sign_in: "signed_in" },
        { id: "codex", tool: "codex", tool_name: "Codex", name: "Codex 1", sign_in: "signed_in" },
        // Issue #7841: a second account in the Claude pool and a tool without
        // an official usage source.
        ...(withPools ? [
          { id: "claude-2", tool: "claude", tool_name: "Claude Code", name: "Claude Code 2", sign_in: "signed_out" },
          { id: "grok", tool: "grok", tool_name: "Grok Build", name: "Grok Build 1", sign_in: "signed_in" },
        ] : []),
      ],
      environments: [
        { id: "claude", name: "Claude Code 1", account_id: "claude", via_tool: "claude", runtime: {} as any },
        { id: "codex", name: "Codex 1", account_id: "codex", via_tool: "codex", runtime: {} as any },
      ],
      saved: false,
      list: ["codex", "claude"],
      pmSaved: false,
      pmList: ["claude", "codex"],
    };
    const environmentsView = () => {
      const environments = environmentState.environments.map(environment => {
        const own = environment.account_id === environment.via_tool;
        const priority = environmentState.list.indexOf(environment.id);
        return {
          ...environment,
          kind: own ? "set" : "via",
          account_name: environmentState.accounts.find(account => account.id === environment.account_id)!.name,
          via_tool_name: TOOL_NAMES[environment.via_tool],
          tool_installed: true,
          delete_blocked_reason: own ? "Created with the account. It is deleted when the account is deleted." : undefined,
          runtime_read_only_reason: own ? undefined
            : `${TOOL_NAMES[environment.via_tool]} reads these from its own settings file. Change them there.`,
          tool_settings: own ? undefined : { path: "/home/fixture/.hermes/config.yaml",
            values: [{ label: "Provider", value: "zai" }, { label: "Model", value: "glm-4.6" }] },
          projects: priority < 0 ? [] : [{ project_root: "/fixture", title: "Usage Fixture", priority: priority + 1 }],
        };
      });
      return {
        kind: "execution_environments",
        view: {
          accounts: environmentState.accounts,
          environments,
          via_options: environmentState.accounts.flatMap(account => ["hermes", "openclaw", "opencode"].map(tool => {
            const taken = environmentState.environments.find(environment => environment.via_tool === tool);
            return {
              account_id: account.id, via_tool: tool, via_tool_name: TOOL_NAMES[tool],
              blocked_reason: taken ? `${TOOL_NAMES[tool]} is already registered as "${taken.name}". It uses the provider set in its own settings.`
                : tool === "opencode" ? "OpenCode is not installed. Install it in Tool management." : undefined,
            };
          })),
          pm: {
            saved: environmentState.pmSaved,
            entries: environmentState.pmList.map(id => {
              const environment = environmentState.environments.find(item => item.id === id)!;
              return { environment_id: id, tool: environment.via_tool, tool_name: TOOL_NAMES[environment.via_tool], name: environment.name };
            }),
          },
          project: {
            project_root: "/fixture", saved: environmentState.saved,
            entries: environmentState.list.map(id => {
              const environment = environmentState.environments.find(item => item.id === id)!;
              return { environment_id: id, tool: environment.via_tool, tool_name: TOOL_NAMES[environment.via_tool], name: environment.name };
            }),
          },
        },
      };
    };
    const sentMessages: any[] = [];
    (window as any).__sentMessages = sentMessages;
    // Answer one execution environment request; `null` when it is not one.
    const answerEnvironmentRequest = (message: any): unknown => {
      switch (message?.kind) {
        case "get_execution_environments":
          break;
        case "add_execution_environment":
          environmentState.environments.push({
            id: `${message.account_id}:${message.via_tool}`, name: message.name,
            account_id: message.account_id, via_tool: message.via_tool, runtime: {},
          });
          break;
        case "update_execution_environment": {
          const environment = environmentState.environments.find(item => item.id === message.environment_id);
          if (!environment) return { kind: "execution_environment_error", message: "This execution environment no longer exists." };
          if (message.name) environment.name = message.name;
          if (message.runtime) environment.runtime = message.runtime;
          break;
        }
        case "delete_execution_environment":
          environmentState.environments = environmentState.environments.filter(item => item.id !== message.environment_id);
          environmentState.list = environmentState.list.filter(id => id !== message.environment_id);
          environmentState.pmList = environmentState.pmList.filter(id => id !== message.environment_id);
          break;
        case "set_pm_execution_environments":
          // The saving side refuses an empty list, whatever the screen allowed.
          if (message.environment_ids.length === 0) {
            return { kind: "execution_environment_error", project_root: "/fixture",
              message: "Keep at least one execution environment. The Project Manager cannot start a session without one." };
          }
          environmentState.pmList = message.environment_ids;
          environmentState.pmSaved = true;
          break;
        case "set_project_execution_environments":
          environmentState.list = message.environment_ids;
          environmentState.saved = true;
          break;
        default:
          return null;
      }
      return environmentsView();
    };

    let socketRef: FixtureWebSocket | null = null;

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url: string) {
        super();
        (this as any).url = url;
        (this as any).readyState = FixtureWebSocket.CONNECTING;
        socketRef = this;
        setTimeout(() => {
          (this as any).readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string) {
        let message: any = null;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        sentMessages.push(message);
        const answer = answerEnvironmentRequest(message);
        if (answer) this.emit(answer);
        if (message?.kind === "frontend_ready") {
          this.emit(workspaceState);
          this.emit(usageSnapshot);
          if (withAgents) this.emit({ kind: "active_work_projection", projection: {
            id: "usage-fixture", active_works: [], local_agent_activity: {
              project_key: location.pathname.split("/")[2], host_id: "usage-fixture",
              observed_at: new Date().toISOString(), agents: ["fable", "opus", "codex"].map(model => ({
                window_id: `usage-${model}`, session_id: `usage-${model}-session`,
                runtime_incarnation: 1, is_pm: false, issue_number: null,
              })),
            },
          } });
        }
      }

      close() {
        (this as any).readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown) {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    (window as any).__emitProviderUsage = (accounts: unknown[], sessions: unknown[] = []) => {
      socketRef?.emit({
        kind: "provider_usage",
        accounts,
        sessions,
        consumption: [],
      });
    };
    (window as any).__setProjectEnvironments = (ids: string[]) => {
      environmentState.list = ids;
      environmentState.saved = true;
      socketRef?.emit(environmentsView());
    };
    (window as any).__emitWorkspaceStateNotice = (notice: unknown) => {
      socketRef?.emit({ kind: "workspace_state_notice", notice });
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  }, { withSettings, withAgents, withPools });
}
