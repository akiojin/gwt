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
    const sent: any[] = [];
    page.on("websocket", socket => socket.on("framesent", ({ payload }) => {
      try { sent.push(JSON.parse(String(payload))); } catch { /* Ignore non-JSON transport frames. */ }
    }));
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

    const projectKey = await page.evaluate(() => location.pathname.split("/")[2]);
    let readSequence = 0;
    const readRoster = async () => {
      const requestId = `catalog-${theme}-read-${++readSequence}`;
      await sendLiveGwtEvent(page, { kind: "agent_roster", project_key: projectKey,
        request_id: requestId, action: { kind: "read" } });
      const roster = await page.waitForFunction(({ projectKey, requestId }) => ((window as any).__gwtPlaywrightMessages ?? [])
        .findLast(({ payload }: any) => payload.kind === "agent_roster" && payload.request_id === requestId
          && payload.roster.project_key === projectKey)?.payload.roster, { projectKey, requestId })
        .then(handle => handle.jsonValue());
      expect(roster.error ?? null).toBeNull();
      return roster;
    };
    await page.locator('.op-rail__surface[data-surface="agents"]').click();
    const agents = page.locator('.agents-surface:not(.agents-surface--preview)');
    const name = `Codex catalog ${theme}`;
    await agents.getByRole("textbox", { name: "Name for new member" }).fill(name);
    await agents.getByRole("combobox", { name: "Tool for new member" }).selectOption("codex");
    await agents.locator('.roster-capacity [data-action="add"]').click();
    const row = agents.locator('.roster-row').filter({ hasText: name });
    await expect(row).toHaveCount(1);
    await row.click();
    const initial = await readRoster();
    const member = initial.members.find((member: any) => member.name === name);
    expect(member).toBeTruthy();
    expect(member.agent_id).toBe("codex");
    expect(member.saved_implementation_core_revision).toBeTruthy();
    const memberId = member.agent_member_id;
    const tuningWrites = () => sent.filter(message => message.kind === "agent_roster"
      && message.project_key === projectKey && message.action?.kind === "set_tuning"
      && message.action.member_id === memberId);
    const detail = agents.locator('.roster-detail');
    const model = detail.locator('[data-member-tuning-field="model"]');
    const notice = detail.locator('[data-model-unavailable]');
    await expect(model).toBeEnabled();
    const retired = "retired-codex-model";
    await model.fill(retired);
    await expect.poll(() => tuningWrites().length).toBe(1);
    const save = tuningWrites()[0];
    expect(save.request_id).toBeTruthy();
    expect(save.action).toMatchObject({ kind: "set_tuning", member_id: memberId,
      model: retired, expected_core_revision: member.saved_implementation_core_revision });
    expect(Object.hasOwn(save.action, "reasoning")).toBe(true);
    await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
    const receipt = await page.waitForFunction(({ requestId, projectKey }) => ((window as any).__gwtPlaywrightMessages ?? [])
      .findLast(({ payload }: any) => payload.kind === "agent_roster" && payload.request_id === requestId
        && payload.roster.project_key === projectKey)?.payload.roster, { requestId: save.request_id, projectKey })
      .then(handle => handle.jsonValue());
    expect(receipt.error ?? null).toBeNull();
    const saved = (await readRoster()).members.find((member: any) => member.agent_member_id === memberId);
    expect(saved.saved_implementation_core.model).toBe(retired);
    expect(saved.saved_implementation_core_revision).not.toBe(member.saved_implementation_core_revision);
    expect(receipt.members.find((member: any) => member.agent_member_id === memberId).saved_implementation_core_revision)
      .toBe(saved.saved_implementation_core_revision);
    await expect(model).toHaveValue(retired);
    await expect(notice).toBeVisible();
    await expect(notice).toContainText(retired);
    await expect(notice).toContainText("not replaced");
    expect(await model.evaluate(input => Array.from((input as HTMLInputElement).list!.options)
      .map(option => ({ value: option.value, label: option.label })))).toEqual(expected);
    await notice.evaluate(node => node.scrollIntoView({ block: "center" }));
    await info.attach(`codex-retired-${theme}`, { body: await page.screenshot({ path: info.outputPath(`codex-retired-${theme}.png`), fullPage: true }), contentType: "image/png" });
    await model.fill(expected[0].value);
    await expect.poll(() => tuningWrites().length).toBe(2);
    const selectionSave = tuningWrites()[1];
    expect(selectionSave.request_id).toBeTruthy();
    expect(selectionSave.request_id).not.toBe(save.request_id);
    expect(selectionSave.action).toMatchObject({ kind: "set_tuning", member_id: memberId,
      model: expected[0].value, expected_core_revision: saved.saved_implementation_core_revision });
    expect(Object.hasOwn(selectionSave.action, "reasoning")).toBe(true);
    const selectionReceipt = await page.waitForFunction(({ requestId, projectKey }) => ((window as any).__gwtPlaywrightMessages ?? [])
      .findLast(({ payload }: any) => payload.kind === "agent_roster" && payload.request_id === requestId
        && payload.roster.project_key === projectKey)?.payload.roster, { requestId: selectionSave.request_id, projectKey })
      .then(handle => handle.jsonValue());
    expect(selectionReceipt.error ?? null).toBeNull();
    await expect(detail.locator('[data-member-tuning-status]')).toHaveText("Saved");
    const selected = (await readRoster()).members.find((member: any) => member.agent_member_id === memberId);
    expect(selected.saved_implementation_core.model).toBe(expected[0].value);
    expect(selected.saved_implementation_core_revision).not.toBe(saved.saved_implementation_core_revision);
    expect(selectionReceipt.members.find((member: any) => member.agent_member_id === memberId).saved_implementation_core_revision)
      .toBe(selected.saved_implementation_core_revision);
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
