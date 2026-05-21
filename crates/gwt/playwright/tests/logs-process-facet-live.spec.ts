/**
 * SPEC-2019 Amendment 2026-05-20 — Logs window Process kind facet live E2E.
 *
 * Drives a real `gwt --headless --no-open` instance through the Add Window
 * picker into the Logs preset and asserts:
 *
 *   - The Logs window mounts with the new `.logs-process-kind-select`
 *     chip exposing All / gh / git / docker / agent / runner options in
 *     the canonical order.
 *   - The chip preserves user-selected value across renders (i.e. the
 *     state model survives the chip change handler firing renderLogs()).
 *
 * Skips when `GWT_PLAYWRIGHT_BASE_URL` is unset (same contract as the
 * release-notes-live and console-window-live specs).
 */
import { test, expect } from "@playwright/test";
import { gotoLiveGwt, openLiveGwtProject } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

const KIND_OPTIONS = ["", "gh", "git", "docker", "agent", "runner"];

test.describe.serial("Logs window Process facet (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }) => {
    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      keepPresetModal: true,
    });
    await openLiveGwtProject(page);
    await expect(page.locator("#op-briefing")).toBeHidden();
    await expect(page.locator("#project-picker")).toBeHidden();
  });

  async function logsWindowIds(page) {
    return await page.evaluate(() =>
      Array.from(document.querySelectorAll(".workspace-window"))
        .filter((node) => node.querySelector(".logs-process-kind-select"))
        .map((node) => (node as HTMLElement).dataset.id)
        .filter(Boolean),
    );
  }

  async function waitForNewLogsWindow(page, beforeIds) {
    const id = await page
      .waitForFunction(
        ({ beforeIds }) => {
          const seen = new Set(beforeIds);
          const node = Array.from(document.querySelectorAll(".workspace-window"))
            .find((candidate) =>
              candidate.querySelector(".logs-process-kind-select") &&
              !seen.has((candidate as HTMLElement).dataset.id || ""),
            );
          return node ? (node as HTMLElement).dataset.id || "" : "";
        },
        { beforeIds },
      )
      .then((handle) => handle.jsonValue());
    return page.locator(`.workspace-window[data-id="${id}"]`);
  }

  async function openLogsWindow(page) {
    const beforeIds = await logsWindowIds(page);
    await page.evaluate(() => {
      const modal = document.getElementById("preset-modal");
      if (modal) {
        modal.setAttribute("aria-hidden", "false");
        modal.classList.add("open");
      }
    });
    const presetModal = page.locator("#preset-modal");
    await expect(presetModal).toBeVisible();
    const logsButton = presetModal.locator("[data-preset='logs']");
    await expect(logsButton).toBeVisible();
    await logsButton.click();
    return await waitForNewLogsWindow(page, beforeIds);
  }

  test("Logs window exposes the Process kind chip with canonical options", async ({
    page,
  }) => {
    const windowRoot = await openLogsWindow(page);

    // The Logs window scaffold lives inside `app.js` and is rendered once
    // the preset click flows through `create_window` -> workspace state.
    const chip = windowRoot.locator(".logs-process-kind-select");
    await expect(chip).toBeVisible();

    const optionValues = await chip.evaluate((select) =>
      Array.from((select as HTMLSelectElement).options).map((option) => option.value),
    );
    expect(optionValues).toEqual(KIND_OPTIONS);
  });

  test("Selecting a kind preserves the value through renderLogs", async ({
    page,
  }) => {
    const windowRoot = await openLogsWindow(page);

    const chip = windowRoot.locator(".logs-process-kind-select");
    await chip.selectOption("docker");
    await expect(chip).toHaveValue("docker");

    // Toggling another filter forces a render; the chip should keep
    // its value because the controller now syncs from `state.processKind`.
    const severity = windowRoot.locator(".logs-severity-select");
    await severity.selectOption("warn");
    await expect(chip).toHaveValue("docker");
  });
});
