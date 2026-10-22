import { test, expect } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";

test("shared migration state preserves tab fences and modal actions", async ({ page }, testInfo) => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "no isolated backend supplied");
  const errors: string[] = [];
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("pageerror", error => errors.push(error.message));
  // Exercise the real receiver and UI while recording the destructive action
  // locally: this test must never migrate the checkout or another repository.
  await page.addInitScript(() => {
    (window as any).__migrationActions = [];
    const originalSend = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      if (typeof data === "string") {
        const message = JSON.parse(data);
        if (["start_migration", "skip_migration", "quit_migration"].includes(message.kind)) {
          (window as any).__migrationActions.push(message);
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
  const detected = { kind: "migration_detected", tab_id: "migration-fixture", project_root: "/fixture/project", branch: "develop" };
  const modal = page.locator("#migration-modal");
  const dialog = modal.locator(".modal-shell");
  const trigger = page.locator("#window-list-button");
  await trigger.focus();
  await inject({ ...detected, has_locked: true });
  await expect(modal).toHaveClass(/open/);
  await expect(dialog).toHaveAttribute("role", "dialog");
  await expect(dialog).toHaveAttribute("aria-modal", "true");
  await expect(dialog).toBeFocused();
  await expect(dialog.getByRole("button")).toHaveCount(1);
  await expect(dialog.getByRole("button", { name: "Migrate", exact: true })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(modal).toHaveClass(/open/);
  await inject(detected);
  await dialog.getByRole("button", { name: "Migrate", exact: true }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__migrationActions)).toEqual([
    { kind: "start_migration", tab_id: detected.tab_id },
  ]);
  await expect(dialog).toHaveAttribute("aria-busy", "true");
  await expect(dialog).toContainText("Validating workspace");
  await inject({ kind: "migration_progress", tab_id: detected.tab_id, phase: "bareify", percent: 42 });
  await expect(dialog.locator("progress")).toHaveAttribute("value", "42");
  await expect(dialog).toContainText("Building bare repository");
  await dialog.hover();
  await dialog.focus();
  const burst = async (last: number, suspendRaf: boolean) => page.evaluate(async ({ tabId, last, suspendRaf }) => {
    const dialog = document.querySelector('#migration-modal .modal-shell')!;
    const before = dialog.innerHTML;
    const focused = document.activeElement;
    const originalRaf = window.requestAnimationFrame;
    if (suspendRaf) window.requestAnimationFrame = () => 0;
    let renders = 0;
    const started = performance.now();
    let timer: ReturnType<typeof setTimeout>;
    let observer: MutationObserver | undefined;
    try {
      const rendered = new Promise<void>((resolve, reject) => {
        observer = new MutationObserver(records => {
          renders += records.reduce((count, record) => count + [...record.addedNodes]
            .filter(node => node.nodeName === 'PROGRESS').length, 0);
          if (dialog.querySelector('progress')?.getAttribute('value') === String(last)) resolve();
        });
        observer.observe(dialog, { childList: true });
        timer = setTimeout(() => reject(new Error('migration frame did not render within one second')), 1000);
      });
      for (let percent = last - 49; percent <= last; percent += 1) {
        window.dispatchEvent(new CustomEvent('__gwt_test_inject', { detail: {
          kind: 'migration_progress', tab_id: tabId, phase: 'bareify', percent,
        } }));
      }
      window.dispatchEvent(new CustomEvent('__gwt_test_inject', { detail: {
        kind: 'migration_progress', tab_id: 'unrelated', phase: 'cleanup', percent: 99,
      } }));
      const unchangedBeforeFrame = dialog.innerHTML === before;
      await rendered;
      return { unchangedBeforeFrame, renders, elapsed: performance.now() - started,
        focused: document.activeElement === focused, hovered: dialog.matches(':hover') };
    } finally {
      clearTimeout(timer!);
      observer?.disconnect();
      window.requestAnimationFrame = originalRaf;
    }
  }, { tabId: detected.tab_id, last, suspendRaf });
  for (const suspendRaf of [false, true]) {
    const result = await burst(suspendRaf ? 75 : 50, suspendRaf);
    expect(result).toMatchObject({ unchangedBeforeFrame: true, renders: 1, focused: true, hovered: true });
    expect(result.elapsed).toBeLessThan(1000);
    await testInfo.attach(suspendRaf ? "migration-fallback" : "migration-frame", {
      body: JSON.stringify(result), contentType: "application/json",
    });
  }
  await inject({ kind: "migration_progress", tab_id: "other-tab", phase: "cleanup", percent: 90 });
  await inject({ kind: "migration_done", tab_id: "other-tab" });
  await inject({ kind: "migration_error", tab_id: "other-tab", message: "Unrelated failure" });
  await expect(dialog.locator("progress")).toHaveAttribute("value", "75");
  await page.screenshot({ animations: "disabled", path: testInfo.outputPath(`migration-running-${theme}.png`) });
  const failure = { kind: "migration_error", tab_id: detected.tab_id, phase: "worktrees", message: "Fixture failure", recovery: "rolled_back" };
  await inject(failure);
  await expect(dialog).toHaveAttribute("aria-busy", "false");
  await expect(dialog).toContainText("Setting up worktrees");
  await expect(dialog).toContainText("Fixture failure");
  await expect(dialog).toContainText("All partial changes were rolled back");
  await page.screenshot({ animations: "disabled", path: testInfo.outputPath(`migration-error-${theme}.png`) });
  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(modal).not.toHaveClass(/open/);
  await expect(modal).toHaveAttribute("aria-hidden", "true");
  await expect(trigger).toBeFocused();
  await inject({ kind: "migration_progress", tab_id: detected.tab_id, percent: 80 });
  await expect(modal).not.toHaveClass(/open/);
  await inject(detected);
  await expect(dialog.getByRole("button", { name: "Migrate", exact: true })).toBeEnabled();
  await inject(failure);
  await page.keyboard.press("Escape");
  await expect(modal).not.toHaveClass(/open/);
  await inject(detected);
  await inject({ kind: "migration_done", tab_id: detected.tab_id });
  await expect(modal).not.toHaveClass(/open/);
  expect(await page.evaluate(() => (window as any).__migrationActions)).toHaveLength(1);
  expect(errors).toEqual([]);
});
