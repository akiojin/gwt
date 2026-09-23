/** SPEC-1924 T-618: real snapshot loading and live Logs scope isolation. */
import { appendFile, mkdir, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { expect, test } from "@playwright/test";
import {
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CHECK_HOME = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";

test("Logs Project and Global isolate snapshots and live events", async ({ page }, testInfo) => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  expect(CHECK_HOME, "Set GWT_PLAYWRIGHT_CHECK_HOME to the browser-check isolated HOME").not.toBe("");
  expect(await realpath(CHECK_HOME)).not.toBe(await realpath(homedir()));

  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    await page.addInitScript(() => {
      window.WebSocket = new Proxy(window.WebSocket, {
        construct(Target, args) {
          const socket = new Target(...args as ConstructorParameters<typeof WebSocket>);
          (window as any).__logsScopeSocket = socket;
          return socket;
        },
      });
    });
    await gotoLiveGwt(page, BASE, { enableTestBridge: true });
    await openLiveGwtProject(page);
    await clearLiveLaunchWizard(page);
    await expect(page.locator(".project-tab[aria-current='page']")).toBeVisible();
    const theme = testInfo.project.use.colorScheme === "light" ? "light" : "dark";
    await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    const scopeHandle = await page.waitForFunction(() => {
      const messages = (window as any).__gwtPlaywrightMessages ?? [];
      const workspace = [...messages].reverse().find((entry: any) =>
        entry.payload?.kind === "workspace_state"
      )?.payload.workspace;
      return workspace?.tabs?.find((tab: any) => tab.id === workspace.active_tab_id)?.project_scope || "";
    });
    const scope = await scopeHandle.jsonValue() as string;
    expect(scope).toMatch(/^[a-zA-Z0-9_-]+$/);
    const marker = `logs-scope-${randomUUID()}`;
    const otherScope = `other-${randomUUID()}`;
    const projectMessage = `${marker}-project-A`;
    const otherMessage = `${marker}-project-B`;
    const globalMessage = `${marker}-global`;
    const date = new Date().toISOString().slice(0, 10);
    const seed = async (directory: string, message: string, projectScope?: string) => {
      await mkdir(directory, { recursive: true });
      // Appending preserves the isolated server's own concurrent log output.
      await appendFile(join(directory, `gwt.log.${date}`), `${JSON.stringify({
        timestamp: new Date().toISOString(),
        level: "INFO",
        target: "gwt.playwright.scope",
        project_scope: projectScope,
        fields: { message },
      })}\n`);
    };
    await seed(join(CHECK_HOME, ".gwt", "projects", scope, "logs"), projectMessage, scope);
    await seed(join(CHECK_HOME, ".gwt", "projects", otherScope, "logs"), otherMessage, otherScope);
    await seed(join(CHECK_HOME, ".gwt", "logs"), globalMessage);

    await sendLiveGwtEvent(page, {
      kind: "create_window", preset: "logs",
      bounds: { x: 80, y: 80, width: 1080, height: 660 },
    });
    const root = page.locator('.workspace-window[data-preset="logs"]').filter({ visible: true }).last();
    const selector = root.locator(".logs-scope-select");
    await expect(selector).toBeVisible();
    await expect(selector.locator("option")).toHaveText(["Project", "Global"]);
    // A preceding theme run can reuse the singleton with Global selected.
    await selector.selectOption("project");
    await root.getByRole("button", { name: "Refresh logs", exact: true }).click();
    await root.locator(".logs-search-input").fill(marker);
    const rows = root.locator(".logs-entry");
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText(projectMessage);
    await expect(root).not.toContainText(otherMessage);
    await expect(root).not.toContainText(globalMessage);

    let entryId = 9000000;
    const inject = async (message: string, projectScope?: string) => {
      await page.evaluate(({ message, projectScope, id }) => {
        (window as any).__logsScopeSocket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({
          kind: "log_entry_appended",
          entry: {
            id, timestamp: new Date().toISOString(), severity: "info",
            source: "gwt.playwright.scope", message, project_scope: projectScope,
            fields: {},
          },
        }) }));
      }, { message, projectScope, id: ++entryId });
    };
    await inject(`${marker}-foreign-live`, otherScope);
    await inject(`${marker}-global-live`);
    await inject(`${marker}-project-live`, scope);
    await expect(rows).toHaveCount(2);
    await expect(rows.first()).toContainText(`${marker}-project-live`);
    await expect(root).not.toContainText(`${marker}-foreign-live`);
    await expect(root).not.toContainText(`${marker}-global-live`);

    await selector.selectOption("global");
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText(globalMessage);
    await expect(root).not.toContainText(projectMessage);
    await expect(root).not.toContainText(otherMessage);
    await inject(`${marker}-foreign-live`, otherScope);
    await inject(`${marker}-project-live`, scope);
    await inject(`${marker}-global-live`);
    await expect(rows).toHaveCount(2);
    await expect(rows.first()).toContainText(`${marker}-global-live`);
    await expect(root).not.toContainText(`${marker}-project-live`);
    await expect(root).not.toContainText(`${marker}-foreign-live`);
    await expect(selector).toHaveValue("global");
    await page.screenshot({ path: testInfo.outputPath("logs-scope.png") });
    expect(errors, "console and page errors").toEqual([]);
  });
});
