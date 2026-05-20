/* SPEC-2009 Phase 3 - Branch Cleanup E2E coverage.
 *
 * Exercises the embedded Branches window with a deterministic WebSocket
 * backend so the cleanup confirmation, force-delete option, progress stream,
 * and result modal are verified without touching a real Git repository.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Branch Cleanup E2E", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("shows cleanup progress and sends force filesystem delete", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installBranchCleanupBackend(page);

    await page.goto(APP_URL);

    const branchesWindow = page.locator(".workspace-window.surface-branches");
    await expect(branchesWindow).toBeVisible({ timeout: 10_000 });

    const firstBranch = branchesWindow.locator(
      ".branch-row[data-branch-name='work/cleanup-one']",
    );
    const secondBranch = branchesWindow.locator(
      ".branch-row[data-branch-name='work/cleanup-two']",
    );
    await expect(firstBranch).toContainText("safe");
    await expect(secondBranch).toContainText("risky");

    await firstBranch.locator(".branch-cleanup-toggle").click();
    await secondBranch.locator(".branch-cleanup-toggle").click();

    const cleanupTrigger = branchesWindow.getByRole("button", {
      name: "Clean Up (2)",
    });
    await expect(cleanupTrigger).toBeEnabled();
    await cleanupTrigger.click();

    const modal = page.locator("#branch-cleanup-modal");
    await expect(modal).toHaveClass(/open/);
    await expect(modal.getByRole("dialog", { name: "Branch cleanup" }))
      .toContainText("Delete 2 selected branches.");
    await expect(modal).toContainText("work/cleanup-one");
    await expect(modal).toContainText("work/cleanup-two");

    const forceToggle = modal.getByLabel("Force remove remaining worktree files");
    await expect(forceToggle).not.toBeChecked();
    await forceToggle.check();
    await expect(forceToggle).toBeChecked();

    await modal.getByRole("button", { name: "Run cleanup" }).click();
    await page.waitForFunction(() =>
      window.__branchCleanupCalls?.some(
        (call) => call.kind === "run_branch_cleanup",
      ),
    );

    const cleanupCall = await page.evaluate(() =>
      window.__branchCleanupCalls.find(
        (call) => call.kind === "run_branch_cleanup",
      ),
    );
    expect(cleanupCall).toMatchObject({
      kind: "run_branch_cleanup",
      id: "branches-1",
      branches: ["work/cleanup-one", "work/cleanup-two"],
      delete_remote: false,
      force_filesystem_delete: true,
    });

    await page.evaluate(() => {
      window.__branchCleanupFixture.emit({
        kind: "branch_cleanup_progress",
        id: "branches-1",
        branch: "work/cleanup-one",
        execution_branch: "work/cleanup-one",
        index: 1,
        total: 2,
        phase: "running",
        message: "Removing worktree for work/cleanup-one",
      });
    });
    await expect(modal).toContainText("Cleaning 1 of 2: work/cleanup-one");
    await expect(modal.locator(".branch-cleanup-progress-item.running"))
      .toContainText("work/cleanup-one");
    await expect(modal).toContainText("Removing worktree for work/cleanup-one");

    await page.evaluate(() => {
      window.__branchCleanupFixture.emit({
        kind: "branch_cleanup_progress",
        id: "branches-1",
        branch: "work/cleanup-one",
        execution_branch: "work/cleanup-one",
        index: 1,
        total: 2,
        phase: "done",
        message: "Deleted local branch",
      });
      window.__branchCleanupFixture.emit({
        kind: "branch_cleanup_progress",
        id: "branches-1",
        branch: "work/cleanup-two",
        execution_branch: "work/cleanup-two",
        index: 2,
        total: 2,
        phase: "running",
        message: "Removing worktree for work/cleanup-two",
      });
    });
    await expect(modal).toContainText("Cleaning 2 of 2: work/cleanup-two");
    await expect(modal.locator(".branch-cleanup-progress-item.done"))
      .toContainText("work/cleanup-one");

    await page.evaluate(() => {
      window.__branchCleanupFixture.emit({
        kind: "branch_cleanup_result",
        id: "branches-1",
        results: [
          {
            branch: "work/cleanup-one",
            execution_branch: "work/cleanup-one",
            status: "success",
            message: "Deleted local branch and worktree",
          },
          {
            branch: "work/cleanup-two",
            execution_branch: "work/cleanup-two",
            status: "success",
            message: "Deleted local branch and worktree",
          },
        ],
      });
    });
    await expect(modal).toContainText("Cleanup result");
    await expect(modal).toContainText("success 2 · partial 0 · failed 0");
    await expect(modal.getByRole("button", { name: "Close" })).toBeVisible();
  });
});

async function installBranchCleanupBackend(page) {
  await page.addInitScript(() => {
    const branchEntries = [
      cleanupBranch({
        name: "work/cleanup-one",
        availability: "safe",
        mergeTarget: { kind: "develop", reference: "origin/develop" },
        risks: [],
      }),
      cleanupBranch({
        name: "work/cleanup-two",
        availability: "risky",
        mergeTarget: { kind: "develop", reference: "origin/develop" },
        risks: ["unmerged"],
      }),
      cleanupBranch({
        name: "develop",
        availability: "blocked",
        isHead: true,
        mergeTarget: null,
        blockedReason: "current_head",
        risks: [],
      }),
    ];

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "branches-1",
                  title: "Branches",
                  preset: "branches",
                  geometry: { x: 96, y: 96, width: 760, height: 620 },
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

    const recordedCalls = [];
    window.__branchCleanupCalls = recordedCalls;

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        window.__branchCleanupFixture = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw) {
        const message = JSON.parse(raw);
        recordedCalls.push(message);
        switch (message.kind) {
          case "frontend_ready":
            this.emit(workspaceState);
            break;
          case "load_branches":
            this.emit({
              kind: "branch_entries",
              id: message.id,
              phase: "hydrated",
              entries: branchEntries,
            });
            break;
          default:
            break;
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

    function cleanupBranch({
      name,
      availability,
      isHead = false,
      mergeTarget,
      blockedReason = null,
      risks,
    }) {
      return {
        name,
        scope: "local",
        is_head: isHead,
        upstream: null,
        ahead: 0,
        behind: 0,
        last_commit_date: "2026-05-20",
        cleanup_ready: true,
        cleanup: {
          availability,
          execution_branch: name,
          merge_target: mergeTarget,
          upstream: null,
          blocked_reason: blockedReason,
          risks,
        },
      };
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
