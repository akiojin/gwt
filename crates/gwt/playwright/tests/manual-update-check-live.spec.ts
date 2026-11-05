import { test, expect } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test.describe("Header manual update check", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no GWT_PLAYWRIGHT_BASE_URL set");

  test("a real Check request reports its network failure and enables retry", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(error.message));
    // The isolated runner gives the GUI an unreachable local release API.
    // No update request or result is intercepted in this scenario.
    await gotoLiveGwt(page, BASE, { suppressProjectSurfaces: true });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const button = page.locator("#update-check-button");
    const checking = await button.evaluate(node => {
      const check = node as HTMLButtonElement;
      check.click();
      return {
        disabled: check.disabled,
        status: document.getElementById("update-check-status")?.textContent,
      };
    });
    expect(checking).toEqual({ disabled: true, status: "Checking…" });
    await expect(page.locator("#update-check-status")).toHaveText(/^Check failed: .+/);
    await expect(button).toBeEnabled();
    expect(errors).toEqual([]);
  });

  test("shows check outcomes and offers a failed newer release through the normal update dialog", async ({ page }, testInfo) => {
    const errors: string[] = [];
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    page.on("pageerror", error => errors.push(error.message));
    // The Rust regressions exercise real release lookup. Keep this browser
    // check deterministic and prevent its synthetic release from being applied.
    await page.addInitScript(() => {
      (window as any).__manualUpdateActions = [];
      const originalSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data) {
        if (typeof data === "string") {
          const payload = JSON.parse(data);
          if (["check_for_updates", "apply_update_start"].includes(payload.kind)) {
            (window as any).__manualUpdateActions.push(payload.kind);
            return;
          }
        }
        return originalSend.call(this, data);
      };
    });
    await gotoLiveGwt(page, BASE, { enableTestBridge: true, suppressProjectSurfaces: true });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const inject = (detail: Record<string, unknown>) => page.evaluate(payload => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: payload }));
    }, detail);
    const button = page.locator("#update-check-button");
    const status = page.locator("#update-check-status");
    await expect(button).toBeVisible();
    expect(await page.locator("#update-check").evaluate(node => node.previousElementSibling?.id)).toBe("app-version");
    await button.click();
    await expect(button).toBeDisabled();
    await expect(status).toHaveText("Checking…");
    await page.evaluate(() => (document.getElementById("update-check-button") as HTMLButtonElement).click());
    expect(await page.evaluate(() => (window as any).__manualUpdateActions)).toEqual(["check_for_updates"]);

    await inject({ kind: "update_check_result", checking: false, result: { state: "up_to_date" }, previous_failure: null });
    await expect(button).toBeEnabled();
    await expect(status).toHaveText("Up to date");
    await expect(page.locator("#update-check-detail")).toBeHidden();
    await button.click();
    await inject({
      kind: "update_check_result", checking: false,
      result: { state: "failed", message: "GitHub rate limit", retry_at: "2026-11-05T10:00:00Z" },
      previous_failure: null,
    });
    await expect(status).toContainText("GitHub rate limit");
    await expect(status).toContainText("Retry after 2026-11-05T10:00:00Z");
    await expect(page.locator("#update-check-detail")).toBeVisible();
    await expect(page.locator("#update-check-reason")).toHaveText("Check failed: GitHub rate limit — Retry after 2026-11-05T10:00:00Z");
    await expect(button).toBeEnabled();
    await inject({ kind: "update_check_result", checking: false, result: { state: "failed", message: "Network unavailable" }, previous_failure: null });
    await expect(status).toHaveText("Check failed: Network unavailable");

    await inject({
      kind: "update_check_result", checking: false,
      result: {
        state: "available", current: "9.149.0", latest: "9.150.0",
        release_url: "https://github.com/akiojin/gwt/releases/tag/v9.150.0",
        asset_url: "https://github.com/akiojin/gwt/releases/download/v9.150.0/gwt.dmg",
      },
      previous_failure: { outcome: "failure", to_version: "9.150.0", observed_version: "9.149.0" },
    });
    await expect(status).toHaveText("Update available: v9.150.0");
    await expect(page.locator("#update-check-detail")).toBeVisible();
    await expect(page.locator("#update-check-failure")).toHaveText("Previous update failed: expected v9.150.0, running v9.149.0.");
    await expect(page.locator("#update-check-retry")).toBeVisible();
    await expect(page.locator("#update-check-install")).toBeVisible();
    await expect(page.locator("#update-check-install")).toHaveAttribute("href", "https://github.com/akiojin/gwt/releases/tag/v9.150.0");
    await expect(page.locator("#update-modal")).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__manualUpdateActions)).toEqual(["check_for_updates", "check_for_updates"]);
    const checkBox = await button.boundingBox();
    const barBox = await page.locator(".project-bar").boundingBox();
    expect(checkBox).not.toBeNull();
    expect(barBox).not.toBeNull();
    expect(checkBox!.y).toBeGreaterThanOrEqual(0);
    expect(checkBox!.y + checkBox!.height).toBeLessThanOrEqual(barBox!.y + barBox!.height);
    const detailBox = await page.locator("#update-check-detail").boundingBox();
    expect(detailBox).not.toBeNull();
    expect(detailBox!.x).toBeGreaterThanOrEqual(0);
    expect(detailBox!.x + detailBox!.width).toBeLessThanOrEqual(page.viewportSize()!.width);
    await page.screenshot({ path: testInfo.outputPath(`manual-update-check-${theme}.png`) });

    await page.locator("#update-check-retry").click();
    const modal = page.locator("#update-modal");
    await expect(modal).toHaveAttribute("data-state", "downloading");
    await expect(modal.locator(".modal-shell")).toHaveAttribute("role", "dialog");
    await expect(modal.locator(".modal-shell")).toHaveAttribute("aria-modal", "true");
    expect(await page.evaluate(() => (window as any).__manualUpdateActions)).toEqual(["check_for_updates", "check_for_updates", "apply_update_start"]);
    await inject({ kind: "update_ready", version: "9.150.0", asset_path: "synthetic-update-payload" });
    await expect(modal).toHaveAttribute("data-state", "ready");
    await expect(modal.getByRole("button", { name: "Later", exact: true })).toBeVisible();
    await expect(modal.getByRole("button", { name: "Restart now", exact: true })).toBeVisible();
    expect(errors).toEqual([]);
  });
});
