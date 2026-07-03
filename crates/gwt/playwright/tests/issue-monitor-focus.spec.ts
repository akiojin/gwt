/* #3165 — real-browser E2E for the Issue Monitor Focus affordance and the
 * compact icon row.
 *
 * Unlike the linkedom unit tests (no layout engine, no real click/focus), this
 * mounts `issue-monitor-surface.js` in a REAL chromium page via the embedded
 * frontend routes, injects a deterministic inbox (no live gwt, no real agents),
 * and asserts what only a real browser can prove: actual `disabled` state,
 * real click → focusWindow, the glyph + hover tooltip, and — via real
 * getComputedStyle/offsetHeight — that the toolbar trio lines up at one height.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Issue Monitor Focus affordance (real browser)", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    // Neutralise the real socket so app.js boot does not interfere; we mount the
    // surface standalone (its CSS vars still come from the loaded stylesheets).
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
      (window as any).__focused = [];
      const host = document.createElement("div");
      host.id = "im-harness";
      document.body.replaceChildren(host);
      const surface = (mod as any).createIssueMonitorSurface({
        document,
        send: () => {},
        focusWindow: (id: string) => (window as any).__focused.push(id),
      });
      surface.mount(host);
      surface.applyStatus({ enabled: false, state: "disabled", max_active_agents: 1 });
      const item = (n: number, state: string, win: string | null) => ({
        issue: { number: n, title: `Issue ${n}`, labels: [], state: "open", body: "b", url: null },
        state,
        claim_id: null,
        blocked_by_owner: null,
        claim_expires_at: null,
        launched_window_id: win,
        error_message: state === "agent_failed" ? "boom" : null,
        launch_plan: {
          branch_name: `work/issue-${n}`,
          linked_issue_kind: "issue",
          prompt: `$gwt-fix-issue #${n}`,
        },
      });
      surface.applyInbox([
        item(42, "launched", "tab-1::agent-42"),
        item(43, "queued", null),
        item(45, "agent_failed", null),
      ]);
    });
  });

  test("Focus is always present, enabled only on the launched row, and focuses on click", async ({
    page,
  }) => {
    const rows = page.locator("#im-harness .issue-monitor-card__item");
    await expect(rows).toHaveCount(3);

    // Present on every row — never shown/hidden by state.
    await expect(
      page.locator('#im-harness .issue-monitor-card__item [data-action="focus-window"]'),
    ).toHaveCount(3);

    const launchedFocus = rows.nth(0).locator('[data-action="focus-window"]');
    await expect(launchedFocus).toBeEnabled();
    await expect(launchedFocus).toHaveText("◎");
    await expect(launchedFocus).toHaveAttribute("title", /Focus/);
    await expect(launchedFocus).toHaveAttribute("aria-label", /Focus/);
    await expect(rows.nth(1).locator('[data-action="focus-window"]')).toBeDisabled();
    await expect(rows.nth(2).locator('[data-action="focus-window"]')).toBeDisabled();

    await launchedFocus.click();
    expect(await page.evaluate(() => (window as any).__focused)).toEqual(["tab-1::agent-42"]);
  });

  test("row actions are compact icon buttons with hover tooltips", async ({ page }) => {
    const row = page.locator("#im-harness .issue-monitor-card__item").nth(1); // queued row
    for (const [action, glyph] of [
      ["open-detail", "ℹ"],
      ["configure-issue", "⚙"],
      ["launch-now", "▶"],
    ] as const) {
      const button = row.locator(`[data-action="${action}"]`);
      await expect(button).toHaveText(glyph);
      await expect(button).toHaveAttribute("title", /.+/);
      // Compact 28px icon button (real layout).
      const box = await button.boundingBox();
      expect(box && box.width).toBeLessThanOrEqual(34);
    }
  });

  test("toolbar Start and Autonomous render at one height (real layout)", async ({ page }) => {
    const dims = await page.evaluate(() => {
      const q = (s: string) => document.querySelector(`#im-harness ${s}`) as HTMLElement;
      return {
        toggle: q(".issue-monitor-card__toggle").offsetHeight,
        autonomous: q(".issue-monitor-card__autonomous").offsetHeight,
        num: q(".issue-monitor-card__number").offsetHeight,
      };
    });
    expect(dims.autonomous).toBe(dims.toggle); // the toolbar-sizing fix
    expect(Math.abs(dims.num - dims.toggle)).toBeLessThanOrEqual(2);
    expect(dims.toggle).toBeGreaterThanOrEqual(28);
    expect(dims.toggle).toBeLessThanOrEqual(34);
  });

  test("detail modal Focus is enabled for launched, disabled otherwise", async ({ page }) => {
    const rows = page.locator("#im-harness .issue-monitor-card__item");

    await rows.nth(0).locator('[data-action="open-detail"]').click();
    const launchedModalFocus = page.locator('.modal-footer [data-action="focus-window"]');
    await expect(launchedModalFocus).toBeEnabled();
    await launchedModalFocus.click();
    expect(await page.evaluate(() => (window as any).__focused)).toEqual(["tab-1::agent-42"]);
    await expect(page.locator("#issue-monitor-detail-modal")).toHaveCount(0);

    await rows.nth(1).locator('[data-action="open-detail"]').click();
    await expect(page.locator('.modal-footer [data-action="focus-window"]')).toBeDisabled();
  });
});
