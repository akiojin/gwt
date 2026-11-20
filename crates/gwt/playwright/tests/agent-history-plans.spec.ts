import { expect, test, type Page } from "@playwright/test";
import { APP_PROJECT_KEY, APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// Issue #7649 (SPEC #7007 T-031): agent history and planned assignments.
//
// Drives the embedded frontend against a WebSocket stub that answers the
// roster and assignment plan reads.
//   - AC-1: the project's agents screen lists every member's sessions, the
//     Project Manager's included, with environment, model, reasoning, result
//   - AC-2: planned assignments are the assignment plan that was read
//   - AC-3: the agent ledger shows no history and no planned assignment
const errors = new WeakMap<Page, string[]>();

test.describe("Agent history and planned assignments", () => {
  test.use({ deviceScaleFactor: 1, viewport: { width: 1440, height: 900 } });
  test.beforeEach(async ({ page }, info) => {
    const captured: string[] = [];
    errors.set(page, captured);
    page.on("pageerror", error => captured.push(error.message));
    page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme),
      info.project.name.includes("light") ? "light" : "dark");
    await installEmbeddedRoutes(page);
    await installRosterBackend(page);
    await page.goto(APP_URL);
  });
  test.afterEach(async ({ page }, info) => {
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      info.project.name.includes("light") ? "light" : "dark");
    expect(errors.get(page), "zero console/page errors").toEqual([]);
  });

  const rail = (page: Page) => page.locator('.op-rail__surface[data-surface="agents"]');
  const header = (page: Page) => page.locator('.app-head [data-destination="agents"]');
  const surface = (page: Page) => page.locator(".split-pane .agents-surface");

  test("AC-1/AC-2: the rail's Agents lists every member's history and the assignment plan", async ({ page }) => {
    await rail(page).click();
    await expect(surface(page).locator("h2")).toHaveText("Agent history & plans");
    await expect(rail(page)).toHaveAttribute("aria-pressed", "true");
    await expect(header(page)).not.toHaveAttribute("aria-current", "page");

    const history = surface(page).getByRole("table", { name: "Agent history" });
    await expect(history.locator("thead th")).toHaveText(
      ["Agent", "Issue", "Execution environment", "Model", "Reasoning", "Result", "Started (UTC)"]);
    const rows = history.locator("tbody tr");
    await expect(rows).toHaveCount(3);
    // Newest first, across members.
    await expect(rows.nth(0).locator("td")).toHaveText(
      ["Oda", "#7048 Shared state model", "Codex pool", "gpt-6.2", "high", "Waiting for input", "2026-11-20 10:00"]);
    // The Project Manager's row is listed and its model is never a dash.
    await expect(rows.nth(1)).toHaveAttribute("data-role", "pm");
    await expect(rows.nth(1).locator("td")).toHaveText(
      ["HikariPM", "Project Manager", "Claude pool", "claude-opus", "Tool default", "Running", "2026-11-20 09:00"]);
    await expect(rows.nth(2).locator("td")).toHaveText(
      ["Oda", "#7047", "codex (not registered as an execution environment)", "Tool default", "Tool default", "Stopped", "2026-11-19 10:00"]);
    await expect(history).toBeVisible();

    const plans = surface(page).getByRole("list", { name: "Planned assignments" }).locator("li");
    await expect(plans.locator("strong")).toHaveText(["Oda → #5561", "Oda → #5562", "Himeno → #7016"]);
    await expect(plans.locator(".roster-plan-reason")).toHaveText(["Queue order", "Assigned by you", "Same SPEC continuation"]);

    // A changed plan replaces the list.
    await page.evaluate(() => window.__emitSchedule([{ agent_member_id: "m2", issues: [{ issue_number: 5561, reason: "assigned_by_you" }], empty_reason: null }]));
    await expect(plans.locator("strong")).toHaveText(["Himeno → #5561"]);
  });

  test("AC-3: the header's Agents opens the ledger without history or planned assignments", async ({ page }) => {
    await header(page).click();
    await expect(surface(page).locator("h2")).toHaveText("Agents");
    await expect(header(page)).toHaveAttribute("aria-current", "page");
    await expect(rail(page)).toHaveAttribute("aria-pressed", "false");
    await expect(surface(page).getByRole("list", { name: "Team members" }).locator(".roster-row")).toHaveCount(3);
    for (const hidden of [".agent-history-plans", ".roster-tabs", ".roster-past", ".roster-plans",
      ".roster-detail > .roster-history:not(.roster-saved-settings)"]) {
      await expect(surface(page).locator(hidden).first(), hidden).toBeHidden();
    }
    await expect(surface(page).getByText("Assignment history")).toBeHidden();
    await expect(surface(page).getByText("Next planned issues")).toBeHidden();
    await expect(surface(page).locator(".roster-saved-settings")).toBeVisible();

    // The project's screen is one click away and shows them again.
    await rail(page).click();
    await expect(surface(page).locator("h2")).toHaveText("Agent history & plans");
    await expect(surface(page).getByRole("table", { name: "Agent history" })).toBeVisible();
    await expect(surface(page).getByText("Next planned issues")).toBeVisible();
    await expect(surface(page).locator(".roster-tabs")).toBeVisible();
  });
});

async function installRosterBackend(page: Page) {
  await page.addInitScript(projectKey => {
    const session = (session_id, role, provider, started_at, extra = {}) => ({
      session_id, role, provider, name: null, model: null, reasoning: null, environment: null, environment_model: null,
      issue_number: null, title: null, result: "Stopped", started_at, ended_at: null, ...extra,
    });
    const member = (agent_member_id, name, extra = {}) => ({
      agent_member_id, agent_id: "codex", name, enabled: true, pm_eligible: false, created_at: "2026-11-01T00:00:00Z",
      assignment: null, provider: "codex", model: null, active_assignments: [], saved_manual_profile: null,
      saved_manual_revision: null, history: [], issue_count: 0, landed_count: null, active_seconds: null, tokens: null, ...extra,
    });
    const roster = {
      project_key: projectKey, revision: 3, revision_decimal: "3", capacity: 2, total_enabled: 3, recommended_capacity: 6,
      past_contributors: [], past_complete: true, by_session: {}, error: null,
      members: [
        member("pm", "Hikari", { agent_id: "claude", provider: "claude", pm_eligible: true, pm_project_key: projectKey, history: [
          session("pm-1", "pm", "claude", "2026-11-20T09:00:00Z", { environment: "Claude pool", environment_model: "claude-opus", result: "Running" }),
        ] }),
        member("m1", "Oda", { history: [
          session("s1", "implementation", "codex", "2026-11-20T10:00:00Z", { model: "gpt-6.2", reasoning: "high", environment: "Codex pool",
            issue_number: 7048, title: "Shared state model", result: "WaitingInput" }),
          session("s0", "implementation", "codex", "2026-11-19T10:00:00Z", { issue_number: 7047, ended_at: "2026-11-19T12:00:00Z" }),
        ] }),
        member("m2", "Himeno"),
      ],
    };
    let scheduleRevision = 7;
    const schedule = members => ({ kind: "agent_schedule", schedule: {
      project_key: projectKey, roster_revision: 3, queue_revision: "queue-v1:one", schedule_revision: scheduleRevision++, members,
    } });
    let plan = [
      { agent_member_id: "m1", issues: [{ issue_number: 5561, reason: "queue_order" }, { issue_number: 5562, reason: "assigned_by_you" }], empty_reason: null },
      { agent_member_id: "m2", issues: [{ issue_number: 7016, reason: "same_spec_continuation" }], empty_reason: null },
    ];
    let socketRef = null;

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        socketRef = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit({ kind: "workspace_state", workspace: {
            app_version: "playwright",
            tabs: [{ id: "tab-1", title: "History Fixture", project_root: "/fixture", project_key: projectKey, kind: "git",
              workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [] } }],
            active_tab_id: "tab-1",
            recent_projects: [],
          } });
        }, 0);
      }

      send(data) {
        let message;
        try {
          message = JSON.parse(data);
        } catch {
          return;
        }
        if (message.kind === "agent_roster") this.emit({ kind: "agent_roster", request_id: message.request_id, roster });
        if (message.kind === "agent_schedule") this.emit(schedule(plan));
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload) {
        setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) })), 0);
      }
    }
    window.WebSocket = FixtureWebSocket;
    window.__emitSchedule = members => { plan = members; socketRef.emit(schedule(plan)); };
  }, APP_PROJECT_KEY);
}
