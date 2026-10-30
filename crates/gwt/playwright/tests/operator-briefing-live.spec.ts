/**
 * Issue #2796 — Mission Briefing must never block a live gwt client.
 *
 * The live contract is outcome-based: whether hydration, the briefing timer,
 * or prior session state wins the race, the overlay must become hidden and
 * stop intercepting pointer input.
 */
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, liveGwtRouteUrl } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test("Monitor strip coalesces receiver bursts and updates without animation frames", async ({ page }, info) => {
  test.skip(!BASE, "requires an isolated checkout server");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  await gotoLiveGwt(page, BASE);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const value = page.locator("#op-strip-issue-monitor-value");
  const batch = await page.evaluate(() => {
    const cell = document.getElementById("op-strip-issue-monitor")!;
    const value = document.getElementById("op-strip-issue-monitor-value")!;
    const originalRaf = window.requestAnimationFrame;
    const originalCancel = window.cancelAnimationFrame;
    const originalSet = cell.setAttribute;
    const frames = new Map<number, FrameRequestCallback>();
    let next = 0, renders = 0;
    const initial = value.textContent;
    window.requestAnimationFrame = callback => { frames.set(++next, callback); return next; };
    window.cancelAnimationFrame = frame => { frames.delete(frame); };
    cell.setAttribute = function (name, text) {
      if (name === "aria-label") renders++;
      originalSet.call(this, name, text);
    };
    try {
      for (let index = 0; index < 50; index++) window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: { kind: "issue_monitor_status", status: { enabled: true, state: "active", queue_len: index, active_count: 1, max_active_agents: 2 } },
      }));
      const before = { renders, unchanged: value.textContent === initial };
      for (const callback of [...frames.values()]) callback(performance.now());
      return { before, renders, text: value.textContent };
    } finally {
      window.requestAnimationFrame = originalRaf;
      window.cancelAnimationFrame = originalCancel;
      cell.setAttribute = originalSet;
    }
  });
  expect(batch).toEqual({ before: { renders: 0, unchanged: true }, renders: 1, text: "Run Q49 A1/2" });
  await page.evaluate(() => {
    (window as any).__monitorStripRaf = window.requestAnimationFrame;
    window.requestAnimationFrame = () => 0;
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
      kind: "issue_monitor_status", status: { enabled: true, state: "error", queue_len: 0, active_count: 0, last_error: "Controlled scan error" },
    } }));
  });
  try {
    await expect(value).toHaveText("Error Q0 A0/2", { timeout: 1000 });
    await expect(page.locator("#op-strip-issue-monitor")).toHaveClass(/op-status-strip__cell--alert/);
    await page.evaluate(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
      kind: "issue_monitor_status", status: { enabled: true, state: "idle", last_error: "" },
    } })));
    await expect(value).toHaveText("Idle Q0 A0/2", { timeout: 1000 });
    await expect(page.locator("#op-strip-issue-monitor")).not.toHaveClass(/op-status-strip__cell--alert/);
  } finally {
    await page.evaluate(() => {
      window.requestAnimationFrame = (window as any).__monitorStripRaf;
      delete (window as any).__monitorStripRaf;
      delete (document as any).visibilityState;
    });
  }
  await page.locator("#op-strip-issue-monitor").click();
  await expect(page.locator('.workspace-window[data-preset="issue"] .issue-bridge-root')).toBeVisible();
  await info.attach("monitor-strip", { body: await page.screenshot({ fullPage: true }), contentType: "image/png" });
  if (process.env.GWT_PLAYWRIGHT_CHECK_HOME) {
    await page.screenshot({ path: `${process.env.GWT_PLAYWRIGHT_CHECK_HOME}/monitor-strip-${info.project.name}.png`, fullPage: true });
  }
  expect(errors, "zero console/page errors").toEqual([]);
});

test.describe("Mission Briefing dismissal (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test("dismisses within the live startup deadline", async ({ page }) => {
    // Issue #4538: the briefing belongs to the Project app at `/p/<key>`.
    await page.goto(await liveGwtRouteUrl(page, BASE));

    const briefing = page.locator("#op-briefing");
    await expect(briefing).toBeHidden({ timeout: 3_000 });
  });

  test("does not intercept pointer input after hydration", async ({ page }) => {
    // Issue #4538: the briefing belongs to the Project app at `/p/<key>`.
    await page.goto(await liveGwtRouteUrl(page, BASE));

    const briefing = page.locator("#op-briefing");
    await expect(briefing).toBeHidden({ timeout: 3_000 });
    await expect(briefing).toHaveCSS("pointer-events", "auto");
    expect(await briefing.evaluate((element) => element.hidden)).toBe(true);
  });

  test("a previously shown briefing stays hidden", async ({ page }) => {
    await page.addInitScript(() => {
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    });
    // Issue #4538: the briefing belongs to the Project app at `/p/<key>`.
    await page.goto(await liveGwtRouteUrl(page, BASE));

    await expect(page.locator("#op-briefing")).toBeHidden({ timeout: 1_000 });
  });
});
