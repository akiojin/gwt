import { expect, test } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";
import { useManagedBrowserSource } from "./_helpers/managed-browser-source";

// Apply the deadline before Playwright creates the headed browser/page fixtures.
test.describe.configure({ timeout: 180_000 });
useManagedBrowserSource(test);

test("Issue #6933 Codex cache drives both pickers and warns about saved retired models", async ({ page }, info) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  test.skip(!base || !home, "requires browser-check isolated checkout and HOME");
  await withLiveGwtBackendLock(base!, info, async () => {
    const cache = JSON.parse(await readFile(join(home!, ".codex/models_cache.json"), "utf8"));
    const expected = cache.models.filter((row: any) => row.visibility === "list")
      .sort((a: any, b: any) => a.priority - b.priority)
      .map((row: any) => ({ value: row.slug, label: row.display_name }));
    expect(expected.length).toBeGreaterThan(0);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "workspace_state"));
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const projectHash = process.env.GWT_PLAYWRIGHT_PROJECT_HASH ?? process.env.GWT_REPO_HASH;
    expect(projectHash).toBeTruthy();
    const reportsPath = join(home!, ".gwt/projects", projectHash!, "project-state/pm-reports.jsonl");
    await expect.poll(async () => {
      const text = await readFile(reportsPath, "utf8").catch(error => {
        if (error.code === "ENOENT") return "";
        throw error;
      });
      return text.split("\n").filter(Boolean).map(line => JSON.parse(line))
        .filter(report => report.body.startsWith("# Codex model list changed")).length;
    }, { timeout: 60_000, message: "weekly comparison runs even with Monitor disabled" }).toBe(1);

    const retired = "retired-codex-model";
    await sendLiveGwtEvent(page, { kind: "issue_monitor_profiles_set", request_id: 693301,
      profiles: [{ agent_id: "codex", model: retired, reasoning: "high" }] });
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "issue_monitor_profiles" && payload.request_id === 693301));
    await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "agents" } })));
    const candidate = page.locator('[data-settings-candidates] fieldset[data-agent-id="codex"]');
    const model = candidate.locator('[data-profile-field="model"]');
    const notice = candidate.locator("[data-model-unavailable]");
    await expect(model).toHaveValue(retired);
    await expect(notice).toBeVisible();
    await expect(notice).toContainText(retired);
    await expect(notice).toContainText("not replaced");
    expect(await model.locator("option").evaluateAll(options => options
      .filter(option => (option as HTMLOptionElement).value !== "retired-codex-model")
      .map(option => ({ value: (option as HTMLOptionElement).value, label: option.textContent })))).toEqual(expected);
    // Status updates can replace the row while Playwright waits for stability.
    await notice.evaluate(node => node.scrollIntoView({ block: "center" }));
    await info.attach(`codex-retired-${theme}`, { body: await page.screenshot({ path: info.outputPath(`codex-retired-${theme}.png`), fullPage: true }), contentType: "image/png" });
    await model.selectOption(expected[0].value);
    await expect(notice).toBeHidden();

    const wizardEvent = async (event: unknown) => {
      const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      await sendLiveGwtEvent(page, event);
      return page.waitForFunction(after => ((window as any).__gwtPlaywrightMessages ?? [])
        .findLast(({ sequence, payload }: any) => sequence > after && payload.kind === "launch_wizard_state"
          && payload.wizard && !payload.wizard.is_hydrating)?.payload.wizard, after).then(handle => handle.jsonValue());
    };
    await wizardEvent({ kind: "open_active_work_launch_wizard", branch_name: process.env.GWT_PLAYWRIGHT_BRANCH_NAME });
    const action = (action: unknown) => wizardEvent({ kind: "launch_wizard_action", action });
    await action({ kind: "use_start_method", method: "configure_and_start" });
    const wizard = await action({ kind: "set_agent", agent_id: "codex" });
    expect(wizard.model_options.map(({ value, label }: any) => ({ value, label }))).toEqual(expected);
    const picker = page.locator("#wizard-modal").getByLabel("Model", { exact: true });
    await expect(picker).toBeVisible();
    await expect.poll(() => picker.locator("option").evaluateAll(options => options
      .map(option => ({ value: (option as HTMLOptionElement).value, label: option.textContent })))).toEqual(expected);
    // Exercise the rendering boundary for the saved-model state proved by the
    // Rust restoration tests, using the actual backend's catalog projection.
    await page.evaluate(({ wizard, retired }) => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "launch_wizard_state", wizard: { ...wizard, selected_model: retired,
        model_fallback_notice: `Codex no longer offers ${retired}; the saved model is kept and not replaced.` } },
    })), { wizard, retired });
    await expect(picker).toHaveValue(retired);
    await expect(picker.locator("option:checked")).toContainText("unavailable");
    await expect(picker.locator("option:checked")).toBeDisabled();
    await picker.evaluate(node => node.scrollIntoView({ block: "center" }));
    await info.attach(`codex-picker-${theme}`, { body: await page.screenshot({ path: info.outputPath(`codex-picker-${theme}.png`), fullPage: true }), contentType: "image/png" });
    await picker.selectOption(expected[0].value);
    await expect(picker).toHaveValue(expected[0].value);
    await page.locator("#wizard-cancel-button").click();
    expect(errors).toEqual([]);
  });
});
