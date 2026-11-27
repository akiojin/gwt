import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test("candidate selection rejects unknown and duplicate drafts and explains an exhausted catalog", async ({ page }, info) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.endsWith("light") ? "light" : "dark";
  await installEmbeddedRoutes(page);
  await page.route(APP_URL, route => route.fulfill({
    contentType: "text/html",
    body: `<!doctype html><html data-theme="${theme}"><head><link rel="icon" href="data:,"><link rel="stylesheet" href="/styles/tokens.css"><link rel="stylesheet" href="/styles/typography.css"><link rel="stylesheet" href="/styles/components.css"><link rel="stylesheet" href="/styles/app.css"></head><body><main class="settings-panel" id="candidates"></main></body></html>`,
  }));
  await page.goto(APP_URL);
  await page.evaluate(async () => {
    const modulePath = "/settings-surface.js";
    const { renderAgentsSettingsPanel } = await import(modulePath);
    const tuningModulePath = "/agent-activity-model.js";
    const { createMemberTuningModel } = await import(tuningModulePath);
    (window as any).candidateRequests = [];
    (window as any).candidateTuningRequests = [];
    const tuningController = createMemberTuningModel("fixture-project", {
      onSave: (message: unknown) => (window as any).candidateTuningRequests.push(message),
    });
    for (const [member_id, model] of [["a", "preserved-model"], ["b", "birch-saved-model"]]) {
      tuningController.applyTuningSnapshot({
        project_key: "fixture-project", role: "implementation", member_id,
        agent_id: "claude", model, reasoning: "high", revision: `core-${member_id}`,
      });
    }
    (window as any).candidateTuning = tuningController;
    (window as any).renderCandidates = (availableMembers: Array<{ member_id: string; agent_id: string; name: string; enabled: boolean }> | null) => {
      renderAgentsSettingsPanel(document.getElementById("candidates"), {
        project: { project_root: "/fixture", name: "Fixture" },
        profiles: [{ agent_member_id: "a", agent_id: "ClaudeCode", model: "preserved-model" }],
        status: { usage_threshold_percent: 80 },
        availableMembers,
        tuningController,
        send: (message: unknown) => (window as any).candidateRequests.push(message),
      });
    };
    (window as any).renderCandidates([{ member_id: "a", name: "Aster", agent_id: "claude", enabled: true }, { member_id: "b", name: "Birch", agent_id: "claude", enabled: true }, { member_id: "c", name: "Cedar", agent_id: "codex", enabled: true }]);
  });
  const tuningBefore = await page.evaluate(() => (window as any).candidateTuning.model.read());
  const pool = page.locator("#candidates");
  const candidate = pool.getByLabel("Add candidate candidate", { exact: true });
  const add = pool.getByRole("button", { name: "Add candidate", exact: true });
  await expect(candidate).toHaveJSProperty("tagName", "SELECT");
  await expect(candidate.locator('option[value=""]')).toHaveText("Select a registered member…");
  await expect(candidate.locator('option[value="a"]')).toHaveCount(0);
  await add.click();
  await expect(pool.getByRole("alert")).toHaveText("Select a registered member to add.");
  expect(await page.evaluate(() => (window as any).candidateRequests)).toEqual([]);
  // A native select prevents arbitrary typing. Even a tampered option must
  // be rejected locally rather than dispatching a save that cannot succeed.
  for (const [id, message] of [
    ["unknown-member", "Unknown member: unknown-member. Select a registered member."],
    ["a", "Member a is already in the candidate pool."],
  ]) {
    await candidate.evaluate((select, id) => {
      select.appendChild(new Option(id, id));
    }, id);
    await candidate.selectOption(id);
    await add.click();
    await expect(pool.getByRole("alert")).toHaveText(message);
    expect(await page.evaluate(() => (window as any).candidateRequests)).toEqual([]);
  }
  await expect(pool.locator('fieldset[data-agent-member-id="a"]').getByLabel("Model", { exact: true }))
    .toHaveValue("preserved-model");
  await candidate.selectOption("b");
  await pool.getByLabel("Agent Aster", { exact: true }).selectOption("b");
  await add.click();
  await expect(pool.getByRole("alert")).toHaveText("Member b is already in the candidate pool.");
  await expect(pool.locator('fieldset[data-agent-member-id="b"]').getByLabel("Model", { exact: true }))
    .toHaveValue("birch-saved-model");
  const requests = await page.evaluate(() => (window as any).candidateRequests);
  expect(requests).toHaveLength(1);
  expect(requests[0].profiles[0]).toMatchObject({ agent_member_id: "b", agent_id: "claude" });
  expect(requests[0].profiles[0]).not.toHaveProperty("model");
  expect(requests[0].profiles[0]).not.toHaveProperty("reasoning");
  await expect(pool.locator("legend")).toHaveText("1. Birch");
  await expect(pool.getByLabel("Tool Birch")).toHaveText("claude");
  await expect(pool.locator('[data-profile-field="agent_id"]')).toHaveCount(0);

  await page.evaluate(() => (window as any).renderCandidates([{ member_id: "a", name: "Aster", agent_id: "claude", enabled: true }]));
  await expect(candidate).toBeDisabled();
  await expect(add).toBeDisabled();
  await expect(pool).toContainText("All registered members are already in the candidate pool.");
  await page.evaluate(() => (window as any).renderCandidates(null));
  await expect(candidate).toBeDisabled();
  await expect(add).toBeDisabled();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  expect(await page.evaluate(() => (window as any).candidateTuning.model.read())).toEqual(tuningBefore);
  expect(await page.evaluate(() => (window as any).candidateTuningRequests)).toEqual([]);
  expect(errors).toEqual([]);
});

// Issue #7729 AC-4 (SPEC #7007 item 197): a candidate names where it runs.
test("a candidate chooses Inherit, Host or Docker with its service and lifecycle", async ({ page }, info) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.endsWith("light") ? "light" : "dark";
  await installEmbeddedRoutes(page);
  await page.route(APP_URL, route => route.fulfill({
    contentType: "text/html",
    body: `<!doctype html><html data-theme="${theme}"><head><link rel="icon" href="data:,"><link rel="stylesheet" href="/styles/tokens.css"><link rel="stylesheet" href="/styles/typography.css"><link rel="stylesheet" href="/styles/components.css"><link rel="stylesheet" href="/styles/app.css"></head><body><main class="settings-panel" id="candidates"></main></body></html>`,
  }));
  await page.goto(APP_URL);
  await page.evaluate(async () => {
    const modulePath = "/settings-surface.js";
    const { renderAgentsSettingsPanel } = await import(modulePath);
    (window as any).candidateRequests = [];
    (window as any).renderCandidates = (dockerServices: string[], profile: Record<string, unknown>) => {
      renderAgentsSettingsPanel(document.getElementById("candidates"), {
        project: { project_root: "/fixture", name: "Fixture" },
        profiles: [{ agent_member_id: "a", agent_id: "codex", ...profile }],
        availableMembers: [{ member_id: "a", name: "Aster", agent_id: "codex", enabled: true }],
        choices: { agents: [{ id: "codex", name: "Codex", session_modes: ["Normal"], supports_skip_permissions: true }],
          docker_services: dockerServices, default_docker_service: dockerServices[0] ?? null, windows_shell_supported: false },
        send: (message: unknown) => (window as any).candidateRequests.push(message),
      });
    };
    (window as any).renderCandidates(["app", "worker"], { runtime_target: "Host" });
  });
  const pool = page.locator("#candidates");
  const runsOn = pool.getByLabel("Runs on Aster", { exact: true });
  const service = pool.getByLabel("Docker service Aster", { exact: true });
  const lifecycle = pool.getByLabel("Docker lifecycle Aster", { exact: true });
  await expect(runsOn.locator("option")).toHaveText(["Inherit (this project's last launch)", "Host", "Docker"]);
  await expect(runsOn).toHaveValue("Host");
  await expect(service).toBeHidden();
  await expect(lifecycle).toBeHidden();

  await runsOn.selectOption("Docker");
  await expect(service).toBeVisible();
  await expect(lifecycle).toBeVisible();
  await service.selectOption("worker");
  await lifecycle.selectOption("Restart");
  await runsOn.selectOption("Inherit");
  await expect(service).toBeHidden();
  await expect(lifecycle).toBeHidden();
  await expect(pool.locator("[data-profile-runtime-note]")).toHaveText(
    "Runs where this project's last launch ran. With no earlier launch, it runs on the host.");
  const sent = (await page.evaluate(() => (window as any).candidateRequests)).map((request: any) => {
    const { runtime_inherit, runtime_target, docker_service, docker_lifecycle_intent } = request.profiles[0];
    return { runtime_inherit, runtime_target, docker_service, docker_lifecycle_intent };
  });
  expect(sent).toEqual([
    { runtime_inherit: false, runtime_target: "Docker", docker_service: null, docker_lifecycle_intent: "Connect" },
    { runtime_inherit: false, runtime_target: "Docker", docker_service: "worker", docker_lifecycle_intent: "Connect" },
    { runtime_inherit: false, runtime_target: "Docker", docker_service: "worker", docker_lifecycle_intent: "Restart" },
    { runtime_inherit: true, runtime_target: "Host", docker_service: "worker", docker_lifecycle_intent: "Restart" },
  ]);

  // Item 197 (1): a project without a Docker setup is not offered Docker.
  await page.evaluate(() => (window as any).renderCandidates([], { runtime_inherit: true, runtime_target: "Host" }));
  await expect(runsOn.locator("option")).toHaveText(["Inherit (this project's last launch)", "Host"]);
  await expect(runsOn).toHaveValue("Inherit");
  await expect(service).toBeHidden();
  // Item 197 (2): a Docker choice saved earlier stays visible and names the fallback.
  await page.evaluate(() => (window as any).renderCandidates([], { runtime_target: "Docker", docker_service: "gone" }));
  await expect(runsOn).toHaveValue("Docker");
  await expect(pool.locator("[data-profile-runtime-note]")).toHaveText(
    "This project has no Docker setup, so launches run on the host.");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  expect(errors).toEqual([]);
});
