/* SPEC-2356 Phase 10 — Quiet Work UI E2E (embedded-routes).
 *
 * Drives the embedded frontend with a stubbed WebSocket so the new
 * Workspace Overview List+Detail surface and Release Notes modal chrome
 * can be exercised end-to-end without a live gwt backend.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Quiet Work UI surfaces (E2E)", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1600, height: 1000 },
  });

  test("Workspace Overview window renders List + Detail shell", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBackend(page);
    await page.goto(APP_URL);

    const overview = page.locator(".workspace-overview-root");
    await expect(overview).toBeVisible();
    await expect(page.locator(".workspace-overview-list-pane")).toBeVisible();
    await expect(page.locator(".workspace-overview-detail-pane")).toBeVisible();
    await expect(page.locator(".workspace-kanban-board")).toHaveCount(0);
    await expect(page.locator("[data-workspace-column]")).toHaveCount(0);

    const rows = page.locator(".workspace-overview-row[data-workspace-id]");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toHaveAttribute("aria-selected", "true");
    await expect(rows.nth(0)).toContainText("Quiet Work UI redesign");

    await rows.nth(1).click();
    await expect(rows.nth(1)).toHaveAttribute("aria-selected", "true");
    await expect(page.locator(".workspace-detail-title")).toHaveText(
      "Completed Workspace",
    );
  });

  test("Workspace detail renders Work → Session with the active conversation highlighted", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBackend(page);
    await page.goto(APP_URL);

    // Row 0 ("Quiet Work UI redesign") is auto-selected; its single Work keeps
    // multiple conversation records, but the UI renders the latest Session.
    const rows = page.locator(".workspace-overview-row[data-workspace-id]");
    await expect(rows.nth(0)).toHaveAttribute("aria-selected", "true");

    const sessions = page.locator(".workspace-detail-session");
    await expect(sessions).toHaveCount(1);

    const active = page.locator('.workspace-detail-session[data-active="true"]');
    await expect(active).toHaveCount(1);
    await expect(active).toContainText("conv-bbb");
    // The rendered latest Session is badged "Current".
    await expect(
      active.locator('.workspace-detail-session-badge[data-session-state="current"]'),
    ).toHaveText("Current");
    await expect(
      page.locator('.workspace-detail-session-badge[data-session-state="past"]'),
    ).toHaveCount(0);

    // Canonical nested Works lead with their purpose. The Session row remains
    // explicitly labelled so conversation history cannot be mistaken for a
    // second Work.
    const heading = page.locator(".workspace-detail-work-heading");
    await expect(heading).toHaveCount(1);
    await expect(heading).toHaveText("Quiet Work UI redesign");
    await expect(sessions.first()).toContainText("Session");
    // Persistent data renders; never the stale "No assigned agents" placeholder.
    await expect(page.locator(".workspace-overview-detail-pane")).not.toContainText(
      "No assigned agents",
    );

    // The surface is titled "Workspace" (the selected entity is a Workspace,
    // not an individual Work).
    await expect(page.locator(".workspace-overview-root .knowledge-heading")).toHaveText(
      "Workspace",
    );
    // Producing continuation lives on the Work. A Session-level Resume remains
    // a history/Inspection action and carries no execution authority.
    await expect(page.locator("[data-action='resume-workspace']")).toHaveCount(0);
    await expect(page.locator("[data-action='resume-work']")).toHaveCount(0);
    const continueWork = page.locator("[data-action='continue-work']");
    await expect(continueWork).toHaveCount(1);
    await expect(continueWork).toHaveAttribute("data-work-id", "work-quiet-ui");
    const sessionResume = page.locator("[data-action='resume-session']");
    await expect(sessionResume).toHaveCount(1);
    await expect(sessionResume).toHaveAttribute("data-session-id", "agent-current");
    await expect(sessionResume).toHaveAttribute(
      "data-agent-session-id",
      "conv-bbbb2222",
    );
  });

  test("Continue work sends opaque intent, ignores stale outcome, and settles on strong fallback", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBackend(page);
    await page.goto(APP_URL);

    const button = page.locator("[data-action='continue-work']");
    await expect(button).toBeVisible();
    await button.click();
    await expect
      .poll(() =>
        page.evaluate(() => (window as any).__continueWorkMessages.length),
      )
      .toBe(1);
    const messages = await page.evaluate(
      () => (window as any).__continueWorkMessages,
    );
    expect(messages).toHaveLength(1);
    expect(Object.keys(messages[0]).sort()).toEqual([
      "bounds",
      "kind",
      "operation_id",
      "work_id",
    ]);
    expect(messages[0].work_id).toBe("work-quiet-ui");
    await expect(button).toBeDisabled();
    await expect(button).toHaveText("Continuing...");

    await page.evaluate(() => {
      const original = (window as any).__continueWorkMessages[0];
      (window as any).__fixtureSocket.emit({
        kind: "continue_work_outcome",
        operation_id: "stale-operation",
        work_id: original.work_id,
        outcome: "continued_conversation",
        retryable: false,
      });
    });
    await expect(button).toBeDisabled();

    await page.evaluate(() => {
      const original = (window as any).__continueWorkMessages[0];
      (window as any).__fixtureSocket.emit({
        kind: "continue_work_outcome",
        operation_id: original.operation_id,
        work_id: original.work_id,
        outcome: "started_with_handoff",
        retryable: false,
      });
    });
    await expect(button).toBeEnabled();
    await expect(button).toHaveText("Continue work");
    await expect(page.getByText("Work continued", { exact: true })).toBeVisible();
    await expect(
      page.getByText(
        "The previous conversation was unavailable, so a new conversation started with handoff context.",
        { exact: true },
      ),
    ).toBeVisible();
  });

  test("Release Notes opens as a modal-style op-global-window", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBackend(page);
    await page.goto(APP_URL);

    const trigger = page.locator("#app-version");
    await expect(trigger).toBeVisible();
    await trigger.click();

    const release = page.locator("#release-notes-window");
    await expect(release).toBeVisible();
    await expect(release).toHaveAttribute("role", "dialog");
    await expect(release).toHaveClass(/op-global-window/);

    const cssPosition = await release.evaluate(
      (el) => getComputedStyle(el as HTMLElement).position,
    );
    expect(cssPosition).toBe("fixed");

    await expect(
      release.locator(".release-notes-sidebar-item.is-selected"),
    ).toHaveAttribute("data-version", "9.42.1");
    await expect(release.locator(".release-notes-content h2")).toHaveText(
      "v9.42.1",
    );

    await page.keyboard.press("Escape");
    await expect(release).toHaveCount(0);
  });
});

async function installBackend(page: any) {
  await page.addInitScript(() => {
    (window as any).__continueWorkMessages = [];
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "9.42.1",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "workspace-window-1",
                  title: "Workspace",
                  preset: "workspace",
                  geometry: { x: 80, y: 80, width: 1280, height: 760 },
                  z_index: 1,
                  status: "running",
                  minimized: false,
                  maximized: false,
                  pre_maximize_geometry: null,
                  persist: true,
                  purpose_title: null,
                  dynamic_title: null,
                  dynamic_title_detail: null,
                  agent_id: null,
                  agent_color: null,
                  tab_group_id: null,
                  tab_group_active: false,
                },
              ],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    const activeAgent = {
      session_id: "agent-current",
      agent_id: "codex",
      display_name: "Codex",
      status_category: "idle",
      title_summary: "Phase 10 implementation",
      current_focus: "Workspace Overview shell",
      sessions: [
        {
          agent_session_id: "conv-aaaa1111",
          started_at: "2026-05-21T03:20:00Z",
          is_active: false,
        },
        {
          agent_session_id: "conv-bbbb2222",
          started_at: "2026-05-21T04:00:00Z",
          is_active: true,
        },
      ],
    };
    const projection = {
      kind: "active_work_projection",
      projection: {
        id: "workspace-current",
        title: "Quiet Work UI redesign",
        status_category: "active",
        status_text: "Phase 10 implementation",
        summary: "Quiet Work UI redesign in flight.",
        owner: "SPEC-2356",
        branch: "work/20260521-0234",
        workspaces: [
          {
            id: "workspace-current",
            title: "Quiet Work UI redesign",
            intent: "Workspace Overview Quiet Work UI",
            summary: "List + Detail surface validation.",
            owner: "SPEC-2356",
            status_category: "active",
            lifecycle_stage: "active",
            branch: "work/20260521-0234",
            worktree_path: "/repo/work/20260521-0234",
            pr_number: 2856,
            pr_state: "open",
            board_refs: ["board-claim-1"],
            agents: [activeAgent],
            works: [
              {
                id: "work-quiet-ui",
                title: "Quiet Work UI redesign",
                work_summary: "Quiet Work UI redesign",
                lifecycle_state: "active",
                status_category: "idle",
                agents: [activeAgent],
                manual_close_allowed: false,
                close_blocked_reason: "",
              },
            ],
            events: [],
          },
          {
            id: "workspace-done",
            title: "Completed Workspace",
            summary: "Already merged.",
            owner: "Issue #2780",
            status_category: "done",
            lifecycle_stage: "done",
            agents: [],
            events: [],
          },
        ],
        unassigned_agents: [],
      },
    };

    const releaseEntries = [
      {
        version: "9.42.1",
        date: "2026-05-21",
        sections: [
          {
            heading: "Fixed",
            items: ["Quiet Work UI guardrails."],
          },
        ],
      },
      {
        version: "9.42.0",
        date: "2026-05-20",
        sections: [
          {
            heading: "Added",
            items: ["Workspace auto resume."],
          },
        ],
      },
    ];

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      url: string;
      readyState: number;

      constructor(url: string) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        (window as any).__fixtureSocket = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string) {
        const message = JSON.parse(raw);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          setTimeout(() => this.emit(projection), 0);
          return;
        }
        if (message.kind === "open_release_notes") {
          this.emit({
            kind: "release_notes_payload",
            id: message.id,
            current_version: "9.42.1",
            focus_version: message.focus_version || "9.42.1",
            entries: releaseEntries,
          });
          return;
        }
        if (message.kind === "continue_work") {
          (window as any).__continueWorkMessages.push(message);
        }
      }

      close() {
        this.readyState = FixtureWebSocket.CLOSED;
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

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
