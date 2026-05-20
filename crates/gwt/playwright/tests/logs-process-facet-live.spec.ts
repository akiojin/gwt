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

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

const KIND_OPTIONS = ["", "gh", "git", "docker", "agent", "runner"];

test.describe.serial("Logs window Process facet (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      try {
        window.sessionStorage.setItem("gwt:ui:briefing", "1");
      } catch {
        /* no-op */
      }
    });
    await page.goto(BASE);
    await page.addStyleTag({
      content: `
        #op-briefing { display: none !important; pointer-events: none !important; }
        .modal-backdrop:not(#preset-modal) { display: none !important; pointer-events: none !important; }
      `,
    });
    await page.evaluate(() => {
      const overlay = document.getElementById("op-briefing");
      if (overlay) overlay.hidden = true;
    });
    await expect(page.locator("#op-briefing")).toBeHidden();
  });

  async function openLogsWindow(page) {
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
  }

  test("Logs window exposes the Process kind chip with canonical options", async ({
    page,
  }) => {
    await openLogsWindow(page);

    // The Logs window scaffold lives inside `app.js` and is rendered once
    // the preset click flows through `create_window` -> workspace state.
    const chip = page.locator(".logs-process-kind-select").last();
    await expect(chip).toBeVisible();

    const optionValues = await chip.evaluate((select) =>
      Array.from((select as HTMLSelectElement).options).map((option) => option.value),
    );
    expect(optionValues).toEqual(KIND_OPTIONS);
  });

  test("Selecting a kind preserves the value through renderLogs", async ({
    page,
  }) => {
    await openLogsWindow(page);

    const chip = page.locator(".logs-process-kind-select").last();
    await chip.selectOption("docker");
    await expect(chip).toHaveValue("docker");

    // Toggling another filter forces a render; the chip should keep
    // its value because the controller now syncs from `state.processKind`.
    const severity = page.locator(".logs-severity-select").last();
    await severity.selectOption("warn");
    await expect(chip).toHaveValue("docker");
  });
});
