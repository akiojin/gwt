/* Issue #7639 (SPEC #7007 T-021) — Home, the conversation with the Project
 * Manager, in a real browser with a deterministic WebSocket backend.
 *
 * AC-1: Home is the PM conversation and shows no project work status or count.
 * AC-2: a decision is answered from the PM's message and the answer goes to
 *       the project it belongs to.
 * AC-3: the path stays whole with no project and with twenty.
 */
import { expect, test, type Page } from "@playwright/test";
import { HUB_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const key = (index: number) => index.toString(16).padStart(16, "0");
const project = (index: number, title: string) => ({ id: `tab-${index}`, project_key: key(index), title, kind: "git" });
const alpha = project(1, "Alpha");
const beta = project(2, "Beta");

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();
const decision = (id: string, minutes: number, question: string, options: string[]) => ({
  id, kind: "decision", body: question, body_html: "", created_at: minutesAgo(minutes),
  structured: { details: "", refs: [], summary: {
    result: `${question} is waiting on you.`, impact: "Only this project is affected.",
    decision: { question, recommendation: options[0], reason: "Nothing else changes by waiting.", options },
  } },
});

type Backend = {
  projects: unknown[];
  recent?: unknown[];
  conversation: Array<{ project_key: string; title: string; pm_available: boolean; decisions: unknown[]; error?: string }>;
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

// The stub answers the catalog, the decisions read and a PM send, and drops an
// answered decision from later reads, the way a PM retires it.
async function installHomeBackend(page: Page, backend: Backend) {
  await page.addInitScript(backend => {
    const state = { sent: [] as any[], answered: new Set<string>() };
    (window as any).__homeSent = state.sent;
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      constructor(public url: string) {
        super();
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
          this.reply({ kind: "home_conversation", request_id: message.request_id, projects: backend.conversation.map(project => ({
            ...project, decisions: project.decisions.filter((item: any) => !state.answered.has(`${project.project_key}:${item.id}`)),
          })) });
        } else if (message.kind === "home_pm_send") {
          state.sent.push(message);
          const refused = (backend.refuse || []).includes(message.project_key);
          if (!refused) {
            const owner = backend.conversation.find(project => project.project_key === message.project_key);
            const answered = owner?.decisions.find((item: any) => message.text.startsWith(`Answer to "${item.structured.summary.decision.question}"`)) as any;
            if (answered) state.answered.add(`${message.project_key}:${answered.id}`);
          }
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

async function openHome(page: Page, theme: string, backend: Backend) {
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  await installEmbeddedRoutes(page);
  await installHomeBackend(page, backend);
  await page.goto(HUB_URL);
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

test.describe("Home conversation with the Project Manager", () => {
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
    expect(sent[0].project_key).toBe(beta.project_key);
    expect(sent[0].text).toBe('Answer to "Fix the mapping and continue the import?": 1. Fix and continue\r');
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
    expect(replies[1].project_key).toBe(alpha.project_key);
    expect(replies[1].text).toBe('Answer to "Approve the plan for the refresh?": Start it, but keep the old screen for a week\r');

    // Decide later is not an answer.
    await topics.nth(0).getByRole("button", { name: "Decide later" }).click();
    await expect(topics.nth(0).getByRole("status")).toHaveText("Kept waiting. Nothing was decided.");
    await expect(home.locator("[data-home-waitjump]")).toHaveText("1 decision waiting");
    expect(await page.evaluate(() => (window as any).__homeSent.length)).toBe(2);

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
