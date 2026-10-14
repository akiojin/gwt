import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = configuredUrl
  ? new URL(`/p/${process.env.GWT_PLAYWRIGHT_PROJECT_KEY || new URL(APP_URL).pathname.split("/")[2]}`, configuredUrl).toString()
  : APP_URL;
const errors = new WeakMap<Page, string[]>();
const slowMessage = "Loading cache-backed work items (taking longer than usual)";

test.describe("Issue #5413: cache-backed load recovery", () => {
  test.use({ viewport: { width: 1600, height: 1000 } });

  test.beforeEach(async ({ page }, info) => {
    const captured: string[] = [];
    errors.set(page, captured);
    page.on("pageerror", error => captured.push(error.message));
    page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), info.project.name.includes("light") ? "light" : "dark");
    if (!configuredUrl) await installEmbeddedRoutes(page);
    await installBackend(page);
    await page.clock.install();
    await page.goto(appUrl);
    await expect(page.locator(".surface-knowledge .knowledge-list")).toBeVisible();
    await expect.poll(() => loadRequests(page)).toHaveLength(1);
  });

  test.afterEach(async ({ page }, info) => {
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
    const screenshot = info.outputPath("load-recovery.png");
    await page.screenshot({ path: screenshot });
    await info.attach("load-recovery", { path: screenshot, contentType: "image/png" });
    expect(errors.get(page), "zero console/page errors").toEqual([]);
  });

  test("one pending request survives the slow hint and late success after the deadline", async ({ page }, info) => {
    const surface = page.locator(".surface-knowledge");
    const refresh = surface.locator("[data-action='refresh-knowledge']");
    await refresh.click();
    await refresh.click();
    await page.clock.runFor(5100);
    await expect(surface.locator('.knowledge-empty[role="status"]')).toHaveText(slowMessage);
    expect(await loadRequests(page)).toHaveLength(1);
    await expect(surface.getByRole("button", { name: /^Issue load failed/ })).toHaveCount(0);
    const slowScreenshot = info.outputPath("slow-load.png");
    await surface.screenshot({ path: slowScreenshot });
    await info.attach("slow-load", { path: slowScreenshot, contentType: "image/png" });

    await page.clock.runFor(55_000);
    await expect(surface.getByRole("button", { name: /^Issue load failed/ })).toBeVisible();
    await expect.poll(() => reports(page)).toContain("Timed out loading cache-backed data");
    expect(await loadRequests(page)).toHaveLength(1);
    const timeoutScreenshot = info.outputPath("timed-out-load.png");
    await surface.screenshot({ path: timeoutScreenshot });
    await info.attach("timed-out-load", { path: timeoutScreenshot, contentType: "image/png" });

    await reply(page, "success");
    await expect(surface.locator(".knowledge-row[data-issue-number='5413']")).toBeVisible();
    await expect(surface.getByRole("button", { name: /^Issue load failed/ })).toHaveCount(0);
    await expect(surface.locator('.knowledge-empty[role="status"]')).toHaveCount(0);
    await expect.poll(() => loadRequests(page)).toHaveLength(2);
    const requests = await loadRequests(page);
    expect(requests[1].refresh, "queued clicks produce one follow-up refresh").toBe(true);
    await reply(page, "success");
    await expect(refresh).not.toHaveAttribute("title", /Timed out/);
  });

  test("a backend failure is reported immediately and a successful refresh clears it", async ({ page }) => {
    const surface = page.locator(".surface-knowledge");
    await reply(page, "error");
    await expect(surface.getByRole("button", { name: /^Issue load failed/ })).toBeVisible();
    await expect.poll(() => reports(page)).toEqual(["Fixture cache read failed"]);
    expect(await loadRequests(page)).toHaveLength(1);
    await surface.locator("[data-action='refresh-knowledge']").click();
    await expect.poll(() => loadRequests(page)).toHaveLength(2);
    await reply(page, "success");
    await expect(surface.locator(".knowledge-row[data-issue-number='5413']")).toBeVisible();
    await expect(surface.getByRole("button", { name: /^Issue load failed/ })).toHaveCount(0);
    await expect(surface.locator("[data-action='refresh-knowledge']")).not.toHaveAttribute("title", /failed/i);
  });
});

async function loadRequests(page: Page) {
  return page.evaluate(() => window.__knowledgeRecoveryMessages.filter(message => message.kind === "load_knowledge_bridge"));
}

async function reports(page: Page) {
  return page.evaluate(() => window.__knowledgeRecoveryMessages.filter(message => message.kind === "report_knowledge_error").map(message => message.message));
}

async function reply(page: Page, outcome: "success" | "error") {
  await page.evaluate(result => window.__replyKnowledgeRecovery(result), outcome);
}

async function installBackend(page: Page) {
  await page.addInitScript(projectKey => {
    window.__knowledgeRecoveryMessages = [];
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = FixtureWebSocket.CONNECTING;

      constructor() {
        super();
        window.__knowledgeRecoverySocket = this;
        queueMicrotask(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        });
      }

      send(raw) {
        const message = JSON.parse(raw);
        window.__knowledgeRecoveryMessages.push(message);
        if (message.kind === "frontend_ready") this.emit({
          kind: "workspace_state",
          workspace: {
            app_version: "playwright",
            tabs: [{ id: "recovery", title: "Load recovery", project_root: "/fixture", project_key: projectKey, kind: "git",
              workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [{
                id: "recovery::issue", title: "Issue", preset: "issue", status: "running", persist: true,
                geometry: { x: 40, y: 50, width: 1450, height: 850 }, z_index: 1,
                tab_group_id: null, tab_group_active: false,
              }] } }],
            active_tab_id: "recovery", recent_projects: [],
          },
        });
      }

      emit(payload) {
        queueMicrotask(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) })));
      }

      close() { this.readyState = FixtureWebSocket.CLOSED; }
    }
    window.__replyKnowledgeRecovery = outcome => {
      const request = window.__knowledgeRecoveryMessages.filter(message => message.kind === "load_knowledge_bridge").at(-1);
      window.__knowledgeRecoverySocket.emit(outcome === "error" ? {
        kind: "knowledge_error", id: request.id, knowledge_kind: request.knowledge_kind,
        request_id: request.request_id, message: "Fixture cache read failed",
      } : {
        kind: "knowledge_entries", id: request.id, knowledge_kind: request.knowledge_kind, request_id: request.request_id,
        selected_number: null, refresh_enabled: true, empty_message: null,
        entries: [{ number: 5413, title: "Recovered cache-backed work item", state: "open", labels: ["bug"],
          is_spec: false, linked_branch_count: 0, related_work_refs: [], monitor_state: "idle", queue_position: null }],
      });
    };
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  }, new URL(appUrl).pathname.split("/")[2]);
}
