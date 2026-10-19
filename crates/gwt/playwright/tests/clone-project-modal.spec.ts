import { test, expect } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test("shared clone state preserves drafts, query fences and clone actions", async ({ page }, testInfo) => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no isolated backend supplied");
  const errors: string[] = [];
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("pageerror", error => errors.push(error.message));
  // Record external actions locally; never search GitHub or clone a repository.
  await page.addInitScript(() => {
    (window as any).__cloneActions = [];
    const originalSend = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      if (typeof data === "string") {
        const message = JSON.parse(data);
        if (["github_repository_search", "clone_project_start"].includes(message.kind)) {
          (window as any).__cloneActions.push(message);
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
  const trigger = page.locator("#window-list-button");
  await trigger.focus();
  // Invoke the existing picker entry without changing the real project catalog.
  await page.locator("#picker-clone-project").dispatchEvent("click");
  const modal = page.locator("#clone-project-modal");
  const dialog = modal.locator(".modal-shell");
  await expect(modal).toHaveClass(/open/);
  await expect(dialog).toHaveAttribute("role", "dialog");
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  await expect(dialog).toBeFocused();
  const url = dialog.locator("#clone-project-url-input");
  await url.fill("https://github.com/fixture/project.git");
  await url.evaluate(input => { (window as any).__cloneInput = input; (input as HTMLInputElement).setSelectionRange(8, 14); });
  await page.keyboard.type("example");
  expect(await url.evaluate(input => input === (window as any).__cloneInput)).toBe(true);
  await expect(url).toBeFocused();
  expect(await url.evaluate(input => (input as HTMLInputElement).selectionStart)).toBe(15);
  await dialog.locator('[data-clone-mode="search"]').click();
  await dialog.locator("#clone-project-search-input").fill(" fixture ");
  await dialog.locator(".clone-project-search-row button").click();
  await expect(dialog).toHaveAttribute("aria-busy", "true");
  await inject({ kind: "github_repository_search_results", query: "stale", repositories: [{ full_name: "Stale" }] });
  await inject({ kind: "github_repository_search_error", query: "stale", message: "Stale failure" });
  await expect(dialog).toContainText("Searching repositories...");
  await inject({ kind: "github_repository_search_error", query: "fixture", message: "Fixture search failure" });
  await expect(dialog).toContainText("Fixture search failure");
  await dialog.locator(".clone-project-search-row button").click();
  const repository = { full_name: "fixture/project", url: "https://github.com/fixture/project.git", description: "Fixture repository" };
  await inject({ kind: "github_repository_search_results", query: "fixture", repositories: [repository] });
  await dialog.locator(".clone-project-result").click();
  await inject({ kind: "clone_project_parent_selected", path: "/fixture/destination" });
  const parent = dialog.locator("#clone-project-parent-input");
  await expect(parent).toHaveValue("/fixture/destination");
  await parent.fill(" /fixture/manual ");
  await dialog.locator("#clone-project-start").click();
  await expect(dialog).toHaveAttribute("aria-busy", "true");
  await expect(dialog.getByRole("button", { name: "Close", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveClass(/open/);
  await inject({ kind: "clone_project_progress", message: "Receiving fixture objects 42%" });
  await expect(dialog).toContainText("Receiving fixture objects 42%");
  await page.screenshot({ animations: "disabled", path: testInfo.outputPath(`clone-running-${theme}.png`) });
  await inject({ kind: "clone_project_error", message: "Fixture clone failure" });
  await expect(dialog).toHaveAttribute("aria-busy", "false");
  await expect(dialog).toContainText("Fixture clone failure");
  await expect(parent).toHaveValue(" /fixture/manual ");
  await expect(dialog.locator(".clone-project-result")).toHaveClass(/selected/);
  await page.screenshot({ animations: "disabled", path: testInfo.outputPath(`clone-error-${theme}.png`) });
  await dialog.locator("#clone-project-start").click();
  await inject({ kind: "clone_project_done" });
  await expect(modal).not.toHaveClass(/open/);
  await expect(modal).toHaveAttribute("aria-hidden", "true");
  await expect(trigger).toBeFocused();
  expect(await page.evaluate(() => (window as any).__cloneActions)).toEqual([
    { kind: "github_repository_search", query: "fixture" },
    { kind: "github_repository_search", query: "fixture" },
    { kind: "clone_project_start", url: repository.url, parent_path: "/fixture/manual" },
    { kind: "clone_project_start", url: repository.url, parent_path: "/fixture/manual" },
  ]);
  expect(errors).toEqual([]);
});
