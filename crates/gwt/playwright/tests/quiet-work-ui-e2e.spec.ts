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
    await expect(heading).toHaveText(
      "Quiet Work UI redesign with a deliberately long task purpose that must not crowd actions",
    );
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
    // Producing continuation lives on the Work. A Session-level Resume
    // reopens the conversation with input enabled; producing authority is
    // recovered by the backend continuation coordinator when applicable.
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
    await expect(sessionResume).toHaveText("Open session");
  });

  test("Workspace detail exposes blocked execution recovery without replacing purpose", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBackend(page);
    await page.goto(APP_URL);

    await expect(page.locator(".workspace-detail-title")).toHaveText(
      "Quiet Work UI redesign",
    );
    const diagnosis = page.locator(
      '[data-section="execution-diagnosis"][data-severity="warning"]',
    );
    await expect(diagnosis).toBeVisible();
    await expect(diagnosis).toHaveAttribute("aria-label", "Execution diagnosis");
    await expect(diagnosis).toContainText("Blocked");
    await expect(diagnosis).toContainText("Stale");
    await expect(diagnosis).toContainText("Verification evidence is stale");
    await expect(diagnosis).toContainText("Host status is temporarily unavailable");
    await expect(diagnosis.locator(".workspace-execution-severity")).toHaveText(
      "Warning",
    );
    await expect(diagnosis.locator(".workspace-execution-recovery-list")).toContainText(
      "verify.run",
    );
    await expect(diagnosis.locator(".workspace-execution-recovery-list")).toContainText(
      "execution.reopen",
    );
  });

  test("Work without Session history keeps Continue work and shows one Work-level guidance", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBackend(page, "zero");
    await page.goto(APP_URL);

    const group = page.locator(
      '.workspace-detail-work-group[data-work-id="work-quiet-ui"]',
    );
    await expect(group.locator("[data-action='continue-work']")).toHaveCount(1);
    await expect(group.locator(".workspace-detail-session")).toHaveCount(0);
    await expect(group.getByText("No session yet", { exact: true })).toHaveCount(0);
    await expect(group.locator(".workspace-detail-session-empty")).toHaveCount(0);

    const guidance = group.locator(".workspace-detail-session-guidance");
    await expect(guidance).toHaveCount(1);
    await expect(guidance).toHaveText(
      "No previous session to open. Continue work can start a new one.",
    );
  });

  test("Work with mixed Session history renders only real Sessions without empty guidance", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBackend(page, "mixed");
    await page.goto(APP_URL);

    const group = page.locator(
      '.workspace-detail-work-group[data-work-id="work-quiet-ui"]',
    );
    const sessions = group.locator(".workspace-detail-session");
    await expect(sessions).toHaveCount(1);
    await expect(sessions.first()).toContainText("conv-bbb");
    await expect(group.locator(".workspace-detail-session-empty")).toHaveCount(0);
    await expect(group.locator(".workspace-detail-session-guidance")).toHaveCount(0);
  });

  test("Task-first Work actions stay readable at large and default window sizes", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBackend(page);
    await page.goto(APP_URL);

    const surfaceWindow = page.locator('.workspace-window[data-preset="workspace"]');
    const group = page.locator(
      '.workspace-detail-work-group[data-work-id="work-quiet-ui"]',
    );
    const diagnostics = page.locator('details[data-section="board-diagnostics"]');

    await expect(group.locator(".workspace-detail-session")).toHaveCount(1);
    await expect(group.locator(".workspace-detail-session-empty")).toHaveCount(0);
    await expect(diagnostics).not.toHaveAttribute("open", "");
    await expect(diagnostics.locator("summary")).toHaveText("Diagnostics (4)");
    const lifecycle = page.locator(".workspace-detail-section").filter({
      has: page.locator(".workspace-detail-section-title", { hasText: "Lifecycle" }),
    });
    await expect(lifecycle).not.toContainText("board-event-raw-4");
    await diagnostics.locator("summary").click();
    await expect(diagnostics).toContainText("board-event-raw-4");
    await diagnostics.locator("summary").click();

    for (const [width, height] of [[1280, 760], [720, 420]]) {
      await surfaceWindow.evaluate((node: HTMLElement, size) => {
        node.style.width = `${size.width}px`;
        node.style.height = `${size.height}px`;
      }, { width, height });
      await group.scrollIntoViewIfNeeded();

      const geometry = await group.evaluate((node: HTMLElement) => {
        const head = node.querySelector<HTMLElement>(".workspace-detail-work-head");
        const rail = node.querySelector<HTMLElement>(".workspace-detail-work-action-rail");
        const buttons = Array.from(
          rail?.querySelectorAll<HTMLElement>(".wizard-button") || [],
        );
        const headRect = head!.getBoundingClientRect();
        const railRect = rail!.getBoundingClientRect();
        const buttonRects = buttons.map((button) => button.getBoundingClientRect());
        const overlaps = buttonRects.some((left, index) =>
          buttonRects.slice(index + 1).some((right) =>
            left.left < right.right
              && left.right > right.left
              && left.top < right.bottom
              && left.bottom > right.top,
          ),
        );
        const root = node.closest<HTMLElement>(".workspace-overview-root")!;
        return {
          railBelowHead: railRect.top >= headRect.bottom - 1,
          overlaps,
          labelsStayOnOneLine: buttons.every(
            (button) => getComputedStyle(button).whiteSpace === "nowrap",
          ),
          noHorizontalOverflow: root.scrollWidth <= root.clientWidth,
        };
      });

      expect(geometry, `${width}x${height}`).toEqual({
        railBelowHead: true,
        overlaps: false,
        labelsStayOnOneLine: true,
        noHorizontalOverflow: true,
      });
    }
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

type SessionHistoryFixture = "default" | "zero" | "mixed";

async function installBackend(
  page: any,
  sessionHistory: SessionHistoryFixture = "default",
) {
  await page.addInitScript(({ sessionHistory }) => {
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
    const emptyNewerAgent = {
      ...activeAgent,
      session_id: "agent-empty-newer",
      updated_at: "2026-05-21T05:00:00Z",
      sessions: [],
    };
    const emptyOlderAgent = {
      ...activeAgent,
      session_id: "agent-empty-older",
      updated_at: "2026-05-21T02:00:00Z",
      sessions: [],
    };
    const emptyDistinctAgent = {
      ...activeAgent,
      session_id: "agent-empty-distinct",
      agent_id: "claude-code",
      display_name: "Claude Code",
      updated_at: "2026-05-21T05:00:00Z",
      sessions: [],
    };
    const workAgents = sessionHistory === "zero"
      ? [{ ...activeAgent, sessions: [] }]
      : sessionHistory === "mixed"
        ? [activeAgent, emptyDistinctAgent]
        : [emptyNewerAgent, activeAgent, emptyOlderAgent];
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
            board_refs: ["board-claim-1", "board-status-2", "board-decision-3"],
            agents: [activeAgent],
            works: [
              {
                id: "work-quiet-ui",
                title:
                  "Quiet Work UI redesign with a deliberately long task purpose that must not crowd actions",
                work_summary:
                  "Quiet Work UI redesign with a deliberately long task purpose that must not crowd actions",
                lifecycle_state: "active",
                status_category: "idle",
                agents: workAgents,
                manual_close_allowed: true,
                close_blocked_reason: "",
                execution_diagnosis: {
                  ecr_status: "blocked",
                  owner_kind: "spec",
                  owner_number: 3393,
                  blocked_reason: "Verification evidence is stale",
                  missing_verification: "User confirmation",
                  generation_id: "generation-2",
                  binding_state: "stale",
                  binding_cause: "current_session_not_authorized",
                  verification_state: "stale_fingerprint",
                  settlement: { blocked: "missing_upstream" },
                  settlement_severity: "warning",
                  settlement_obligation_open: true,
                  open_obligations: ["user_verification"],
                  available_recoveries: ["verify.run", "execution.reopen"],
                  warnings: ["Host status is temporarily unavailable"],
                },
              },
            ],
            events: [
              {
                kind: "status",
                title: "board-event-raw-4",
                summary: "board-event-raw-4",
                board_entry_id: "board-event-raw-4",
                updated_at: "2026-05-21T05:30:00Z",
              },
            ],
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
  }, { sessionHistory });
}
