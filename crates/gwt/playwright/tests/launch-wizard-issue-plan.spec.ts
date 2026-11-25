/* Issue #7771 (SPEC #7007 mock v3) — a new launch that starts work on an
 * Issue is planned in three stages after the start method is chosen: Issue &
 * assignee, Launch plan, Confirm.
 *
 * Boots the embedded frontend with a deterministic WebSocket stub (no live gwt
 * process) and injects one `launch_wizard_state` per stage through the
 * `__gwt_test_inject` seam, then drives the real DOM in Chromium for both
 * colour schemes (chromium-dark / chromium-light projects).
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const ASSIGNEES = [
  {
    agent_member_id: "busy",
    name: "Himeno Daiki",
    selected: false,
    available: false,
    status: "Working on #7007 now. It can be chosen once that work ends.",
    reasoning: "Standard",
    speed: "Fast",
    personality: "Screens and experience first.",
  },
  {
    agent_member_id: "oda",
    name: "Oda Yohei",
    selected: true,
    available: true,
    status: "Assigned to this launch",
    reasoning: "Max",
    speed: "Fastest",
    personality: "Implementation first.",
  },
  {
    agent_member_id: "ando",
    name: "Ando Kenji",
    selected: false,
    available: true,
    status: "Can take this launch",
    reasoning: "Standard",
    speed: "Tool default",
    personality: null,
  },
];

const rail = (states: [string, string, string]) => [
  { key: "assignee", label: "Issue & assignee", state: states[0], detail: null },
  { key: "plan", label: "Launch plan", state: states[1], detail: null },
  { key: "confirm", label: "Confirm", state: states[2], detail: null },
];

const BASE = {
  title: "Launch Agent",
  mode: "knowledge",
  branch_name: "work/issue-5561",
  selected_branch_name: "develop",
  branch_mode: "create_new",
  linked_issue_number: 5561,
  show_linked_issue: true,
  show_branch_controls: false,
  show_agent_settings: false,
  runtime_context_resolved: false,
  show_manual_setup: false,
  show_runtime_confirmation: false,
  show_confirm: false,
  show_start_methods: false,
  show_back_button: true,
  primary_action_enabled: true,
  selected_launch_target: "agent",
  selected_agent_id: "codex",
  selected_runtime_target: "host",
  launch_summary: [],
};

const plan = (stage: string | null) => ({
  stage,
  issue_number: 5561,
  assignees: ASSIGNEES,
  branch: "work/issue-5561",
  branch_detail: "Created from develop",
  permissions: "Skips permission prompts",
});

const START_METHODS = [
  { kind: "configure_and_start", label: "Configure and start", badge: "Settings", group: "recommended", recommended: true, summary: "Choose the assignee and plan the launch", enabled: true },
  { kind: "start_with_last_settings", label: "Start with last settings", badge: "New", group: "available", recommended: false, summary: "New session with saved settings", enabled: true },
  { kind: "continue_last_session", label: "Continue last session", badge: "Session", group: "available", recommended: false, summary: "Resume conversation history", enabled: true },
  { kind: "focus_running_session", label: "Focus running session", badge: "Running", group: "unavailable", recommended: false, summary: "Switch to running session", enabled: false, disabled_reason: "No running session" },
  { kind: "open_shell", label: "Open a shell", badge: "Shell", group: "available", recommended: false, summary: "Plain shell on this branch", enabled: true },
];

const START_SCREEN = {
  ...BASE,
  show_start_methods: true,
  show_back_button: false,
  start_methods: START_METHODS,
  open_branch_candidates: ["origin/feature/existing"],
  primary_action_label: "Choose start method",
  primary_action_enabled: false,
  progress_steps: rail(["pending", "pending", "pending"]),
  issue_plan: plan(null),
};

const STAGE_ASSIGNEE = {
  ...BASE,
  show_manual_setup: true,
  primary_action_label: "Continue to launch plan",
  progress_steps: rail(["active", "pending", "pending"]),
  issue_plan: plan("assignee"),
};

const STAGE_PLAN = {
  ...BASE,
  runtime_context_resolved: true,
  show_runtime_confirmation: true,
  show_runtime_target: true,
  runtime_target_options: [
    { value: "host", label: "Host", description: "Run on this machine" },
    { value: "docker", label: "Docker", description: "Run in a Compose service" },
  ],
  runtime_target_note: "Same as this project's last launch.",
  primary_action_label: "Continue to confirm",
  progress_steps: rail(["done", "active", "pending"]),
  issue_plan: plan("plan"),
  execution_environment: {
    name: "Codex 1",
    via_tool_name: "Codex",
    priority: 2,
    list_len: 2,
    note: "Number 2 of 2 on this project's list.",
    skipped: [
      { priority: 1, name: "Claude Code 1", reason: "Claude Code is not installed on this machine." },
    ],
  },
};

const STAGE_CONFIRM = {
  ...BASE,
  runtime_context_resolved: true,
  show_confirm: true,
  primary_action_label: "Start",
  progress_steps: rail(["done", "done", "active"]),
  issue_plan: plan("confirm"),
  launch_summary: [
    { label: "Issue", value: "#5561" },
    { label: "Assignee", value: "Oda Yohei · Reasoning Max · Speed Fastest · Implementation first." },
    { label: "Branch", value: "work/issue-5561" },
    { label: "Runtime", value: "host" },
    { label: "Permissions", value: "skip" },
  ],
};

test.describe("Launch Wizard — an Issue launch in three stages", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("the three stages complete a launch", async ({ page }, testInfo) => {
    const browserErrors = collectBrowserErrors(page);
    const modal = await openFixture(page);
    const steps = modal.locator(".wizard-progress-step");
    const submit = page.locator("#wizard-submit-button");

    // Stage 1 — Issue & assignee: every registered agent with its traits.
    await injectWizard(page, STAGE_ASSIGNEE);
    await expect(modal).toHaveClass(/open/);
    await expect(steps.locator(".wizard-progress-label")).toHaveText([
      "Issue & assignee",
      "Launch plan",
      "Confirm",
    ]);
    await expect(steps.nth(0)).toHaveAttribute("data-state", "active");
    await expect(modal.locator(".issue-plan-issue")).toHaveText("#5561");
    const options = modal.locator(".assignee-option");
    await expect(options).toHaveCount(3);
    await expect(options.nth(1)).toHaveAttribute("aria-pressed", "true");
    await expect(options.nth(1).locator(".assignee-trait")).toHaveText([
      "Reasoning Max",
      "Speed Fastest",
      "Personality Implementation first.",
    ]);
    await expect(options.nth(2).locator(".assignee-trait")).toHaveText([
      "Reasoning Standard",
      "Speed Tool default",
    ]);
    await expect(options.nth(0)).toBeDisabled();
    await expect(options.nth(0)).toContainText("Working on #7007 now.");
    // The settings form is replaced by the stage, not drawn next to it.
    await expect(modal.getByRole("combobox", { name: "Model", exact: true })).toHaveCount(0);
    await expect(modal.getByText("Linked issue")).toHaveCount(0);
    // The Issue exists, so the stage does not ask to register one.
    await expect(modal.locator(".launch-intake-input")).toHaveCount(0);
    await testInfo.attach(`issue-plan-assignee-${testInfo.project.name}`, {
      body: await modal.screenshot(),
      contentType: "image/png",
    });

    await options.nth(2).click();
    await expect
      .poll(() => page.evaluate(() => sentWizardActions()))
      .toContainEqual({ kind: "set_agent_member", agent_member_id: "ando" });
    await expect(submit).toHaveText("Continue to launch plan");
    await submit.click();
    await expect
      .poll(() => page.evaluate(() => sentWizardActions()))
      .toContainEqual({ kind: "submit" });

    // Stage 2 — Launch plan: assignee, execution environment, branch, the
    // place to run and permissions.
    await injectWizard(page, STAGE_PLAN);
    await expect(steps.nth(0)).toHaveAttribute("data-state", "done");
    await expect(steps.nth(1)).toHaveAttribute("data-state", "active");
    await expect(modal.locator(".issue-plan-assignee")).toContainText("Oda Yohei");
    await expect(modal.locator(".launch-environment__title")).toContainText([
      "Execution environment · Codex 1",
    ]);
    await expect(modal.locator(".launch-environment__skipped")).toHaveText(
      "1. Claude Code 1 — Claude Code is not installed on this machine.",
    );
    await expect(modal.locator(".issue-plan-branch")).toContainText("work/issue-5561");
    await expect(modal.locator(".issue-plan-branch")).toContainText("Created from develop");
    await expect(modal.locator(".issue-plan-permissions")).toContainText(
      "Skips permission prompts",
    );
    await expect(modal.locator(".launch-runtime-note")).toHaveText(
      "Same as this project's last launch.",
    );
    await expect(modal.locator(".assignee-option")).toHaveCount(0);
    await testInfo.attach(`issue-plan-plan-${testInfo.project.name}`, {
      body: await modal.screenshot(),
      contentType: "image/png",
    });

    await modal.getByRole("button", { name: "Change assignee" }).click();
    await expect
      .poll(() => page.evaluate(() => sentWizardActions()))
      .toContainEqual({ kind: "goto_step", phase: "settings" });
    // The first stage of the rail goes back to the same phase.
    await steps.nth(0).click();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            sentWizardActions().filter(
              (action: any) => action.kind === "goto_step" && action.phase === "settings",
            ).length,
        ),
      )
      .toBe(2);
    await expect(submit).toHaveText("Continue to confirm");

    // Stage 3 — Confirm: the summary and the button that starts the session.
    await injectWizard(page, STAGE_CONFIRM);
    await expect(steps.nth(2)).toHaveAttribute("data-state", "active");
    const summary = modal.locator(".wizard-confirm-summary .wizard-summary-item");
    await expect(summary.locator(".wizard-summary-label")).toHaveText([
      "Issue",
      "Assignee",
      "Branch",
      "Runtime",
      "Permissions",
    ]);
    await expect(summary.nth(1).locator(".wizard-summary-value")).toHaveText(
      "Oda Yohei · Reasoning Max · Speed Fastest · Implementation first.",
    );
    await expect(modal.locator(".issue-plan-assignee")).toHaveCount(0);
    await expect(submit).toHaveText("Start");
    await expect(submit).toBeEnabled();
    await testInfo.attach(`issue-plan-confirm-${testInfo.project.name}`, {
      body: await modal.screenshot(),
      contentType: "image/png",
    });
    const submitsBefore = await page.evaluate(
      () => sentWizardActions().filter((action: any) => action.kind === "submit").length,
    );
    await submit.click();
    await expect
      .poll(() =>
        page.evaluate(
          () => sentWizardActions().filter((action: any) => action.kind === "submit").length,
        ),
      )
      .toBe(submitsBefore + 1);
    expect(browserErrors).toEqual([]);
  });

  test("the start-method screen stays in front with every way in", async ({
    page,
  }, testInfo) => {
    const browserErrors = collectBrowserErrors(page);
    const modal = await openFixture(page);
    await injectWizard(page, START_SCREEN);
    await expect(modal).toHaveClass(/open/);

    // The three stages are announced, and none has started.
    const steps = modal.locator(".wizard-progress-step");
    await expect(steps).toHaveCount(3);
    for (const index of [0, 1, 2]) {
      await expect(steps.nth(index)).toHaveAttribute("data-state", "pending");
    }
    await expect(modal.locator(".assignee-option")).toHaveCount(0);

    const titles = modal.locator(".start-method-group .start-method-title");
    await expect(titles).toHaveText([
      "Configure and start",
      "Start with last settings",
      "Continue last session",
      "Open a shell",
      "Focus running session",
    ]);
    await expect(modal.locator("[data-existing-branch]")).toHaveCount(1);
    await testInfo.attach(`issue-plan-start-${testInfo.project.name}`, {
      body: await modal.screenshot(),
      contentType: "image/png",
    });

    // Each way in tells the backend which one it is, as before.
    for (const [label, method] of [
      ["Start with last settings", "start_with_last_settings"],
      ["Continue last session", "continue_last_session"],
      ["Open a shell", "open_shell"],
      ["Configure and start", "configure_and_start"],
    ]) {
      await injectWizard(page, START_SCREEN);
      await modal.locator(".start-method-button", { hasText: label }).click();
      await expect
        .poll(() => page.evaluate(() => sentWizardActions()))
        .toContainEqual({ kind: "use_start_method", method });
    }
    await injectWizard(page, START_SCREEN);
    await modal.locator("[data-existing-branch]").click();
    await expect
      .poll(() => page.evaluate(() => sentWizardActions()))
      .toContainEqual({ kind: "select_existing_branch", branch_name: "origin/feature/existing" });
    expect(browserErrors).toEqual([]);
  });
});

// Wizard actions travel as `{ kind: "launch_wizard_action", action }` envelopes.
declare function sentWizardActions(): any[];

async function openFixture(page: any) {
  await installEmbeddedRoutes(page);
  await installWorkspaceFixture(page);
  await page.goto(APP_URL);
  await keepLaunchWizardModalVisible(page);
  await expect(page.locator("#close-project-button")).toBeVisible({ timeout: 10_000 });
  return page.locator("#wizard-modal");
}

function collectBrowserErrors(page: any): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error: Error) => errors.push(error.message));
  page.on("console", (message: any) => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
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
    (window as any).sentWizardActions = () =>
      (window as any).__sent
        .filter((message: any) => message && message.kind === "launch_wizard_action")
        .map((message: any) => message.action);
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
