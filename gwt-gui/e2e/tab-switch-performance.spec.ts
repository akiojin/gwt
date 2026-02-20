import { expect, test, type Page } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";

const defaultRecentProject = {
  path: "/tmp/gwt-playwright",
  lastOpened: "2026-02-13T00:00:00.000Z",
};

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

async function dismissSkillRegistrationScopeDialogIfPresent(page: Page) {
  const dialog = page.getByRole("dialog", {
    name: "Skill registration scope",
  });
  const visible = await dialog
    .isVisible({ timeout: 500 })
    .catch(() => false);
  if (!visible) {
    return;
  }

  await dialog.getByRole("button", { name: "Skip for now" }).click();
  await expect(dialog).toBeHidden();
}

async function openRecentProject(page: Page) {
  await dismissSkillRegistrationScopeDialogIfPresent(page);

  const recentItem = page.locator("button.recent-item").first();
  await expect(recentItem).toBeVisible();
  await recentItem.click();
}

test("measures terminal tab-switch latency (p95 budget)", async ({ page }) => {
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "new-terminal",
    });
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "new-terminal",
    });
  });

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
      throw new Error("Not enough terminal tabs for performance check");
    }

    const [tabA, tabB] = terminalTabIds;
    const rounds = 20;

    for (let i = 0; i < rounds; i++) {
      const target = i % 2 === 0 ? tabA : tabB;
      const targetEl = document.querySelector<HTMLElement>(
        `.tab[data-tab-id="${target}"]`,
      );
      if (!targetEl) {
        throw new Error(`Tab not found: ${target}`);
      }

      const start = performance.now();
      targetEl.click();
      await waitForTerminalVisible(target);
      samples.push(performance.now() - start);
    }

    const average =
      samples.reduce((sum, value) => sum + value, 0) / samples.length;
    const p95 = percentile(samples, 95);
    const max = Math.max(...samples);

    return { samples, average, p95, max };
  });

  expect(metrics.average).toBeLessThan(120);
  expect(metrics.p95).toBeLessThan(180);
  expect(metrics.max).toBeLessThan(300);
});
