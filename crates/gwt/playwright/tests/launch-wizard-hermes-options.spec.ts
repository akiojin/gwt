/* Issue #3863 — Hermes launch options (Provider / Model / Profile / Toolsets /
 * Skills) render as config-sourced pickers with an "Other…" free-text
 * fallback, and degrade to "config default + Other" when no candidates exist.
 *
 * Boots the embedded frontend with a deterministic WebSocket stub (no live
 * gwt process) and injects a `launch_wizard_state` for the Hermes agent via
 * the `__gwt_test_inject` seam, then drives the real DOM in Chromium for both
 * colour schemes (chromium-dark / chromium-light projects).
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const HERMES_WIZARD = {
  title: "Launch Agent",
  branch_name: "work/hermes",
  selected_branch_name: "work/hermes",
  branch_mode: "use_selected",
  show_back_button: false,
  show_branch_controls: false,
  show_manual_setup: true,
  show_runtime_confirmation: false,
  show_confirm: false,
  show_start_methods: false,
  show_agent_settings: false,
  runtime_context_resolved: true,
  primary_action_label: "Launch",
  primary_action_enabled: true,
  launch_summary: [],
  progress_steps: [],
  agent_options: [],
  selected_agent_id: "hermes",
  selected_model: "",
  show_hermes_options: true,
  hermes_needs_setup: false,
  hermes_provider: "",
  hermes_provider_options: ["zai", "ollama-launch"],
  hermes_model_options: ["glm-5.2"],
  hermes_profile: "pirate",
  hermes_profile_options: ["concise", "pirate"],
  hermes_toolsets: "web,custom-x",
  hermes_toolset_options: ["terminal", "web"],
  hermes_skills: "",
  hermes_skill_options: ["github"],
  hermes_max_turns: "",
  hermes_safe_mode: false,
};

test.describe("Launch Wizard — Hermes options", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("pickers render from config choices and dispatch selections", async ({
    page,
  }, testInfo) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, HERMES_WIZARD);
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/);

    // AC-1: Model candidates come from providers.<id>.models of the selected
    // (here: config default) provider, in the 3-tier default / choice / Other layout.
    const model = modal.getByRole("combobox", { name: "Model", exact: true });
    await expect(model).toHaveValue("");
    await expect(model.locator("option")).toHaveText([
      "(use config default)",
      "glm-5.2",
      "Other…",
    ]);
    await model.selectOption("glm-5.2");
    await expect
      .poll(() => page.evaluate(() => sentWizardActions()))
      .toContainEqual({ kind: "set_model", model: "glm-5.2" });

    // AC-2 / AC-7: Profile candidates come from agent.personalities and the
    // restored previous value is preselected.
    const profile = modal.getByRole("combobox", { name: "Profile", exact: true });
    await expect(profile).toHaveValue("pirate");
    await expect(profile.locator("option")).toHaveText([
      "(use config default)",
      "concise",
      "pirate",
      "Other…",
    ]);

    // AC-4: Other… reveals the free-text input; a custom value still reaches
    // the backend through the same action.
    const customModel = modal.getByRole("textbox", { name: "Custom model" });
    await expect(customModel).toBeHidden();
    await model.selectOption("__other__");
    await expect(customModel).toBeVisible();
    await customModel.fill("anthropic/claude-sonnet-4");
    await customModel.press("Enter");
    await expect
      .poll(() => page.evaluate(() => sentWizardActions()))
      .toContainEqual({ kind: "set_model", model: "anthropic/claude-sonnet-4" });

    // AC-3: Toolsets / Skills are multi-choice; the CSV state is split into
    // checked candidates plus an Other input for values missing from config.
    const web = modal.getByRole("checkbox", { name: "Toolsets: web" });
    const terminal = modal.getByRole("checkbox", { name: "Toolsets: terminal" });
    await expect(web).toBeChecked();
    await expect(terminal).not.toBeChecked();
    const otherToolsets = modal.getByRole("textbox", { name: "Other toolsets" });
    await expect(otherToolsets).toHaveValue("custom-x");
    await terminal.check();
    await expect
      .poll(() => page.evaluate(() => sentWizardActions()))
      .toContainEqual({
        kind: "set_hermes_option",
        field: "toolsets",
        value: "terminal,web,custom-x",
      });
    await expect(modal.getByRole("checkbox", { name: "Skills: github" })).not.toBeChecked();

    await testInfo.attach(`hermes-options-${testInfo.project.name}`, {
      body: await modal.screenshot(),
      contentType: "image/png",
    });
    expect(browserErrors).toEqual([]);
  });

  test("degrades to config default + Other when no candidates exist (AC-5)", async ({
    page,
  }) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, {
      ...HERMES_WIZARD,
      hermes_profile: "",
      hermes_toolsets: "fs,web",
      hermes_provider_options: [],
      hermes_model_options: [],
      hermes_profile_options: [],
      hermes_toolset_options: [],
      hermes_skill_options: [],
    });
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/);

    for (const label of ["Provider", "Model", "Profile"]) {
      const select = modal.getByRole("combobox", { name: label, exact: true });
      await expect(select.locator("option")).toHaveText(["(use config default)", "Other…"]);
      await expect(select).toHaveValue("");
    }
    // Multi-choice fields keep only the CSV input, pre-filled with the
    // current value so nothing typed before is lost.
    await expect(modal.getByRole("checkbox", { name: /^Toolsets:/ })).toHaveCount(0);
    await expect(modal.getByRole("textbox", { name: "Other toolsets" })).toHaveValue("fs,web");
    await expect(modal.getByRole("textbox", { name: "Other skills" })).toHaveValue("");
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
