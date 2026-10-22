/* Issue #4911 — the Issue Monitor settings form repeats the Agent Settings
 * block once per launch candidate: `＋` adds a set, `−` removes one, the arrows
 * reorder them, and the order is the launch order. Only the open set carries
 * the editable form; the others show what they will launch with.
 *
 * Boots the embedded frontend with a deterministic WebSocket stub (no live gwt
 * process) and injects `launch_wizard_state` through the `__gwt_test_inject`
 * seam, then drives the real DOM in Chromium for both colour schemes
 * (chromium-dark / chromium-light projects). The backend half — what each
 * action does to the sets and what the save writes — is covered by the
 * app_runtime tests and, against a real backend, by
 * issue-monitor-agent-settings-sets-live.spec.ts.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const LAST_SET_REASON =
  "At least one Agent Settings set is required: Issue Monitor needs an agent to launch.";
const ALL_AGENTS_USED_REASON = "Every available agent already has an Agent Settings set.";

const CLAUDE_SUMMARY = [
  { label: "Agent", value: "Claude Code" },
  { label: "Model", value: "default" },
  { label: "Reasoning", value: "auto" },
  { label: "Runtime", value: "host" },
];

const SETTINGS_WIZARD = {
  title: "Configure Issue Monitor",
  branch_name: "develop",
  selected_branch_name: "develop",
  branch_mode: "use_selected",
  show_back_button: false,
  show_branch_controls: false,
  show_manual_setup: true,
  show_runtime_confirmation: false,
  show_confirm: false,
  show_start_methods: false,
  show_agent_settings: true,
  runtime_context_resolved: false,
  primary_action_label: "Continue",
  primary_action_enabled: true,
  launch_summary: [],
  progress_steps: [],
  launch_target_options: [
    { value: "agent", label: "Agent", description: "Launch a coding agent terminal" },
    { value: "shell", label: "Shell", description: "Open a plain shell terminal" },
  ],
  selected_launch_target: "agent",
  agent_options: [
    { value: "codex", label: "Codex", description: "Detected · 0.153.4" },
    { value: "claude", label: "Claude Code", description: "Detected · 2.1.0" },
    { value: "grok", label: "Grok Build", description: "Detected · 1.0.3" },
  ],
  selected_agent_id: "codex",
  model_options: [{ value: "gpt-6-astra", label: "gpt-6-astra", description: "" }],
  selected_model: "gpt-6-astra",
  issue_monitor_pool: {
    active_index: 0,
    sets: [
      { agent_id: "codex", summary: [] },
      { agent_id: "claude", summary: CLAUDE_SUMMARY },
    ],
    add_disabled_reason: null,
    remove_disabled_reason: null,
    resulting_summary:
      "auto (2): codex / gpt-6-astra / auto / host / fast:off | claude / default / auto / host / fast:off",
  },
};

test.describe("Launch Wizard — Issue Monitor Agent Settings sets", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("adds, removes, reorders and opens sets through wizard actions", async ({
    page,
  }, testInfo) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator("#close-project-button")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, SETTINGS_WIZARD);
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/);

    const sets = modal.locator(".launch-agent-set");
    await expect(sets).toHaveCount(2);
    await expect(sets.nth(0)).toContainText("Agent Settings 1");
    await expect(sets.nth(1)).toContainText("Agent Settings 2");
    // The open set carries the same form a single set always had.
    await expect(sets.nth(0)).toHaveClass(/is-open/);
    await expect(sets.nth(0)).toHaveAttribute("data-agent-id", "codex");
    await expect(sets.nth(0).locator(".launch-section").first()).toContainText("Launch");
    await expect(sets.nth(0).getByLabel("Model")).toHaveValue("gpt-6-astra");
    // A set that is not open shows what it launches with instead.
    await expect(sets.nth(1)).not.toHaveClass(/is-open/);
    await expect(sets.nth(1)).toContainText("Claude Code");
    await expect(sets.nth(1).locator(".launch-select")).toHaveCount(0);
    await expect(modal.locator(".launch-agent-sets__order")).toContainText("auto (2): codex");

    await modal.getByRole("button", { name: "Add Agent Settings" }).click();
    await modal.getByRole("button", { name: "Move Agent Settings 2 up", exact: true }).click();
    await modal.getByRole("button", { name: "Remove Agent Settings 2", exact: true }).click();
    await modal.getByRole("button", { name: "Edit Agent Settings 2", exact: true }).click();
    expect(await sentWizardActions(page)).toEqual([
      { kind: "add_agent_settings_set" },
      { kind: "move_agent_settings_set", index: 1, to: 0 },
      { kind: "remove_agent_settings_set", index: 1 },
      { kind: "select_agent_settings_set", index: 1 },
    ]);
    // The first set cannot move up and the last cannot move down.
    await expect(
      modal.getByRole("button", { name: "Move Agent Settings 1 up", exact: true }),
    ).toBeDisabled();
    await expect(
      modal.getByRole("button", { name: "Move Agent Settings 2 down", exact: true }),
    ).toBeDisabled();

    // The backend answers an add with the new set open at the end.
    await injectWizard(page, {
      ...SETTINGS_WIZARD,
      selected_agent_id: "grok",
      model_options: [],
      selected_model: "",
      issue_monitor_pool: {
        active_index: 2,
        sets: [
          {
            agent_id: "codex",
            summary: [
              { label: "Agent", value: "Codex" },
              { label: "Model", value: "gpt-6-astra" },
              { label: "Reasoning", value: "auto" },
              { label: "Runtime", value: "host" },
            ],
          },
          { agent_id: "claude", summary: CLAUDE_SUMMARY },
          { agent_id: "grok", summary: [] },
        ],
        add_disabled_reason: ALL_AGENTS_USED_REASON,
        remove_disabled_reason: null,
        resulting_summary: "auto (3): codex | claude | grok",
      },
    });
    await expect(sets).toHaveCount(3);
    await expect(sets.nth(2)).toHaveClass(/is-open/);
    await expect(sets.nth(2)).toHaveAttribute("data-agent-id", "grok");
    await expect(sets.nth(2).locator(".launch-section").first()).toContainText("Launch");
    await expect(sets.nth(0).locator(".launch-section")).toHaveCount(0);
    await expect(modal.getByRole("button", { name: "Add Agent Settings" })).toBeDisabled();
    await expect(modal.locator(".launch-agent-sets__footer")).toContainText(ALL_AGENTS_USED_REASON);

    await expect(page.locator("html")).toHaveAttribute(
      "data-theme",
      testInfo.project.name.endsWith("light") ? "light" : "dark",
    );
    await testInfo.attach(`agent-settings-sets-${testInfo.project.name}`, {
      body: await modal.screenshot(),
      contentType: "image/png",
    });
    expect(browserErrors).toEqual([]);
  });

  test("keeps the last set and says why it cannot be removed", async ({ page }) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator("#close-project-button")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, {
      ...SETTINGS_WIZARD,
      issue_monitor_pool: {
        active_index: 0,
        sets: [{ agent_id: "codex", summary: [] }],
        add_disabled_reason: null,
        remove_disabled_reason: LAST_SET_REASON,
        resulting_summary: "codex / gpt-6-astra / auto / host / fast:off",
      },
    });
    const modal = page.locator("#wizard-modal");
    await expect(modal.locator(".launch-agent-set")).toHaveCount(1);
    await expect(
      modal.getByRole("button", { name: "Remove Agent Settings 1", exact: true }),
    ).toBeDisabled();
    await expect(modal.locator(".launch-agent-sets__footer")).toContainText(LAST_SET_REASON);
    await expect(modal.getByRole("button", { name: "Add Agent Settings" })).toBeEnabled();
    expect(browserErrors).toEqual([]);
  });

  test("an ordinary launch wizard has no sets", async ({ page }) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator("#close-project-button")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, {
      ...SETTINGS_WIZARD,
      title: "Launch Agent",
      issue_monitor_pool: null,
    });
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/);
    await expect(modal.locator(".launch-agent-set")).toHaveCount(0);
    await expect(modal.getByRole("button", { name: "Add Agent Settings" })).toHaveCount(0);
    // The form sections stay directly in the panel, as before.
    await expect(
      modal.locator(".launch-panel > .launch-section", { hasText: "Choose what to launch" }),
    ).toHaveCount(1);
    expect(browserErrors).toEqual([]);
  });
});

function collectBrowserErrors(page: any): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error: Error) => errors.push(error.message));
  page.on("console", (message: any) => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
}

// Wizard actions travel as `{ kind: "launch_wizard_action", action }` envelopes.
async function sentWizardActions(page: any): Promise<unknown[]> {
  return page.evaluate(() =>
    (window as any).__sent
      .filter((message: any) => message && message.kind === "launch_wizard_action")
      .map((message: any) => message.action),
  );
}

async function injectWizard(page: any, wizard: Record<string, unknown>): Promise<void> {
  await page.evaluate((payload: Record<string, unknown>) => {
    window.dispatchEvent(
      new CustomEvent("__gwt_test_inject", {
        detail: { kind: "launch_wizard_state", wizard: payload },
      }),
    );
  }, wizard);
}

async function keepLaunchWizardModalVisible(page: any): Promise<void> {
  await page.addStyleTag({
    content: `
      #wizard-modal[aria-hidden="false"],
      #wizard-modal.open {
        display: flex !important;
        pointer-events: auto !important;
      }
      #wizard-modal[aria-hidden="true"] {
        display: none !important;
        pointer-events: none !important;
      }
    `,
  });
}

async function installWorkspaceFixture(page: any): Promise<void> {
  await page.addInitScript(() => {
    (window as any).__sent = [];
    try {
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    } catch {
      /* no-op */
    }

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: "git",
            workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [] },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      url: string;
      readyState: number;

      constructor(url: string) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(workspaceState);
        }, 0);
      }

      send(raw: string): void {
        let message: any;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        (window as any).__sent.push(message);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
        }
      }

      close(): void {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: any): void {
        setTimeout(() => {
          this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  });
}
