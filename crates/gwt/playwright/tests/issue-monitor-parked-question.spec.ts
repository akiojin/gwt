/* Issue #3478 — real-browser E2E for the parked-question surface (AC-9).
 *
 * The linkedom unit tests prove the DOM shape but have no layout engine, so
 * they cannot show that a human can actually READ the question that stalled
 * the autonomous queue. This mounts `issue-monitor-surface.js` in a real
 * chromium page via the embedded frontend routes (no live gwt, no real
 * agents), injects a deterministic parked-question status, and asserts what
 * only a real browser can prove: the question block is visible, laid out
 * inside its row, and legible rather than clipped to zero height.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const PARKED_QUESTION = "Should the legacy migration table be dropped?";

test.describe("Issue Monitor parked question (real browser)", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      class NoopSocket {
        constructor() {}
        send() {}
        close() {}
        addEventListener() {}
        removeEventListener() {}
      }
      // @ts-ignore
      window.WebSocket = NoopSocket;
    });
    await installEmbeddedRoutes(page);
    await page.goto(APP_URL);

    await page.evaluate(async (question) => {
      const mod = await import("/issue-monitor-surface.js");
      const host = document.createElement("div");
      host.id = "im-harness";
      document.body.replaceChildren(host);
      const surface = (mod as any).createIssueMonitorSurface({
        document,
        send: () => {},
        focusWindow: () => {},
      });
      surface.mount(host);
      surface.applyStatus({
        enabled: true,
        state: "idle",
        max_active_agents: 1,
        autonomous_mode: true,
        autonomous_issues: [
          {
            issue_number: 3164,
            phase: "needs_human",
            attempts: 1,
            needs_human: true,
            needs_human_reason:
              "Autonomous execution reached a question that needs human judgment",
            pending_question: {
              handoff_id: "handoff-3164",
              question,
              options: ["Drop it", "Keep it"],
              reason_code: "irreversible_action",
              session_id: "session-abc",
              provider: "claude-code",
              created_at: "2026-08-06T05:00:00Z",
              resumable: true,
            },
          },
          { issue_number: 3165, phase: "implementing", attempts: 1, needs_human: false },
        ],
      });
      const item = (n: number, state: string) => ({
        issue: {
          number: n,
          title: `Issue ${n}`,
          labels: ["auto-merge"],
          state: "open",
          body: "b",
          url: null,
        },
        state,
        claim_id: null,
        blocked_by_owner: null,
        claim_expires_at: null,
        launched_window_id: null,
        error_message: null,
        launch_plan: {
          branch_name: `work/issue-${n}`,
          linked_issue_kind: "issue",
          prompt: `$gwt-execute #${n}`,
        },
      });
      surface.applyInbox([item(3164, "needs_human"), item(3165, "launched")]);
    }, PARKED_QUESTION);
  });

  test("the parked question is visible and legible on its own row", async ({ page }) => {
    const rows = page.locator("#im-harness .issue-monitor-card__item");
    await expect(rows).toHaveCount(2);

    const question = rows.nth(0).locator(".issue-monitor-card__autonomous-question");
    await expect(question).toBeVisible();
    await expect(question).toContainText(PARKED_QUESTION);
    await expect(question).toContainText("Drop it");
    await expect(question).toContainText("irreversible_action");
    await expect(question).toContainText("session-abc");
    await expect(question).toContainText("Resumable");
    await expect(question).toHaveAttribute("data-handoff-id", "handoff-3164");
    await expect(question).toHaveAttribute("data-resumable", "true");

    // Real layout: the block occupies readable height inside its row and is
    // not collapsed or clipped away.
    const box = await question.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.height).toBeGreaterThanOrEqual(12);
    const rowBox = await rows.nth(0).boundingBox();
    expect(rowBox).not.toBeNull();
    expect(box!.y).toBeGreaterThanOrEqual(rowBox!.y - 1);
    expect(box!.y + box!.height).toBeLessThanOrEqual(rowBox!.y + rowBox!.height + 1);
  });

  test("an issue that is not parked on a question renders no question block", async ({ page }) => {
    const rows = page.locator("#im-harness .issue-monitor-card__item");
    await expect(
      rows.nth(1).locator(".issue-monitor-card__autonomous-question"),
    ).toHaveCount(0);
  });
});
