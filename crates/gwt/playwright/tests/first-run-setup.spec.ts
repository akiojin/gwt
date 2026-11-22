/* Issue #7731 (SPEC #7007 slice N-4) — the first-run setup on the Hub page,
 * in a real browser with a deterministic WebSocket backend.
 *
 * AC-1: the screen never says it is shown only at first launch, and a
 *       Settings link opens the same screen on the step it names.
 * AC-2: one row chooses the Project Manager; nothing goes on until one is chosen.
 * AC-3: the first launch ends with a registered tool and a designated PM.
 * AC-4: without a registered tool the setup does not go on and says why.
 */
import { expect, test, type Page } from "@playwright/test";
import { HUB_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

type Tool = { name: string; tool: string; installed: boolean };
type Member = { member_id: string; name: string };
type Backend = {
  tools: Tool[];
  members: Member[];
  pm?: { member_id?: string; name: string } | null;
  // What "Check again" finds: the tools after the person installed one.
  toolsAfterRetry?: Tool[];
};

const claude: Tool = { name: "Claude Code 1", tool: "claude", installed: true };

function collectBrowserErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  return errors;
}

// The stub keeps the machine's setup the way the backend does: a tool gate on
// the save, and `needs_setup` derived from the tools, the ledger and the PM.
async function installSetupBackend(page: Page, backend: Backend) {
  await page.addInitScript(backend => {
    const state = { tools: backend.tools, pm: backend.pm ?? null, reads: 0, sent: [] as any[] };
    (window as any).__setupSent = state.sent;
    const view = () => {
      const registered = state.tools.some(tool => tool.installed);
      return { kind: "first_run_setup", view: {
        tools: state.tools, members: backend.members, pm: state.pm, name_max_chars: 40,
        needs_setup: !registered || (backend.members.length === 0 && !state.pm),
      } };
    };
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
          this.reply({ kind: "hub_state", hub: { app_version: "9.9.9", projects: [], recent_projects: [] } });
        } else if (message.kind === "load_home_conversation") {
          this.reply({ kind: "home_conversation", request_id: message.request_id, projects: [] });
        } else if (message.kind === "get_first_run_setup") {
          state.reads += 1;
          if (state.reads > 1 && backend.toolsAfterRetry) state.tools = backend.toolsAfterRetry;
          this.reply(view());
        } else if (message.kind === "save_first_run_setup") {
          state.sent.push(message);
          if (!state.tools.some(tool => tool.installed)) {
            this.reply({ kind: "first_run_setup_error", message: "Register a tool before choosing the Project Manager." });
            return;
          }
          const member = backend.members.find(member => member.member_id === message.pm_member_id);
          state.pm = member ? { member_id: member.member_id, name: member.name } : { name: message.pm_name };
          this.reply(view());
        }
      }
      close() {
        this.readyState = 3;
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  }, backend);
}

async function openHub(page: Page, theme: string, backend: Backend, search = "") {
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  await installEmbeddedRoutes(page);
  await installSetupBackend(page, backend);
  await page.goto(`${HUB_URL}${search}`);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
}

const themeOf = (name: string) => (name.includes("light") ? "light" : "dark");

test.describe("First-run setup", () => {
  test("AC-3: a new machine is taken through its tool and its Project Manager to Home", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await openHub(page, themeOf(testInfo.project.name), { tools: [claude], members: [] });
    const setup = page.locator("[data-setup]");
    // Nobody asked for the setup: the machine needs it, so it opens.
    await expect(setup).toBeVisible();
    await expect(page.locator("[data-home]")).toBeHidden();
    await expect(setup.getByRole("heading", { level: 1, name: "First-run setup" })).toBeVisible();
    await expect(setup.getByRole("heading", { level: 2, name: "Welcome to gwt" })).toBeVisible();
    await expect(setup.locator('.gwt-setup__steps [aria-current="step"]')).toHaveText("1. Welcome");

    await setup.getByRole("button", { name: "Start — Tools" }).click();
    await expect(setup.getByRole("heading", { level: 2, name: "Register a tool" })).toBeFocused();
    await expect(setup.locator("[data-setup-tool]")).toHaveText("Claude Code 1Registered");
    await setup.getByRole("button", { name: "Next — Project Manager" }).click();

    // The ledger is empty, so the Project Manager is a new agent and needs a name.
    const next = setup.getByRole("button", { name: "Next — Project" });
    await expect(setup.getByRole("radio", { name: "A new agent" })).toBeChecked();
    await expect(next).toBeDisabled();
    await expect(setup.locator("[data-setup-reason]")).toHaveText("Next becomes available once the new agent has a name.");
    await setup.getByRole("textbox", { name: "Name of the new agent" }).fill("Aoi");
    await expect(setup.locator("[data-setup-reason]")).toHaveCount(0);
    await next.click();
    await expect(setup.getByRole("heading", { level: 2, name: "Open a project" })).toBeVisible();
    expect(await page.evaluate(() => (window as any).__setupSent)).toEqual([{ kind: "save_first_run_setup", pm_name: "Aoi" }]);

    // A project is optional.
    await setup.getByRole("button", { name: "Next — Done" }).click();
    await expect(setup.getByRole("heading", { level: 2, name: "You are ready" })).toBeVisible();
    await expect(setup).toContainText("1 tool registered.");
    await expect(setup).toContainText("Aoi serves as Project Manager.");
    await setup.getByRole("button", { name: "Go to Home" }).click();
    await expect(page.locator("[data-home]")).toBeVisible();
    await expect(setup).toBeHidden();
    expect(errors).toEqual([]);
  });

  test("AC-4: without a registered tool the setup stops and says why", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await openHub(page, themeOf(testInfo.project.name), {
      tools: [{ name: "Codex 1", tool: "codex", installed: false }], members: [], toolsAfterRetry: [claude],
    });
    const setup = page.locator("[data-setup]");
    await setup.getByRole("button", { name: "Start — Tools" }).click();
    const next = setup.getByRole("button", { name: "Next — Project Manager" });
    await expect(next).toBeDisabled();
    await expect(setup.locator("[data-setup-reason]")).toHaveText("Next is unavailable because no tool is registered on this machine.");
    await expect(next).toHaveAttribute("aria-describedby", "gwt-setup-reason");
    await expect(setup.locator('[data-setup-tool][data-installed="false"]')).toHaveText("Codex 1Not installed");
    await expect(setup.locator("[data-setup-tools-empty]")).toContainText("Install a supported agent tool, such as Claude Code or Codex, and sign in to it.");
    await expect(setup.locator('.gwt-setup__steps [aria-current="step"]')).toHaveText("2. Tools");

    // Once a tool is installed, checking again opens the way.
    await setup.getByRole("button", { name: "Check again" }).click();
    await expect(setup.locator("[data-setup-tool]")).toHaveText("Claude Code 1Registered");
    await expect(setup.locator("[data-setup-reason]")).toHaveCount(0);
    await expect(next).toBeEnabled();
    expect(errors).toEqual([]);
  });

  test("AC-1, AC-2: a Settings link reopens the Project Manager step, where one row chooses the PM", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    const members = [{ member_id: "m1", name: "Aoi" }, { member_id: "m2", name: "Ren" }];
    await openHub(page, themeOf(testInfo.project.name), { tools: [claude], members }, "?view=setup&step=pm");
    const setup = page.locator("[data-setup]");
    await expect(setup.getByRole("heading", { level: 2, name: "Choose the Project Manager" })).toBeVisible();
    // AC-1: reopened on a machine that is already set up, and it says so.
    await expect(setup).toContainText("Every step stays available from Settings.");
    await expect(setup).not.toContainText(/first launch|only (at|on|during|shown)/i);

    // AC-2: one row, every candidate in it, and no sentence that names the PM for you.
    await expect(setup.locator("[data-setup-pm]")).toHaveCount(1);
    const choices = setup.getByRole("radiogroup", { name: "Project Manager" }).getByRole("radio");
    await expect(choices).toHaveCount(3);
    for (const choice of await choices.all()) await expect(choice).not.toBeChecked();
    const next = setup.getByRole("button", { name: "Next — Project" });
    await expect(next).toBeDisabled();
    await expect(setup.locator("[data-setup-reason]")).toHaveText("Next becomes available once a Project Manager is chosen.");

    await setup.getByRole("radio", { name: "Ren" }).check();
    await next.click();
    await expect(setup.getByRole("heading", { level: 2, name: "Open a project" })).toBeVisible();
    expect(await page.evaluate(() => (window as any).__setupSent)).toEqual([{ kind: "save_first_run_setup", pm_member_id: "m2" }]);

    // Going back shows the saved choice in the same row.
    await setup.getByRole("button", { name: "Back" }).click();
    await expect(setup.getByRole("radio", { name: "Ren" })).toBeChecked();
    expect(errors).toEqual([]);
  });

  test("a machine that is already set up opens on Home", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await openHub(page, themeOf(testInfo.project.name), { tools: [claude], members: [{ member_id: "m1", name: "Aoi" }] });
    await expect(page.locator("[data-home]")).toBeVisible();
    await expect(page.locator("[data-setup]")).toBeHidden();
    expect(errors).toEqual([]);
  });
});
