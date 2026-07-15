/* Knowledge Bridge Work Item compatibility coverage.
 *
 * The fixture serves the embedded frontend assets directly through Playwright
 * routes and replaces WebSocket with a deterministic cache-backed backend.
 * That keeps browser coverage active in CI without depending on a live gwt GUI
 * process, GitHub cache state, or the user's local workspace.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Legacy SPEC preset Work Item compatibility", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 3840, height: 1100 },
  });

  test("renders gwt-spec entries through the unified Work Item list", async ({
    page,
  }, testInfo) => {
    await installEmbeddedRoutes(page);
    await installSpecPresetBackend(page, {
      theme: testInfo.project.name.includes("light") ? "light" : "dark",
    });

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-list")).toBeVisible();
    await expect(page.locator(".surface-knowledge .knowledge-heading")).toHaveText(
      "Cached work items",
    );
    await expect(page.locator(".surface-knowledge .knowledge-search")).toHaveAttribute(
      "placeholder",
      "Semantic search work items",
    );
    await expect(page.locator(".surface-knowledge .kanban-board")).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(5);
    await expect(page.getByText("SPEC Issue Kanban View")).toBeVisible();
    await expect(page.getByText("Merge Kanban implementation bundle")).toHaveCount(0);
  });
});

test.describe("Issue Bridge load recovery", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("renders cached issues as an Issue list instead of lifecycle Kanban", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-list")).toBeVisible();
    await expect(page.locator(".surface-knowledge .knowledge-list")).toHaveAttribute(
      "aria-label",
      "Cached work items",
    );
    await expect(page.locator(".surface-knowledge .kanban-board")).toHaveCount(0);
    await expect(
      page.locator(".surface-knowledge .kanban-column[data-phase='planning']"),
    ).toHaveCount(0);
    await expect(
      page.locator(".surface-knowledge .kanban-column[data-phase='implementation']"),
    ).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(3);
    await expect(page.locator(".surface-knowledge .knowledge-heading")).toHaveText(
      "Cached work items",
    );
    await expect(page.locator(".surface-knowledge .knowledge-search")).toHaveAttribute(
      "placeholder",
      "Semantic search work items",
    );
    await expect(page.getByText("Closed issue hidden by default")).toHaveCount(0);
    await expect(page.getByText("Design-required work item shares Issue list")).toBeVisible();
    await expect(page.getByText("(plain)")).toHaveCount(0);
  });

  test("Issue state filter defaults to open and can show closed or all issues", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(3);
    await expect(page.getByText("Closed issue hidden by default")).toHaveCount(0);

    await page.locator(".surface-knowledge [data-issue-filter='closed']").click();

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(1);
    await expect(page.getByText("Closed issue hidden by default")).toBeVisible();

    await page.locator(".surface-knowledge [data-issue-filter='all']").click();

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(4);
  });

  test("selecting an Issue row renders cached detail in the right pane", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page);

    await page.goto(APP_URL);

    await page.locator(".surface-knowledge .knowledge-row[data-issue-number='3096']").click();

    await expect(
      page.locator(".surface-knowledge .knowledge-detail-pane"),
    ).toContainText("Issue Bridge detail body");
    await expect(
      page.locator(".surface-knowledge .knowledge-detail-pane"),
    ).toContainText("Launch Agent");
  });

  test("Issue auto refresh stays cache-first while browsing cached issues", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page, {
      errorOnForcedRefresh: true,
      triggerAutoRefreshOnce: true,
    });

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(3);
    await page.locator(".surface-knowledge .knowledge-row[data-issue-number='3095']").click();
    await expect(
      page.locator(".surface-knowledge .knowledge-detail-pane"),
    ).toContainText("Issue #3095");
    await page.waitForFunction(
      () => typeof window.__triggerKnowledgeAutoRefresh === "function",
    );
    await page.evaluate(() => window.__triggerKnowledgeAutoRefresh());
    await page.waitForFunction(() =>
      window.__knowledgeLoadMessages?.filter(
        (message) => message.kind === "load_knowledge_bridge",
      ).length >= 2,
    );

    await expect(page.locator(".surface-knowledge .knowledge-status.error")).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .knowledge-status")).toHaveText("");
    const refreshFlags = await page.evaluate(() =>
      window.__knowledgeLoadMessages
        .filter((message) => message.kind === "load_knowledge_bridge")
        .map((message) => message.refresh),
    );
    expect(refreshFlags).not.toContain(true);
  });

  test("requests cached issues when a stale detail exists but the list is empty", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page, { staleDetailBeforeWorkspace: true });

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .knowledge-list")).toBeVisible();
    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(3);
  });

  test("manual refresh recovers a stale empty loading state", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIssueBridgeBackend(page, { ignoreFirstLoad: true });

    await page.goto(APP_URL);

    const refresh = page.locator(
      ".surface-knowledge [data-action='refresh-knowledge']",
    );
    await expect(refresh).toBeEnabled();
    await refresh.click();

    await expect(page.locator(".surface-knowledge .knowledge-row")).toHaveCount(3);
  });
});

async function installSpecPresetBackend(page, { theme }) {
  await page.addInitScript(
    ({ theme: selectedTheme }) => {
      const entries = [
        {
          number: 2017,
          title: "SPEC Issue Kanban View",
          state: "open",
          meta: "Phase 4 visual coverage",
          labels: ["gwt-spec", "phase/implementation"],
          linked_branch_count: 2,
          match_score: 99,
          phase: "implementation",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 1935,
          title: "Coordination hooks and Board reminders",
          state: "open",
          meta: "Planning refinement",
          labels: ["gwt-spec", "phase/planning"],
          linked_branch_count: 1,
          match_score: 88,
          phase: "planning",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2008,
          title: "Window host interaction model",
          state: "open",
          meta: "Review follow-up",
          labels: ["gwt-spec", "phase/review"],
          linked_branch_count: 3,
          match_score: 82,
          phase: "review",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2077,
          title: "Runtime daemon event transport",
          state: "open",
          meta: "Draft architecture",
          labels: ["gwt-spec", "phase/draft"],
          linked_branch_count: 0,
          match_score: 76,
          phase: "draft",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2359,
          title: "Work Kanban stabilization",
          state: "open",
          meta: "Unscheduled backlog",
          labels: ["gwt-spec"],
          linked_branch_count: 0,
          match_score: 71,
          phase: null,
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 2470,
          title: "Merge Kanban implementation bundle",
          state: "closed",
          meta: "Completed rollout",
          labels: ["gwt-spec", "phase/done"],
          linked_branch_count: 1,
          match_score: 100,
          phase: "done",
          has_unknown_phase: false,
          is_spec: true,
        },
      ];

      const workspaceState = {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
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
                    id: "spec-kanban",
                    title: "SPEC Kanban",
                    preset: "spec",
                    geometry: { x: 96, y: 76, width: 3600, height: 820 },
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

      localStorage.setItem("gwt:ui:theme", selectedTheme);

      class FixtureWebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;

        constructor(url) {
          super();
          this.url = url;
          this.readyState = FixtureWebSocket.CONNECTING;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw) {
          const message = JSON.parse(raw);
          if (message.kind === "frontend_ready") {
            this.emit(workspaceState);
            return;
          }
          if (message.kind === "load_knowledge_bridge") {
            this.emit({
              kind: "knowledge_entries",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              entries,
              selected_number: 2017,
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
                title: `SPEC #${message.number}`,
                state: "open",
                subtitle: "Deterministic fixture detail",
                labels: ["gwt-spec"],
                launch_issue_number: message.number,
                sections: [
                  {
                    title: "Acceptance",
                    body: "Kanban columns stay readable in dark and light themes.",
                  },
                ],
              },
            });
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

      Object.defineProperty(window, "WebSocket", {
        configurable: true,
        value: FixtureWebSocket,
      });
    },
    { theme },
  );
}

async function installIssueBridgeBackend(
  page,
  {
    errorOnForcedRefresh = false,
    ignoreFirstLoad = false,
    staleDetailBeforeWorkspace = false,
    triggerAutoRefreshOnce = false,
  } = {},
) {
  await page.addInitScript(
    ({
      errorOnForcedRefresh: shouldErrorOnForcedRefresh,
      ignoreFirstLoad: shouldIgnoreFirstLoad,
      staleDetailBeforeWorkspace: shouldSeedStaleDetail,
      triggerAutoRefreshOnce: shouldTriggerAutoRefreshOnce,
    }) => {
      window.__knowledgeLoadMessages = [];
      if (shouldTriggerAutoRefreshOnce) {
        window.__knowledgeAutoRefreshCallbacks = [];
        window.__triggerKnowledgeAutoRefresh = () => {
          const callbacks = window.__knowledgeAutoRefreshCallbacks || [];
          for (const callback of callbacks) {
            callback();
          }
        };
        const originalSetInterval = window.setInterval.bind(window);
        window.setInterval = (callback, delay, ...args) => {
          if (delay === 60000) {
            window.__knowledgeAutoRefreshCallbacks.push(() => callback(...args));
          }
          return originalSetInterval(callback, delay, ...args);
        };
      }
      const entries = [
        {
          number: 3273,
          title: "Design-required work item shares Issue list",
          state: "open",
          meta: "gwt-spec tagged work item",
          labels: ["GWT-SPEC", "phase/implementation"],
          linked_branch_count: 0,
          match_score: 98,
          phase: "implementation",
          has_unknown_phase: false,
          is_spec: true,
        },
        {
          number: 3096,
          title: "Issue Bridge shows empty columns despite cached issues",
          state: "open",
          meta: "Regression fixture",
          labels: ["bug"],
          linked_branch_count: 0,
          match_score: 100,
          phase: null,
          has_unknown_phase: false,
          is_spec: false,
        },
        {
          number: 3094,
          title: "Closed issue hidden by default",
          state: "closed",
          meta: "Cached closed issue",
          labels: ["bug"],
          linked_branch_count: 1,
          match_score: 87,
          phase: null,
          has_unknown_phase: false,
          is_spec: false,
        },
        {
          number: 3095,
          title: "Session TOML corruption on new agent session",
          state: "open",
          meta: "Cached plain issue",
          labels: ["bug"],
          linked_branch_count: 0,
          match_score: 96,
          phase: null,
          has_unknown_phase: false,
          is_spec: false,
        },
      ];

      const workspaceState = {
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
                windows: [
                  {
                    id: "issue-kanban",
                    title: "Issue",
                    preset: "issue",
                    geometry: { x: 40, y: 60, width: 1320, height: 760 },
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
          active_tab_id: "tab-issue",
          recent_projects: [],
        },
      };

      class FixtureWebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;

        loadCount = 0;

        constructor(url) {
          super();
          this.url = url;
          this.readyState = FixtureWebSocket.CONNECTING;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw) {
          const message = JSON.parse(raw);
          window.__knowledgeLoadMessages.push(message);
          if (message.kind === "frontend_ready") {
            if (shouldSeedStaleDetail) {
              this.emit({
                kind: "knowledge_detail",
                id: "issue-kanban",
                knowledge_kind: "issue",
                request_id: 0,
                detail: {
                  number: 3095,
                  title: "Stale cached detail",
                  state: "open",
                  subtitle: "Detail survived without entries",
                  labels: ["bug"],
                  launch_issue_number: 3095,
                  sections: [],
                },
              });
            }
            this.emit(workspaceState);
            return;
          }
          if (message.kind === "load_knowledge_bridge") {
            this.loadCount += 1;
            if (shouldIgnoreFirstLoad && this.loadCount === 1) {
              return;
            }
            if (shouldErrorOnForcedRefresh && message.refresh === true) {
              this.emit({
                kind: "knowledge_error",
                id: message.id,
                knowledge_kind: message.knowledge_kind,
                request_id: message.request_id,
                message: "gh issue list: HTTP 401: Requires authentication",
              });
              return;
            }
            this.emit({
              kind: "knowledge_entries",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              entries,
              selected_number: 3096,
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
                state: message.number === 3094 ? "closed" : "open",
                subtitle: "Cached Issue detail",
                labels: ["bug"],
                launch_issue_number: message.number,
                sections: [
                  {
                    title: "Description",
                    body: "Issue Bridge detail body",
                    body_html: "<p>Issue Bridge detail body</p>",
                  },
                  {
                    title: "Linked branches",
                    body: message.number === 3094 ? "work/closed" : "None",
                    body_html: message.number === 3094 ? "<p>work/closed</p>" : "<p>None</p>",
                  },
                ],
              },
            });
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

      Object.defineProperty(window, "WebSocket", {
        configurable: true,
        value: FixtureWebSocket,
      });
    },
    {
      errorOnForcedRefresh,
      ignoreFirstLoad,
      staleDetailBeforeWorkspace,
      triggerAutoRefreshOnce,
    },
  );
}
