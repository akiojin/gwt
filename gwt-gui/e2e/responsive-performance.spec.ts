import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  openRecentProject,
  setMockCommandResponses,
  waitForMenuActionListener,
  emitTauriEvent,
  waitForInvokeCommand,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("app renders correctly at small viewport", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 });
  await page.goto("/");
  await openRecentProject(page);

  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();
});

test("app renders correctly at large viewport", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto("/");
  await openRecentProject(page);

  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();
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

  await expect(
    page.locator(".branch-name", { hasText: "main" }),
  ).toBeVisible();
  // Some branches from the list should be rendered
  await expect(
    page.locator(".branch-name", { hasText: "feature/branch-000" }),
  ).toBeVisible();
});

test("tab switching performance with two terminals", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await expect
    .poll(async () => page.locator(".tab .tab-dot.terminal").count())
    .toBe(2);

  const metrics = await page.evaluate(async () => {
    const TERMINAL_PREFIX = "terminal-";
    const samples: number[] = [];

    function percentile(values: number[], p: number): number {
      if (values.length === 0) return 0;
      const sorted = [...values].sort((a, b) => a - b);
      const idx = Math.max(0, Math.ceil((p / 100) * sorted.length) - 1);
      return sorted[idx];
    }

    function waitForTerminalVisible(tabId: string, timeoutMs = 1200) {
      const paneId = tabId.startsWith(TERMINAL_PREFIX)
        ? tabId.slice(TERMINAL_PREFIX.length)
        : "";
      return new Promise<void>((resolve, reject) => {
        const start = performance.now();
        const tick = () => {
          const active = document.querySelector<HTMLElement>(
            ".terminal-wrapper.active .terminal-container",
          );
          if (active?.dataset.paneId === paneId) {
            resolve();
            return;
          }
          if (performance.now() - start > timeoutMs) {
            reject(new Error(`Timed out waiting for active pane: ${paneId}`));
            return;
          }
          requestAnimationFrame(tick);
        };
        tick();
      });
    }

    const terminalTabIds = Array.from(
      document.querySelectorAll<HTMLElement>(".tab[data-tab-id]"),
    )
      .map((tab) => tab.dataset.tabId ?? "")
      .filter((id) => id.startsWith(TERMINAL_PREFIX));

    if (terminalTabIds.length < 2) {
      throw new Error("Not enough terminal tabs");
    }

    const [tabA, tabB] = terminalTabIds;
    const rounds = 10;

    for (let i = 0; i < rounds; i++) {
      const target = i % 2 === 0 ? tabA : tabB;
      const targetEl = document.querySelector<HTMLElement>(
        `.tab[data-tab-id="${target}"]`,
      );
      if (!targetEl) throw new Error(`Tab not found: ${target}`);

      const start = performance.now();
      targetEl.click();
      await waitForTerminalVisible(target);
      samples.push(performance.now() - start);
    }

    return {
      average:
        samples.reduce((sum, v) => sum + v, 0) / samples.length,
      p95: percentile(samples, 95),
      max: Math.max(...samples),
    };
  });

  expect(metrics.average).toBeLessThan(200);
  expect(metrics.p95).toBeLessThan(300);
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

  // Click a branch and measure response time
  const start = Date.now();
  await page
    .locator(".branch-item")
    .filter({ hasText: "feature/perf-test-15" })
    .click();
  await expect(page.locator(".branch-detail h2")).toContainText(
    "feature/perf-test-15",
  );
  const duration = Date.now() - start;

  expect(duration).toBeLessThan(3000);
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

test("viewport resize does not break layout", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);

  // Resize to a smaller viewport
  await page.setViewportSize({ width: 640, height: 480 });
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  // Resize back to normal
  await page.setViewportSize({ width: 1280, height: 720 });
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();
});
