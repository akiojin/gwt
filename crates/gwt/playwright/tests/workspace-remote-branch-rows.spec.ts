// SPEC-2359 US-83 (UX revision 2026-06-24) — eligible existing remote branches
// fold into the SAME Workspace list as ordinary rows, distinguished only by the
// shared "Remote" tag (no separate "Start work on a branch" section, no
// local/remote split). Picking ▶ continues on the branch via the existing
// open_launch_wizard path. This E2E renders the real UI so the rows are proven
// to appear, styled, on the reachable Workspace surface.

import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.beforeEach(async ({ page }) => {
  await installEmbeddedRoutes(page);
  await installBackend(page);
});

test("remote branches fold into the unified Workspace list as Remote-tagged rows", async ({
  page,
}) => {
  await page.goto(APP_URL);

  // The Workspace surface (the reachable one) mounts.
  await expect(page.locator(".workspace-overview-root")).toBeVisible();

  // There is no separate remote-branch section — the rows are in the list.
  await expect(page.locator(".workspace-overview-remote-branches")).toHaveCount(0);

  const list = page.locator(".workspace-overview-list");
  // The local Work and the two eligible remote branches share one list.
  await expect(list.locator('.workspace-overview-row[data-workspace-id="work-local"]')).toBeVisible();
  const remoteRows = list.locator('.workspace-overview-row[data-workspace-id^="remote-start:"]');
  await expect(remoteRows).toHaveCount(2);

  const firstRemote = list.locator(
    '.workspace-overview-row[data-workspace-id="remote-start:feature-foo"]',
  );
  // Bare name (origin/ stripped) + the shared Remote tag, styled like any row.
  await expect(firstRemote.locator(".workspace-overview-row-title")).toHaveText("feature-foo");
  await expect(firstRemote.locator(".workspace-overview-remote")).toHaveText("Remote");
  await expect(firstRemote).toHaveAttribute("data-attention", "paused");

  // ▶ continues on the branch via open_launch_wizard; the backend normalizes
  // the (already stripped) name to continue-on-branch.
  await firstRemote.locator("[data-action='launch-workspace-row']").click();
  const sent = await page.evaluate(
    () => (window as unknown as { __sent: Array<{ kind: string; branch_name?: string }> }).__sent,
  );
  const launch = sent.find((m) => m.kind === "open_launch_wizard");
  expect(launch).toBeTruthy();
  expect(launch?.branch_name).toBe("feature-foo");
});

async function installBackend(page: any) {
  await page.addInitScript(() => {
    (window as unknown as { __sent: unknown[] }).__sent = [];

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
                  preset: "work",
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

    const projection = {
      kind: "active_work_projection",
      projection: {
        id: "workspace-current",
        title: "Fixture",
        status_category: "active",
        status_text: "",
        summary: "",
        owner: null,
        branch: null,
        workspaces: [],
        active_works: [
          {
            id: "work-local",
            title: "Local paused work",
            status_category: "idle",
            lifecycle_state: "paused",
            branch: "work/local-1",
            active_agents: 0,
            blocked_agents: 0,
            agents: [],
          },
        ],
        unassigned_agents: [],
        agents: [],
        journal_entries: [],
      },
    };

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
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string) {
        const message = JSON.parse(raw);
        (window as unknown as { __sent: unknown[] }).__sent.push(message);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          setTimeout(() => this.emit(projection), 0);
          return;
        }
        if (message.kind === "request_remote_start_work_branches") {
          this.emit({
            kind: "remote_start_work_branches",
            id: message.id,
            branches: ["origin/feature-foo", "origin/feature/bar"],
          });
          return;
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
