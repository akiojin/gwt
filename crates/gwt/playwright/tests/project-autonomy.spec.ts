/* Issue #7728 (SPEC #7007 N-1) — the one project stop/start on the Hub
 * project list, in a real browser with a deterministic WebSocket backend.
 *
 * AC-1: the list uses the same confirmation and operation as Settings.
 * AC-2: monitoring has no switch of its own; the project stop covers it.
 */
import { expect, test, type Page } from "@playwright/test";
import { HUB_PROJECTS_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

const project = { id: "tab-1", project_key: "0000000000000001", title: "Alpha", kind: "git" };

function collectBrowserErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  return errors;
}

// The stub answers the catalog and records each stop/start. The test moves
// the project through Stop pending to Stopped the way the runtime reports it.
async function installHubBackend(page: Page) {
  await page.addInitScript(project => {
    const sent: any[] = [];
    let socket: any = null;
    let autonomy = { monitor_enabled: true, pm_paused: false, active_agents: 1 };
    const catalog = () => ({ kind: "hub_state", hub: { app_version: "9.9.9", recent_projects: [], projects: [{ ...project, autonomy }] } });
    (window as any).__autonomySent = sent;
    (window as any).__setAutonomy = (next: typeof autonomy) => { autonomy = next; socket?.reply(catalog()); };
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
      readyState = 0;
      constructor(public url: string) {
        super();
        socket = this;
        setTimeout(() => { this.readyState = 1; this.dispatchEvent(new Event("open")); }, 0);
      }
      reply(event: unknown) {
        setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(event) })), 0);
      }
      send(raw: string) {
        const message = JSON.parse(raw);
        if (message.kind === "frontend_ready") this.reply(catalog());
        else if (message.kind === "load_home_conversation") this.reply({ kind: "home_conversation", request_id: message.request_id, projects: [] });
        else if (message.kind === "set_project_autonomy") sent.push(message);
      }
      close() { this.readyState = 3; }
    }
    (window as any).WebSocket = FixtureWebSocket;
  }, project);
}

test("Issue #7728 the Hub list stops and starts a project through the shared confirmation", async ({ page }, info) => {
  const errors = collectBrowserErrors(page);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  await installEmbeddedRoutes(page);
  await installHubBackend(page);
  await page.goto(HUB_PROJECTS_URL);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  const row = page.locator(`[data-project-autonomy="${project.project_key}"]`);
  const state = row.locator("[data-project-autonomy-state]");
  await expect(state).toHaveText("Running");
  const sent = () => page.evaluate(() => (window as any).__autonomySent);

  const dialog = page.getByRole("dialog", { name: "Stop this project?" });
  await row.getByRole("button", { name: "Stop…" }).click();
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("Alpha");
  await expect(dialog).toContainText("New assignments and the Project Manager patrol stop now");
  await expect(dialog).toContainText("A stop cannot be cancelled");
  const path = info.outputPath(`project-stop-hub-${info.project.name}.png`);
  await page.screenshot({ path });
  await info.attach(`project-stop-hub-${info.project.name}`, { path, contentType: "image/png" });
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await sent()).toEqual([]);

  await row.getByRole("button", { name: "Stop…" }).click();
  await dialog.getByRole("button", { name: "Stop project" }).click();
  await expect(dialog).toHaveCount(0);
  expect(await sent()).toEqual([{ kind: "set_project_autonomy", project_key: project.project_key, running: false }]);

  await page.evaluate(() => (window as any).__setAutonomy({ monitor_enabled: false, pm_paused: true, active_agents: 1 }));
  await expect(state).toHaveText("Stop pending · 1 agent finishing");
  await expect(row.getByRole("button")).toHaveCount(0);
  await page.evaluate(() => (window as any).__setAutonomy({ monitor_enabled: false, pm_paused: true, active_agents: 0 }));
  await expect(state).toHaveText("Stopped");
  await row.getByRole("button", { name: "Start" }).click();
  await expect(dialog).toHaveCount(0);
  expect((await sent()).at(-1)).toEqual({ kind: "set_project_autonomy", project_key: project.project_key, running: true });
  expect(errors).toEqual([]);
});
