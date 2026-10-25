/* Issue #4538 AC-1 / AC-3 — Hub picker and route not-found in a real
 * browser with a deterministic WebSocket backend (no live gwt required).
 */
import { expect, test, type Page } from "@playwright/test";
import { HUB_URL, ORIGIN_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { withLiveGwtBackendLock } from "./_helpers/live-gwt";

const projectA = { id: "tab-a", project_key: "0123456789abcdef", title: "Alpha", kind: "git" };
const projectB = { id: "tab-b", project_key: "fedcba9876543210", title: "Beta", kind: "git" };

test("live Clone opens the committed Project tab and offers an existing Project on duplicate", async ({ page, context }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const sources = process.env.GWT_PLAYWRIGHT_CLONE_SOURCES;
  const destinations = process.env.GWT_PLAYWRIGHT_CLONE_DESTINATIONS;
  test.skip(!base || !sources || !destinations, "Requires the isolated local Clone fixture");
  await withLiveGwtBackendLock(base!, testInfo, async () => {
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    const errors = collectBrowserErrors(page);
    const otherHub = await context.newPage();
    const otherErrors = collectBrowserErrors(otherHub);
    let foreignCloneEvents = 0;
    otherHub.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => {
      try { if (JSON.parse(String(payload)).kind.startsWith("clone_project_")) foreignCloneEvents += 1; }
      catch { /* Non-JSON frames are not Clone replies. */ }
    }));
    let project: Page | undefined;
    let duplicate: Page | undefined;
    try {
      await otherHub.goto(new URL("/?action=clone", base!).href);
      await expect(otherHub.getByRole("dialog", { name: "Clone Project" })).toBeVisible();
      await page.goto(new URL("/?action=clone", base!).href);
      const dialog = page.getByRole("dialog", { name: "Clone Project" });
      await expect(dialog).toBeVisible();
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await page.locator("#clone-project-url-input").fill(`${sources}/clone-${theme}`);
      await page.locator("#clone-project-parent-input").fill(`${destinations}/${theme}`);
      [project] = await Promise.all([page.waitForEvent("popup"), page.locator("#clone-project-start").click()]);
      const projectErrors = collectBrowserErrors(project);
      await expect(project).toHaveURL(/\/p\/[a-f0-9]{16}$/, { timeout: 60_000 });
      await expect(project.locator("#close-project-button")).toBeVisible();
      await expect(project.locator("#app")).toBeVisible();
      await expect(project).toHaveTitle(new RegExp(`clone-${theme}`));
      await expect(project.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(dialog).toBeHidden();
      await expect(page.locator("[data-hub]")).toBeVisible();
      await expect(page.locator(".gwt-hub__notice a")).toHaveAttribute("href", new URL(project.url()).pathname);
      await expect(otherHub.getByRole("dialog", { name: "Clone Project" })).toBeVisible();
      await expect(otherHub.locator(".gwt-hub__notice")).toBeHidden();
      expect(foreignCloneEvents).toBe(0);
      await project.screenshot({ path: testInfo.outputPath("cloned-project.png") });

      await page.locator('[data-hub-action="clone"]').click();
      [duplicate] = await Promise.all([page.waitForEvent("popup"), page.locator("#clone-project-start").click()]);
      await expect.poll(() => duplicate!.isClosed()).toBe(true);
      await expect(dialog).toContainText(/already registered/i);
      await expect(dialog.getByRole("link", { name: "Open existing project" })).toHaveAttribute("href", new URL(project.url()).pathname);
      await page.screenshot({ path: testInfo.outputPath("clone-duplicate.png") });
      expect([...errors, ...otherErrors, ...projectErrors]).toEqual([]);
      expect(foreignCloneEvents).toBe(0);
    } finally {
      if (duplicate && !duplicate.isClosed()) await duplicate.close();
      if (project && !project.isClosed()) await project.close();
      await otherHub.close();
    }
  });
});

test("live isolated Hub opens a manual path and acknowledges an already-open Project", async ({ page }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const projectRoot = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT;
  test.skip(!base || !projectRoot, "Requires an isolated checkout server and project path");
  await withLiveGwtBackendLock(base!, testInfo, async () => {
    const errors = collectBrowserErrors(page);
    let openedEvents = 0;
    page.on("websocket", (socket) => socket.on("framereceived", ({ payload }) => {
      try {
        if (JSON.parse(String(payload)).kind === "project_opened") openedEvents += 1;
      } catch { /* Ignore non-JSON frames. */ }
    }));
    await page.goto(new URL("/", base!).href);
    await expect(page.locator("[data-hub]")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    await page.getByText("Enter folder path…", { exact: true }).click();
    await page.locator("[data-open-project-path]").fill(projectRoot!);
    const notice = page.locator(".gwt-hub__notice");
    const previousOpened = openedEvents;
    await page.locator("[data-open-project-submit]").click();
    await expect.poll(() => openedEvents).toBe(previousOpened + 1);
    await expect(notice).toContainText("Opened ");
    await expect(notice.getByRole("link", { name: "Open in a new tab" })).toHaveAttribute("href", /^\/p\/[a-f0-9]+$/);
    const projectUrl = await notice.locator("a").getAttribute("href");
    const projects = page.locator('[data-hub-list="open"] a');
    await expect(page.locator(`[data-hub-list="open"] a[href="${projectUrl}"]`)).toHaveCount(1);
    const openCount = await projects.count();
    await page.locator("[data-open-project-submit]").click();
    await expect.poll(() => openedEvents).toBe(previousOpened + 2);
    await expect(notice).toContainText("Opened ");
    await expect(notice.locator("a")).toHaveAttribute("href", projectUrl!);
    await expect(projects).toHaveCount(openCount);
    await expect(page.locator(".gwt-hub__error")).toBeHidden();
    await page.screenshot({ path: testInfo.outputPath("live-hub-reopened.png") });
    await page.goto(new URL(projectUrl!, base!).href);
    await expect(page.locator("#close-project-button")).toBeVisible();
    await expect(page.locator("#project-home-link")).toHaveCount(0);
    await expect(page.getByRole("link", { name: "Open the gwt Hub in a new tab" })).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("live-project-header.png") });
    await page.goto(new URL("/?action=clone", base!).href);
    const cloneDialog = page.getByRole("dialog", { name: "Clone Project" });
    await expect(cloneDialog).toBeVisible();
    await expect(cloneDialog).toHaveAttribute("aria-modal", "true");
    await page.screenshot({ path: testInfo.outputPath("live-tray-clone.png") });
    await cloneDialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(cloneDialog).toBeHidden();
    expect(errors).toEqual([]);
  });
});

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
    (window as any).__gwtRouteSockets = [];
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
        (window as any).__gwtRouteSockets.push(this);
        if (!this.projectKey) (window as any).__gwtHubReply = (event: unknown) =>
          this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(event) }));
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
        if (reply.kind === 'workspace_state') (window as any).__gwtRouteWorkspace = reply.workspace;
        setTimeout(() => this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(reply) })), 0);
      }
      close() {
        this.readyState = 2;
        setTimeout(() => {
          this.readyState = 3;
          this.dispatchEvent(new CloseEvent("close"));
        }, 0);
      }
    }
    Object.defineProperty(window, "WebSocket", { configurable: true, value: FixtureWebSocket });
  }, { projects: [projectA, projectB] });
}

test.describe("Project Hub and routes", () => {
  test("Hub root catalog pulls consecutive snapshots and keeps action focus", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    if (!base) await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(new URL("/", base || ORIGIN_URL).href);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const control = page.locator('[data-hub-action="open-folder"]');
    await expect(control).toBeVisible();
    await control.focus();
    await control.evaluate(element => { (window as any).__hubRootControl = element; });
    const literal = '<img src=x onerror="window.__hubRootExecuted=true"> Updated catalog';
    for (const [index, title] of ["First catalog", literal].entries()) {
      await page.evaluate(({ title, index, project }) => {
        (window as any).__gwtHubReply({ kind: "hub_state", hub: {
          app_version: `9.9.${20 + index}`,
          projects: [{ ...project, title }],
          recent_projects: [{ title, project_key: null, path: "/private/catalog-path" }],
        } });
      }, { title, index, project: projectA });
      await expect(page.locator(".gwt-hub__version")).toHaveText(`v9.9.${20 + index}`);
      const link = page.locator('[data-hub-list="open"] a');
      await expect(link).toHaveCount(1);
      await expect(link).toHaveText(title);
      await expect(link).toHaveAttribute("href", `/p/${projectA.project_key}`);
      await expect(link).toHaveAttribute("target", "_blank");
      await expect(link).toHaveAttribute("rel", "noopener");
      await expect(page.locator('[data-hub-list="recent"] .is-pending')).toHaveText(title);
      await expect(control).toBeFocused();
      expect(await control.evaluate(element => element === (window as any).__hubRootControl)).toBe(true);
    }
    await expect(page.locator(".gwt-hub img")).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__hubRootExecuted)).toBeUndefined();
    expect(await page.locator(".gwt-hub").textContent()).not.toContain("/private/catalog-path");
    const notice = page.locator(".gwt-hub__notice");
    const error = page.locator(".gwt-hub__error");
    for (const project of [projectA, { ...projectB, title: literal }]) {
      await page.evaluate(project => (window as any).__gwtHubReply({ ...project, kind: "project_opened" }), project);
      await expect(notice).toHaveText(`Opened ${project.title}. Open in a new tab`);
      await expect(notice).toBeVisible();
      await expect(notice.locator("a")).toHaveAttribute("href", `/p/${project.project_key}`);
      await expect(notice.locator("a")).toHaveAttribute("target", "_blank");
      await expect(notice.locator("a")).toHaveAttribute("rel", "noopener");
      await expect(control).toBeFocused();
    }
    // A new local open clears the prior result; the same server failure must
    // render again after that reset rather than being treated as unchanged.
    await page.getByText("Enter folder path…", { exact: true }).click();
    for (const message of [literal, literal, ""]) {
      await page.locator("[data-open-project-path]").fill("/private/open-path");
      await page.locator("[data-open-project-submit]").click();
      await expect(notice).toBeHidden();
      await expect(error).toBeHidden();
      await control.focus();
      await page.evaluate(message => (window as any).__gwtHubReply({ kind: "project_open_error", message }), message);
      await expect(error).toHaveText(message || "Could not open the project.");
      await expect(error).toBeVisible();
      await expect(control).toBeFocused();
      expect(await control.evaluate(element => element === (window as any).__hubRootControl)).toBe(true);
    }
    await expect(page.locator(".gwt-hub img")).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__hubRootExecuted)).toBeUndefined();
    await page.screenshot({ path: testInfo.outputPath("hub-root-catalog.png"), fullPage: true });
    expect(errors).toEqual([]);
  });

  test("Project hub catalog pulls consecutive snapshots without replacing active controls", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    if (!base) await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(new URL(`/p/${projectA.project_key}`, base || ORIGIN_URL).href);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(page.locator("#close-project-button")).toBeVisible();
    const control = page.locator("#window-list-button");
    await control.focus();
    await control.evaluate(element => { (window as any).__hubCatalogControl = element; });
    const emit = (hub: Record<string, unknown>) => page.evaluate(hub => {
      (window as any).__gwtHubReply({ kind: "hub_state", hub });
    }, hub);
    for (const version of ["9.9.10", "9.9.11"]) {
      await emit({ app_version: version, projects: [projectA, projectB], recent_projects: [] });
      await expect(page.locator("#app-version")).toHaveText(`v${version}`);
      await expect(page.locator("#close-project-button")).toBeVisible();
      await expect(page.locator("#project-picker")).not.toHaveClass(/visible/);
      await expect(control).toBeFocused();
      expect(await control.evaluate(element => element === (window as any).__hubCatalogControl)).toBe(true);
    }
    expect(await page.evaluate(() => (window as any).__gwtRouteSockets.filter((socket: any) =>
      socket.projectKey === "0123456789abcdef").length)).toBe(1);
    // The recent list belongs to the picker, so expose it by removing the
    // current Project from the catalog before updating its entries.
    const literal = '<img src=x onerror="window.__hubCatalogExecuted=true"> Recent catalog';
    for (const [index, title] of ["First recent catalog", literal].entries()) {
      await emit({ app_version: `9.9.${12 + index}`, projects: [projectB],
        recent_projects: [{ path: "/fixture/recent", title, kind: "git" }] });
      await expect(page.locator("#app-version")).toHaveText(`v9.9.${12 + index}`);
      await expect(page.locator("#project-picker")).toBeVisible();
      await expect(page.locator(".recent-project-row")).toHaveCount(1);
      await expect(page.locator(".recent-project-title")).toHaveText(title);
    }
    await expect(page.locator(".recent-project-row img")).toHaveCount(0);
    expect(await page.evaluate(() => (window as any).__hubCatalogExecuted)).toBeUndefined();
    await page.locator(".recent-project-row").click();
    await expect.poll(() => page.evaluate(() => (window as any).__gwtSent.some((entry: any) =>
      entry.scope === null && entry.message.kind === "reopen_recent_project"
      && entry.message.path === "/fixture/recent"))).toBe(true);
    await page.screenshot({ path: testInfo.outputPath("hub-catalog.png"), fullPage: true });
    expect(errors).toEqual([]);
  });

  test("Work Projection Prune feedback preserves results and repeated errors", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    const messages: string[] = [];
    page.on("dialog", async dialog => {
      expect(dialog.type()).toBe("alert");
      messages.push(dialog.message());
      await dialog.accept();
    });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    if (!base) await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(new URL(`/p/${projectA.project_key}`, base || ORIGIN_URL).href);
    await expect(page.locator("#close-project-button")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    expect(messages).toEqual([]);
    const emit = (event: Record<string, unknown>) => page.evaluate(({ event, projectKey }) => {
      const socket = (window as any).__gwtRouteSockets.find((socket: any) =>
        socket.projectKey === projectKey && socket.readyState === 1);
      if (!socket) throw new Error("Project socket is not open");
      socket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(event) }));
    }, { event, projectKey: projectA.project_key });
    for (const mode of ["dry_run", "apply"]) {
      await emit({ kind: "workspace_projection_prune_result", mode, archived: 2, deleted: 0, skipped: 1 });
      await expect.poll(() => messages.length).toBe(mode === "dry_run" ? 1 : 2);
    }
    for (let count = 3; count <= 4; count += 1) {
      await emit({ kind: "workspace_projection_prune_error", message: "Prune was refused" });
      await expect.poll(() => messages.length).toBe(count);
    }
    await emit({ kind: "window_list", windows: [] });
    await expect(page.locator("#close-project-button")).toBeVisible();
    expect(messages).toEqual([
      "Work Projection Prune (dry-run)\n  archived: 2\n  deleted: 0\n  skipped: 1",
      "Work Projection Prune (applied)\n  archived: 2\n  deleted: 0\n  skipped: 1",
      "Work Projection Prune error: Prune was refused",
      "Work Projection Prune error: Prune was refused",
    ]);
    await page.screenshot({ path: testInfo.outputPath("work-projection-prune-feedback.png") });
    expect(errors).toEqual([]);
  });

  test("Project not-found state fences other projects and closes route connections", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme),
      testInfo.project.name.includes("light") ? "light" : "dark");
    const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
    if (!base) await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(new URL(`/p/${projectA.project_key}`, base || ORIGIN_URL).href);
    await expect(page.locator("#close-project-button")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    await page.clock.install();
    const emit = (key: string) => page.evaluate(({ key, projectKey }) => {
      const sockets = (window as any).__gwtRouteSockets.filter((socket: any) =>
        socket.projectKey === projectKey && socket.readyState === 1);
      if (!sockets.length) throw new Error("Project socket is not open");
      if (key === projectKey) sockets[0].dispatchEvent(new MessageEvent('message', {
        data: JSON.stringify({ kind: 'workspace_state', workspace: (window as any).__gwtRouteWorkspace }),
      }));
      sockets[0].dispatchEvent(new MessageEvent("message", {
        data: JSON.stringify({ kind: "project_not_found", project_key: key }),
      }));
    }, { key, projectKey: projectA.project_key });
    await emit(projectB.project_key);
    await page.clock.runFor(100);
    await expect(page.locator("[data-route-not-found]")).toHaveCount(0);
    await expect(page.locator("#close-project-button")).toBeVisible();
    await emit(projectA.project_key);
    await page.clock.runFor(100);
    const view = page.locator("[data-route-not-found]");
    await expect(view).toBeVisible();
    await expect(page).toHaveTitle("gwt — Project not found");
    await expect(view.locator("a")).toHaveAttribute("href", "/");
    await expect(page.locator("#app")).toHaveCount(0);
    expect(await view.textContent()).not.toMatch(/\/fixture|secret|[A-Z]:\\/);
    const sockets = await page.evaluate(() => (window as any).__gwtRouteSockets.map((socket: any) => socket.readyState));
    expect(sockets.length).toBeGreaterThanOrEqual(2);
    expect(sockets.every((state: number) => state === 3)).toBe(true);
    await page.clock.fastForward(30_000);
    expect(await page.evaluate(() => (window as any).__gwtRouteSockets.length)).toBe(sockets.length);
    await expect(page.locator('.connection-overlay')).toHaveCount(0);
    await expect(page.locator('.render-degradation-banner')).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("project-not-found-state.png") });
    await view.getByRole('link', { name: 'Go to Hub' }).click();
    await expect(page.locator('[data-hub]')).toBeVisible();
    await expect(page).toHaveTitle('gwt — Hub');
    expect(errors).toEqual([]);
  });

  test("tray Open Project URL opens the folder picker without another click", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(`${HUB_URL}?action=open-folder`);
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    await expect(page.locator('[data-hub-action="open-folder"]')).toHaveText("Choosing folder…");
    await expect.poll(() => page.evaluate(() => (window as any).__gwtSent
      .filter((entry: any) => entry.message.kind === "open_project_dialog").length)).toBe(1);
    await page.evaluate(() => (window as any).__gwtHubReply({ kind: "picker_started", purpose: "open", request_id: 1 }));
    await page.evaluate(() => (window as any).__gwtHubReply({ kind: "picker_cancelled", purpose: "open", request_id: 1 }));
    await expect(page.locator('[data-hub-action="open-folder"]')).toHaveText("Open Folder…");
    await page.screenshot({ path: testInfo.outputPath("tray-open-project.png") });
    expect(errors).toEqual([]);
  });

  test("tray Clone URL opens the shared clone dialog without another click", async ({ page }, testInfo) => {
    const errors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(`${HUB_URL}?action=clone`);
    await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.includes("light") ? "light" : "dark");
    const dialog = page.getByRole("dialog", { name: "Clone Project" });
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveClass(/modal-shell/);
    await expect(dialog).toHaveAttribute("aria-modal", "true");
    await expect(dialog).toBeFocused();
    await page.screenshot({ path: testInfo.outputPath("tray-clone.png") });
    await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(dialog).toBeHidden();
    expect(errors).toEqual([]);
  });

  test("folder picker is pending once, cancels neutrally, and supports manual paths", async ({ page }) => {
    const errors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(HUB_URL);
    const open = page.locator('[data-hub-action="open-folder"]');
    await open.click();
    await expect(open).toHaveText("Choosing folder…");
    await page.evaluate(() => (window as any).__gwtHubReply({ kind: 'picker_started', purpose: 'open', request_id: 1 }));
    await open.click();
    expect(await page.evaluate(() => (window as any).__gwtSent.filter((entry: any) => entry.message.kind === 'open_project_dialog').length)).toBe(1);
    await expect(page.locator('.gwt-hub__error')).toContainText('already open');
    await page.evaluate(() => (window as any).__gwtHubReply({ kind: 'picker_cancelled', purpose: 'open', request_id: 1 }));
    await expect(open).toHaveText('Open Folder…');
    await expect(page.locator('.gwt-hub__error')).toBeHidden();
    await page.getByText('Enter folder path…', { exact: true }).click();
    await page.locator('[data-open-project-path]').fill('E:\\repos\\alpha');
    await page.locator('[data-open-project-submit]').click();
    await page.evaluate(() => (window as any).__gwtHubReply({ kind: 'project_opened', project_key: '0123456789abcdef', title: 'Alpha' }));
    await expect(page.locator('.gwt-hub__notice')).toContainText('Opened Alpha');
    await expect(page.locator('.gwt-hub__notice a')).toHaveAttribute('href', '/p/0123456789abcdef');
    await page.locator('[data-hub-action="clone"]').click();
    await page.locator('#clone-project-parent-input').fill('E:\\repos');
    await page.locator('#clone-project-url-input').fill('https://github.com/o/r.git');
    await expect(page.locator('#clone-project-start')).toBeEnabled();
    await page.locator('#clone-project-parent-button').click();
    await page.evaluate(() => (window as any).__gwtHubReply({ kind: 'picker_started', purpose: 'clone_parent', request_id: 2 }));
    await expect(page.locator('#clone-project-parent-button')).toHaveText('Choosing folder…');
    await page.evaluate(() => (window as any).__gwtHubReply({ kind: 'picker_cancelled', purpose: 'clone_parent', request_id: 2 }));
    await expect(page.locator('#clone-project-parent-button')).toHaveText('Choose Folder...');
    await expect(page.locator('#clone-project-parent-input')).toHaveValue('E:\\repos');
    expect(errors).toEqual([]);
  });

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

  test("a Project route binds its own scope without a header Hub link", async ({ page }) => {
    const errors = collectBrowserErrors(page);
    await installEmbeddedRoutes(page);
    await installRouteBackend(page);
    await page.goto(`${ORIGIN_URL}p/${projectA.project_key}`);
    await expect(page.locator("#close-project-button")).toBeVisible();
    await expect(page.locator("#project-tabs")).toHaveCount(0);
    const scopes = await page.evaluate(() => (window as any).__gwtSent
      .filter((entry: any) => entry.message.kind === "frontend_ready").map((entry: any) => entry.scope));
    expect(scopes).toContain(projectA.project_key);
    expect(scopes.filter((scope: string | null) => scope && scope !== projectA.project_key)).toEqual([]);
    await expect(page.locator("#project-home-link")).toHaveCount(0);
    await expect(page.getByRole("link", { name: "Open the gwt Hub in a new tab" })).toHaveCount(0);
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
