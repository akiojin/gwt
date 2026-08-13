/**
 * SPEC-1921 Phase 81 / Issue #3547 — deterministic embedded-browser contract
 * for a live manual-launch generation holder. The same scenario runs in the
 * configured chromium-dark and chromium-light projects.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import {
  gotoLiveGwt,
  openLiveGwtProject,
} from "./_helpers/live-gwt";

const LIVE_BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe("Launch Wizard generation conflict", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("offers fenced local recovery and fails closed for a remote holder", async ({
    page,
  }, testInfo) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    const failedRequests: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") {
        consoleErrors.push(`${message.text()} @ ${message.location().url}`);
      }
    });
    page.on("requestfailed", (request) => {
      failedRequests.push(`${request.url()} (${request.failure()?.errorText ?? "unknown"})`);
    });

    await page.addInitScript(() => {
      document.addEventListener(
        "DOMContentLoaded",
        () => {
          const icon = document.createElement("link");
          icon.rel = "icon";
          icon.href = "data:,";
          document.head.append(icon);
        },
        { once: true },
      );
    });
    await installEmbeddedRoutes(page);
    await installFixtureBackend(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);
    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });

    const expectedDark = testInfo.project.name.includes("dark");
    expect(
      await page.evaluate(() => matchMedia("(prefers-color-scheme: dark)").matches),
    ).toBe(expectedDark);

    await emitConflict(page, {
      holder_label: "Codex · stopped pane candidate",
      detail: "This work already has a live execution holder.",
      can_focus: true,
      can_stop_and_start: true,
    });

    const wizard = page.locator("#wizard-modal");
    const dialog = wizard.locator(".modal-shell.is-wizard");
    await expect(wizard).toHaveClass(/\bopen\b/);
    await expect(dialog).toHaveAttribute("role", "dialog");
    await expect(dialog).toHaveAttribute("aria-modal", "true");
    await expect(wizard.locator("#wizard-body")).toContainText(
      "Codex · stopped pane candidate",
    );
    await expect(wizard.locator("#wizard-body")).toContainText(
      "This work already has a live execution holder.",
    );

    const move = wizard.getByRole("button", { name: "Move to existing pane" });
    const stopAndStart = wizard.getByRole("button", {
      name: "Stop and start successor",
    });
    const cancel = wizard.getByRole("button", { name: "Cancel", exact: true });
    await expect(move).toBeEnabled();
    await expect(stopAndStart).toBeEnabled();
    await expect(wizard.locator("#wizard-submit-button")).toBeHidden();
    await expect(cancel).toBeFocused();

    // Two synchronous activations of the same detached DOM node exercise the
    // local latch itself (not merely the replacement button's disabled bit).
    await stopAndStart.evaluate((button: HTMLButtonElement) => {
      button.click();
      button.click();
    });
    await expect
      .poll(() => conflictActions(page))
      .toEqual(["stop_and_start_generation_successor"]);
    await expect(dialog).toHaveAttribute("aria-busy", "true");

    // A new backend view clears local pending state. Unknown/non-local holder
    // liveness must keep both authority-changing choices disabled.
    await emitConflict(page, {
      holder_label: "Remote agent",
      detail: "Holder liveness cannot be proven from this app instance.",
      can_focus: false,
      can_stop_and_start: false,
    });
    await expect(
      wizard.getByRole("button", { name: "Move to existing pane" }),
    ).toBeDisabled();
    await expect(
      wizard.getByRole("button", { name: "Stop and start successor" }),
    ).toBeDisabled();
    await expect(cancel).toBeFocused();

    await page.keyboard.press("Escape");
    await expect
      .poll(() => conflictActions(page))
      .toEqual(["stop_and_start_generation_successor", "cancel"]);
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
    expect(failedRequests).toEqual([]);
  });
});

test.describe("Launch Wizard generation conflict checkout smoke", () => {
  test.skip(!LIVE_BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test("loads the real backend wizard shell without browser errors", async ({
    page,
  }, testInfo) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    await page.addInitScript(() => {
      document.addEventListener(
        "DOMContentLoaded",
        () => {
          const icon = document.createElement("link");
          icon.rel = "icon";
          icon.href = "data:,";
          document.head.append(icon);
        },
        { once: true },
      );
    });

    await gotoLiveGwt(page, LIVE_BASE, { enableTestBridge: true });
    await openLiveGwtProject(page);
    expect(
      await page.evaluate(() => matchMedia("(prefers-color-scheme: dark)").matches),
    ).toBe(testInfo.project.name.includes("dark"));

    const wizard = page.locator("#wizard-modal");
    await expect(page.locator(".project-tab")).toBeVisible();
    await expect(wizard.locator(".modal-shell.is-wizard")).toHaveCount(1);
    await expect(wizard).toHaveAttribute("aria-hidden", "true");
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });
});

type ConflictView = {
  holder_label: string;
  detail: string;
  can_focus: boolean;
  can_stop_and_start: boolean;
};

async function conflictActions(page: any): Promise<string[]> {
  return page.evaluate(() =>
    ((window as any).__sent || [])
      .filter((message: any) => message.kind === "launch_wizard_action")
      .map((message: any) => message.action?.kind),
  );
}

async function emitConflict(page: any, generationConflict: ConflictView): Promise<void> {
  await page.evaluate((conflict) => {
    (window as any).__emitBackend({
      kind: "launch_wizard_state",
      wizard: {
        title: "Launch Agent",
        mode: "branch",
        branch_name: "work/issue-3547",
        selected_branch_name: "work/issue-3547",
        linked_issue_number: 3547,
        is_hydrating: false,
        runtime_context_resolved: true,
        hydration_error: null,
        start_methods: [],
        quick_start_entries: [],
        live_sessions: [],
        selected_launch_path: "manual_setup",
        selected_quick_start_index: null,
        selected_live_session_index: null,
        branch_mode: "use_selected",
        branch_type_options: [],
        selected_branch_type: null,
        launch_target_options: [],
        selected_launch_target: "agent",
        agent_options: [],
        selected_agent_id: "codex",
        model_options: [],
        selected_model: "gpt-5.6-sol",
        reasoning_options: [],
        selected_reasoning: "high",
        runtime_target_options: [],
        selected_runtime_target: "host",
        windows_shell_options: [],
        selected_windows_shell: null,
        docker_service_options: [],
        selected_docker_service: null,
        docker_lifecycle_options: [],
        selected_docker_lifecycle: "keep",
        version_options: [],
        selected_version: "latest",
        execution_mode_options: [],
        selected_execution_mode: "normal",
        skip_permissions: false,
        show_agent_settings: false,
        show_reasoning: false,
        show_runtime_target: false,
        show_windows_shell: false,
        show_docker_service: false,
        show_docker_lifecycle: false,
        show_version: false,
        show_execution_mode: false,
        show_skip_permissions: false,
        show_fast_mode: false,
        show_codex_fast_mode: false,
        show_hermes_options: false,
        hermes_needs_setup: false,
        show_opencode_options: false,
        opencode_needs_setup: false,
        hermes_provider: "",
        hermes_provider_options: [],
        hermes_profile: "",
        hermes_toolsets: "",
        hermes_skills: "",
        hermes_max_turns: "",
        hermes_safe_mode: false,
        show_branch_controls: false,
        show_start_methods: false,
        show_back_button: false,
        show_manual_setup: false,
        show_runtime_confirmation: false,
        show_confirm: false,
        show_linked_issue: true,
        runtime_resolution_pending: false,
        runtime_resolution_message: null,
        launch_materialization_pending: false,
        launch_materialization_message: null,
        primary_action_label: "Launch",
        primary_action_enabled: false,
        progress_steps: [],
        fast_mode: false,
        codex_fast_mode: false,
        launch_summary: [],
        phase: "confirm",
        error: null,
        generation_conflict: conflict,
      },
    });
  }, generationConflict);
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

async function installFixtureBackend(page: any): Promise<void> {
  await page.addInitScript(() => {
    try {
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    } catch {
      /* no-op */
    }
    (window as any).__sent = [];

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
      readyState = FixtureWebSocket.CONNECTING;
      url: string;

      constructor(url: string) {
        super();
        this.url = url;
        (window as any).__emitBackend = (payload: unknown) => this.emit(payload);
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
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
        if (message.kind === "frontend_ready") this.emit(workspaceState);
      }

      close(): void {
        this.readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown): void {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
