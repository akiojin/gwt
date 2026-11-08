import { expect, test, type Page } from "@playwright/test";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { homedir } from "node:os";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt, openLiveGwtProject, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const configuredUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
const appUrl = configuredUrl
  ? new URL(`/p/${process.env.GWT_PLAYWRIGHT_PROJECT_KEY || new URL(APP_URL).pathname.split("/")[2]}`, configuredUrl).toString()
  : APP_URL;
const errors = new WeakMap<Page, string[]>();
const slowMessage = "Loading cache-backed work items (taking longer than usual)";

test("ordinary reconnect recovers a lost Knowledge cache reply in the mounted window", async ({ page }, info) => {
  const home = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  test.skip(!configuredUrl || !home, "requires browser-check checkout and fresh HOME");
  test.setTimeout(120_000);
  await withLiveGwtBackendLock(configuredUrl!, info, async () => {
    expect(home).not.toBe(homedir());
    const identity = JSON.parse(await readFile(join(home!, "issue-4925-isolated.json"), "utf8"));
    const cache = join(home!, ".gwt/cache/issues", identity.repo_hash, "50160001");
    await mkdir(cache, { recursive: true });
    await writeFile(join(cache, "meta.json"), JSON.stringify({ number: 50160001, author_login: "akiojin",
      title: "KLC recovered cache row", state: "open", labels: ["bug"], updated_at: new Date().toISOString(), comment_ids: [] }));
    await writeFile(join(cache, "body.md"), "Knowledge reconnect fixture.\n");
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const captured: string[] = [];
    const requests: any[] = [];
    let lostRequestId: number | undefined;
    let droppedReplies = 0;
    let reconnecting = false;
    let replacement: { reads: any[]; reply?: any } | undefined;
    page.on("pageerror", error => captured.push(error.message));
    page.on("console", message => { if (message.type() === "error") captured.push(message.text()); });
    await page.setViewportSize({ width: 1600, height: 1000 });
    await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
    await page.routeWebSocket(/\/ws\?.*repo_hash=/, socket => {
      const connection: { reads: any[]; reply?: any } = { reads: [] };
      if (reconnecting && !replacement) replacement = connection;
      const server = socket.connectToServer();
      socket.onMessage(message => {
        const request = typeof message === "string" ? JSON.parse(message) : null;
        if (request?.kind) requests.push(request);
        if (request?.kind === "open_pm_agent") return;
        if (request?.kind === "load_knowledge_bridge") {
          connection.reads.push(request);
          if (lostRequestId === undefined) lostRequestId = request.request_id;
        }
        server.send(message);
      });
      server.onMessage(message => {
        const reply = typeof message === "string" ? JSON.parse(message) : null;
        if (reply?.kind === "knowledge_entries") {
          if (reply.request_id === lostRequestId) { droppedReplies++; return; }
          connection.reply = reply;
        }
        socket.send(message);
      });
    });
    await gotoLiveGwt(page, configuredUrl!, { enableTestBridge: true });
    await openLiveGwtProject(page, identity.project);
    await page.bringToFront();
    await page.locator('.op-rail [data-surface="issues"]').click();
    const surface = page.locator(".surface-knowledge").first();
    const input = surface.locator(".knowledge-search");
    await expect(surface).toBeVisible();
    await expect.poll(() => droppedReplies).toBeGreaterThan(0);
    await input.fill("KLC");
    await input.evaluate(element => { (window as any).__knowledgeReconnectInput = element; });
    await expect(surface.locator(".knowledge-row")).toHaveCount(0);
    reconnecting = true;
    await page.evaluate(() => {
      (window as any).__knowledgeReconnectDocument = document;
      const socket = (window as any).__gwtPlaywrightSockets.findLast((socket: WebSocket) =>
        socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has("repo_hash"));
      if (!socket) throw new Error("missing owning project socket");
      socket.close();
    });
    await expect.poll(() => replacement?.reply, { timeout: 20_000 }).toBeDefined();
    expect(replacement!.reads).toHaveLength(1);
    expect(replacement!.reads[0].request_id).toBeGreaterThan(lostRequestId!);
    expect(replacement!.reply.request_id).toBe(replacement!.reads[0].request_id);
    expect(replacement!.reply.entries.some((row: any) => row.number === 50160001)).toBe(true);
    await expect(surface.locator('.knowledge-row[data-issue-number="50160001"]')).toBeVisible();
    await expect(input).toHaveValue("KLC");
    expect(await input.evaluate(element => element.isConnected && element === (window as any).__knowledgeReconnectInput)).toBe(true);
    expect(await page.evaluate(() => document === (window as any).__knowledgeReconnectDocument)).toBe(true);
    await expect(surface.getByRole("button", { name: /^Issue load failed/ })).toHaveCount(0);

    // Hold the real socket in the CLOSING-before-close observation window.
    // The read stays in the app's real pending queue until forced recovery.
    await page.clock.install();
    await page.clock.pauseAt(new Date(Date.now() + 1000));
    await page.evaluate(() => {
      const socket = (window as any).__gwtPlaywrightSockets.findLast((socket: WebSocket) =>
        socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has("repo_hash"));
      if (!socket) throw new Error("missing owning project socket");
      (window as any).__knowledgeQueuedSocket = socket;
      Object.defineProperty(socket, "readyState", { configurable: true, value: WebSocket.CLOSING });
      // Invalidate the cache view through the existing receive test bridge.
      // A user Refresh would force a remote GitHub sync instead of a cache read.
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: { kind: "issue_monitor_inbox", items: [] },
      }));
    });
    const beforeQueued = requests.filter(request => request.kind === "load_knowledge_bridge").length;
    const refresh = surface.locator("[data-action='refresh-knowledge']");
    await page.clock.runFor(2000);
    await page.clock.runFor(5000);
    await page.clock.fastForward(56000);
    await expect(surface.getByRole("button", { name: /^Issue load failed/ })).toHaveCount(0);
    expect(requests.filter(request => request.kind === "load_knowledge_bridge")).toHaveLength(beforeQueued);
    replacement = undefined;
    // Controlled inbound frames overflow the actual receive dispatcher and
    // force socket recovery while the cache read is still in the real queue.
    await page.evaluate(() => {
      const socket = (window as any).__knowledgeQueuedSocket;
      for (let index = 0; index <= 2048; index++) {
        socket.dispatchEvent(new MessageEvent("message", {
          data: JSON.stringify({ kind: "issue_monitor_inbox", items: [] }),
        }));
      }
    });
    await page.clock.runFor(100);
    await page.clock.resume();
    await expect.poll(() => replacement?.reply, { timeout: 20_000 }).toBeDefined();
    expect(replacement!.reads).toHaveLength(1);
    expect(replacement!.reads[0].refresh).toBe(false);
    expect(replacement!.reply.request_id).toBe(replacement!.reads[0].request_id);
    await expect(surface.locator('.knowledge-row[data-issue-number="50160001"]')).toBeVisible();
    await expect(refresh).toBeEnabled();
    await expect(input).toHaveValue("KLC");
    expect(await input.evaluate(element => element.isConnected && element === (window as any).__knowledgeReconnectInput)).toBe(true);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    expect(requests.filter(request => /^(update_|save_|set_knowledge|run_branch|start_issue|launch_)/.test(request.kind))).toEqual([]);
    const screenshot = info.outputPath(`knowledge-reconnect-${theme}.png`);
    await page.screenshot({ path: screenshot });
    await info.attach("knowledge reconnect", { path: screenshot, contentType: "image/png" });
    await info.attach("knowledge reconnect requests", { body: JSON.stringify({ requests, droppedReplies, lostRequestId, replacement }), contentType: "application/json" });
    expect(captured, "zero console/page errors").toEqual([]);
  });
});

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

    // Skip repeated animation callbacks while advancing to the load deadline.
    await page.clock.fastForward(55_000);
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
