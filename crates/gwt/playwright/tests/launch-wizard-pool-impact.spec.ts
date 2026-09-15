/* Issue #4079 AC-2 — the Issue Monitor Agent Settings form always writes
 * candidate 1 of the launch candidate pool, so it must say which candidate that
 * replaces and what the pool summary becomes before the operator commits. The
 * reported symptom was the opposite: switching to Codex looked like a no-op
 * because the save had rewritten a lower candidate and left Claude launching.
 *
 * Boots the embedded frontend with a deterministic WebSocket stub (no live gwt
 * process) and injects a `launch_wizard_state` through the `__gwt_test_inject`
 * seam, then drives the real DOM in Chromium for both colour schemes
 * (chromium-dark / chromium-light projects).
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const RESULTING_SUMMARY = "codex / gpt-6-astra / high / host";

const POOL_IMPACT = {
  action: "replace_head",
  agent_id: "codex",
  replaced_agent_id: "claude",
  title: "Replaces candidate 1 (claude)",
  detail:
    "Agent Settings always writes candidate 1, the agent Issue Monitor launches first. " +
    `After saving: ${RESULTING_SUMMARY}`,
  resulting_summary: RESULTING_SUMMARY,
};

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
  runtime_context_resolved: true,
  primary_action_label: "Save settings",
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
  model_options: [{ value: "gpt-6-astra", label: "gpt-6-astra", description: "" }],
  selected_model: "gpt-6-astra",
  issue_monitor_pool_impact: POOL_IMPACT,
};

test.describe("Launch Wizard — Issue Monitor candidate pool impact", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("names the replaced candidate and the resulting pool summary", async ({
    page,
  }, testInfo) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, SETTINGS_WIZARD);
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/);

    const note = modal.locator(".launch-pool-impact");
    await expect(note).toHaveCount(1);
    await expect(note).toBeVisible();
    // AC-2: which candidate is written, and what the pool becomes.
    await expect(note).toContainText("Replaces candidate 1 (claude)");
    await expect(note).toContainText(RESULTING_SUMMARY);
    await expect(note).toHaveAttribute("data-pool-action", "replace_head");
    await expect(note).toHaveAttribute("data-agent-id", "codex");

    // It is a hint, not the error banner: the save stays available.
    await expect(page.locator("#wizard-error")).toBeHidden();
    await expect(modal.getByRole("button", { name: "Save settings" })).toBeEnabled();

    await testInfo.attach(`pool-impact-${testInfo.project.name}`, {
      body: await modal.screenshot(),
      contentType: "image/png",
    });
    expect(browserErrors).toEqual([]);
  });

  test("renders nothing extra for an ordinary launch wizard", async ({ page }) => {
    const browserErrors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    await injectWizard(page, {
      ...SETTINGS_WIZARD,
      title: "Launch Agent",
      primary_action_label: "Launch",
      issue_monitor_pool_impact: null,
    });
    const modal = page.locator("#wizard-modal");
    await expect(modal).toHaveClass(/open/);

    await expect(modal.locator(".launch-pool-impact")).toHaveCount(0);
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
