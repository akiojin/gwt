import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt } from "./_helpers/live-gwt";

test("live wizard open errors retain interaction deferral and local recovery", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
  test.skip(!base, "isolated checkout backend required");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  await page.addInitScript(() => {
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      if (typeof data === "string" && JSON.parse(data).kind === "launch_wizard_action") return;
      return send.call(this, data);
    };
  });
  await gotoLiveGwt(page, base, { enableTestBridge: true });
  await expect(page.locator("html")).toHaveAttribute("data-theme",
    testInfo.project.name.includes("light") ? "light" : "dark");
  const trigger = page.locator("#window-list-button");
  await expect(trigger).toBeEnabled();
  await trigger.focus();
  await expect(trigger).toBeFocused();
  const emit = async (detail: unknown) => page.evaluate(detail => {
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail }));
  }, detail);
  const wizard = {
    title: "Choose Agent", show_manual_setup: true, show_branch_controls: false,
    show_agent_settings: true, runtime_context_resolved: true,
    launch_summary: [], progress_steps: [], selected_launch_target: "agent",
    launch_target_options: [{ label: "Agent", value: "agent" }],
    agent_options: [{ label: "Codex", value: "codex" }], selected_agent_id: "codex",
    model_options: [{ label: "Model A", value: "model-a" }], selected_model: "model-a",
  };
  await emit({ kind: "launch_wizard_state", wizard });
  const modal = page.locator("#wizard-modal");
  const model = modal.getByRole("combobox", { name: "Model", exact: true });
  await expect(model).toBeEnabled();
  await model.focus();
  await model.evaluate(element => { (window as any).__wizardModel = element; });
  // Exercise the real delegated interaction guard without opening an OS menu.
  await model.dispatchEvent("pointerdown");
  await emit({ kind: "launch_wizard_open_error", title: "Superseded error", message: "Old failure" });
  const error = { kind: "launch_wizard_open_error", title: "Open failed", message: "<img src=x onerror=alert(1)>" };
  await emit(error);
  await expect(modal.locator("#wizard-title")).toHaveText("Choose Agent");
  await expect(model).toBeFocused();
  expect(await model.evaluate(element => element === (window as any).__wizardModel)).toBe(true);
  await model.dispatchEvent("change");
  await expect(modal.locator("#wizard-title")).toHaveText("Open failed");
  await expect(modal.locator("#wizard-error")).toHaveText(error.message);
  await expect(modal.locator("img")).toHaveCount(0);
  await expect(modal).not.toContainText("Old failure");
  await emit({ kind: "launch_wizard_state", wizard: null });
  await expect(modal.locator("#wizard-error")).toHaveText(error.message);
  await testInfo.attach("wizard-open-error", {
    body: await modal.screenshot({ path: testInfo.outputPath("wizard-open-error.png") }),
    contentType: "image/png",
  });
  await page.keyboard.press("Escape");
  await expect(modal).toBeHidden();
  await expect(trigger).toBeFocused();
  // A distinct receipt with identical fields must still open after local close.
  await emit(error);
  await expect(modal).toBeVisible();
  await expect(modal.locator("#wizard-error")).toHaveText(error.message);
  await emit({ kind: "launch_wizard_state", wizard });
  await expect(modal.locator("#wizard-title")).toHaveText("Choose Agent");
  await expect(modal.locator("#wizard-error")).toBeHidden();
  await emit({ kind: "launch_wizard_state", wizard: null });
  await expect(modal).toBeHidden();
  await expect(trigger).toBeFocused();
  expect(errors, "console and page errors").toEqual([]);
});

test("live work advisory results preserve the prompt and ignore stale responses", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
  test.skip(!base, "isolated checkout backend required");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  // Keep the real transport and receiver; isolate only the advisory query and
  // prompt acknowledgement so backend timing cannot replace the injected view.
  await page.addInitScript(() => {
    (window as any).__advisoryRequests = [];
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      if (typeof data === "string") {
        const message = JSON.parse(data);
        if (message.kind === "request_work_advisory") {
          (window as any).__advisoryRequests.push(message);
          return;
        }
        if (message.kind === "launch_wizard_action") return;
      }
      return send.call(this, data);
    };
  });
  await gotoLiveGwt(page, base, { enableTestBridge: true });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.locator("#window-list-button")).toBeEnabled();
  await page.evaluate(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
    detail: { kind: "launch_wizard_state", wizard: {
      title: "Register work", show_branch_controls: false,
      show_manual_setup: false, show_runtime_confirmation: false,
      show_confirm: false, show_start_methods: false,
      runtime_context_resolved: true, launch_summary: [], progress_steps: [],
    } },
  })));
  const modal = page.locator("#wizard-modal");
  await expect(modal).toBeVisible();
  const prompt = modal.locator("textarea.launch-intake-input");
  await prompt.fill("Investigate shared state rendering");
  await prompt.evaluate((element: HTMLTextAreaElement) => {
    element.setSelectionRange(4, 11);
    (window as any).__advisoryPrompt = element;
  });
  const panel = modal.locator("#wizard-work-advisory");
  await expect(panel).toContainText("Searching related work");
  await expect.poll(() => page.evaluate(() =>
    (window as any).__advisoryRequests.at(-1)?.request_id ?? 0,
  )).toBeGreaterThan(0);
  const reply = async (requestId: number, results: unknown[]) => page.evaluate(({ requestId, results }) => {
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "work_advisory_result", request_id: requestId, results },
    }));
  }, { requestId, results });
  await reply(10, [{ scope: "specs", title: "<img src=x onerror=alert(1)>", subtitle: "Shared state" }]);
  await expect(panel).toContainText("<img src=x onerror=alert(1)>");
  await expect(panel.locator("img")).toHaveCount(0);
  await expect(panel).not.toContainText("Searching related work");
  await reply(11, [{ scope: "issues", title: "Latest related work" }]);
  await expect(panel).toContainText("Latest related work");
  // The debounced query has advanced the real request fence beyond request 0.
  await reply(0, [{ title: "Stale response" }]);
  await expect(panel).toContainText("Latest related work");
  await expect(panel).not.toContainText("Stale response");
  await expect(prompt).toBeFocused();
  await expect(prompt).toHaveValue("Investigate shared state rendering");
  expect(await prompt.evaluate((element: HTMLTextAreaElement) => [
    element === (window as any).__advisoryPrompt, element.selectionStart, element.selectionEnd,
  ])).toEqual([true, 4, 11]);
  await testInfo.attach("work-advisory-results", { body: await modal.screenshot(), contentType: "image/png" });
  await reply(12, []);
  await expect(panel).toBeHidden();
  await expect(prompt).toBeFocused();
  expect(errors, "console and page errors").toEqual([]);
});

test.describe("Launch Wizard states", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("deprecated Intake command has no rail entry, pending wizard, or wire", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    page.on("pageerror", (error) => {
      pageErrors.push(error.message);
    });

    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);

    await expect(page.locator("#close-project-button")).toBeVisible({
      timeout: 10_000,
    });

    await expect
      .soft(page.locator('.op-rail__item[data-cmd="intake-session"]'))
      .toHaveCount(0);

    await page.evaluate(async () => {
      document.dispatchEvent(
        new CustomEvent("op:command", { detail: { id: "intake-session" } }),
      );
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    });

    const wizard = page.locator("#wizard-modal");
    await expect.soft(wizard).not.toHaveClass(/open/);
    await expect.soft(wizard).toHaveAttribute("aria-hidden", "true");
    await expect.soft(wizard.locator(".launch-pending-note")).toHaveCount(0);
    expect.soft(await page.evaluate(() => (window as any).__sentKinds)).not.toContain(
      "open_intake_session",
    );
    expect(pageErrors).toEqual([]);
  });

  test("exact holder decision renders safe actions without browser errors", async ({
    page,
  }, testInfo) => {
    const browserErrors: string[] = [];
    page.on("pageerror", (error) => {
      browserErrors.push(error.message);
    });
    page.on("console", (message) => {
      if (message.type() === "error") browserErrors.push(message.text());
    });

    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);
    await keepLaunchWizardModalVisible(page);

    await page.evaluate(() => {
      window.dispatchEvent(
        new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "launch_wizard_state",
            wizard: {
              title: "Launch Agent",
              branch_name: "work/holder-decision",
              selected_branch_name: "work/holder-decision",
              branch_mode: "use_selected",
              show_back_button: true,
              show_branch_controls: true,
              show_manual_setup: false,
              show_runtime_confirmation: false,
              show_confirm: false,
              show_start_methods: false,
              runtime_context_resolved: true,
              primary_action_enabled: false,
              launch_summary: [],
              progress_steps: [],
              holder_decision: {
                fingerprint: "fp-live-holder",
                holder_session_id: "session-live-holder",
                holder_window_id: "window-live-holder",
                holder_summary: "Codex · local active holder",
                stop_available: true,
                stop_unavailable_reason: null,
                move_available: true,
                move_unavailable_reason: null,
              },
            },
          },
        }),
      );
    });

    const modal = page.locator("#wizard-modal");
    const dialog = modal.getByRole("dialog");
    await expect(modal).toHaveClass(/open/);
    await expect(dialog).toHaveAttribute("aria-modal", "true");
    await expect(dialog).toHaveAttribute("aria-labelledby", "wizard-title");
    await expect(modal.locator(".modal-header")).toBeVisible();
    await expect(modal.locator("#wizard-body")).toContainText(
      "Codex · local active holder",
    );

    const actions = modal.locator(".wizard-actions > button:visible");
    await expect(actions).toHaveText([
      "Move existing pane",
      "Cancel",
      "Stop and start successor",
    ]);
    await expect(actions.nth(0)).toBeEnabled();
    await expect(actions.nth(2)).toBeEnabled();
    await expect(actions.nth(2)).toHaveClass(/destructive/);
    await expect(actions.nth(0)).not.toHaveClass(/destructive/);

    await testInfo.attach("holder-theme", {
      body: Buffer.from(testInfo.project.name),
      contentType: "text/plain",
    });
    expect(browserErrors).toEqual([]);
  });
});

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
    (window as any).__sentKinds = [];

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
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [],
            },
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
        (window as any).__sentKinds.push(message.kind);
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
