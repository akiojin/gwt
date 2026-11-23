/* Issue #7643 (SPEC #7007 T-025) — the Themes screen of a project.
 *
 * AC-1: theme list → theme detail → the issues under it, and an issue opens
 *       in the Issues screen.
 * AC-2: "Issues completed today" is the number the backend counted.
 * AC-3: a project with no themes says what a theme is.
 * A-23: deleting a theme asks first and says what happens: its GitHub Issue
 *       is closed, the issues under it stay, and it can be reopened.
 * Issue #7644: a theme the PM recorded from the conversation on Home is listed
 *       before any issue sits under it, says where it came from, and opens
 *       the PM on that message.
 *
 * Runs against the embedded frontend with a stub backend, in both themes,
 * with zero console / page errors.
 */
import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

type Sent = { kind?: string; preset?: string; number?: number; request_id?: number };

const issue = (number: number, title: string, state: string, extra: Record<string, unknown> = {}) => ({
  number, title, state, url: `https://github.com/example/repo/issues/${number}`, in_repository: true, ...extra,
});
const THEMES = [
  {
    number: 7007, title: "Full screen refresh", summary: "Rebuild every screen around the conversation with the Project Manager.",
    url: "https://github.com/example/repo/issues/7007", issues_total: 4, issues_completed: 1,
    issues: [
      issue(7015, "Shared status bar", "open"),
      issue(7016, "End-to-end regression check", "open"),
      issue(7021, "Home and decisions", "closed"),
      issue(12, "Design tokens", "open", { in_repository: false, url: "https://github.com/example/design/issues/12" }),
    ],
  },
  {
    number: 7541, title: "Daemon shutdown fix", summary: "",
    url: "https://github.com/example/repo/issues/7541", issues_total: 3, issues_completed: 3,
    issues: [issue(7542, "Stop on signal", "closed"), issue(7543, "Drain queue", "closed"), issue(7544, "Regression test", "closed")],
  },
];

function captureErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(String(error)));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  return errors;
}

async function installThemesBackend(page: Page, themes: unknown[], options: { refuseDelete?: string } = {}) {
  await page.addInitScript(({ themes, refuseDelete, appOrigin, theme }) => {
    // Init scripts can also run in the opaque document before app navigation.
    if (location.origin === appOrigin) localStorage.setItem("gwt:ui:theme", theme);
    try { window.sessionStorage.setItem("gwt:ui:briefing", "1"); } catch { /* no-op */ }
    (window as any).__gwtPlaywrightTestBridge = true;
    let current = themes as any[];
    const sent: unknown[] = [];
    (window as any).__themesSent = sent;
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [{
          id: "tab-1", title: "Themes Fixture", project_root: "/fixture",
          project_key: location.pathname.split("/")[2], kind: "git",
          workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [] },
        }],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      constructor(url: string) {
        super();
        (this as any).url = url;
        (this as any).readyState = FixtureWebSocket.CONNECTING;
        setTimeout(() => {
          (this as any).readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      send(raw: string) {
        const message = JSON.parse(raw);
        sent.push(message);
        const view = () => ({ themes: current, completed_today: 5, truncated: false });
        if (message.kind === "frontend_ready") this.emit(workspaceState);
        if (message.kind === "load_themes") this.emit({ kind: "themes", request_id: message.request_id, view: view() });
        if (message.kind === "delete_theme") {
          if (refuseDelete) {
            this.emit({ kind: "theme_error", request_id: message.request_id, message: refuseDelete });
            return;
          }
          current = current.filter(theme => theme.number !== message.number);
          this.emit({ kind: "themes", request_id: message.request_id, view: view() });
        }
      }
      close() {
        (this as any).readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }
      emit(payload: unknown) {
        setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) })), 0);
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  }, {
    themes, refuseDelete: options.refuseDelete ?? null, appOrigin: new URL(APP_URL).origin,
    theme: test.info().project.name.includes("light") ? "light" : "dark",
  });
}

const sentOf = (page: Page, kind: string): Promise<Sent[]> =>
  page.evaluate(kind => (window as any).__themesSent.filter((message: Sent) => message.kind === kind), kind);

test.describe("Themes", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  async function open(page: Page, themes: unknown[], options: { refuseDelete?: string } = {}) {
    await installEmbeddedRoutes(page);
    await installThemesBackend(page, themes, options);
    await page.goto(APP_URL);
    await expect(page.locator("html")).toHaveAttribute("data-theme", test.info().project.name.includes("light") ? "light" : "dark");
    await page.locator('.op-rail__surface[data-surface="themes"]').click();
    await expect(page.locator('.op-rail__surface[data-surface="themes"]')).toHaveAttribute("aria-pressed", "true");
    return page.locator(".project-themes");
  }

  test("a theme opens from the list and an issue under it opens in Issues (AC-1, AC-2)", async ({ page }, info) => {
    const errors = captureErrors(page);
    const screen = await open(page, THEMES);

    await expect(screen.getByRole("heading", { level: 1, name: "Themes" })).toBeVisible();
    const rows = screen.locator("[data-themes-list] [data-theme-number]");
    await expect(rows).toHaveCount(2);
    await expect(rows.nth(0)).toContainText("Full screen refresh (#7007)");
    await expect(rows.nth(0)).toContainText("4 issues under it · 1 completed");
    await expect(rows.nth(0)).toContainText("In progress");
    await expect(rows.nth(1)).toContainText("Almost done");
    // AC-2: the number the backend counted, next to the theme counts.
    await expect(screen.locator('[data-themes-stat="completed_today"]')).toHaveText("5");
    await expect(screen.locator('[data-themes-stat="in_progress"]')).toHaveText("1");
    await expect(screen.locator('[data-themes-stat="almost_done"]')).toHaveText("1");
    await expect(screen).not.toContainText("Unknown");
    // The bar is drawn: a quarter of its track for one of four issues.
    const bar = rows.nth(0).getByRole("progressbar");
    const [track, fill] = await Promise.all([bar.boundingBox(), bar.locator("i").boundingBox()]);
    expect(Math.round((fill!.width / track!.width) * 100)).toBe(25);
    await page.screenshot({ path: info.outputPath("themes-list.png"), fullPage: true });

    await rows.nth(0).click();
    await expect(screen).toHaveAttribute("data-themes-view", "theme");
    await expect(screen.getByRole("heading", { level: 1, name: "Full screen refresh (#7007)" })).toBeFocused();
    await expect(screen).toContainText("This theme is a GitHub Issue: #7007");
    await expect(screen.getByRole("link", { name: "Open on GitHub" })).toHaveAttribute("href", "https://github.com/example/repo/issues/7007");
    await expect(screen).toContainText("Rebuild every screen around the conversation");
    const issues = screen.locator("[data-theme-issues] [data-theme-issue]");
    await expect(issues).toHaveCount(4);
    await expect(issues.nth(2)).toContainText("Completed");
    await expect(issues.nth(3)).toHaveAttribute("href", "https://github.com/example/design/issues/12");
    await page.screenshot({ path: info.outputPath("theme-detail.png"), fullPage: true });

    // A completed issue opens the same way as an open one.
    await issues.nth(2).click();
    await expect.poll(() => sentOf(page, "create_window")).toEqual([{ kind: "create_window", preset: "issue" }]);

    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("a project without themes says what a theme is (AC-3)", async ({ page }, info) => {
    const errors = captureErrors(page);
    const screen = await open(page, []);

    const empty = screen.locator("[data-themes-empty]");
    await expect(empty.getByRole("heading", { name: "No themes yet" })).toBeVisible();
    await expect(empty).toContainText("The PM records a theme when you ask for something or decide something in the conversation on Home.");
    await expect(screen.locator("[data-themes-list]")).toHaveCount(0);
    await expect(screen.locator('[data-themes-stat="completed_today"]')).toHaveText("5");
    await page.screenshot({ path: info.outputPath("themes-empty.png"), fullPage: true });

    await screen.getByRole("button", { name: "Refresh" }).click();
    await expect.poll(async () => (await sentOf(page, "load_themes")).length).toBe(2);
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("deleting a theme asks first and names what happens to its issues (A-23)", async ({ page }, info) => {
    const errors = captureErrors(page);
    const screen = await open(page, THEMES);
    await screen.locator('[data-theme-number="7007"]').click();

    const trigger = screen.getByRole("button", { name: "Delete theme" });
    await trigger.click();
    const dialog = page.getByRole("dialog", { name: "Delete this theme?" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
    await expect(dialog).toContainText('"Full screen refresh" leaves this list: GitHub Issue #7007 is closed on GitHub as not planned.');
    await expect(dialog).toContainText("The 4 issues under it stay as they are.");
    await expect(dialog).toContainText("Nothing is erased. Reopen the Issue on GitHub to bring the theme back.");
    await page.screenshot({ path: info.outputPath("theme-delete-dialog.png") });

    // Escape cancels and hands focus back; nothing was deleted.
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(trigger).toBeFocused();
    expect(await sentOf(page, "delete_theme")).toEqual([]);

    await trigger.click();
    await dialog.getByRole("button", { name: "Delete theme" }).click();
    await expect(dialog).toHaveCount(0);
    await expect(screen).toHaveAttribute("data-themes-view", "themes");
    await expect(screen).toContainText("Theme #7007 was deleted.");
    await expect(screen.locator("[data-themes-list] [data-theme-number]")).toHaveCount(1);
    const deletes = await sentOf(page, "delete_theme");
    expect(deletes.map(message => message.number)).toEqual([7007]);
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("a theme recorded from the conversation leads back to it (Issue #7644)", async ({ page }, info) => {
    const errors = captureErrors(page);
    const recorded = {
      number: 7800, title: "Search in the log screen", summary: "Add search to the log screen.",
      url: "https://github.com/example/repo/issues/7800", issues_total: 0, issues_completed: 0, issues: [],
      recorded_from: { session_id: "pm-session", message_id: "native-turn", recorded_at: "2026-11-23T10:00:00Z" },
    };
    const screen = await open(page, [recorded, ...THEMES]);
    // A theme made on GitHub has no conversation to open.
    await screen.locator('[data-theme-number="7007"]').click();
    await expect(screen.locator("[data-theme-origin]")).toHaveCount(0);
    await screen.locator("[data-themes-back]").click();

    const row = screen.locator('[data-theme-number="7800"]');
    await expect(row).toContainText("0 issues under it · 0 completed");
    await row.click();
    const origin = screen.locator("[data-theme-origin]");
    await expect(origin.getByRole("heading", { name: "Where it came from" })).toBeVisible();
    await expect(origin).toContainText("Recorded automatically from the conversation on Home");
    await expect(screen.locator("[data-theme-no-issues]")).toContainText("No issues sit under this theme yet.");
    await page.screenshot({ path: info.outputPath("theme-recorded.png"), fullPage: true });

    // The PM opens on the message the theme came from.
    await origin.getByRole("button", { name: "Show in conversation" }).click();
    await expect.poll(async () => (await sentOf(page, "open_pm_agent")).length).toBe(1);
    expect(errors, "zero console/page errors").toEqual([]);
  });

  test("a refused delete stays in the dialog with the reason", async ({ page }) => {
    const errors = captureErrors(page);
    const screen = await open(page, THEMES, { refuseDelete: "GitHub did not close Issue #7007. Closing an Issue needs write access to the repository." });
    await screen.locator('[data-theme-number="7007"]').click();
    await screen.getByRole("button", { name: "Delete theme" }).click();
    const dialog = page.getByRole("dialog", { name: "Delete this theme?" });
    await dialog.getByRole("button", { name: "Delete theme" }).click();
    await expect(dialog.getByRole("alert")).toContainText("GitHub did not close Issue #7007.");
    await expect(dialog.getByRole("button", { name: "Delete theme" })).toBeEnabled();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(screen.getByRole("heading", { level: 1, name: "Full screen refresh (#7007)" })).toBeVisible();
    expect(errors, "zero console/page errors").toEqual([]);
  });
});
