/* SPEC-3671 — Issue window as the primary surface, amended by Issue #3884.
 *
 * An Issue Monitor auto-launch must not add a window to the canvas. It becomes an
 * `issue_preview` placement. Since Issue #3884 that placement is presented as an
 * interactive inline terminal on the Issue row (no selection needed), it is not
 * drawn on the Fleet Minimap, and the Status Strip RUNNING cell breaks out how
 * many running agents live inline. Windowize remains the only hand-off to the
 * canvas, after which the row releases the terminal (one input face per PTY).
 *
 * The fixture serves the embedded frontend through Playwright routes and replaces
 * WebSocket with a deterministic backend, matching `tests/kanban.spec.ts`.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

function collectPageErrors(page) {
  const consoleErrors: string[] = [];
  const pageErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });
  page.on("pageerror", (error) => pageErrors.push(String(error)));
  return { consoleErrors, pageErrors };
}

test.describe("Issue inline terminal placement", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1600, height: 1000 },
  });

  // 受け入れシナリオ 1 / FR-004, plus Issue #3884 AC-1 / AC-3 / AC-5: with three
  // auto-launched agents and an otherwise empty canvas, nothing suggests a
  // vanished window — the minimap shows only the Issue window, and RUNNING says
  // where the agents are.
  test("auto-launched agents add no canvas window, no minimap cell, and RUNNING explains itself", async ({
    page,
  }) => {
    const { consoleErrors, pageErrors } = collectPageErrors(page);
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window.surface-knowledge")).toBeVisible();
    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(0);

    // AC-1: only the canvas-placed Issue window has a radar cell.
    const cells = page.locator("#fleet-minimap .fleet-minimap__cell");
    await expect(cells).toHaveCount(1);
    await expect(cells.first()).toHaveAttribute("data-window-id", "tab-issue::issue-1");

    // AC-3: RUNNING 3 with all 3 inline, and the hover explains it.
    await expect(page.locator("#op-strip-running")).toHaveText("3");
    const inline = page.locator("#op-strip-running-inline");
    await expect(inline).toBeVisible();
    await expect(inline).toHaveText("3 inline");
    await expect(page.locator(".op-status-strip__cell--running")).toHaveAttribute(
      "title",
      "3 of 3 running agents are inline terminals in the Issue window",
    );

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // Issue #3884 AC-6 (SPEC #3885 AC-2): every launched Issue row shows its agent
  // terminal inline without any selection, and typing into it reaches the PTY.
  test("each launched Issue row hosts an interactive inline terminal without selection", async ({
    page,
  }) => {
    const { consoleErrors, pageErrors } = collectPageErrors(page);
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    const rows = page.locator(".surface-knowledge .knowledge-row");
    await expect(rows).toHaveCount(4);
    await expect(page.locator(".surface-knowledge .knowledge-row.selected")).toHaveCount(0);

    const inlineTerminals = page.locator(".surface-knowledge .issue-inline-terminal");
    await expect(inlineTerminals).toHaveCount(3);
    for (const [issue, id] of [
      ["3671", "tab-issue::agent-preview"],
      ["3672", "tab-issue::agent-preview-2"],
      ["3673", "tab-issue::agent-preview-3"],
    ]) {
      const section = page.locator(
        `.surface-knowledge [data-issue-number='${issue}'] .issue-inline-terminal`,
      );
      await expect(section).toHaveAttribute("data-window-id", id);
      // SPEC #3885 T-004: the agent status is the row's single primary badge.
      const row = page.locator(`.surface-knowledge [data-issue-number='${issue}']`);
      await expect(row.locator(".knowledge-row-badge")).toHaveText("Running");
      await expect(section.locator(".knowledge-monitor-chip")).toHaveCount(0);
    }
    // SPEC #3885 AC-5: one primary badge, at most two secondary items, at most
    // two visible actions on every row.
    for (const issue of ["3671", "3672", "3673", "3674"]) {
      const row = page.locator(`.surface-knowledge [data-issue-number='${issue}']`);
      await expect(row.locator(".knowledge-row-badge")).toHaveCount(1);
      expect(await row.locator(".knowledge-row-secondary-item").count()).toBeLessThanOrEqual(2);
      const visibleActions = await row
        .locator("button[data-action]:not(.knowledge-row-menu-list button)")
        .count();
      expect(visibleActions).toBeLessThanOrEqual(2);
      await expect(row.locator(".knowledge-chip")).toHaveCount(0);
      await expect(row.locator(".knowledge-state-chip")).toHaveCount(0);
    }
    await expect(
      page.locator(".surface-knowledge [data-issue-number='3674'] .issue-inline-terminal"),
    ).toHaveCount(0);
    // The old selection-bound mirror is gone, and no copy says "preview".
    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveCount(0);
    await expect(page.locator(".surface-knowledge")).not.toContainText(/preview/i);

    const first = page.locator(
      ".surface-knowledge [data-issue-number='3671'] .issue-inline-terminal",
    );
    await expect(first.locator(".issue-inline-terminal-title")).toHaveText("Issue #3671 agent");
    const terminal = first.locator(".issue-inline-terminal-body .terminal-root");
    await expect(terminal).toBeVisible();
    await expect(terminal.locator(".xterm-rows")).toBeVisible();

    await page.evaluate(() => window.__emitAgentOutput("SPEC-3671 inline line"));
    await expect(terminal).toContainText("SPEC-3671 inline line");

    await terminal.click();
    await page.keyboard.type("echo hi");
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "terminal_input")
            .map((message) => `${message.id}:${message.data}`)
            .join(""),
        ),
      )
      .toContain("tab-issue::agent-preview:e");
    const inputIds = await page.evaluate(() =>
      Array.from(
        new Set(
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "terminal_input")
            .map((message) => message.id),
        ),
      ),
    );
    expect(inputIds).toEqual(["tab-issue::agent-preview"]);
    // Typing into the terminal is not a row-selection gesture.
    await expect(page.locator(".surface-knowledge .knowledge-row.selected")).toHaveCount(0);

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // 受け入れシナリオ 4 / FR-010, plus Issue #3884 AC-7 / AC-1 / AC-3: Windowize
  // puts the agent on the canvas, the row releases its terminal so the PTY has a
  // single input face, the minimap gains the cell, and RUNNING's breakdown drops.
  test("Windowize moves the inline terminal to the canvas and the row releases it", async ({
    page,
  }) => {
    const { consoleErrors, pageErrors } = collectPageErrors(page);
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await expect(page.locator(".surface-knowledge .issue-inline-terminal")).toHaveCount(3);
    await page
      .locator(
        ".surface-knowledge [data-issue-number='3671'] [data-action='windowize-inline-terminal']",
      )
      .click();

    const undocked = await page.evaluate(() =>
      window.__knowledgeLoadMessages.filter(
        (message) => message.kind === "undock_agent_window",
      ),
    );
    expect(undocked).toHaveLength(1);
    expect(undocked[0].id).toBe("tab-issue::agent-preview");

    const canvasWindow = page.locator(".workspace-window.surface-terminal:visible");
    await expect(canvasWindow).toHaveCount(1);
    await expect(canvasWindow).toHaveAttribute("data-id", "tab-issue::agent-preview");
    // SPEC #3885 T-005 / AC-2a: the row keeps the Issue ↔ agent link as a
    // "Shown on canvas" face with no second input face for the PTY.
    const face = page.locator(
      ".surface-knowledge [data-issue-number='3671'] .issue-inline-terminal",
    );
    await expect(face).toHaveCount(1);
    await expect(face).toHaveClass(/is-on-canvas/);
    await expect(face).toContainText("Shown on canvas");
    await expect(face.locator(".terminal-root")).toHaveCount(0);
    await expect(face.locator("[data-action='windowize-inline-terminal']")).toHaveCount(0);
    await expect(
      page.locator(".surface-knowledge [data-issue-number='3671'] .knowledge-row-badge"),
    ).toHaveText("Running");
    await expect(page.locator(".surface-knowledge .issue-inline-terminal .terminal-root")).toHaveCount(2);
    // The Windowized canvas window overlaps the Issue window in the fixture, so
    // dispatch the click instead of relying on hit-testing through it.
    await face.locator("[data-action='focus-canvas-window']").dispatchEvent("click");
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "focus_window")
            .map((message) => message.id)
            .at(-1),
        ),
      )
      .toBe("tab-issue::agent-preview");
    await expect(page.locator(".surface-knowledge .knowledge-row.selected")).toHaveCount(0);
    // AC-7: exactly one xterm instance exists per window id — the canvas one for
    // the windowized agent, the inline ones for the other two.
    await expect(canvasWindow.locator(".terminal-root .xterm")).toHaveCount(1);
    await expect(page.locator(".xterm")).toHaveCount(3);

    // AC-1: the windowized agent now has a radar cell alongside the Issue window.
    await expect(page.locator("#fleet-minimap .fleet-minimap__cell")).toHaveCount(2);
    await expect(
      page.locator("#fleet-minimap .fleet-minimap__cell[data-window-id='tab-issue::agent-preview']"),
    ).toHaveCount(1);

    // AC-3: RUNNING stays 3, of which 2 are still inline.
    await expect(page.locator("#op-strip-running")).toHaveText("3");
    await expect(page.locator("#op-strip-running-inline")).toHaveText("2 inline");

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // 受け入れシナリオ 5 / FR-011.
  test("an errored agent is badged in the Issue row, not opened on the canvas", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page, { agentStatus: "error" });

    await page.goto(APP_URL);

    await expect(
      page.locator(".surface-knowledge [data-issue-number='3671'] .knowledge-row-badge"),
    ).toHaveText("Error");
    await expect(
      page.locator(".surface-knowledge [data-issue-number='3671'] .knowledge-row-badge"),
    ).toHaveAttribute("data-tone", "blocked");
    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(0);
    // An errored agent is not "running", so it is not counted inline either.
    await expect(page.locator("#op-strip-running-inline")).toBeHidden();
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

    // SPEC #3885 T-004: the Work state is folded into the row — the attention
    // reason and the PR are the two secondary items under the primary badge, and
    // the Work actions sit in the row's overflow menu while the agent is live.
    const row = page.locator(".surface-knowledge [data-issue-number='3671']");
    await expect(row.locator(".knowledge-row-work")).toHaveCount(0);
    const secondary = row.locator(".knowledge-row-secondary-item");
    await expect(secondary).toHaveCount(2);
    await expect(secondary.nth(0)).toHaveAttribute("data-kind", "reason");
    await expect(secondary.nth(0)).toHaveText("Waiting on review");
    await expect(secondary.nth(1)).toHaveText("PR #3699 · open");
    await expect(
      row.locator("button[data-action]:not(.knowledge-row-menu-list button)"),
    ).toHaveText(["Windowize"]);
    const menu = row.locator(".knowledge-row-menu");
    await expect(menu).toHaveCount(1);
    await menu.locator("summary").click();
    await expect(menu).toHaveAttribute("open", "");
    await expect(menu.locator('[data-action="continue-work"]')).toBeVisible();
    await expect(menu.locator('[data-action="continue-work"]')).toBeEnabled();
    await expect(menu.locator('[data-action="resume-work"]')).toBeEnabled();
    // The backend owns cleanup eligibility; a live agent keeps the action off.
    await expect(menu.locator('[data-action="cleanup-work"]')).toBeDisabled();
    await expect(page.locator(".surface-knowledge .knowledge-row.selected")).toHaveCount(0);

    // An Issue with no correlated Work row has no PR chip and no Work actions.
    const other = page.locator(".surface-knowledge [data-issue-number='3672']");
    await expect(other.locator('.knowledge-row-secondary-item[data-key="pr"]')).toHaveCount(0);
    await expect(other.locator('[data-action="continue-work"]')).toHaveCount(0);
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
        agentWindow("tab-issue::agent-preview-3", 3673, "Issue #3673 agent"),
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

      const entries = [3671, 3672, 3673, 3674].map((number) => ({
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
              // Issue #3884 AC-6: nothing is selected — the rows must still show
              // their inline terminals.
              selected_number: null,
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
