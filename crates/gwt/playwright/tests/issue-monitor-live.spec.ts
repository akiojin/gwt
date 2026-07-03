import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, openLiveGwtProject, sendLiveGwtEvent } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const EXPECTED_MIN_COUNT = Number(process.env.GWT_PLAYWRIGHT_ISSUE_MONITOR_EXPECTED_COUNT ?? "2");

test.describe.serial("Issue Monitor live backend", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(
      testInfo.project.name !== "chromium-dark",
      "Issue Monitor live E2E runs once against the shared backend",
    );
    if (
      testInfo.title.includes("shows prompt and reacts immediately") ||
      testInfo.title.includes("keeps the Status Strip monitor cell")
    ) {
      await page.addInitScript(() => {
        const NativeWebSocket = window.WebSocket;
        window.WebSocket = class IssueMonitorFixtureWebSocket extends NativeWebSocket {
          constructor(url, protocols) {
            super(url, protocols);
            this.addEventListener(
              "message",
              (event) => {
                if (!(window as any).__gwtDropBackendIssueMonitorEvents) return;
                try {
                  const payload = JSON.parse(String(event.data));
                  if (
                    (payload?.kind === "workspace_state" &&
                      (window as any).__gwtDropBackendWorkspaceState) ||
                    payload?.kind === "issue_monitor_status" ||
                    payload?.kind === "issue_monitor_inbox" ||
                    payload?.kind === "issue_monitor_launch_failed" ||
                    payload?.kind === "issue_monitor_toast"
                  ) {
                    event.stopImmediatePropagation();
                  }
                } catch {
                  /* no-op */
                }
              },
              { capture: true },
            );
          }
        };
        Object.assign(window.WebSocket, NativeWebSocket);
        (window as any).__gwtDropBackendIssueMonitorEvents = true;
      });
    }
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, keepPresetModal: true });
    await clearBackendLaunchWizard(page);
    await openLiveGwtProject(page);
    await clearBackendLaunchWizard(page);
  });

  test("lists multiple issues and makes launch setting source visible", async ({ page }) => {
    await page.locator("#add-button").click();
    await page.locator('#preset-modal [data-preset="issue_monitor"]').click();

    const monitor = page
      .locator(".workspace-window.surface-issue-monitor:visible")
      .last()
      .locator(".issue-monitor-card");
    await expect(monitor).toBeVisible();
    const issueRows = monitor.locator(".issue-monitor-card__item");
    await expect
      .poll(() => issueRows.count(), { timeout: 15_000 })
      .toBeGreaterThanOrEqual(EXPECTED_MIN_COUNT);
    const issueCount = await issueRows.count();
    await expect(monitor.locator(".issue-monitor-card__detail")).toContainText(
      `Total ${issueCount}`,
    );
    await expect(monitor.locator(".issue-monitor-card__settings")).toContainText(
      /Agent settings (Saved|Last settings|Default)/,
    );
    await expect(monitor.getByRole("button", { name: "Configure" }).first()).toBeVisible();
    await expect(monitor.getByRole("button", { name: "Launch now" }).first()).toBeVisible();

    const rows = issueRows;
    const firstTitle = await rows
      .nth(0)
      .locator(".issue-monitor-card__issue-title-text")
      .innerText();
    const secondTitle = await rows
      .nth(1)
      .locator(".issue-monitor-card__issue-title-text")
      .innerText();
    await rows.nth(0).getByRole("button", { name: "Move down" }).click();
    await expect(rows.nth(0).locator(".issue-monitor-card__issue-title-text")).toHaveText(
      secondTitle,
    );
    await expect(rows.nth(1).locator(".issue-monitor-card__issue-title-text")).toHaveText(
      firstTitle,
    );

    await rows.nth(1).getByRole("button", { name: "Detail" }).click();
    const detail = page.locator("#issue-monitor-detail-modal [role='dialog']");
    await expect(detail).toBeVisible();
    await expect(detail).toContainText(firstTitle);
    await expect(detail).toContainText(/Labels|URL|Body|Details/);
    await detail.getByRole("button", { name: "Close issue detail" }).click();
    await expect(detail).toBeHidden();
  });

  test("prompts for launch settings before auto-run when none exist", async ({ page }) => {
    await page.locator("#add-button").click();
    await page.locator('#preset-modal [data-preset="issue_monitor"]').click();

    const monitor = page
      .locator(".workspace-window.surface-issue-monitor:visible")
      .last()
      .locator(".issue-monitor-card");
    await expect(monitor).toBeVisible();
    const issueRows = monitor.locator(".issue-monitor-card__item");
    await expect
      .poll(() => issueRows.count(), { timeout: 15_000 })
      .toBeGreaterThanOrEqual(EXPECTED_MIN_COUNT);
    await expect(monitor.locator(".issue-monitor-card__settings")).toContainText(
      "Agent settings Default",
    );
    const startButton = monitor.getByRole("button", { name: "Start" });
    const stopButton = monitor.getByRole("button", { name: "Stop" });
    if (await stopButton.isVisible().catch(() => false)) {
      await stopButton.click();
      await expect(startButton).toBeVisible();
    }

    const terminalWindowCount = await page
      .locator(".workspace-window.surface-terminal:visible")
      .count();

    await startButton.click();
    await expect(stopButton).toBeVisible();
    await expect
      .poll(async () => (await monitor.locator(".issue-monitor-card__state").innerText()).trim(), {
        timeout: 15_000,
      })
      .toBe("SETTINGS REQUIRED");
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await expect(wizard.locator("#wizard-title")).toHaveText("Configure Issue Monitor");
    await expect(monitor.locator(".issue-monitor-card__detail")).toContainText(/Active 0\/\d+/);
    await expect(monitor.locator(".issue-monitor-card__error")).toHaveText("");
    await expect(issueRows.first()).toHaveAttribute("data-state", "queued");
    await expect(issueRows.first().locator(".issue-monitor-card__state-badge")).toHaveText(
      "Queued",
    );
    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(
      terminalWindowCount,
    );
    await expect(page.locator("#op-strip-running")).toHaveText("0");
    await expect(page.locator("#op-strip-idle")).toHaveText("0");

    await page.locator("#wizard-cancel-button").click();
    await expect(wizard).toBeHidden();
    await stopButton.click();
    await expect(startButton).toBeVisible();
  });

  test("keeps the Status Strip monitor cell visible after closing the Issue Monitor window", async ({ page }) => {
    await page.locator("#add-button").click();
    await page.locator('#preset-modal [data-preset="issue_monitor"]').click();

    const monitorWindow = page.locator(".workspace-window.surface-issue-monitor:visible").last();
    await expect(monitorWindow).toBeVisible();

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_status",
            status: {
              enabled: true,
              state: "launching",
              queue_len: 3,
              active_count: 1,
              active_issue_number: 3165,
              max_active_agents: 2,
              total_candidates: 4,
              launch_profile_source: "last_settings",
              launch_profile_summary: "codex / gpt-5 / high / host",
            },
          },
        }),
      );
    });

    const stripCell = page.locator("#op-strip-issue-monitor");
    await expect(stripCell).toBeVisible();
    await expect(stripCell.locator("#op-strip-issue-monitor-value")).toHaveText("Run Q3 A1/2");
    await expect(stripCell).toHaveAttribute("aria-label", "Issue Monitor: Run Q3 A1/2");

    await monitorWindow.getByLabel("Close window").click();
    await page.locator('#window-close-confirm-modal [data-role="window-close-confirm"]').click();
    await expect(page.locator(".workspace-window.surface-issue-monitor:visible")).toHaveCount(0);

    await expect(stripCell).toBeVisible();
    await expect(stripCell.locator("#op-strip-issue-monitor-value")).toHaveText("Run Q3 A1/2");

    await stripCell.click();
    await expect(page.locator(".workspace-window.surface-issue-monitor:visible").last()).toBeVisible();
  });

  test("marks a launching row as failed when backend reports launch failure", async ({ page }) => {
    await page.locator("#add-button").click();
    await page.locator('#preset-modal [data-preset="issue_monitor"]').click();

    const monitor = page
      .locator(".workspace-window.surface-issue-monitor:visible")
      .last()
      .locator(".issue-monitor-card");
    await expect(monitor).toBeVisible();

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_status",
            status: {
              enabled: true,
              state: "launching",
              queue_len: 1,
              active_count: 1,
              active_issue_number: 3164,
              max_active_agents: 1,
              total_candidates: 2,
              launch_profile_source: "last_settings",
              launch_profile_summary: "codex / gpt-5 / high / host",
            },
          },
        }),
      );
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_inbox",
            items: [
              {
                issue: {
                  number: 3164,
                  title: "Launch error fixture",
                  labels: ["gwt-spec"],
                  state: "open",
                  body: "Launch failure should be visible in the queue.",
                  url: "https://github.com/akiojin/gwt/issues/3164",
                },
                state: "launching",
                claim_id: null,
                blocked_by_owner: null,
                claim_expires_at: null,
                launched_window_id: "window-3164",
              },
              {
                issue: {
                  number: 3165,
                  title: "Queued fixture",
                  labels: ["gwt-spec"],
                  state: "open",
                  body: "Queued work remains visible.",
                  url: "https://github.com/akiojin/gwt/issues/3165",
                },
                state: "queued",
                claim_id: null,
                blocked_by_owner: null,
                claim_expires_at: null,
                launched_window_id: null,
              },
            ],
          },
        }),
      );
    });

    const rows = monitor.locator(".issue-monitor-card__item");
    await expect(rows).toHaveCount(2);
    await expect(rows.first().locator(".issue-monitor-card__state-badge")).toHaveText(
      "Launching",
    );

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_launch_failed",
            issue_number: 3164,
            message: "remote branch failed",
          },
        }),
      );
    });

    const badge = rows.first().locator(".issue-monitor-card__state-badge");
    await expect(badge).toHaveText("Launch failed");
    await expect(badge).toHaveAttribute("data-state", "launch_failed");
    await expect(monitor.locator(".issue-monitor-card__state")).toHaveText("Error");
    await expect(monitor.locator(".issue-monitor-card__detail")).toContainText("Active 0/1");
    await expect(monitor.locator(".issue-monitor-card__error")).toContainText(
      "issue #3164: remote branch failed",
    );
  });

  test("marks a launched row as agent failed when runtime reports an error", async ({ page }) => {
    await page.locator("#add-button").click();
    await page.locator('#preset-modal [data-preset="issue_monitor"]').click();

    const monitor = page
      .locator(".workspace-window.surface-issue-monitor:visible")
      .last()
      .locator(".issue-monitor-card");
    await expect(monitor).toBeVisible();

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_status",
            status: {
              enabled: true,
              state: "error",
              queue_len: 1,
              active_count: 0,
              active_issue_number: null,
              max_active_agents: 1,
              total_candidates: 1,
              launch_profile_source: "last_settings",
              launch_profile_summary: "codex / gpt-5 / high / host",
              last_error: "issue #3164: Stop-block hit an error",
            },
          },
        }),
      );
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_inbox",
            items: [
              {
                issue: {
                  number: 3164,
                  title: "Runtime error fixture",
                  labels: ["gwt-spec"],
                  state: "open",
                  body: "Runtime failure should be visible in the queue.",
                  url: "https://github.com/akiojin/gwt/issues/3164",
                },
                state: "agent_failed",
                claim_id: null,
                blocked_by_owner: null,
                claim_expires_at: null,
                launched_window_id: "window-3164",
                error_message: "Stop-block hit an error",
              },
            ],
          },
        }),
      );
    });

    const row = monitor
      .locator(".issue-monitor-card__item")
      .filter({ hasText: "Runtime error fixture" })
      .first();
    await expect(row.locator(".issue-monitor-card__state-badge")).toHaveText("Agent failed");
    await expect(row.locator(".issue-monitor-card__state-badge")).toHaveAttribute(
      "data-state",
      "agent_failed",
    );
    await expect(row).toContainText("Error: Stop-block hit an error");
    await expect(monitor.locator(".issue-monitor-card__state")).toHaveText("Error");
    await expect(monitor.locator(".issue-monitor-card__detail")).toContainText("Active 0/1");
    await expect(monitor.locator(".issue-monitor-card__error")).toContainText(
      "issue #3164: Stop-block hit an error",
    );

    await row.getByRole("button", { name: "Detail" }).click();
    const detail = page.locator("#issue-monitor-detail-modal [role='dialog']");
    await expect(detail).toBeVisible();
    await expect(detail).toContainText("Stop-block hit an error");
    await detail.getByRole("button", { name: "Close issue detail" }).click();
    await expect(detail).toBeHidden();
  });

  test("shows prompt and reacts immediately when Start is clicked", async ({ page }) => {
    await page.evaluate(() => {
      (window as any).__issueMonitorStartEvents = [];
      const originalSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data: string | ArrayBufferLike | Blob | ArrayBufferView) {
        try {
          const payload = typeof data === "string" ? JSON.parse(data) : null;
          if (payload && payload.kind === "set_issue_monitor_enabled") {
            (window as any).__issueMonitorStartEvents.push(payload);
            return;
          }
        } catch {
          /* no-op */
        }
        return originalSend.call(this, data);
      };
    });

    await page.locator("#add-button").click();
    await page.locator('#preset-modal [data-preset="issue_monitor"]').click();

    const monitor = page
      .locator(".workspace-window.surface-issue-monitor:visible")
      .last()
      .locator(".issue-monitor-card");
    await expect(monitor).toBeVisible();

    await page.evaluate(() => {
      (window as any).__gwtDropBackendWorkspaceState = true;
    });

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_status",
            status: {
              enabled: false,
              state: "disabled",
              queue_len: 1,
              active_count: 0,
              active_issue_number: null,
              max_active_agents: 1,
              total_candidates: 1,
              launch_profile_source: "last_settings",
              launch_profile_summary: "codex / gpt-5 / high / host",
            },
          },
        }),
      );
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_inbox",
            items: [
              {
                issue: {
                  number: 3164,
                  title: "Prompt visibility fixture",
                  labels: ["gwt-spec"],
                  state: "open",
                  body: "The monitor should show the prompt before launch.",
                  url: "https://github.com/akiojin/gwt/issues/3164",
                },
                state: "queued",
                claim_id: null,
                blocked_by_owner: null,
                claim_expires_at: null,
                launched_window_id: null,
                launch_plan: {
                  branch_name: "feature/spec-3164",
                  linked_issue_kind: "spec",
                  prompt: "$gwt-build-spec SPEC-3164",
                },
              },
            ],
          },
        }),
      );
    });

    const row = monitor
      .locator(".issue-monitor-card__item")
      .filter({ hasText: "Prompt visibility fixture" })
      .first();
    await expect(row).toContainText("Prompt: $gwt-build-spec SPEC-3164");
    await expect(row).toContainText("Branch: feature/spec-3164");

    await row.getByRole("button", { name: "Detail" }).click();
    const detail = page.locator("#issue-monitor-detail-modal [role='dialog']");
    await expect(detail).toBeVisible();
    await expect(detail).toContainText("$gwt-build-spec SPEC-3164");
    await expect(detail).toContainText("feature/spec-3164");
    await detail.getByRole("button", { name: "Close issue detail" }).click();
    await expect(detail).toBeHidden();

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_status",
            status: {
              enabled: false,
              state: "disabled",
              queue_len: 1,
              active_count: 0,
              active_issue_number: null,
              max_active_agents: 1,
              total_candidates: 1,
              launch_profile_source: "last_settings",
              launch_profile_summary: "codex / gpt-5 / high / host",
            },
          },
        }),
      );
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_inbox",
            items: [
              {
                issue: {
                  number: 3164,
                  title: "Prompt visibility fixture",
                  labels: ["gwt-spec"],
                  state: "open",
                  body: "The monitor should show the prompt before launch.",
                  url: "https://github.com/akiojin/gwt/issues/3164",
                },
                state: "queued",
                claim_id: null,
                blocked_by_owner: null,
                claim_expires_at: null,
                launched_window_id: null,
                launch_plan: {
                  branch_name: "feature/spec-3164",
                  linked_issue_kind: "spec",
                  prompt: "$gwt-build-spec SPEC-3164",
                },
              },
            ],
          },
        }),
      );
    });

    const fixtureMonitor = page
      .locator(".issue-monitor-card")
      .filter({ hasText: "Prompt visibility fixture" })
      .last();
    await fixtureMonitor.getByRole("button", { name: "Start" }).evaluate((button) => {
      (button as HTMLButtonElement).click();
    });
    const currentMonitor = page
      .locator(".issue-monitor-card")
      .filter({ hasText: "Prompt visibility fixture" })
      .last();
    await expect(currentMonitor.getByRole("button", { name: "Stop" })).toBeVisible();
    await expect(currentMonitor.locator(".issue-monitor-card__state")).toHaveText("Starting");
    await expect
      .poll(() =>
        page.evaluate(() => JSON.stringify((window as any).__issueMonitorStartEvents ?? [])),
      )
      .toBe(JSON.stringify([{ kind: "set_issue_monitor_enabled", enabled: true }]));
  });

  test("autonomous events accumulate in a scrollable, dismissible side-toast stack", async ({
    page,
  }) => {
    // SPEC #3200 FR-034/FR-035: each Issue Monitor toast surfaces as a persistent
    // side notification; many accumulate in a scrollable stack (newest on top),
    // and each is dismissible.
    const region = page.locator(".autonomous-notifications");
    await expect(region).toHaveAttribute("role", "log");

    await page.evaluate(() => {
      for (let i = 1; i <= 6; i += 1) {
        window.dispatchEvent(
          new CustomEvent("__gwt_test_inject", {
            detail: {
              kind: "issue_monitor_toast",
              level: i % 2 === 0 ? "error" : "info",
              issue_number: 3200 + i,
              message: `autonomous event ${i}`,
            },
          }),
        );
      }
    });

    const items = region.locator(".autonomous-notifications__item");
    await expect(items).toHaveCount(6);
    // Newest on top.
    await expect(items.first()).toContainText("#3206");
    await expect(items.first()).toContainText("autonomous event 6");

    // The list is height-bounded and scrolls rather than growing unboundedly.
    const list = region.locator(".autonomous-notifications__list");
    const overflowY = await list.evaluate((el) => getComputedStyle(el).overflowY);
    expect(overflowY).toBe("auto");

    // Dismiss the newest notice.
    await items.first().getByRole("button", { name: "Dismiss notification" }).click();
    await expect(items).toHaveCount(5);
  });
});

async function clearBackendLaunchWizard(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  await expect(async () => {
    await sendLiveGwtEvent(page, {
      kind: "launch_wizard_action",
      action: { kind: "cancel" },
      bounds: null,
    });
    await sendLiveGwtEvent(page, { kind: "frontend_ready" });
    await expect(wizard).toBeHidden({ timeout: 1_000 });
  }).toPass({ timeout: 10_000 });
}
