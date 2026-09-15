/* Issue #3962 AC-5 — a saved launch profile pinned to a model that left the
 * agent's catalog (`gpt-5.4` after the 2026-09-05 Codex picker snapshot) falls
 * back to the current default model, and the wizard says so beside the Model
 * field instead of swapping the selection silently.
 *
 * Boots the embedded frontend with a deterministic WebSocket stub (no live gwt
 * process) and injects a `launch_wizard_state` through the `__gwt_test_inject`
 * seam, then drives the real DOM in Chromium for both colour schemes
 * (chromium-dark / chromium-light projects).
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const CODEX_MODEL_OPTIONS = [
  { value: "gpt-6-astra", label: "gpt-6-astra", description: "Our most capable model for complex, demanding work" },
  { value: "gpt-5.6-sol", label: "gpt-5.6-sol", description: "Reliable agentic workhorse for everyday tasks" },
  { value: "gpt-5.5", label: "gpt-5.5", description: "Proven previous-generation model for coding and general work" },
];

const FALLBACK_NOTICE = "Codex no longer offers gpt-5.4; using gpt-6-astra instead.";

const CODEX_WIZARD = {
  title: "Launch Agent",
  branch_name: "work/codex",
  selected_branch_name: "work/codex",
  branch_mode: "use_selected",
  show_back_button: false,
  show_branch_controls: false,
  show_manual_setup: true,
  show_runtime_confirmation: false,
  show_confirm: false,
  show_start_methods: false,
  show_agent_settings: true,
  runtime_context_resolved: true,
  primary_action_label: "Launch",
  primary_action_enabled: true,
  launch_summary: [],
  progress_steps: [],
  launch_target_options: [
    { value: "agent", label: "Agent", description: "Launch a coding agent terminal" },
    { value: "shell", label: "Shell", description: "Open a plain shell terminal" },
  ],
  selected_launch_target: "agent",
  agent_options: [{ value: "codex", label: "Codex", description: "Detected · 0.153.4" }],
  selected_agent_id: "codex",
  model_options: CODEX_MODEL_OPTIONS,
  selected_model: "gpt-6-astra",
  model_fallback_notice: FALLBACK_NOTICE,
};

test.describe("Launch Wizard — retired model fallback notice", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("names the dropped and the replacement model without blocking the launch", async ({
    page,
  }, testInfo) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, CODEX_WIZARD);
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/);

    // The Model select already shows the fallback row.
    const model = modal.getByRole("combobox", { name: "Model", exact: true });
    await expect(model).toHaveValue("gpt-6-astra");

    // AC-5: the swap is visible, and it is a hint — not the error banner, which
    // would read as a failed launch.
    await expect(modal.getByText(FALLBACK_NOTICE)).toBeVisible();
    await expect(page.locator("#wizard-error")).toBeHidden();
    await expect(modal.locator(".launch-note", { hasText: FALLBACK_NOTICE })).toHaveCount(1);

    // The launch stays available with the fallback model.
    await expect(modal.getByRole("button", { name: "Launch" })).toBeEnabled();

    // Choosing a model still dispatches normally; the backend clears the hint.
    await model.selectOption("gpt-5.6-sol");
    await expect
      .poll(() => page.evaluate(() => sentWizardActions()))
      .toContainEqual({ kind: "set_model", model: "gpt-5.6-sol" });

    await testInfo.attach(`model-fallback-${testInfo.project.name}`, {
      body: await modal.screenshot(),
      contentType: "image/png",
    });
    expect(browserErrors).toEqual([]);
  });

  test("renders nothing extra when no model was dropped", async ({ page }) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, { ...CODEX_WIZARD, model_fallback_notice: null });
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/);

    await expect(modal.getByRole("combobox", { name: "Model", exact: true })).toHaveValue(
      "gpt-6-astra",
    );
    await expect(modal.getByText("no longer offers")).toHaveCount(0);
    expect(browserErrors).toEqual([]);
  });
});

// Wizard actions travel as `{ kind: "launch_wizard_action", action }` envelopes.
declare function sentWizardActions(): unknown[];

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
