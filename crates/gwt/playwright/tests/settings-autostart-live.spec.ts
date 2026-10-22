/** Settings shared-state replies through the isolated live backend. */
import { expect, test, type WebSocketRoute } from "@playwright/test";
import { gotoLiveGwt, openLiveGwtProject, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? process.cwd();

test.describe("Autostart Settings (isolated live backend)", () => {
  test.skip(!BASE, "requires browser-check checkout and fresh HOME");
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(120_000);

  test.afterEach(async ({ page }, info) => {
    const received = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .map(({ sequence, payload }: any) => ({ sequence, kind: payload.kind })));
    await info.attach("settings-backend-replies", {
      body: JSON.stringify(received), contentType: "application/json",
    });
  });

  test("Update status batches bursts while toggles preserve immediate rollback", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
      let transport: WebSocketRoute | undefined;
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        transport = socket;
        const server = socket.connectToServer();
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (!["get_update_settings", "set_auto_apply_updates"].includes(payload?.kind)) server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (!["update_settings", "issue_settings_status", "system_settings_error"].includes(payload?.kind)) socket.send(message);
        });
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true, suppressUpdateApplyStart: true });
      await openLiveGwtProject(page, PROJECT);
      await page.bringToFront();
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "system" } })));
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const panel = settings.locator("[data-update-settings-status]");
      const toggle = settings.getByLabel("Auto-apply updates", { exact: true });
      const status = { auto_apply_updates: true, auto_apply_updates_effective: true, update_drain: null };
      transport!.send(JSON.stringify({ kind: "update_settings", project_root: PROJECT, status }));
      await expect(panel).toContainText("Saved: On · Effective: On");
      await expect(toggle).toBeChecked();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await toggle.focus();
      await toggle.hover();
      const burst = await panel.evaluate(async (panel, { status, root }) => {
        const before = panel.innerHTML;
        const focused = document.activeElement;
        let renders = 0;
        const observer = new MutationObserver(records => {
          renders += records.filter(record => record.type === "childList" && record.removedNodes.length > 0).length;
        });
        observer.observe(panel, { childList: true });
        try {
          for (let index = 1; index <= 50; index++) {
            window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
              kind: "update_settings", project_root: root, status: { ...status,
                update_drain: { version: `fixture-${index}`, blocking: [] },
              },
            } }));
          }
          const unchanged = panel.innerHTML === before;
          (focused as HTMLInputElement).checked = false;
          window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
            kind: "system_settings_error", message: "Fixture save denied",
          } }));
          const rollback = (focused as HTMLInputElement).checked;
          await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          return { unchanged, rollback, renders, text: panel.textContent,
            focusPreserved: document.activeElement === focused, hoverPreserved: focused?.matches(":hover") };
        } finally { observer.disconnect(); }
      }, { status, root: PROJECT });
      expect(burst).toMatchObject({ unchanged: true, rollback: true, renders: 1, focusPreserved: true, hoverPreserved: true });
      expect(burst.text).toContain("fixture-50");
      await info.attach("update-status-frame-burst", { body: JSON.stringify(burst), contentType: "application/json" });
      await page.evaluate(({ status, root }) => {
        (window as any).__updateStatusRaf = window.requestAnimationFrame;
        window.requestAnimationFrame = () => 0;
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
          kind: "update_settings", project_root: root, status: { ...status,
            update_drain: { version: "fallback", blocking: [] },
          },
        } }));
      }, { status, root: PROJECT });
      try {
        await expect(panel).toContainText("fallback", { timeout: 1000 });
      } finally {
        await page.evaluate(() => { window.requestAnimationFrame = (window as any).__updateStatusRaf; });
      }
      await info.attach(`update-status-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
      expect(errors, "zero console/page errors through update status burst and rollback").toEqual([]);
    });
  });

  test("Agent Backend snapshots preserve per-agent lists, updates, errors and reopen", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      const requests: any[] = [];
      let transport: WebSocketRoute | undefined;
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        transport = socket;
        const server = socket.connectToServer();
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind === "list_agent_backends") requests.push(payload);
          else server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (!/^agent_backend_(list|saved|deleted|error)$/.test(payload?.kind ?? "")) socket.send(message);
        });
      });
      const reply = (payload: object) => transport!.send(JSON.stringify(payload));
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page, PROJECT);
      await page.bringToFront();
      const open = () => page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "agent-backends" } })));
      await open();
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const panel = settings.locator('[data-settings-panel="agent-backends"]');
      await expect.poll(() => requests.some(request => request.agent === "claudeCode") && requests.some(request => request.agent === "codex")).toBe(true);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const profile = { id: "fixture-backend", display_name: "Fixture backend", base_url: "https://example.invalid", model: "fixture-model" };
      reply({ kind: "agent_backend_list", agent: "claudeCode", backends: [profile] });
      reply({ kind: "agent_backend_list", agent: "codex", backends: [{ ...profile, id: "codex-fixture", display_name: "Codex fixture" }] });
      await expect(panel).toContainText("Fixture backend");
      await expect(panel).toContainText("Codex fixture");
      const tab = settings.getByRole("tab", { name: "Agent Backends", exact: true });
      await tab.focus();
      await tab.hover();
      const burst = await panel.evaluate(async (panel, profile) => {
        const before = panel.innerHTML;
        const focused = document.activeElement;
        let renders = 0;
        const observer = new MutationObserver(records => {
          renders += records.flatMap(record => Array.from(record.addedNodes))
            .filter(node => node instanceof HTMLElement && node.dataset.agent === "claudeCode").length;
        });
        observer.observe(panel, { childList: true });
        try {
          for (let index = 1; index <= 50; index++) {
            window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
              kind: "agent_backend_list", agent: "claudeCode",
              backends: [{ ...profile, display_name: `Backend ${index}` }],
            } }));
          }
          window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
            kind: "agent_backend_list", agent: "codex",
            backends: [{ ...profile, id: "codex-fixture", display_name: "Codex fixture", model: "codex-latest" }],
          } }));
          const unchanged = panel.innerHTML === before;
          await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          return { unchanged, renders, text: panel.textContent, focusPreserved: document.activeElement === focused,
            hoverPreserved: focused?.matches(":hover") };
        } finally { observer.disconnect(); }
      }, profile);
      expect(burst.unchanged).toBe(true);
      expect(burst.renders).toBe(1);
      expect(burst.text).toContain("Backend 50");
      expect(burst.text).toContain("codex-latest");
      expect(burst.focusPreserved).toBe(true);
      expect(burst.hoverPreserved).toBe(true);
      await info.attach("agent-backends-frame-burst", { body: JSON.stringify(burst), contentType: "application/json" });
      await page.evaluate(profile => {
        (window as any).__backendFrameRaf = window.requestAnimationFrame;
        window.requestAnimationFrame = () => 0;
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
          kind: "agent_backend_list", agent: "claudeCode",
          backends: [{ ...profile, display_name: "Fallback backend" }],
        } }));
      }, profile);
      try {
        await expect(panel).toContainText("Fallback backend", { timeout: 1000 });
      } finally {
        await page.evaluate(() => { window.requestAnimationFrame = (window as any).__backendFrameRaf; });
      }
      reply({ kind: "agent_backend_saved", agent: "claudeCode", profile: { ...profile, display_name: "Updated backend" } });
      await expect(panel).toContainText("Updated backend");
      await expect(panel.locator('[data-agent="claudeCode"] .agent-backend-row')).toHaveCount(1);
      reply({ kind: "agent_backend_deleted", agent: "claudeCode", id: profile.id });
      await expect(panel).toContainText("No Claude Code backend profiles saved.");
      await expect(panel).toContainText("Codex fixture");
      reply({ kind: "agent_backend_error", message: "Fixture backend unavailable" });
      await expect(settings.locator('[data-settings-panel="custom-agents"]')).toContainText("Fixture backend unavailable");
      const id = await settings.getAttribute("data-id");
      expect(id).toBeTruthy();
      await page.locator('.op-rail__item[data-surface="agents"]').click();
      await sendLiveGwtEvent(page, { kind: "close_window", id });
      await expect(settings).toHaveCount(0, { timeout: 30_000 });
      const reads = requests.length;
      await open();
      await expect.poll(() => requests.length).toBeGreaterThan(reads);
      await expect(panel).toContainText("Codex fixture");
      await expect(panel).toContainText("No Claude Code backend profiles saved.");
      await info.attach(`agent-backends-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
      expect(errors).toEqual([]);
    });
  });

  test("Custom Agent snapshots render replies and preserve environment save and delete contracts", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      const requests: any[] = [];
      let transport: WebSocketRoute | undefined;
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        transport = socket;
        const server = socket.connectToServer();
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          // Exercise the receiver and controls without persisting fixture agents.
          if (["list_custom_agents", "update_custom_agent", "delete_custom_agent"].includes(payload?.kind)) requests.push(payload);
          else server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (!/^custom_agent_(list|saved|deleted|error)$/.test(payload?.kind ?? "")) socket.send(message);
        });
      });
      const reply = (payload: object) => transport!.send(JSON.stringify(payload));
      const agent = { id: "fixture-cli", display_name: "Fixture CLI", command: "fixture-command",
        default_args: ["--fixture"], mode_args: { normal: [], continue: ["--continue"], resume: ["--resume"] },
        skip_permissions_args: [], env: { FIXTURE_TOKEN: "***REDACTED***" } };
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page, PROJECT);
      await page.bringToFront();
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "custom-agents" } })));
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const panel = settings.locator('[data-settings-panel="custom-agents"]');
      await expect.poll(() => requests.some(request => request.kind === "list_custom_agents")).toBe(true);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(panel).toContainText("Loading custom agents…");
      reply({ kind: "custom_agent_list", agents: [agent] });
      await expect(panel).toContainText("Fixture CLI");
      const tab = settings.getByRole("tab", { name: "Custom Agents", exact: true });
      await tab.focus();
      await tab.hover();
      const burst = await panel.evaluate(async (panel, agent) => {
        const before = panel.innerHTML;
        const focused = document.activeElement;
        let renders = 0;
        const observer = new MutationObserver(records => {
          renders += records.flatMap(record => Array.from(record.addedNodes))
            .filter(node => node instanceof HTMLButtonElement && node.textContent?.startsWith("＋ Add Claude Code backend")).length;
        });
        observer.observe(panel, { childList: true });
        try {
          for (let index = 1; index <= 50; index++) {
            window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
              kind: "custom_agent_list", agents: [{ ...agent, display_name: `Custom Agent ${index}` }],
            } }));
          }
          const unchanged = panel.innerHTML === before;
          await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
          return { unchanged, renders, text: panel.textContent, focusPreserved: document.activeElement === focused,
            hoverPreserved: focused?.matches(":hover") };
        } finally { observer.disconnect(); }
      }, agent);
      expect(burst).toMatchObject({ unchanged: true, renders: 1, focusPreserved: true, hoverPreserved: true });
      expect(burst.text).toContain("Custom Agent 50");
      await info.attach("custom-agents-frame-burst", { body: JSON.stringify(burst), contentType: "application/json" });
      await info.attach(`custom-agents-frame-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
      await page.evaluate(agent => {
        (window as any).__customAgentsFrameRaf = window.requestAnimationFrame;
        window.requestAnimationFrame = () => 0;
        (window as any).__customAgentsFrameStart = performance.now();
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
          kind: "custom_agent_list", agents: [{ ...agent, display_name: "Fallback custom agent" }],
        } }));
      }, agent);
      try {
        await expect(panel).toContainText("Fallback custom agent", { timeout: 1000 });
        const elapsed = await page.evaluate(() => performance.now() - (window as any).__customAgentsFrameStart);
        expect(elapsed).toBeLessThan(1000);
        await info.attach("custom-agents-frame-fallback", { body: JSON.stringify({ elapsed }), contentType: "application/json" });
      } finally {
        await page.evaluate(() => { window.requestAnimationFrame = (window as any).__customAgentsFrameRaf; });
      }
      await panel.getByLabel("Edit agent environment", { exact: true }).click();
      await expect(panel.getByLabel("Environment value", { exact: true })).toHaveValue("***REDACTED***");
      await panel.getByLabel("Environment value", { exact: true }).fill("fixture-updated-value");
      const savedEditor = await panel.getByLabel("Save environment changes", { exact: true }).evaluate(async button => {
        const panel = button.closest('[data-settings-panel="custom-agents"]')!;
        button.click();
        const closed = !button.isConnected;
        (panel.querySelector('[aria-label="Edit agent environment"]') as HTMLButtonElement).click();
        const draft = panel.querySelector('[aria-label="Environment value"]') as HTMLInputElement;
        draft.value = "fresh-unsaved-draft";
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
        return { closed, draftPreserved: draft.isConnected && draft.value === "fresh-unsaved-draft" };
      });
      expect(savedEditor, "save closes synchronously and its pending frame preserves a reopened draft")
        .toEqual({ closed: true, draftPreserved: true });
      await panel.getByLabel("Cancel environment editing", { exact: true }).click();
      await expect(panel).toContainText("Saving custom agent…");
      await expect.poll(() => requests.filter(request => request.kind === "update_custom_agent").length).toBe(1);
      expect(requests.find(request => request.kind === "update_custom_agent").agent).toEqual({ ...agent,
        display_name: "Fallback custom agent", env: { FIXTURE_TOKEN: "fixture-updated-value" } });
      reply({ kind: "custom_agent_saved", agent: { ...agent, display_name: "Updated CLI" } });
      await expect(panel).toContainText("Updated CLI");
      await expect(panel).toContainText('Saved custom agent "fixture-cli".');
      reply({ kind: "backend_connection_result", models: ["fixture-model"] });
      await expect(panel).toContainText("/v1/models returned 1 model(s).");
      reply({ kind: "custom_agent_error", code: "invalid", message: "Fixture rejected" });
      await expect(panel).toContainText("Error [invalid]: Fixture rejected");
      page.once("dialog", dialog => dialog.accept());
      await panel.getByLabel("Delete agent", { exact: true }).click();
      await expect.poll(() => requests.some(request => request.kind === "delete_custom_agent" && request.agent_id === agent.id)).toBe(true);
      reply({ kind: "custom_agent_deleted", agent_id: agent.id });
      await expect(panel).toContainText("No custom agents configured yet.");
      await expect(panel).toContainText('Deleted custom agent "fixture-cli".');
      const id = await settings.getAttribute("data-id");
      expect(id).toBeTruthy();
      await page.locator('.op-rail__item[data-surface="agents"]').click();
      await sendLiveGwtEvent(page, { kind: "close_window", id });
      await expect(settings).toHaveCount(0, { timeout: 30_000 });
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "custom-agents" } })));
      reply({ kind: "custom_agent_list", agents: [agent] });
      await expect(panel).toContainText("Fixture CLI");
      expect(requests.filter(request => ["update_custom_agent", "delete_custom_agent"].includes(request.kind))).toHaveLength(2);
      await info.attach(`custom-agents-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
      expect(errors).toEqual([]);
    });
  });

  test("System snapshots preserve guarded controls, drafts and resource update contracts", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      const requests: any[] = [];
      let transport: WebSocketRoute | undefined;
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        transport = socket;
        const server = socket.connectToServer();
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          // Control System replies without changing real hooks or process policy.
          if (["get_system_settings", "update_system_settings"].includes(payload?.kind)) requests.push(payload);
          else if (payload?.kind !== "get_autostart_status") server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          // The explicit autostart reply below must not race a real status
          // receipt that replaces the entire System panel during the burst probe.
          if (!/^(?:system_settings(?:_updated|_error)?|autostart_status|autostart_error)$/.test(payload?.kind ?? "")) socket.send(message);
        });
      });
      const resource = { enabled: true, preset: "custom", priority: "idle", cpu_limit_percent: 35, build_jobs: 2 };
      const reply = (payload: object) => transport!.send(JSON.stringify(payload));
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page, PROJECT);
      await page.bringToFront();
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "system" } })));
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const panel = settings.locator('[data-system-settings-content]');
      const language = panel.getByLabel("Output Language", { exact: true });
      const trust = panel.getByLabel("Trust gwt-managed Codex hooks", { exact: true });
      const status = panel.locator('[data-role="system-settings-status"]');
      await expect.poll(() => requests.some(request => request.kind === "get_system_settings")).toBe(true);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      reply({ kind: "system_settings", language: "en", board_provider: "slack", agent_resource: resource });
      await expect(language).toHaveValue("en");
      await expect(panel.getByLabel("Build parallelism per agent (jobs)", { exact: true })).toHaveValue("2");
      await language.focus();
      await language.dispatchEvent("pointerdown");
      await language.evaluate(element => element.setAttribute("data-system-probe", "original"));
      const before = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
      reply({ kind: "system_settings_updated", language: "ja", codex_trust_managed_hooks: false, agent_resource: resource });
      reply({ kind: "autostart_status", enabled: false });
      await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ sequence, payload }: any) => sequence > before && payload.kind === "autostart_status"), before);
      await expect(language).toHaveAttribute("data-system-probe", "original");
      await expect(language).toHaveValue("en");
      await language.blur();
      await expect(language).toHaveValue("ja");
      await expect(trust).not.toBeChecked();
      await expect(status).toHaveText("Saved system settings.");
      await trust.focus();
      await panel.hover();
      for (const suspendRaf of [false, true]) {
        const result = await panel.evaluate(async (panel, { resource, suspendRaf }) => {
          const before = panel.innerHTML;
          const status = panel.querySelector('[data-role="system-settings-status"]')!;
          const focused = document.activeElement;
          const originalRaf = window.requestAnimationFrame;
          if (suspendRaf) window.requestAnimationFrame = () => 0;
          const descriptor = Object.getOwnPropertyDescriptor(Node.prototype, "textContent")!;
          let writes = 0;
          let renderFinished: (() => void) | undefined;
          Object.defineProperty(status, "textContent", {
            configurable: true,
            get() { return descriptor.get!.call(this); },
            set(value) { writes++; descriptor.set!.call(this, value); queueMicrotask(() => renderFinished?.()); },
          });
          let timer: ReturnType<typeof setTimeout>;
          const started = performance.now();
          try {
            const rendered = new Promise<void>((resolve, reject) => {
              // A scalar patch may write the same text and produce no DOM mutation.
              // Resolve from the actual renderer write after its synchronous work ends.
              renderFinished = resolve;
              timer = setTimeout(() => reject(new Error(`System frame exceeded one second: writes=${writes}, connected=${status.isConnected}, same=${panel.querySelector('[data-role="system-settings-status"]') === status}`)), 1000);
            });
            for (let index = 1; index <= 50; index++) {
              window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
                kind: "system_settings", language: "ja", board_provider: "slack", codex_trust_managed_hooks: false,
                agent_resource: { ...resource, build_jobs: index },
              } }));
            }
            const unchangedBeforeFrame = panel.innerHTML === before && writes === 0;
            await rendered;
            return { unchangedBeforeFrame, writes, elapsed: performance.now() - started,
              jobs: panel.querySelector<HTMLInputElement>('[id$="-settings-system-agent-build-jobs"]')!.value,
              sameStatus: panel.querySelector('[data-role="system-settings-status"]') === status,
              focused: document.activeElement === focused, hovered: panel.matches(":hover") };
          } finally {
            clearTimeout(timer!);
            delete (status as any).textContent;
            window.requestAnimationFrame = originalRaf;
          }
        }, { resource, suspendRaf });
        expect(result).toMatchObject({ unchangedBeforeFrame: true, writes: 1, jobs: "50",
          sameStatus: true, focused: true, hovered: true });
        expect(result.elapsed).toBeLessThan(1000);
        await info.attach(suspendRaf ? "system-settings-fallback" : "system-settings-frame", {
          body: JSON.stringify(result), contentType: "application/json",
        });
      }
      const jobs = panel.getByLabel("Build parallelism per agent (jobs)", { exact: true });
      await jobs.fill("4");
      await jobs.evaluate(element => element.setAttribute("data-focused-resource", "original"));
      await jobs.press("Enter");
      await expect(status).toHaveText("Saving…");
      await expect(jobs).toBeFocused();
      await expect(jobs).toHaveAttribute("data-focused-resource", "original");
      await trust.click();
      await expect(trust).toBeChecked();
      await expect(trust).toBeFocused();
      await expect.poll(() => requests.filter(request => request.kind === "update_system_settings").length).toBe(2);
      expect(requests.filter(request => request.kind === "update_system_settings")).toEqual([
        { kind: "update_system_settings", language: "ja", agent_resource: { ...resource, build_jobs: 4 } },
        { kind: "update_system_settings", language: "ja", codex_trust_managed_hooks: true },
      ]);
      reply({ kind: "system_settings_error", message: "System fixture rejected" });
      await expect(status).toHaveText("System fixture rejected");
      const secret = panel.locator('.board-config-form input[type="password"]');
      await secret.fill("fixture-unsaved-system-secret");
      const beforeDraftReply = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
      reply({ kind: "system_settings", language: "en", board_provider: "slack", agent_resource: resource });
      await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ sequence, payload }: any) => sequence > before && payload.kind === "system_settings" && payload.language === "en"), beforeDraftReply);
      await expect(secret).toHaveValue("fixture-unsaved-system-secret");
      await expect(secret).toBeFocused();
      // A clean re-mount reads the latest model version; no write is replayed.
      await secret.blur();
      const id = await settings.getAttribute("data-id");
      expect(id).toBeTruthy();
      await page.locator('.op-rail__item[data-surface="agents"]').click();
      await sendLiveGwtEvent(page, { kind: "close_window", id });
      await expect(settings).toHaveCount(0, { timeout: 30_000 });
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "system" } })));
      await expect(language).toHaveValue("en");
      await expect(panel.getByLabel("Build parallelism per agent (jobs)", { exact: true })).toHaveValue("2");
      await expect(secret).toHaveValue("");
      expect(requests.filter(request => request.kind === "update_system_settings")).toHaveLength(2);
      const provider = await panel.evaluate(panel => {
        const select = panel.querySelector<HTMLSelectElement>('[id$="-settings-system-board-provider"]')!;
        select.value = "teams";
        select.dispatchEvent(new Event("change", { bubbles: true }));
        return panel.querySelector<HTMLElement>(".board-config-form")?.dataset.provider;
      });
      expect(provider).toBe("teams");
      await settings.evaluate(element => element.querySelector('select[id$="-settings-system-language"]')?.scrollIntoView({ block: "center" }));
      await info.attach(`system-settings-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
      expect(errors).toEqual([]);
    });
  });

  test("Board auth snapshots preserve guarded controls and unsaved provider drafts", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
      const requests: any[] = [];
      let transport: WebSocketRoute | undefined;
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        const server = socket.connectToServer();
        transport = socket;
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          // Exercise the actual receiver without external OAuth or secret writes.
          if (["get_board_auth_status", "board_provider_sign_out", "update_board_provider_config"].includes(payload?.kind)) {
            requests.push(payload);
          } else server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind === "board_auth_status") return;
          if (payload?.kind === "system_settings") {
            socket.send(JSON.stringify({ ...payload, board_provider: "slack" }));
          } else socket.send(message);
        });
      });
      const reply = (slack: boolean, client: string) => transport!.send(JSON.stringify({
        kind: "board_auth_status", slack, slack_client_id: client, oauth_redirect_port: 8765,
      }));
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page, PROJECT);
      await page.bringToFront();
      await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "system" } })));
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const panel = settings.locator('[data-system-settings-content]');
      await expect(panel.locator(".board-auth-status")).toBeVisible();
      await expect.poll(() => requests.filter(request => request.kind === "get_board_auth_status").length).toBeGreaterThan(0);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      reply(true, "client-a");
      await expect(panel.locator(".board-auth-status")).toHaveAttribute("data-signed-in", "true");
      const language = panel.getByLabel("Output Language", { exact: true });
      await language.focus();
      await language.dispatchEvent("pointerdown");
      await language.evaluate(element => element.setAttribute("data-probe", "guarded"));
      const before = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
      reply(false, "client-b");
      // A native-select guard keeps the node until focus leaves it.
      await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ sequence, payload }: any) => sequence > before && payload.kind === "board_auth_status" && payload.slack === false), before);
      await expect(language).toHaveAttribute("data-probe", "guarded");
      await expect(panel.locator(".board-auth-status")).toHaveAttribute("data-signed-in", "true");
      await language.blur();
      await expect(panel.locator(".board-auth-status")).toHaveAttribute("data-signed-in", "false");
      await expect(panel.locator('.board-config-form input[type="text"]').first()).toHaveValue("client-b");
      const secret = panel.locator('.board-config-form input[type="password"]');
      await secret.fill("fixture-unsaved-secret");
      reply(true, "client-c");
      await expect(secret).toHaveValue("fixture-unsaved-secret");
      await panel.locator(".board-config-form button").click();
      await expect.poll(() => requests.filter(request => request.kind === "update_board_provider_config").length).toBe(1);
      reply(true, "client-c");
      await expect(secret).toHaveValue("");
      await expect(panel.locator('.board-config-form input[type="text"]').first()).toHaveValue("client-c");
      await panel.getByRole("button", { name: "Sign out", exact: true }).click();
      await expect.poll(() => requests.some(request => request.kind === "board_provider_sign_out" && request.provider === "slack")).toBe(true);
      reply(false, "client-c");
      await expect(panel.locator(".board-auth-status")).toHaveText("Not signed in to slack");
      await settings.evaluate(element => element.querySelector(".board-auth-status")
        ?.scrollIntoView({ block: "center" }));
      await info.attach(`board-auth-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
      expect(errors).toEqual([]);
    });
  });

  test("delayed status, query failure, update failure and retry keep other settings responsive", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

      // Forward the real backend protocol except login-launch operations. Holding
      // their replies makes slowness deterministic and prevents OS login changes.
      const requests: Array<{ kind: string; enabled?: boolean }> = [];
      let autostartSocket: WebSocketRoute | undefined;
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        const server = socket.connectToServer();
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind === "get_autostart_status" || payload?.kind === "update_autostart") {
            requests.push(payload);
            autostartSocket = socket;
          } else server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind !== "autostart_status" && payload?.kind !== "autostart_error") socket.send(message);
        });
      });
      const reply = (payload: object) => {
        expect(autostartSocket, "an autostart request reached the controlled transport").toBeDefined();
        autostartSocket!.send(JSON.stringify(payload));
      };
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page, PROJECT);
      await page.bringToFront();
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const open = async () => {
        // A new page can restore the previous theme's Settings window. Focusing
        // that window sends no read; its initial snapshot is already in flight.
        const before = await settings.count() === 0
          ? await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0)
          : 0;
        await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "system" } })));
        await expect(settings).toBeVisible();
        await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ sequence, payload }: any) => sequence > before && payload.kind === "system_settings"),
        before, { timeout: 30_000 });
      };
      const close = async () => {
        const id = await settings.getAttribute("data-id");
        expect(id).toBeTruthy();
        // A fixed host recreates its selected surface when its window closes.
        // Leave Settings first so this lifecycle probe actually unmounts it.
        await page.locator('.op-rail__item[data-surface="agents"]').click();
        await sendLiveGwtEvent(page, { kind: "close_window", id });
        // Closing is a backend round trip, not a latency assertion for this Issue.
        await expect(settings).toHaveCount(0, { timeout: 30_000 });
      };
      const panel = settings.locator('[data-system-settings-content]');
      const toggle = panel.getByLabel("Launch GWT at login", { exact: true });
      const status = panel.locator('[data-role="autostart-settings-status"]');
      const systemStatus = panel.locator('[data-role="system-settings-status"]');
      const language = panel.getByLabel("Output Language", { exact: true });
      const saveLanguage = async (value: string) => {
        const before = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
        await language.selectOption(value);
        await page.waitForFunction(({ before, value }) => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ sequence, payload }: any) => sequence > before
            && payload.kind === "system_settings_updated" && payload.language === value),
        { before, value }, { timeout: 30_000 });
        await expect(systemStatus).toHaveText("Saved system settings.", { timeout: 30_000 });
        await expect(language).toHaveValue(value);
      };

      await open();
      await expect.poll(() => requests.filter(request => request.kind === "get_autostart_status").length).toBeGreaterThan(0);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(toggle).toBeDisabled();
      await expect(status).toHaveText("Checking login launch status…");
      const queriesBeforeCheckingReopen = requests.filter(request => request.kind === "get_autostart_status").length;
      await close();
      await open();
      await expect(toggle).toBeDisabled();
      await expect(status).toHaveText("Checking login launch status…");
      await settings.getByRole("tab", { name: "Supported Agents", exact: true }).click();
      await expect(settings.locator('[data-settings-panel="supported-agents"] tbody [data-agent-id]').first())
        .toBeVisible({ timeout: 60_000 });
      await expect(settings.locator('[data-settings-panel="supported-agents"]')).not.toContainText("Loading agent detection…");
      await settings.getByRole("tab", { name: "System", exact: true }).click();
      await expect(status).toHaveText("Checking login launch status…");
      await expect(language).toBeEnabled();
      const originalLanguage = await language.inputValue();
      const nextLanguage = originalLanguage === "ja" ? "en" : "ja";
      await saveLanguage(nextLanguage);
      await expect(status).toHaveText("Checking login launch status…");
      expect(requests.filter(request => request.kind === "get_autostart_status").length,
        "reopening while checking reuses the in-flight query").toBe(queriesBeforeCheckingReopen);

      reply({ kind: "autostart_error", message: "Fixture status query unavailable." });
      await expect(status).toHaveText("Login launch status unavailable. Fixture status query unavailable.");
      await expect(toggle).toBeDisabled();
      await expect(systemStatus).toHaveText("Saved system settings.");
      await settings.evaluate(element => element.querySelector('[data-role="autostart-settings-status"]')
        ?.scrollIntoView({ block: "center" }));
      await info.attach(`autostart-unavailable-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });

      const queriesBeforeReopen = requests.filter(request => request.kind === "get_autostart_status").length;
      await close();
      await open();
      await expect.poll(() => requests.filter(request => request.kind === "get_autostart_status").length).toBeGreaterThan(queriesBeforeReopen);
      await expect(status).toHaveText("Checking login launch status…");
      await expect(toggle).toBeDisabled();
      reply({ kind: "autostart_status", enabled: true, mechanism: "fixture", install_path: "/fixture/gwt" });
      await expect(toggle).toBeEnabled();
      await expect(toggle).toBeChecked();

      await toggle.uncheck();
      await expect(toggle).toBeDisabled();
      await expect(status).toHaveText("Saving…");
      await expect.poll(() => requests.filter(request => request.kind === "update_autostart")).toEqual([
        { kind: "update_autostart", enabled: false },
      ]);
      await saveLanguage(originalLanguage);
      await expect(status).toHaveText("Saving…");
      reply({ kind: "autostart_error", message: "Fixture login setting denied." });
      await expect(status).toHaveText("Fixture login setting denied.");
      await expect(toggle).toBeEnabled();
      await expect(toggle).toBeChecked();
      await expect(systemStatus).toHaveText("Saved system settings.");

      await toggle.uncheck();
      const queriesBeforePendingReopen = requests.filter(request => request.kind === "get_autostart_status").length;
      await close();
      await open();
      await expect(status).toHaveText("Saving…");
      await expect(toggle).toBeDisabled();
      expect(requests.filter(request => request.kind === "get_autostart_status").length,
        "reopening while saving cannot race the update with a new query").toBe(queriesBeforePendingReopen);
      reply({ kind: "autostart_status", enabled: false, mechanism: "fixture", install_path: "/fixture/gwt" });
      await expect(toggle).toBeEnabled();
      await expect(toggle).not.toBeChecked();
      await expect(status).toHaveText("Saved login launch setting.");
      await expect(systemStatus).toHaveText("Saved system settings.");
      await settings.evaluate(element => element.querySelector('[data-role="autostart-settings-status"]')
        ?.scrollIntoView({ block: "center" }));
      await info.attach(`autostart-saved-${theme}`, { body: await settings.screenshot(), contentType: "image/png" });
      expect(errors, "zero console/page errors through query failure and update recovery").toEqual([]);
      // The page fixture and wrapper's owned-process cleanup close the final window.
    });
  });
});
