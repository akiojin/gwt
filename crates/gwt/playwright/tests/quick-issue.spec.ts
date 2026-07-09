/* SPEC-3214 T-050 (FR-004/FR-005) — real-browser E2E for the Quick issue
 * toolbar on the Issue Monitor card.
 *
 * Mounts `issue-monitor-surface.js` in a REAL chromium page via the embedded
 * frontend routes (no live gwt, no real agents) and asserts what only a real
 * browser can prove: real typing, a real Enter keydown, and real clicks.
 *
 *   - typing a title + Enter registers a plain `investigation` issue
 *     (`quick_register_issue` with launch:false) and clears the input
 *   - ⚡ Register & Launch hands the issue to the monitor pipeline
 *     (launch:true)
 *   - an empty title never sends an event
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Issue Monitor Quick issue toolbar (real browser)", () => {
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

    await page.evaluate(async () => {
      const mod = await import("/issue-monitor-surface.js");
      (window as any).__sent = [];
      const host = document.createElement("div");
      host.id = "im-harness";
      document.body.replaceChildren(host);
      const surface = (mod as any).createIssueMonitorSurface({
        document,
        send: (event: unknown) => (window as any).__sent.push(event),
        focusWindow: () => {},
      });
      surface.mount(host);
      surface.applyStatus({ enabled: false, state: "disabled", max_active_agents: 1 });
      surface.applyInbox([]);
    });
  });

  test("Enter registers the issue without launch and clears the input", async ({
    page,
  }) => {
    const input = page.locator("#im-harness .issue-monitor-card__quick-issue-input");
    await expect(input).toBeVisible();
    await expect(input).toHaveAttribute("placeholder", /Quick issue title/);

    await input.click();
    await input.fill("investigate flaky shutdown ordering");
    await input.press("Enter");

    expect(
      await page.evaluate(() =>
        // The surface issues a list_issue_monitor refresh on mount; only the
        // Quick issue sends are under test here.
        (window as any).__sent.filter(
          (event: any) => event.kind !== "list_issue_monitor",
        ),
      ),
    ).toEqual([
      {
        kind: "quick_register_issue",
        title: "investigate flaky shutdown ordering",
        launch: false,
      },
    ]);
    await expect(input).toHaveValue("");
  });

  test("Register & Launch hands the issue to the monitor pipeline", async ({
    page,
  }) => {
    const input = page.locator("#im-harness .issue-monitor-card__quick-issue-input");
    const launch = page.locator(
      "#im-harness .issue-monitor-card__quick-issue-launch",
    );
    await expect(launch).toHaveText(/Register & Launch/);

    await input.fill("ship the intake worktree GC");
    await launch.click();

    expect(
      await page.evaluate(() =>
        // The surface issues a list_issue_monitor refresh on mount; only the
        // Quick issue sends are under test here.
        (window as any).__sent.filter(
          (event: any) => event.kind !== "list_issue_monitor",
        ),
      ),
    ).toEqual([
      {
        kind: "quick_register_issue",
        title: "ship the intake worktree GC",
        launch: true,
      },
    ]);
    await expect(input).toHaveValue("");
  });

  test("an empty title never sends an event", async ({ page }) => {
    const input = page.locator("#im-harness .issue-monitor-card__quick-issue-input");
    await input.click();
    await input.press("Enter");
    await page
      .locator("#im-harness .issue-monitor-card__quick-issue-launch")
      .click();

    expect(
      await page.evaluate(() =>
        // The surface issues a list_issue_monitor refresh on mount; only the
        // Quick issue sends are under test here.
        (window as any).__sent.filter(
          (event: any) => event.kind !== "list_issue_monitor",
        ),
      ),
    ).toEqual([]);
  });
});
