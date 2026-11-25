/* Issue #7639 (SPEC #7007 T-021) — Home, the conversation with the Project
 * Manager, in a real browser with a deterministic WebSocket backend.
 *
 * AC-1: Home is the PM conversation and shows no project work status or count.
 * AC-2: a decision is answered from the PM's message and the answer goes to
 *       the project it belongs to.
 * AC-3: the path stays whole with no project and with twenty.
 */
import { randomUUID } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { expect, test, type Page } from "@playwright/test";
import { HUB_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const key = (index: number) => index.toString(16).padStart(16, "0");
const project = (index: number, title: string) => ({ id: `tab-${index}`, project_key: key(index), title, kind: "git" });
const alpha = project(1, "Alpha");
const beta = project(2, "Beta");

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();
// One item of a project's gwtd decision queue (Issue #7742).
const decision = (id: string, minutes: number, question: string, options: string[]) => ({
  id, issue: 7742, kind: "product", target: id, question, materials: `${question} is waiting on you.`,
  impact: "Only this project is affected.", options, recommendation: options[0],
  reason: "Nothing else changes by waiting.", status: "open", created_at: minutesAgo(minutes), deferred_seq: null,
});

type NativeMessage = { id: string; role: string; text: string };
type Conversation = {
  availability: string;
  messages: NativeMessage[];
  older_messages?: NativeMessage[];
  has_older?: boolean;
};

type Backend = {
  projects: unknown[];
  recent?: unknown[];
  conversation: Array<{
    project_key: string; title: string; pm_available: boolean; decisions: unknown[]; error?: string;
    conversation?: Conversation;
  }>;
  historyPageSize?: number;
  pm?: { name: string; avatar: { kind: "preset"; preset_id: string } | { kind: "upload"; image: string } };
  // Hold the pane receipt so the test can observe delivery and reply separately.
  holdSend?: boolean;
  // Project keys whose PM refuses a message.
  refuse?: string[];
};

function collectBrowserErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
}

// The stub answers the catalog and a PM send, and keeps one decision queue per
// project the way gwtd does: an answer takes the decision out of the queue,
// "decide later" moves it to the tail, and the waiting count is the queue's.
async function installHomeBackend(page: Page, backend: Backend) {
  await page.addInitScript(backend => {
    const state = { sent: [] as any[], seq: 0, rejectHomeRead: false };
    const queues = new Map<string, any[]>(backend.conversation.map(project => [project.project_key, [...project.decisions]]));
    const queueOf = (projectKey: string) => {
      const items = queues.get(projectKey) || [];
      return { waiting_count: items.length, items };
    };
    (window as any).__homeSent = state.sent;
    const sockets: FixtureWebSocket[] = [];
    (window as any).__homeEvent = (event: object) => sockets.at(-1)?.reply(event);
    (window as any).__homeProjects = (projects: Backend["conversation"]) => {
      backend.conversation = projects;
    };
    (window as any).__setHomePm = (pm: Backend["pm"]) => {
      backend.pm = pm;
      state.rejectHomeRead = pm === undefined;
    };
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      constructor(public url: string) {
        super();
        sockets.push(this);
        setTimeout(() => {
          this.readyState = 1;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      reply(event: unknown) {
        setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(event) })), 0);
      }
      send(raw: string) {
        const message = JSON.parse(raw);
        if (message.kind === "frontend_ready") {
          this.reply({ kind: "hub_state", hub: { app_version: "9.9.9", projects: backend.projects, recent_projects: backend.recent || [] } });
        } else if (message.kind === "load_home_conversation") {
          if (message.history) state.sent.push(message);
          this.reply({ kind: "home_conversation", request_id: message.request_id, pm: backend.pm, projects: backend.conversation.map(project => {
            let conversation = project.conversation;
            if (conversation && backend.historyPageSize) {
              const { messages } = conversation;
              const cursor = message.history?.[project.project_key];
              const end = cursor ? messages.findIndex(item => item.id === cursor) : messages.length;
              const start = Math.max(0, end - backend.historyPageSize);
              conversation = {
                ...conversation,
                messages: messages.slice(-backend.historyPageSize),
                older_messages: cursor && end >= 0 ? messages.slice(start, end) : [],
                has_older: start > 0,
              };
            }
            return {
              ...project, conversation,
              ...(state.rejectHomeRead ? { error: "Home read deferred", decisions: [], waiting_count: 0 }
                : { decisions: queueOf(project.project_key).items, waiting_count: queueOf(project.project_key).waiting_count }),
            };
          }) });
        } else if (message.kind === "home_answer_decision" || message.kind === "home_defer_decision") {
          state.sent.push(message);
          const items = queues.get(message.project_key) || [];
          const found = items.find(item => item.id === message.decision_id);
          const rest = items.filter(item => item !== found);
          const answer = message.kind === "home_answer_decision";
          if (found) queues.set(message.project_key, answer ? rest : [...rest, { ...found, deferred_seq: ++state.seq }]);
          this.reply({ kind: "decision_result", id: message.project_key, request_id: message.request_id,
            decision_id: message.decision_id, action: answer ? "answer" : "defer", ok: Boolean(found),
            error: found ? null : `decision ${message.decision_id} was not found`, decisions: queueOf(message.project_key) });
        } else if (message.kind === "home_pm_send") {
          state.sent.push(message);
          if (backend.holdSend) return;
          const refused = (backend.refuse || []).includes(message.project_key);
          this.reply({ kind: "pane_send_result", ok: !refused, window_id: refused ? null : "tab::pm",
            error: refused ? "No Project Manager is running for this project. Open the project, then try again." : null,
            request_id: message.request_id });
        }
      }
      close() {
        this.readyState = 3;
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  }, backend);
}

async function openHome(page: Page, theme: string, backend: Backend, beforeNavigation?: () => Promise<void>) {
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  const baseUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;
  if (!baseUrl) await installEmbeddedRoutes(page);
  await installHomeBackend(page, backend);
  await beforeNavigation?.();
  await page.goto(baseUrl ? new URL(new URL(HUB_URL).pathname, baseUrl).href : HUB_URL);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.locator("[data-home]")).toBeVisible();
}

const themeOf = (name: string) => (name.includes("light") ? "light" : "dark");

const twoProjects: Backend = {
  projects: [alpha, beta],
  conversation: [
    { project_key: alpha.project_key, title: "Alpha", pm_available: true, decisions: [
      decision("d-new", 26, "Save and push the unsaved changes?", ["Save and push", "Leave them"]),
      decision("d-old", 60, "Approve the plan for the refresh?", ["Approve and start", "Ask for changes"]),
    ] },
    { project_key: beta.project_key, title: "Beta", pm_available: true, decisions: [
      decision("d-mid", 48, "Fix the mapping and continue the import?", ["Fix and continue", "Stop the import"]),
    ] },
  ],
};

for (const width of [1440, 390]) {
  test(`Issue #7864 Home loads a long conversation at ${width}px`, async ({ page }, info) => {
    await page.setViewportSize({ width, height: 900 });
    const errors = collectBrowserErrors(page);
    const theme = themeOf(info.project.name);
    const history = Array.from({ length: 3000 }, (_, index) => ({
      id: `history-${index}`,
      role: index % 5 === 0 ? "user" : "assistant",
      text: `Conversation entry ${index}`,
    }));
    const fixtureReady = performance.now();
    await openHome(page, theme, { projects: [alpha], historyPageSize: 50, conversation: [{
      project_key: alpha.project_key, title: alpha.title, pm_available: true, decisions: [],
      conversation: { availability: "ready", messages: history },
    }] });
    const home = page.locator("[data-home]");
    const log = home.getByRole("log");
    const messages = log.getByText(/^Conversation entry \d+$/);
    const entry = (index: number) => log.getByText(history[index].text, { exact: true });
    await expect(messages).toHaveCount(50);
    await expect(entry(2999)).toBeInViewport();
    const initialMs = Math.round(performance.now() - fixtureReady);
    await expect(log.locator("details")).toHaveCount(0);
    await expect(entry(2949)).toHaveCount(0);

    const samples: Array<{ mounted: number; loadMs: number; anchorDeltaPx: number }> = [];
    for (const oldest of [2950, 2900, 2850]) {
      const started = performance.now();
      const before = await entry(oldest).evaluate(node => {
        const log = node.closest('[role="log"]')!;
        log.scrollTop = 0;
        return node.getBoundingClientRect().top - log.getBoundingClientRect().top;
      });
      await expect(entry(oldest - 50)).toBeVisible();
      const after = await entry(oldest).evaluate(node =>
        node.getBoundingClientRect().top - node.closest('[role="log"]')!.getBoundingClientRect().top);
      const mounted = await messages.count();
      const anchorDeltaPx = Math.abs(after - before);
      expect(anchorDeltaPx).toBeLessThanOrEqual(2);
      expect(mounted).toBeLessThanOrEqual(100);
      samples.push({ mounted, loadMs: Math.round(performance.now() - started), anchorDeltaPx });
    }
    const ids = (await messages.allTextContents()).map(text => Number(text.replace("Conversation entry ", "")));
    expect(ids).toEqual(Array.from({ length: ids.length }, (_, index) => 2800 + index));
    expect(await page.evaluate(() => (window as any).__homeSent.filter((message: any) => message.history)
      .map((message: any) => Object.values(message.history)[0]))).toEqual(["history-2950", "history-2900", "history-2850"]);

    const screenshot = info.outputPath(`pm-history-${theme}-${width}.png`);
    await home.screenshot({ path: screenshot });
    await info.attach("PM conversation history", { path: screenshot, contentType: "image/png" });
    const metrics = info.outputPath(`pm-history-${theme}-${width}.json`);
    await writeFile(metrics, JSON.stringify({ theme, width, history: history.length, initialMs, samples }, null, 2));
    await info.attach("PM history metrics", { path: metrics, contentType: "application/json" });

    for (let attempt = 0; attempt < 4 && (await entry(2999).count()) === 0; attempt++) {
      const last = await messages.last().textContent();
      await log.evaluate(node => { node.scrollTop = node.scrollHeight; });
      await expect(messages.last()).not.toHaveText(last!);
    }
    await log.evaluate(node => { node.scrollTop = node.scrollHeight; });
    await expect(entry(2999)).toBeInViewport();
    expect(errors).toEqual([]);
  });
}

test.describe("Home conversation with the Project Manager", () => {
  test("Issue #7638 Home and the shared header refresh the designated PM avatar", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await page.clock.install();
    await openHome(page, themeOf(testInfo.project.name), {
      ...twoProjects,
      pm: { name: "Cedar", avatar: { kind: "preset", preset_id: "fox" } },
    });
    const faces = page.locator("[data-home] .gwt-home__face, .app-head [data-destination=pm] .app-head__avatar");
    await expect(page.locator(".gwt-home__head").getByRole("heading", { name: "Cedar" })).toBeVisible();
    await expect(faces.first()).toBeVisible();
    await expect(faces.last()).toBeVisible();
    await expect.poll(() => faces.allTextContents()).toEqual(["🦊", "🦊", "🦊", "🦊", "🦊", "🦊"]);

    const image = await page.evaluate(() => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 128;
      const context = canvas.getContext("2d")!;
      context.fillStyle = "#2980b9";
      context.fillRect(0, 0, canvas.width, canvas.height);
      return canvas.toDataURL("image/png");
    });
    await page.evaluate(image => (window as any).__setHomePm({ name: "Cedar", avatar: { kind: "upload", image } }), image);
    await page.clock.runFor(5_100);
    await expect(faces.locator("img")).toHaveCount(6);
    for (const avatar of await faces.locator("img").all()) {
      await expect(avatar).toHaveAttribute("src", image);
      await expect(avatar).toHaveJSProperty("naturalWidth", 128);
      const box = await avatar.boundingBox();
      expect(box?.width).toBeGreaterThan(0);
      expect(box?.width).toBe(box?.height);
    }
    await page.evaluate(() => (window as any).__setHomePm(undefined));
    await page.clock.runFor(5_100);
    await expect(page.locator("[data-home]").getByRole("alert")).toHaveCount(2);
    await expect(faces.locator("img")).toHaveCount(3);
    expect(errors).toEqual([]);
  });

  test("AC-1: Home is the conversation and carries no project status or count", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await openHome(page, themeOf(testInfo.project.name), twoProjects);
    const home = page.locator("[data-home]");
    await expect(page).toHaveTitle("gwt — Home");
    await expect(home.getByRole("heading", { name: "Project Manager" })).toBeVisible();
    const topics = home.locator(".gwt-home__timeline > [data-home-topic]");
    await expect(topics).toHaveCount(3);
    // Oldest at the top, newest at the bottom, and the newest is in view.
    await expect(topics.nth(0)).toContainText("Approve the plan for the refresh?");
    await expect(topics.nth(1)).toContainText("Fix the mapping and continue the import?");
    await expect(topics.nth(2)).toContainText("Save and push the unsaved changes?");
    await expect(topics.nth(2)).toBeInViewport();
    await expect(topics.nth(0).locator(".gwt-home__meta")).toContainText("Alpha");
    await expect(topics.nth(0).locator(".gwt-home__meta")).toContainText("Decision waiting");
    await expect(topics.nth(0)).toContainText("I recommend: Approve and start. Nothing else changes by waiting.");
    // The head and the composer stay put while the timeline scrolls.
    await expect(home.locator(".gwt-home__head")).toBeInViewport();
    await expect(home.locator(".gwt-home__composer")).toBeInViewport();
    await expect(home.locator("[data-home-waitjump]")).toHaveText("3 decisions waiting");
    await expect(page.locator('.app-head [data-destination="pm"] .app-head__count')).toHaveText("3");
    await expect(page.locator('.app-head [data-destination="pm"]')).toHaveAttribute("aria-current", "page");
    // Nothing about a project beyond its name is on Home.
    await expect(page.locator("[data-hub-projects]")).toBeHidden();
    for (const selector of ["a[href^='/p/']", "[data-hub-list]", "[data-project-execution]", "[data-project-health]", "[data-close-project]"]) {
      await expect(home.locator(selector)).toHaveCount(0);
    }
    // The waiting count jumps to the first decision that waits.
    await home.locator("[data-home-waitjump]").click();
    await expect(topics.nth(0)).toBeFocused();
    await expect(topics.nth(0)).toHaveClass(/is-highlighted/);
    await page.screenshot({ path: testInfo.outputPath("home-conversation.png"), fullPage: true });
    expect(errors).toEqual([]);
  });

  test("AC-2: an answer from the PM's message reaches its project and settles the topic", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await openHome(page, themeOf(testInfo.project.name), twoProjects);
    const home = page.locator("[data-home]");
    const topics = home.locator(".gwt-home__timeline > [data-home-topic]");
    await topics.nth(1).getByRole("button", { name: "Fix and continue" }).click();
    await expect(topics).toHaveCount(2);
    const sent = await page.evaluate(() => (window as any).__homeSent);
    expect(sent).toHaveLength(1);
    // Issue #7742: the answer is a queue operation of the decision's project.
    expect(sent[0]).toMatchObject({ kind: "home_answer_decision", project_key: beta.project_key,
      decision_id: "d-mid", answer: "1. Fix and continue" });
    // Settled topics sit above the open ones, with a search and the exchange.
    const resolved = home.locator("[data-home-resolved]");
    await expect(resolved.locator("summary")).toContainText("Fix the mapping and continue the import?");
    await expect(home.locator("[data-home-waitjump]")).toHaveText("2 decisions waiting");
    await expect(page.locator('.app-head [data-destination="pm"] .app-head__count')).toHaveText("2");
    expect(await resolved.evaluate(node => node.compareDocumentPosition(document.querySelector("[data-home-greeting]")!) & Node.DOCUMENT_POSITION_FOLLOWING)).toBeTruthy();

    // A free-form reply goes through the one composer, to the topic it names.
    await topics.nth(0).getByRole("button", { name: "Reply to this topic" }).click();
    const draft = home.getByRole("textbox", { name: "Message to the Project Manager" });
    await expect(draft).toBeFocused();
    await expect(home.locator("[data-home-replychip]")).toContainText("Approve the plan for the refresh?");
    await draft.fill("Start it, but keep the old screen for a week");
    await draft.press("Enter");
    await expect(topics).toHaveCount(1);
    await expect(draft).toHaveValue("");
    await expect(home.locator("[data-home-replychip]")).toHaveCount(0);
    const replies = await page.evaluate(() => (window as any).__homeSent);
    expect(replies[1]).toMatchObject({ kind: "home_answer_decision", project_key: alpha.project_key,
      decision_id: "d-old", answer: "Start it, but keep the old screen for a week" });

    // Decide later is not an answer: the decision keeps waiting and counting,
    // and the header shows the same count (AC-4).
    await topics.nth(0).getByRole("button", { name: "Decide later" }).click();
    await expect(topics.nth(0).getByRole("status")).toHaveText("Kept waiting. Nothing was decided.");
    await expect(home.locator("[data-home-waitjump]")).toHaveText("1 decision waiting");
    await expect(page.locator('.app-head [data-destination="pm"] .app-head__count')).toHaveText("1");
    const deferred = await page.evaluate(() => (window as any).__homeSent);
    expect(deferred).toHaveLength(3);
    expect(deferred[2]).toMatchObject({ kind: "home_defer_decision", project_key: alpha.project_key, decision_id: "d-new" });

    // Resolved topics are searched by name and deleted only after a confirmation.
    await resolved.getByRole("searchbox", { name: "Search resolved topics" }).fill("mapping");
    await expect(resolved.locator("details")).toHaveCount(1);
    await resolved.locator("details summary").click();
    await resolved.getByRole("button", { name: "Delete this topic" }).click();
    const dialog = page.getByRole("dialog", { name: "Delete this topic?" });
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(resolved.locator("details")).toHaveCount(1);
    await resolved.getByRole("button", { name: "Delete this topic" }).click();
    await page.getByRole("dialog", { name: "Delete this topic?" }).getByRole("button", { name: "Delete topic" }).click();
    await expect(dialog).toHaveCount(0);
    await expect(resolved.locator("details")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("home-answered.png"), fullPage: true });
    expect(errors).toEqual([]);
  });

  test("AC-2: a new message names its project and a refusal keeps the message", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await openHome(page, themeOf(testInfo.project.name), { ...twoProjects, refuse: [alpha.project_key] });
    const home = page.locator("[data-home]");
    const draft = home.getByRole("textbox", { name: "Message to the Project Manager" });
    const send = home.locator("[data-home-send]");
    await draft.fill("Move the nightly update to the evening");
    // Several projects are open: nothing is sent until one is named.
    await expect(send).toBeDisabled();
    await expect(home.locator(".gwt-home__composer-status")).toHaveText("Choose the project this is for, or name it with @Project name.");
    await draft.fill("@beta Move the nightly update to the evening");
    await expect(home.locator("[data-home-project]")).toHaveValue(beta.project_key);
    await send.click();
    await expect(home.locator("[data-home-sent]")).toContainText("Delivered to the Project Manager of Beta.");
    await expect(home.locator("[data-home-sent]")).toBeInViewport();
    await expect(draft).toHaveValue("");
    await home.locator("[data-home-project]").selectOption(alpha.project_key);
    await draft.fill("And tell me when it is done");
    await send.click();
    await expect(home.locator(".gwt-home__composer-status")).toContainText("Not sent: No Project Manager is running for this project.");
    await expect(draft).toHaveValue("And tell me when it is done");
    const sent = await page.evaluate(() => (window as any).__homeSent.map((message: any) => message.project_key));
    expect(sent).toEqual([beta.project_key, alpha.project_key]);
    expect(errors).toEqual([]);
  });

  for (const width of [1440, 390]) {
    test(`Issue #7844 Home shows reply waiting at ${width}px`, async ({ page }, info) => {
      await page.setViewportSize({ width, height: 900 });
      const errors = collectBrowserErrors(page);
      const project = { project_key: alpha.project_key, title: "Alpha", pm_available: true,
        decisions: [decision("older", 5, "Older decision", ["Approve", "Defer"])] };
      const theme = themeOf(info.project.name);
      await page.clock.install();
      await openHome(page, theme, { projects: [alpha], conversation: [project], holdSend: true });
      const home = page.locator("[data-home]");
      const log = home.getByRole("log");
      const typing = log.getByRole("status", { name: "Project Manager is replying" });
      const draft = home.getByRole("textbox", { name: "Message to the Project Manager" });
      async function send(body: string) {
        await draft.fill(body);
        await home.locator("[data-home-send]").click();
        await expect(typing).toHaveText("…");
        await expect(typing).toBeInViewport();
        await expect(log.locator(":scope > :last-child")).toHaveText("…");
      }
      const receipt = (ok: boolean) => page.evaluate(ok => {
        const win = window as any;
        win.__homeEvent({ kind: "pane_send_result", ok, request_id: win.__homeSent.at(-1).request_id });
      }, ok);
      await send("What is next?");
      await receipt(true);
      await expect(draft).toHaveValue("");
      await expect(typing).toBeVisible();
      const screenshot = info.outputPath(`reply-waiting-${theme}-${width}.png`);
      await home.screenshot({ path: screenshot });
      await info.attach("PM reply waiting", { path: screenshot, contentType: "image/png" });
      const conversation = { availability: "ready", messages: [{ id: "u1", role: "user", text: "What is next?" }] };
      await page.evaluate(project => (window as any).__homeProjects([project]), { ...project, conversation });
      await page.clock.runFor(5000);
      await expect(typing).toBeVisible();
      await page.evaluate(project => (window as any).__homeProjects([project]), { ...project, conversation: {
        ...conversation, messages: [...conversation.messages, { id: "a1", role: "assistant", text: "Check the queue." }],
      } });
      await page.clock.runFor(5000);
      await expect(log.locator(":scope > :last-child")).toContainText("Check the queue.");
      await expect(typing).toHaveCount(0);

      await send("Please confirm.");
      await receipt(false);
      await expect(typing).toHaveCount(0);
      await expect(draft).toHaveValue("Please confirm.");

      await send("Are you still there?");
      await receipt(true);
      await expect(draft).toHaveValue("");
      await page.evaluate(project => (window as any).__homeProjects([project]), { ...project, pm_interrupted: true });
      await page.clock.runFor(5000);
      await expect(typing).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }

  test("AC-3: with no project the PM guides to the ways of adding one", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await openHome(page, themeOf(testInfo.project.name), { projects: [], conversation: [] });
    const home = page.locator("[data-home]");
    await expect(home.locator("[data-home-greeting]")).toContainText("There are no projects yet");
    await expect(home.getByRole("button", { name: /Import from GitHub/ })).toBeVisible();
    await expect(home.getByRole("button", { name: /Open a folder/ })).toBeVisible();
    await expect(home.getByRole("button", { name: /Clone from a URL/ })).toBeVisible();
    await expect(home.locator("[data-home-waitjump]")).toBeDisabled();
    await expect(home.locator("[data-home-send]")).toBeDisabled();
    await page.screenshot({ path: testInfo.outputPath("home-empty.png"), fullPage: true });
    await home.getByRole("button", { name: /Clone from a URL/ }).click();
    const clone = page.getByRole("dialog", { name: "Clone Project" });
    await expect(clone).toBeVisible();
    await expect(clone.locator("#clone-project-url-input")).toBeVisible();
    expect(errors).toEqual([]);
  });

  test("AC-3: twenty projects stay one conversation and Projects is one step away", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    const many = Array.from({ length: 20 }, (_, index) => project(index + 1, `Project ${index + 1}`));
    await openHome(page, themeOf(testInfo.project.name), {
      projects: many,
      conversation: many.map((entry, index) => ({
        project_key: entry.project_key, title: entry.title, pm_available: true,
        decisions: index === 7 ? [decision("d-1", 5, "Ship it?", ["Ship", "Hold"])] : [],
      })),
    });
    const home = page.locator("[data-home]");
    await expect(home.locator(".gwt-home__timeline > [data-home-topic]")).toHaveCount(1);
    await expect(home.locator(".gwt-home__meta").filter({ hasText: "Decision waiting" })).toContainText("Project 8");
    await expect(home.locator("a[href^='/p/']")).toHaveCount(0);
    await expect(home.locator("[data-home-project] option")).toHaveCount(21);
    // No sideways scroll at phone width (design item 247).
    await page.setViewportSize({ width: 390, height: 780 });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await expect(home.locator(".gwt-home__composer")).toBeInViewport();
    await page.setViewportSize({ width: 1280, height: 720 });

    await page.locator('.app-head [data-destination="projects"]').click();
    const projects = page.locator("[data-hub-projects]");
    await expect(projects).toBeVisible();
    await expect(home).toBeHidden();
    await expect(page.locator('.app-head [data-destination="projects"]')).toHaveAttribute("aria-current", "page");
    await expect(projects.locator('[data-hub-list="open"] a')).toHaveCount(20);
    await projects.getByRole("searchbox", { name: "Search projects" }).fill("project 2");
    await expect(projects.locator('[data-hub-list="open"] a')).toHaveText(["Project 2", "Project 20"]);
    await expect(projects.locator('[data-hub-list="open"] a').first()).toHaveAttribute("href", `/p/${key(2)}`);
    await page.screenshot({ path: testInfo.outputPath("projects.png"), fullPage: true });
    await page.locator('.app-head [data-destination="pm"]').click();
    await expect(home).toBeVisible();
    await expect(projects).toBeHidden();
    expect(errors).toEqual([]);
  });
});

for (const width of [1440, 390]) {
  test(`Issue #7848 Home sends attachments at ${width}px`, async ({ page }, info) => {
    await page.setViewportSize({ width, height: 900 });
    const errors = collectBrowserErrors(page);
    const theme = themeOf(info.project.name);
    const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aH6cAAAAASUVORK5CYII=", "base64");
    const project = { project_key: alpha.project_key, title: alpha.title, pm_available: true, decisions: [] };
    await openHome(page, theme, { projects: [alpha], conversation: [project] }, async () => {
      if (!process.env.GWT_PLAYWRIGHT_BASE_URL) {
        await page.route("**/internal/attachment-upload-token", route => route.fulfill({ json: { token: "test-token" } }));
        await page.route("**/internal/attachments/upload?*", route => {
          const query = new URL(route.request().url()).searchParams;
          return route.fulfill({ json: { upload_id: randomUUID(), filename: query.get("filename"),
            mime_type: query.get("mime_type"), size: Number(query.get("size")) } });
        });
      }
    });
    const home = page.locator("[data-home]");
    const draft = home.getByRole("textbox", { name: "Message to the Project Manager" });
    await expect(home.getByRole("button", { name: "Attach files" })).toBeVisible();
    const choosing = page.waitForEvent("filechooser");
    await home.getByRole("button", { name: "Attach files" }).click();
    await (await choosing).setFiles([
      { name: "brief.txt", mimeType: "text/plain", buffer: Buffer.from("Read this") },
      { name: "remove.txt", mimeType: "text/plain", buffer: Buffer.from("Remove this") },
    ]);
    await draft.evaluate(node => {
      const transfer = new DataTransfer();
      transfer.items.add(new File(["Dropped"], "notes.txt", { type: "text/plain" }));
      node.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: transfer }));
    });
    await draft.evaluate((node, bytes) => {
      const transfer = new DataTransfer();
      transfer.items.add(new File([new Uint8Array(bytes)], "clipboard.png", { type: "image/png" }));
      node.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: transfer }));
    }, [...png]);
    const list = home.getByRole("list", { name: "Draft attachments" });
    await expect(list.getByRole("listitem")).toHaveCount(4);
    await list.getByRole("button", { name: "Remove remove.txt" }).click();
    await expect(list.getByRole("listitem")).toHaveCount(3);
    await draft.fill("Please review these attachments.");
    await home.locator("[data-home-send]").click();
    const sent = home.locator("[data-home-sent]");
    await expect(sent).toContainText("Please review these attachments.");
    await expect(sent.getByText("brief.txt")).toBeVisible();
    await expect(sent.getByText("notes.txt")).toBeVisible();
    const preview = sent.getByRole("img", { name: "clipboard.png" });
    await expect(preview).toBeVisible();
    await expect.poll(() => preview.evaluate((image: HTMLImageElement) => image.naturalWidth)).toBe(1);
    await expect(list).toHaveCount(0);
    const sentEvents = await page.evaluate(() => (window as any).__homeSent);
    expect(sentEvents).toHaveLength(1);
    expect(sentEvents[0].text).toBe("Please review these attachments.\r");
    expect(sentEvents[0].attachments.map((file: any) => file.filename)).toEqual(["brief.txt", "notes.txt", "clipboard.png"]);
    const paths = sentEvents[0].attachments.map((file: any) =>
      `".gwt/drop-files/${file.upload_id.replaceAll("-", "")}-${file.filename}"`);
    await page.evaluate(project => (window as any).__homeProjects([project]), {
      ...project, conversation: { availability: "ready", messages: [{
        id: "native-user", role: "user",
        text: `Please review these attachments.\n\nFiles: [${paths.join(", ")}]`,
      }] },
    });
    await expect(sent).toHaveCount(0, { timeout: 10000 });
    const native = home.getByRole("list", { name: "Attachments", exact: true });
    await expect(native.getByRole("img", { name: "clipboard.png" })).toBeVisible();
    await expect(native.getByText("brief.txt")).toBeVisible();
    const screenshot = info.outputPath(`pm-attachments-${theme}-${width}.png`);
    await home.screenshot({ path: screenshot });
    await info.attach("PM chat attachments", { path: screenshot, contentType: "image/png" });
    expect(errors).toEqual([]);
  });
}
