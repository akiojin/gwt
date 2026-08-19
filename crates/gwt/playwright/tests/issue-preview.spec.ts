/* SPEC-3671 — Issue window as the primary surface.
 *
 * An Issue Monitor auto-launch must not add a window to the canvas. It becomes an
 * `issue_preview` placement that the Issue window mirrors read-only in its right
 * pane, and only an explicit Windowize puts it back on the canvas.
 *
 * The fixture serves the embedded frontend through Playwright routes and replaces
 * WebSocket with a deterministic backend, matching `tests/kanban.spec.ts`.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Issue preview placement", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1600, height: 1000 },
  });

  // 受け入れシナリオ 1 / FR-004.
  test("an auto-launched agent does not add a window to the canvas", async ({ page }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window.surface-knowledge")).toBeVisible();
    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(0);

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // 受け入れシナリオ 2 / FR-007 / FR-008.
  test("the Issue window mirrors the agent output read-only", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    const preview = page.locator(".surface-knowledge .issue-preview");
    await expect(preview).toHaveCount(1);
    await expect(preview).toHaveAttribute("data-window-id", "tab-issue::agent-preview");
    await expect(preview.locator(".issue-preview-title")).toHaveText("Issue #3671 agent");

    const terminal = preview.locator(".issue-preview-terminal .terminal-root");
    await expect(terminal).toBeVisible();

    await page.evaluate(() => window.__emitAgentOutput("SPEC-3671 preview line"));
    await expect(terminal).toContainText("SPEC-3671 preview line");

    await terminal.click();
    await page.keyboard.type("rm -rf /");
    const inputMessages = await page.evaluate(() =>
      window.__knowledgeLoadMessages.filter((message) => message.kind === "terminal_input"),
    );
    expect(inputMessages).toEqual([]);
  });

  // 受け入れシナリオ 3 / FR-009.
  test("only the selected Issue's agent is mirrored", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveAttribute(
      "data-window-id",
      "tab-issue::agent-preview",
    );

    await page.locator(".surface-knowledge [data-issue-number='3672']").click();

    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveCount(1);
    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveAttribute(
      "data-window-id",
      "tab-issue::agent-preview-2",
    );
  });

  // 受け入れシナリオ 4 / FR-010.
  test("Windowize puts the mirrored agent on the canvas", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await page
      .locator(".surface-knowledge [data-action='windowize-issue-preview']")
      .click();

    const undocked = await page.evaluate(() =>
      window.__knowledgeLoadMessages.filter(
        (message) => message.kind === "undock_agent_window",
      ),
    );
    expect(undocked).toHaveLength(1);
    expect(undocked[0].id).toBe("tab-issue::agent-preview");

    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(1);
    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveCount(0);
  });

  // 受け入れシナリオ 5 / FR-011.
  test("an errored agent is badged in the Issue row, not opened on the canvas", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page, { agentStatus: "error" });

    await page.goto(APP_URL);

    await expect(
      page.locator(".surface-knowledge .issue-preview .knowledge-monitor-chip"),
    ).toHaveText("Error");
    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(0);
  });

  // 受け入れシナリオ 8 / FR-012 / FR-013 / T-025: the Work information and the
  // Work actions are reachable without ever opening the Work window.
  test("the Issue row carries Work state and Work actions without a Work window", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window.surface-work")).toHaveCount(0);

    const work = page.locator(
      ".surface-knowledge [data-issue-number='3671'] .knowledge-row-work",
    );
    await expect(work).toHaveCount(1);
    await expect(work.locator(".knowledge-work-lifecycle")).toHaveText("Active");
    await expect(work.locator(".knowledge-work-attention")).toHaveText("Waiting on review");
    await expect(work.locator(".knowledge-work-pr")).toHaveText("PR #3699 · open");
    await expect(work.locator('[data-action="continue-work"]')).toBeEnabled();
    await expect(work.locator('[data-action="resume-work"]')).toBeEnabled();
    // The backend owns cleanup eligibility; a live agent keeps the action off.
    await expect(work.locator('[data-action="cleanup-work"]')).toBeDisabled();

    // An Issue with no correlated Work row shows no Work band.
    await expect(
      page.locator(".surface-knowledge [data-issue-number='3672'] .knowledge-row-work"),
    ).toHaveCount(0);
  });
});

async function installIssuePreviewBackend(page, { agentStatus = "running" } = {}) {
  await page.addInitScript(
    ({ agentStatus: status }) => {
      window.__knowledgeLoadMessages = [];

      const issueWindow = {
        id: "tab-issue::issue-1",
        title: "Issue",
        preset: "issue",
        geometry: { x: 40, y: 60, width: 1400, height: 860 },
        z_index: 1,
        status: "running",
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: null,
        agent_color: null,
        tab_group_id: null,
        tab_group_active: false,
      };

      const agentWindow = (id, issueNumber, title) => ({
        id,
        title,
        preset: "agent",
        geometry: { x: 120, y: 120, width: 1280, height: 800 },
        z_index: 2,
        status,
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: "codex",
        agent_color: null,
        tab_group_id: null,
        tab_group_active: false,
        placement: {
          kind: "issue_preview",
          issue_window_id: "tab-issue::issue-1",
          issue_number: issueNumber,
        },
      });

      let windows = [
        issueWindow,
        agentWindow("tab-issue::agent-preview", 3671, "Issue #3671 agent"),
        agentWindow("tab-issue::agent-preview-2", 3672, "Issue #3672 agent"),
      ];

      const workspaceState = () => ({
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-issue",
              title: "Fixture Project",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: windows.map((entry) => ({ ...entry })),
              },
            },
          ],
          active_tab_id: "tab-issue",
          recent_projects: [],
        },
      });

      const entries = [3671, 3672].map((number) => ({
        number,
        title: `Issue #${number}`,
        state: "open",
        meta: "Auto-launch fixture",
        labels: ["bug"],
        linked_branch_count: 0,
        match_score: 100,
        phase: null,
        has_unknown_phase: false,
        is_spec: false,
        monitor_state: "launched",
        queue_position: null,
        exclusion_reason: null,
        // SPEC-3671 FR-012: the backend-computed Issue -> Work correlation.
        related_work_refs: [
          { id: `work-${number}`, branch: `work/issue-${number}`, updated_at: "" },
        ],
      }));

      // SPEC-3671 FR-012: the active Work projection the frontend already
      // receives. The Issue row joins it; it is never re-derived.
      const activeWorkProjection = {
        id: "projection-1",
        title: "Fixture Project",
        status_category: "active",
        status_text: "",
        board_refs: [],
        journal_entries: [],
        works: [],
        agents: [],
        unassigned_agents: [],
        active_works: [
          {
            id: "work-3671",
            title: "Issue window as the primary surface",
            status_category: "blocked",
            status_text: "Implementing P4",
            blocked_reason: "Waiting on review",
            lifecycle_state: "active",
            active_agents: 1,
            blocked_agents: 1,
            branch: "work/issue-3671",
            worktree_path: "/fixture/work/issue-3671",
            pr_number: 3699,
            pr_url: "https://example.invalid/pull/3699",
            pr_state: "open",
            board_refs: [],
            agents: [],
            works: [],
            cleanup_candidate: null,
            cleanup_blocked_reason: "live_agent",
            updated_at: "2026-08-19T00:00:00Z",
          },
        ],
      };

      class FixtureWebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;

        constructor(url) {
          super();
          this.url = url;
          this.readyState = FixtureWebSocket.CONNECTING;
          window.__fixtureSocket = this;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw) {
          const message = JSON.parse(raw);
          window.__knowledgeLoadMessages.push(message);
          if (message.kind === "frontend_ready") {
            this.emit(workspaceState());
            this.emit({
              kind: "active_work_projection",
              projection: activeWorkProjection,
            });
            return;
          }
          if (message.kind === "load_knowledge_bridge") {
            this.emit({
              kind: "knowledge_entries",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              entries,
              selected_number: 3671,
              empty_message: null,
              refresh_enabled: true,
            });
            return;
          }
          if (message.kind === "select_knowledge_bridge_entry") {
            this.emit({
              kind: "knowledge_detail",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              detail: {
                number: message.number,
                title: `Issue #${message.number}`,
                state: "open",
                subtitle: "Cached Issue detail",
                labels: ["bug"],
                launch_issue_number: message.number,
                sections: [
                  {
                    title: "Description",
                    body: "Issue preview fixture body",
                    body_html: "<p>Issue preview fixture body</p>",
                  },
                ],
              },
            });
            return;
          }
          if (message.kind === "undock_agent_window") {
            // The backend answers a Windowize by moving the window to the canvas.
            windows = windows.map((entry) =>
              entry.id === message.id
                ? { ...entry, placement: { kind: "canvas" } }
                : entry,
            );
            this.emit(workspaceState());
          }
        }

        close() {
          this.readyState = FixtureWebSocket.CLOSED;
          this.dispatchEvent(new CloseEvent("close"));
        }

        emit(payload) {
          setTimeout(() => {
            this.dispatchEvent(
              new MessageEvent("message", { data: JSON.stringify(payload) }),
            );
          }, 0);
        }
      }

      window.__emitAgentOutput = (text) => {
        window.__fixtureSocket?.emit({
          kind: "terminal_output",
          id: "tab-issue::agent-preview",
          data_base64: btoa(`${text}\r\n`),
        });
      };

      Object.defineProperty(window, "WebSocket", {
        configurable: true,
        value: FixtureWebSocket,
      });
    },
    { agentStatus },
  );
}
