/* SPEC-2356 Phase 11 — Text-first live opacity guard.
 *
 * Drives a real `gwt serve` instance because parent opacity regressions are
 * only obvious once the Canvas grid, window chrome, and terminal renderer are
 * composited by a headed browser.
 */
import { expect, test } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Text-first UI live readability", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("workspace windows remain fully opaque across Light/Dark themes", async ({
    page,
  }, testInfo) => {
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "workspace_state",
            workspace: {
              app_version: "playwright",
              tabs: [
                {
                  id: "text-first-tab",
                  title: "Text First Fixture",
                  project_root: "/fixture/text-first",
                  kind: "git",
                  workspace: {
                    viewport: { x: 0, y: 0, zoom: 1 },
                    windows: [
                      {
                        id: "text-first-idle-agent",
                        title: "Claude Code",
                        preset: "agent",
                        geometry: { x: 80, y: 80, width: 760, height: 420 },
                        z_index: 11,
                        status: "idle",
                        minimized: false,
                        maximized: false,
                        pre_maximize_geometry: null,
                        persist: true,
                        purpose_title: null,
                        dynamic_title: "Claude Code",
                        dynamic_title_detail: "Readable idle transcript",
                        agent_id: "agent-idle",
                        agent_color: "yellow",
                        tab_group_id: null,
                        tab_group_active: false,
                      },
                      {
                        id: "text-first-not-started-agent",
                        title: "Codex",
                        preset: "codex",
                        geometry: { x: 880, y: 120, width: 520, height: 360 },
                        z_index: 12,
                        status: "not_started",
                        minimized: false,
                        maximized: false,
                        pre_maximize_geometry: null,
                        persist: true,
                        purpose_title: null,
                        dynamic_title: "Codex",
                        dynamic_title_detail: "Readable not-started transcript",
                        agent_id: "agent-not-started",
                        agent_color: "cyan",
                        tab_group_id: null,
                        tab_group_active: false,
                      },
                    ],
                  },
                },
              ],
              active_tab_id: "text-first-tab",
              recent_projects: [],
            },
          },
        }),
      );
    });

    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    for (const id of ["text-first-idle-agent", "text-first-not-started-agent"]) {
      const workspaceWindow = page.locator(`.workspace-window[data-id="${id}"]`);
      await expect(workspaceWindow).toBeVisible();
      await expect(workspaceWindow).toHaveCSS("opacity", "1");
      await expect(workspaceWindow.locator(".window-body")).not.toHaveCSS(
        "background-color",
        "rgba(0, 0, 0, 0)",
      );
    }

    await page.screenshot({
      path: testInfo.outputPath(`text-first-${theme}-window.png`),
      fullPage: true,
    });
  });
});
