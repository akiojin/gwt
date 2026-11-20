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

  async function open(page: any, withSettings = false) {
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page, withSettings);
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
    await expect(codex).toContainText("Provider OpenAI · Via Codex · 1 account");
    await expect(codex).toContainText("96% used · resets");
    await expect(codex.locator(".exec-env__usage")).toHaveAttribute("data-severity", "danger");
    await expect(codex.locator("time")).toHaveAttribute("datetime", "2030-01-02T03:04:05Z");
    const claude = surface(page).locator('[data-environment="claude_code"]');
    await expect(claude).toContainText("80% used");
    await expect(claude.locator(".exec-env__usage")).toHaveAttribute("data-severity", "warning");

    // A limit reported by the backend reaches the environment row as it is.
    await emitProviderUsage(page, [{ provider: "codex", plan: "pro", limit_reached: true,
      windows: [{ kind: "weekly", used_percent: 100, resets_at: "2030-01-06T03:04:05Z" }], state: { kind: "ok" } }]);
    await expect(cards).toHaveCount(1);
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
    await expect(surface(page).locator('[data-account="codex"]')).toContainText("pro · Up to date");
    await page.screenshot({ path: info.outputPath("accounts.png"), fullPage: true });

    await surface(page).getByRole("button", { name: "Details for the Codex account" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Codex account" })).toBeFocused();
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
    await expect(surface(page).locator('[data-account="codex"] .exec-env__chip')).toHaveText("Limit reached");
    await surface(page).getByRole("button", { name: "← Execution environments" }).click();
    await expect(surface(page).getByRole("heading", { level: 1, name: "Execution environments" })).toBeFocused();
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("Claude usage collection lives in its account detail and follows snapshots", async ({ page }) => {
    const errors = captureUsageErrors(page);
    await open(page);
    await page.locator('.app-head [data-destination="environments"]').click();
    await surface(page).getByRole("button", { name: "Open the Claude Code account" }).click();
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

async function installProviderUsageBackend(page: any, withSettings = false, withAgents = false): Promise<void> {
  await page.addInitScript(({ withSettings, withAgents }) => {
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
    (window as any).__emitWorkspaceStateNotice = (notice: unknown) => {
      socketRef?.emit({ kind: "workspace_state_notice", notice });
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  }, { withSettings, withAgents });
}
