import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  captureUxSnapshot,
  openRecentProject,
  setMockCommandResponses,
  waitForInvokeCommand,
} from "./support/helpers";

async function countInvokeCommands(
  page: import("@playwright/test").Page,
  commands: string[],
): Promise<number> {
  return page.evaluate((targetCommands) => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
    };
    return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).filter((entry) =>
      targetCommands.includes(entry.cmd),
    ).length;
  }, commands);
}

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "gwt.projectTabs.v2",
      JSON.stringify({
        version: 2,
        byProjectPath: {
          "/tmp/gwt-playwright": {
            tabs: [
              { type: "agentCanvas", id: "agentCanvas", label: "Agent Canvas" },
              { type: "branchBrowser", id: "branchBrowser", label: "Branch Browser" },
            ],
            activeTabId: "agentCanvas",
          },
        },
      }),
    );
  });
});

test("app renders correctly at small viewport", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await page.goto("/");
  await openRecentProject(page);

  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();
});

test("app renders correctly at large viewport", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto("/");
  await openRecentProject(page);

  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();
});

test("large branch list renders without timeout", async ({ page }) => {
  const manyBranches = Array.from({ length: 50 }, (_, i) => ({
    ...branchFeature,
    name: `feature/branch-${String(i).padStart(3, "0")}`,
    is_current: false,
    commit_timestamp: 1_700_000_000 + i,
  }));

  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, ...manyBranches],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);
  await page.getByRole("tab", { name: "Branch Browser" }).click();
  await expect(page.getByTestId("branch-browser-panel")).toBeVisible();

  await expect(
    page.locator(".branch-name", { hasText: "main" }),
  ).toBeVisible();
  // Some branches from the list should be rendered
  await expect(
    page.locator(".branch-name", { hasText: "feature/branch-000" }),
  ).toBeVisible();
});

test("top-level tab switching stays responsive", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();
  await page.getByRole("tab", { name: "Branch Browser" }).click();
  await expect(page.getByTestId("branch-browser-panel")).toBeVisible();
  await page.getByRole("tab", { name: "Agent Canvas" }).click();
  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();

  const metrics = await page.evaluate(async () => {
    const samples: number[] = [];

    function percentile(values: number[], p: number): number {
      if (values.length === 0) return 0;
      const sorted = [...values].sort((a, b) => a - b);
      const idx = Math.max(0, Math.ceil((p / 100) * sorted.length) - 1);
      return sorted[idx];
    }

    function waitForShellSurface(tabId: string, timeoutMs = 1200) {
      return new Promise<void>((resolve, reject) => {
        const start = performance.now();
        const tick = () => {
          if (
            tabId === "agentCanvas" &&
            document.querySelector('[data-testid="agent-canvas-board"]')
          ) {
            resolve();
            return;
          }
          if (
            tabId === "branchBrowser" &&
            document.querySelector('[data-testid="branch-browser-panel"]')
          ) {
            resolve();
            return;
          }
          if (performance.now() - start > timeoutMs) {
            reject(new Error(`Timed out waiting for shell surface: ${tabId}`));
            return;
          }
          requestAnimationFrame(tick);
        };
        tick();
      });
    }

    const shellTabIds = Array.from(
      document.querySelectorAll<HTMLElement>(".tab[data-tab-id]"),
    )
      .map((tab) => tab.dataset.tabId ?? "")
      .filter((id) => id === "agentCanvas" || id === "branchBrowser");

    if (shellTabIds.length < 2) {
      throw new Error("Not enough shell tabs");
    }

    const [tabA, tabB] = shellTabIds;
    const rounds = 10;

    for (let i = 0; i < rounds; i++) {
      const target = i % 2 === 0 ? tabA : tabB;
      const targetEl = document.querySelector<HTMLElement>(
        `.tab[data-tab-id="${target}"]`,
      );
      if (!targetEl) throw new Error(`Tab not found: ${target}`);

      const start = performance.now();
      targetEl.click();
      await waitForShellSurface(target);
      samples.push(performance.now() - start);
    }

    return {
      average:
        samples.reduce((sum, v) => sum + v, 0) / samples.length,
      p95: percentile(samples, 95),
      max: Math.max(...samples),
    };
  });

  expect(metrics.average).toBeLessThan(150);
  expect(metrics.p95).toBeLessThan(250);
});

test("branch selection is responsive with many branches", async ({
  page,
}) => {
  const manyBranches = Array.from({ length: 30 }, (_, i) => ({
    ...branchFeature,
    name: `feature/perf-test-${i}`,
    is_current: false,
    commit_timestamp: 1_700_000_000 + i,
  }));

  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, ...manyBranches],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);
  await page.getByRole("tab", { name: "Branch Browser" }).click();
  await expect(page.getByTestId("branch-browser-panel")).toBeVisible();

  // Click a branch and measure response time
  const start = Date.now();
  await page
    .locator(".branch-row")
    .filter({ hasText: "feature/perf-test-15" })
    .click();
  await expect(page.getByTestId("branch-browser-detail")).toContainText(
    "feature/perf-test-15",
  );
  const duration = Date.now() - start;

  expect(duration).toBeLessThan(1000);
});

test("Open Project page renders quickly", async ({ page }) => {
  const start = Date.now();
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  const duration = Date.now() - start;

  // Should render within 5 seconds including server startup variance
  expect(duration).toBeLessThan(5000);
});

test("project open stays interactive while issue cache warmup runs in background", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    sync_issue_cache: {
      __delayMs: 5_000,
      value: {
        mode: "diff",
        updatedCount: 0,
        deletedCount: 0,
        durationMs: 5_000,
      },
    },
  });

  const duration = await page.evaluate(async () => {
    const recentItem = document.querySelector<HTMLButtonElement>(
      "button.recent-item",
    );
    if (!recentItem) throw new Error("recent project button missing");

    const start = performance.now();
    recentItem.click();

    await new Promise<void>((resolve, reject) => {
      const timeoutMs = 5_000;
      const tick = () => {
        const heading = Array.from(document.querySelectorAll("h2")).find(
          (node) => node.textContent?.trim() === "Agent Canvas",
        );
        const board = document.querySelector('[data-testid="agent-canvas-board"]');
        if (heading && board) {
          resolve();
          return;
        }
        if (performance.now() - start > timeoutMs) {
          reject(new Error("Agent Canvas did not become interactive in time"));
          return;
        }
        requestAnimationFrame(tick);
      };
      tick();
    });

    return performance.now() - start;
  });

  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();
  expect(duration).toBeLessThan(1_000);
  await waitForInvokeCommand(page, "sync_issue_cache");
  await captureUxSnapshot(page, testInfo, "startup-interactive-cache-warmup");
});

test("viewport resize does not break layout", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  // Resize to a smaller viewport
  await page.setViewportSize({ width: 640, height: 480 });
  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();

  // Resize back to normal
  await page.setViewportSize({ width: 1280, height: 720 });
  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();
});

test("maximize-style resize stays interactive and does not refetch heavy data", async ({
  page,
}, testInfo) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    fetch_pr_status: {
      statuses: {},
      ghStatus: { available: true, authenticated: true },
    },
  });
  await openRecentProject(page);
  await expect(page.getByRole("heading", { name: "Agent Canvas" })).toBeVisible();

  await page.locator(".tab").filter({ hasText: "Branch Browser" }).click();
  await expect(page.getByTestId("branch-browser-panel")).toBeVisible();

  const trackedCommands = [
    "list_branch_inventory",
    "fetch_pr_status",
    "sync_issue_cache",
  ];
  const beforeCount = await countInvokeCommands(page, trackedCommands);

  const start = Date.now();
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.locator(".tab").filter({ hasText: "Branch Browser" }).click();
  await expect(page.getByTestId("branch-browser-panel")).toBeVisible();
  const duration = Date.now() - start;

  expect(duration).toBeLessThan(300);

  await page.waitForTimeout(200);
  const afterCount = await countInvokeCommands(page, trackedCommands);
  expect(afterCount).toBe(beforeCount);
  await captureUxSnapshot(page, testInfo, "maximize-layout-stability");
});
