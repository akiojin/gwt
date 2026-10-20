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
    (window as any).candidateRequests = [];
    (window as any).renderCandidates = (availableAgents: Array<{ id: string; name: string }> | null) => {
      renderAgentsSettingsPanel(document.getElementById("candidates"), {
        project: { project_root: "/fixture", name: "Fixture" },
        profiles: [{ agent_id: "ClaudeCode", model: "preserved-model" }],
        status: { usage_threshold_percent: 80 },
        availableAgents,
        send: (message: unknown) => (window as any).candidateRequests.push(message),
      });
    };
    (window as any).renderCandidates([{ id: "claude", name: "Claude Code" }, { id: "codex", name: "Codex" }]);
  });
  const pool = page.locator("#candidates");
  const candidate = pool.getByLabel("Add candidate candidate", { exact: true });
  const add = pool.getByRole("button", { name: "Add candidate", exact: true });
  await expect(candidate).toHaveJSProperty("tagName", "SELECT");
  await expect(candidate.locator('option[value=""]')).toHaveText("Select an agent…");
  await expect(candidate.locator('option[value="claude"]')).toHaveCount(0);
  await add.click();
  await expect(pool.getByRole("alert")).toHaveText("Select an agent to add.");
  expect(await page.evaluate(() => (window as any).candidateRequests)).toEqual([]);
  // A native select prevents arbitrary typing. Even a tampered option must
  // be rejected locally rather than dispatching a save that cannot succeed.
  for (const [id, message] of [
    ["unknown-agent", "Unknown agent: unknown-agent. Select a known agent."],
    ["claude", "Agent claude is already in the candidate pool."],
  ]) {
    await candidate.evaluate((select, id) => {
      select.appendChild(new Option(id, id));
    }, id);
    await candidate.selectOption(id);
    await add.click();
    await expect(pool.getByRole("alert")).toHaveText(message);
    expect(await page.evaluate(() => (window as any).candidateRequests)).toEqual([]);
  }
  await candidate.selectOption("codex");
  await pool.getByLabel("Agent ClaudeCode", { exact: true }).fill("Codex");
  await add.click();
  await expect(pool.getByRole("alert")).toHaveText("Agent codex is already in the candidate pool.");
  await expect(pool.getByLabel("Model ClaudeCode", { exact: true })).toHaveValue("preserved-model");
  const requests = await page.evaluate(() => (window as any).candidateRequests);
  expect(requests).toHaveLength(1);
  expect(requests[0].profiles[0].agent_id).toBe("Codex");

  await page.evaluate(() => (window as any).renderCandidates([{ id: "claude", name: "Claude Code" }]));
  await expect(candidate).toBeDisabled();
  await expect(add).toBeDisabled();
  await expect(pool).toContainText("All known agents are already in the candidate pool.");
  await page.evaluate(() => (window as any).renderCandidates(null));
  await expect(candidate).toBeDisabled();
  await expect(add).toBeDisabled();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  expect(errors).toEqual([]);
});
