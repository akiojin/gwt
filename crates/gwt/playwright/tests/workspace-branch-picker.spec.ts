// SPEC-2359 US-83 — the "Open a branch…" action lives on the REACHABLE
// Workspace surface (preset "work" → mountWorkSurface), opens the remote-branch
// picker, shows bare branch names, and picking one launches via the existing
// open_launch_wizard message. This E2E exists specifically because an earlier
// attempt put the affordance on the unreachable Branches window; here we assert
// the button renders on the surface a user actually opens.

import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.beforeEach(async ({ page }) => {
  await installEmbeddedRoutes(page);
  await installBackend(page);
});

test("Open a branch… renders on the Workspace surface and opens the picker", async ({
  page,
}) => {
  await page.goto(APP_URL);

  // The Workspace surface (the reachable one) mounts.
  await expect(page.locator(".workspace-overview-root")).toBeVisible();

  // The "Open a branch…" toolbar action is present and visible on it.
  const openBranch = page.locator(
    "[data-action='open-workspace-branch-picker']",
  );
  await expect(openBranch).toBeVisible();
  await expect(openBranch).toHaveText(/Open a branch/);

  // Clicking it opens the picker modal.
  await openBranch.click();
  const modal = page.locator("#workspace-branch-picker-modal");
  await expect(modal).toHaveClass(/open/);

  // The backend response renders rows with BARE names (origin/ stripped) and a
  // remote hint — the launch-facing "abstract remote/local" display.
  const rows = modal.locator(".workspace-branch-picker-row");
  await expect(rows).toHaveCount(2);
  await expect(
    modal.locator(".workspace-branch-picker-row-name").first(),
  ).toHaveText("feature-foo");
  await expect(
    modal.locator(".workspace-branch-picker-row-tag").first(),
  ).toHaveText("remote");

  // Picking a branch hands off to the launch wizard via open_launch_wizard with
  // the raw ref (the backend normalizes origin/ for continue-on-branch).
  await rows.first().click();
  const sent = await page.evaluate(
    () => (window as unknown as { __sent: Array<{ kind: string; branch_name?: string }> }).__sent,
  );
  const launch = sent.find((m) => m.kind === "open_launch_wizard");
  expect(launch).toBeTruthy();
  expect(launch?.branch_name).toBe("origin/feature-foo");

  // The picker closes after handing off.
  await expect(modal).not.toHaveClass(/open/);
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
        active_works: [],
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
