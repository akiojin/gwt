/* Issue #4538 AC-1 / AC-3 — Hub picker and route not-found in a real
 * browser with a deterministic WebSocket backend (no live gwt required).
 */
import { expect, test, type Page } from "@playwright/test";
import { HUB_URL, ORIGIN_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const projectA = { id: "tab-a", project_key: "0123456789abcdef", title: "Alpha", kind: "git" };
const projectB = { id: "tab-b", project_key: "fedcba9876543210", title: "Beta", kind: "git" };

function collectBrowserErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
}

async function installRouteBackend(page: Page) {
  await page.addInitScript(({ projects }) => {
    (window as any).__gwtSent = [];
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      url: string;
      readyState = 0;
      projectKey: string | null;
      constructor(url: string) {
        super();
        this.url = url;
        this.projectKey = new URL(url).searchParams.get("repo_hash");
        setTimeout(() => {
          this.readyState = 1;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      send(raw: string) {
        const message = JSON.parse(raw);
        (window as any).__gwtSent.push({ scope: this.projectKey, message });
        if (message.kind !== "frontend_ready") return;
        const known = projects.find((project: any) => project.project_key === this.projectKey);
        const reply = this.projectKey
          ? known
            ? { kind: "workspace_state", workspace: { app_version: "e2e", tabs: [{ ...known, project_root: "/fixture", workspace: { viewport: { x: 0, y: 0, zoom: 1 }, windows: [] } }], active_tab_id: known.id, recent_projects: [] } }
            : { kind: "project_not_found", project_key: this.projectKey }
          : {
              kind: "hub_state",
              hub: {
                app_version: "9.9.9",
                projects,
                recent_projects: [{ path: "/Users/e2e/secret/beta", title: "Beta", kind: "git", project_key: projects[1].project_key }],
              },
            };
        setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(reply) })), 0);
      }
      close() {
        this.readyState = 3;
        this.dispatchEvent(new CloseEvent("close"));
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  }, { projects: [projectA, projectB] });
}

test.describe("Project Hub and routes", () => {
  test("root is a picker with path-free new-tab Project links", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(HUB_URL);
    await expect(page).toHaveTitle("gwt — Hub");
    await expect(page.locator("[data-hub]")).toBeVisible();
    for (const selector of ["#app", "#project-tabs", "#op-rail", ".workspace-window", "#project-switcher-button"]) {
      await expect(page.locator(selector)).toHaveCount(0);
    }
    await expect(page.locator('[data-hub-action="open-folder"]')).toBeVisible();
    await expect(page.locator('[data-hub-action="clone"]')).toBeVisible();
    const open = page.locator('[data-hub-list="open"] a');
    await expect(open).toHaveCount(2);
    await expect(open.nth(0)).toHaveAttribute("href", "/p/0123456789abcdef");
    await expect(open.nth(1)).toHaveAttribute("href", "/p/fedcba9876543210");
    await expect(page.locator('[data-hub-list="recent"] a')).toHaveAttribute("href", "/p/fedcba9876543210");
    const links = page.locator("[data-hub] a");
    for (const link of await links.all()) {
      await expect(link).toHaveAttribute("target", "_blank");
      await expect(link).toHaveAttribute("rel", "noopener");
    }
    expect(await page.content()).not.toContain("/Users/e2e/secret");
    await page.locator('[data-hub-action="open-folder"]').click();
    await expect.poll(() => page.evaluate(() => (window as any).__gwtSent.map((entry: any) => entry.message.kind)))
      .toContain("open_project_dialog");
    await page.screenshot({ path: testInfo.outputPath("hub.png") });
    expect(errors).toEqual([]);
  });

  test("a Project route binds its own scope and links the Hub in a new tab", async ({ page }) => {
    const errors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(`${ORIGIN_URL}p/${projectA.project_key}`);
    await expect(page.locator(".project-tab[aria-current='page']")).toBeVisible();
    const scopes = await page.evaluate(() => (window as any).__gwtSent
      .filter((entry: any) => entry.message.kind === "frontend_ready").map((entry: any) => entry.scope));
    expect(scopes).toContain(projectA.project_key);
    expect(scopes.filter((scope: string | null) => scope && scope !== projectA.project_key)).toEqual([]);
    const home = page.locator("#project-home-link");
    await expect(home).toBeVisible();
    await expect(home).toHaveAttribute("href", "/");
    await expect(home).toHaveAttribute("target", "_blank");
    await expect(home).toHaveAttribute("rel", "noopener");
    expect(errors).toEqual([]);
  });

  test("an unknown Project hash shows the path-free not-found view", async ({ page }) => {
    const errors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(`${ORIGIN_URL}p/cccccccccccccccc`);
    const view = page.locator("[data-route-not-found]");
    await expect(view).toBeVisible();
    await expect(page).toHaveTitle("gwt — Project not found");
    await expect(view.locator("a")).toHaveAttribute("href", "/");
    await expect(page.locator("#app")).toHaveCount(0);
    expect(errors).toEqual([]);
  });
});
