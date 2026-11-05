import { expect, test, type Page } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt } from "./_helpers/live-gwt";

type FixtureTab = {
  id: string;
  title: string;
  project_root: string;
  kind: "git" | "non_repo";
  workspace: {
    viewport: { x: number; y: number; zoom: number };
    windows: unknown[];
  };
};

type BrowserErrors = {
  pageErrors: string[];
  consoleErrors: string[];
};

const browserErrorsByPage = new WeakMap<Page, BrowserErrors>();

test.describe("Project open entry surfaces", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test.beforeEach(({ page }) => {
    const errors: BrowserErrors = { pageErrors: [], consoleErrors: [] };
    browserErrorsByPage.set(page, errors);
    page.on("pageerror", (error) => errors.pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") errors.consoleErrors.push(message.text());
    });
  });

  test.afterEach(({ page }) => {
    const errors = browserErrorsByPage.get(page);
    expect(errors?.pageErrors ?? [], "page errors").toEqual([]);
    expect(errors?.consoleErrors ?? [], "console errors").toEqual([]);
  });

  test("the live harness leaves the project picker rendered by default", async ({
    page,
  }) => {
    await bootProjectOpenFixture(page, []);

    const picker = page.locator("#project-picker");
    await expect(picker).toHaveClass(/visible/);
    await expect(picker).toBeVisible();
    await expect(picker).toHaveCSS("display", "flex");
    await expect(picker).not.toHaveCSS("pointer-events", "none");
    expect(await picker.evaluate((element) => element.hidden)).toBe(false);
  });

  test("Open Folder click dispatches the project dialog command", async ({
    page,
  }) => {
    await bootProjectOpenFixture(page, []);

    expect(await centerHitBelongsTo(page, "#picker-open-project", "#picker-open-project"))
      .toBe(true);
    await page.locator("#picker-open-project").click();

    await expect.poll(() => sentMessageKinds(page)).toContain("open_project_dialog");
  });

  test("active Project Ctrl+O exposes pending, cancellation and manual path fallback", async ({ page }, testInfo) => {
    const liveBase = process.env.GWT_PLAYWRIGHT_BASE_URL;
    if (liveBase) {
      await page.addInitScript(() => {
        const recordedSends: string[] = [];
        (window as any).__gwtProjectOpenFixture = {
          recordedSends,
          reply: (detail: unknown) => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail })),
        };
        const originalSend = WebSocket.prototype.send;
        WebSocket.prototype.send = function (data) {
          if (typeof data === "string" && ["open_project_dialog", "reopen_recent_project"].includes(JSON.parse(data).kind)) {
            recordedSends.push(data);
            return;
          }
          return originalSend.call(this, data);
        };
      });
      await gotoLiveGwt(page, liveBase, { enableTestBridge: true });
    } else {
      await bootProjectOpenFixture(page, [fixtureTab("git")]);
    }
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const trigger = page.locator("#op-palette-button");
    await expect(trigger).toBeEnabled();
    await trigger.focus();
    await expect(trigger).toBeFocused();
    await page.keyboard.press('Control+o');
    const modal = page.locator('#open-project-path-modal');
    await expect(modal).toBeVisible();
    await expect(modal).toHaveClass('modal-backdrop open');
    await expect(modal.locator('.modal-shell > .modal-header')).toBeVisible();
    await expect(modal.locator('.modal-shell > .modal-body')).toBeVisible();
    await expect(modal.locator('.modal-shell > .modal-footer')).toBeVisible();
    await expect(modal.getByRole('dialog')).toHaveAttribute('aria-modal', 'true');
    await expect(modal.getByRole('status')).toContainText('Choose a folder');
    const draft = modal.locator('[data-open-project-path]');
    await draft.fill('/draft/project');
    await draft.evaluate(element => {
      (window as any).__pickerDraft = element;
      (element as HTMLInputElement).setSelectionRange(2, 5);
    });
    await page.evaluate(() => (window as any).__gwtProjectOpenFixture.reply({ kind: 'picker_started', purpose: 'open', request_id: 1 }));
    await page.evaluate(() => {
      const fixture = (window as any).__gwtProjectOpenFixture;
      fixture.reply({ kind: 'picker_cancelled', purpose: 'clone_parent', request_id: 1 });
      fixture.reply({ kind: 'picker_cancelled', purpose: 'open', request_id: 99 });
    });
    await expect(modal.getByRole('dialog')).toHaveAttribute('aria-busy', 'true');
    expect(await draft.evaluate(element => element === (window as any).__pickerDraft)).toBe(true);
    expect(await draft.evaluate(element => [(element as HTMLInputElement).selectionStart, (element as HTMLInputElement).selectionEnd])).toEqual([2, 5]);
    await expect(draft).toBeFocused();
    await modal.getByRole('button', { name: 'Choosing folder…' }).click();
    expect((await sentMessageKinds(page)).filter((kind) => kind === 'open_project_dialog')).toHaveLength(1);
    await page.evaluate(() => (window as any).__gwtProjectOpenFixture.reply({ kind: 'picker_cancelled', purpose: 'open', request_id: 1 }));
    await expect(modal.getByRole('button', { name: 'Choose Folder…' })).toBeVisible();
    await expect(modal.getByRole('status')).toBeHidden();
    await modal.getByRole('button', { name: 'Choose Folder…' }).click();
    await page.evaluate(() => {
      (window as any).__gwtProjectOpenFixture.reply({ kind: 'picker_started', purpose: 'open', request_id: 2 });
      (window as any).__gwtProjectOpenFixture.reply({ kind: 'picker_error', purpose: 'open', request_id: 2, message: 'Picker timed out' });
    });
    await expect(modal.getByRole('status')).toHaveText('Picker timed out');
    await modal.getByRole('button', { name: 'Choose Folder…' }).click();
    await page.evaluate(() => (window as any).__gwtProjectOpenFixture.reply({ kind: 'picker_busy', purpose: 'open', message: 'Finish the existing dialog first' }));
    await expect(modal.getByRole('status')).toHaveText('Finish the existing dialog first');
    await expect(modal.getByRole('dialog')).toHaveAttribute('aria-busy', 'false');
    await modal.getByRole('button', { name: 'Choose Folder…' }).click();
    await page.evaluate(() => {
      const fixture = (window as any).__gwtProjectOpenFixture;
      fixture.reply({ kind: 'picker_started', purpose: 'open', request_id: 3 });
      fixture.reply({ kind: 'picker_cancelled', purpose: 'open', request_id: 2 });
    });
    await expect(modal.getByRole('dialog')).toHaveAttribute('aria-busy', 'true');
    await page.evaluate(() => (window as any).__gwtProjectOpenFixture.reply({ kind: 'picker_cancelled', purpose: 'open', request_id: 3 }));
    await expect(modal.getByRole('status')).toBeHidden();
    await expect(modal.getByRole('dialog')).toHaveAttribute('aria-busy', 'false');
    await modal.locator('[data-open-project-path]').fill('E:\\repos\\manual');
    await modal.locator('[data-open-project-submit]').click();
    await expect.poll(() => sentMessageKinds(page)).toContain('reopen_recent_project');
    await expect(modal).toBeVisible();
    const firstRequestId = await page.evaluate(() => (window as any).__gwtProjectOpenFixture.recordedSends.map(JSON.parse).filter((message: any) => message.kind === 'reopen_recent_project').at(-1).request_id);
    expect(firstRequestId).toBeTruthy();
    await page.evaluate(() => {
      const fixture = (window as any).__gwtProjectOpenFixture;
      fixture.reply({ kind: 'project_opened', request_id: 'another-client', project_key: '0123456789abcdef', title: 'Other Project' });
      fixture.reply({ kind: 'project_open_error', message: 'Unrelated failure' });
    });
    await expect(modal.getByRole('status')).toHaveText('Opening project…');
    const pickerError = page.locator('#project-picker-error');
    await expect(pickerError).toHaveText('Unrelated failure');
    const input = modal.locator('[data-open-project-path]');
    await input.focus();
    await input.evaluate(element => {
      (window as any).__openProjectInput = element;
      (element as HTMLInputElement).setSelectionRange(3, 7);
    });
    const errorMessage = '<b>Folder does not exist</b>';
    await page.evaluate(({ request_id, message }) => (window as any).__gwtProjectOpenFixture.reply({ kind: 'project_open_error', request_id, message }), { request_id: firstRequestId, message: errorMessage });
    await expect(modal.getByRole('status')).toHaveText(errorMessage);
    await expect(pickerError).toHaveText(errorMessage);
    await expect(pickerError.locator('*')).toHaveCount(0);
    expect(await pickerError.evaluate(element => element.hidden)).toBe(false);
    await expect(page.locator('#project-picker')).not.toHaveClass(/visible/);
    await expect(page.locator('#project-onboarding')).not.toHaveClass(/visible/);
    await expect(modal.locator('[data-open-project-path]')).toHaveValue('E:\\repos\\manual');
    expect(await input.evaluate(element => element === (window as any).__openProjectInput)).toBe(true);
    expect(await input.evaluate(element => [(element as HTMLInputElement).selectionStart, (element as HTMLInputElement).selectionEnd])).toEqual([3, 7]);
    await expect(input).toBeFocused();
    await page.screenshot({ animations: "disabled", path: testInfo.outputPath(`open-project-error-${theme}.png`) });
    await modal.locator('[data-open-project-path]').fill('E:\\repos\\valid');
    await modal.locator('[data-open-project-submit]').click();
    const secondRequestId = await page.evaluate(() => (window as any).__gwtProjectOpenFixture.recordedSends.map(JSON.parse).filter((message: any) => message.kind === 'reopen_recent_project').at(-1).request_id);
    expect(secondRequestId).not.toBe(firstRequestId);
    await page.evaluate((request_id) => (window as any).__gwtProjectOpenFixture.reply({ kind: 'project_opened', request_id, project_key: '0123456789abcdef', title: 'Superseded Project' }), firstRequestId);
    await expect(modal).toBeVisible();
    await page.evaluate((request_id) => (window as any).__gwtProjectOpenFixture.reply({ kind: 'project_opened', request_id, project_key: '0123456789abcdef', title: 'Valid Project' }), secondRequestId);
    await expect(modal).toBeHidden();
    await expect(trigger).toBeFocused();
    // Controlled inbound state checks the legacy picker render after errors.
    await page.evaluate(() => (window as any).__gwtProjectOpenFixture.reply({ kind: 'hub_state', hub: { app_version: 'e2e', projects: [], recent_projects: [] } }));
    await expect(page.locator('#project-picker')).toBeVisible();
    await expect(trigger).toBeEnabled();
    await page.evaluate(() => (window as any).__gwtProjectOpenFixture.reply({ kind: 'workspace_state', workspace: { app_version: 'e2e', tabs: [], active_tab_id: null, recent_projects: [] } }));
    await expect(pickerError).toBeHidden();
    await expect(pickerError).toHaveJSProperty('hidden', true);
    await page.evaluate(() => (window as any).__gwtProjectOpenFixture.reply({ kind: 'project_open_error', message: 'Another folder failed' }));
    await expect(page.locator('#project-picker')).toBeVisible();
    await expect(pickerError).toBeVisible();
    await expect(pickerError).toHaveText('Another folder failed');
    await expect(page.locator('#project-onboarding')).not.toHaveClass(/visible/);
    await page.screenshot({ animations: 'disabled', path: testInfo.outputPath(`project-picker-error-${theme}.png`) });
  });

  test("Clone from GitHub click opens the clone modal from the picker", async ({
    page,
  }) => {
    await bootProjectOpenFixture(page, []);

    expect(await centerHitBelongsTo(page, "#picker-clone-project", "#picker-clone-project"))
      .toBe(true);
    await page.locator("#picker-clone-project").click();

    // Issue #3754 owns the picker click path. Issue #3753 separately owns the
    // modal-vs-picker stacking order and its overlapping-surface hit test.
    await expect(page.locator("#clone-project-modal")).toHaveClass(/open/);
    await expect(page.locator("#clone-project-modal")).not.toHaveAttribute(
      "aria-hidden",
      "true",
    );
  });

  test("a non_repo project renders the onboarding card", async ({ page }) => {
    await bootProjectOpenFixture(page, [fixtureTab("non_repo")]);

    const onboarding = page.locator("#project-onboarding");
    await expect(onboarding).toHaveClass(/visible/);
    await expect(onboarding).toBeVisible();
    await expect(page.locator("#project-onboarding-title")).toHaveText(
      "Project setup required",
    );
    await expect(page.locator("#project-onboarding-copy")).toContainText(
      "is not a Git workspace yet",
    );
    await expect(page.locator("#project-picker")).not.toHaveClass(/visible/);

  });

  test("an active-project clone modal wins the dialog hit test", async ({ page }) => {
    await bootProjectOpenFixture(page, []);

    // This isolates the modal's own reachability contract. Issue #3753 adds
    // the stronger picker/onboarding overlap case after the z-index fix.
    await page.locator("#picker-clone-project").click();
    await expect(page.locator("#clone-project-modal")).toHaveClass(/open/);
    await expect(page.locator("#clone-project-url-input")).toBeVisible();

    expect(
      await centerHitBelongsTo(
        page,
        "#clone-project-url-input",
        "#clone-project-modal",
      ),
    ).toBe(true);
  });
});

async function bootProjectOpenFixture(
  page: Page,
  tabs: FixtureTab[],
): Promise<void> {
  await installEmbeddedRoutes(page);
  await installProjectOpenBackend(page, tabs);
  await gotoLiveGwt(page, APP_URL);

  if (tabs.length === 0) {
    await expect(page.locator("#project-picker")).toHaveClass(/visible/);
  } else {
    await expect(page.locator("#close-project-button")).toBeVisible();
    await expect(page.locator("#project-tabs")).toHaveCount(0);
  }
}

async function installProjectOpenBackend(
  page: Page,
  tabs: FixtureTab[],
): Promise<void> {
  await page.addInitScript((fixtureTabs: FixtureTab[]) => {
    const recordedSends: string[] = [];
    let hubSocket: ProjectOpenFixtureWebSocket | undefined;
    let projectSocket: ProjectOpenFixtureWebSocket | undefined;
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: fixtureTabs,
        active_tab_id: fixtureTabs[0]?.id ?? null,
        recent_projects: [],
      },
    };

    class ProjectOpenFixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      readyState = ProjectOpenFixtureWebSocket.CONNECTING;

      constructor(public readonly url: string) {
        super();
        if (new URL(url).searchParams.get('repo_hash')) projectSocket = this;
        else hubSocket = this;
        setTimeout(() => {
          this.readyState = ProjectOpenFixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string): void {
        recordedSends.push(raw);
        try {
          if (JSON.parse(raw)?.kind === "frontend_ready") {
            this.emit(workspaceState);
          }
        } catch {
          // The production socket ignores malformed JSON frames as well.
        }
      }

      close(): void {
        this.readyState = ProjectOpenFixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown): void {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: ProjectOpenFixtureWebSocket,
    });
    Object.defineProperty(window, "__gwtProjectOpenFixture", {
      configurable: true,
      value: {
        recordedSends,
        reply: (event: { kind: string }) => {
          // Project snapshots must pass the Project connection's guard.
          const socket = event.kind === 'workspace_state' ? projectSocket : hubSocket;
          socket?.emit(event);
        },
      },
    });
  }, tabs);
}

function fixtureTab(kind: FixtureTab["kind"]): FixtureTab {
  return {
    id: `tab-${kind}`,
    title: kind === "git" ? "Git Project" : "Plain Folder",
    project_root: kind === "git" ? "/fixture/git" : "/fixture/plain-folder",
    kind,
    workspace: {
      viewport: { x: 0, y: 0, zoom: 1 },
      windows: [],
    },
  };
}

async function sentMessageKinds(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const fixture = (window as any).__gwtProjectOpenFixture;
    return (fixture?.recordedSends ?? []).flatMap((raw: string) => {
      try {
        const kind = JSON.parse(raw)?.kind;
        return typeof kind === "string" ? [kind] : [];
      } catch {
        return [];
      }
    });
  });
}

async function centerHitBelongsTo(
  page: Page,
  targetSelector: string,
  ownerSelector: string,
): Promise<boolean> {
  return page.evaluate(({ targetSelector, ownerSelector }) => {
    const target = document.querySelector(targetSelector);
    const owner = document.querySelector(ownerSelector);
    if (!(target instanceof HTMLElement) || !(owner instanceof HTMLElement)) {
      return false;
    }
    const bounds = target.getBoundingClientRect();
    const hit = document.elementFromPoint(
      bounds.left + bounds.width / 2,
      bounds.top + bounds.height / 2,
    );
    return Boolean(hit && owner.contains(hit));
  }, { targetSelector, ownerSelector });
}

// A real restored backend context and a socket do not constitute a Project view.
// PM auto-start is disabled by browser-check; this checks the shared real shell
// admission, while the PM-specific auto-start guard has runtime test coverage.
test('live backend-only context refuses a shell until a visible Project receipt', async ({ page, context }, testInfo) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const checkHome = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  test.skip(!base || !checkHome, 'requires browser-check isolated checkout HOME');
  const { execFileSync } = await import('node:child_process');
  const { mkdtemp, mkdir, writeFile } = await import('node:fs/promises');
  const { join } = await import('node:path');
  const { focusLiveWindow, readLiveHubCatalog, withLiveGwtBackendLock, sendLiveGwtEvent } = await import('./_helpers/live-gwt');
  await withLiveGwtBackendLock(base!, testInfo, async () => {
    const root = await mkdtemp(join(checkHome!, 'cold-view-'));
    const actual = await context.newPage();
    const errors: string[] = [];
    for (const current of [page, actual]) {
      current.on('pageerror', error => errors.push(error.message));
      current.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    }
    let key = '';
    try {
      execFileSync('git', ['init', '--quiet', '--initial-branch=develop', '--separate-git-dir', `${root}.git`, root]);
      execFileSync('git', ['-C', root, '-c', 'user.name=GWT acceptance', '-c', 'user.email=acceptance@example.invalid', 'commit', '--quiet', '--allow-empty', '-m', 'chore: seed Project receipt acceptance']);
      const envelope = JSON.parse(execFileSync(process.env.GWT_PLAYWRIGHT_GWTD!, { cwd: root, encoding: 'utf8',
        input: JSON.stringify({ schema_version: 1, operation: 'project.execution.status', params: {} }) }));
      expect(envelope.ok).toBe(true);
      key = JSON.parse(envelope.output).project_key;
      expect(key).toMatch(/^[0-9a-f]{16}$/);
      const state = join(checkHome!, '.gwt', 'projects', key, 'project-state'); await mkdir(state, { recursive: true });
      await writeFile(join(state, 'pm.json'), JSON.stringify({ settings: { auto_start: false } }));
      await writeFile(join(state, 'issue-monitor.json'), JSON.stringify({ enabled: false, max_active_agents: 1, priority_order: [] }));
      await readLiveHubCatalog(page, base!, { send: { kind: 'reopen_recent_project', path: root },
        until: (catalog, projectKey) => catalog.projects.some(project => project.project_key === projectKey), arg: key });
      await gotoLiveGwt(page, base!, { hub: true });
      const before = await page.evaluate(({ base, key }) => new Promise<any>((resolve, reject) => {
        const url = new URL('/ws', base); url.protocol = 'ws:'; url.searchParams.set('repo_hash', key);
        const socket = new WebSocket(url); (window as any).__coldObserver = socket;
        const events: any[] = []; (window as any).__coldEvents = events;
        let attempted = false, attemptObserved = false;
        const timer = setTimeout(() => reject(new Error('cold admission observation timed out')), 30_000);
        socket.onopen = () => socket.send(JSON.stringify({ kind: 'frontend_ready' }));
        socket.onmessage = message => {
          const event = JSON.parse(String(message.data)); events.push(event);
          if (event.kind === 'workspace_state') {
            if (!attempted) {
              attempted = true;
              socket.send(JSON.stringify({ kind: 'startup_auto_resume_ready' }));
              socket.send(JSON.stringify({ kind: 'create_window', preset: 'shell' }));
            } else {
              attemptObserved = true;
              socket.send(JSON.stringify({ kind: 'get_project_execution_status' }));
            }
          }
          if (event.kind === 'project_execution_status' && attemptObserved) {
            clearTimeout(timer); resolve({ status: event.status, events });
          }
        };
      }), { base: base!, key });
      expect(before.status.start_reason).toContain('actual_view_required');
      expect(before.status.view_state).not.toBe('open');
      expect(before.events.filter((event: any) => event.kind === 'terminal_status' && ['starting', 'running', 'idle'].includes(event.status))).toEqual([]);
      expect(before.events.some((event: any) => event.kind === 'terminal_status' && event.status === 'error'
        && /selection_pending|actual_view_required/.test(event.detail || ''))).toBe(true);
      expect(before.events.filter((event: any) => event.kind === 'workspace_state').at(-1).workspace.tabs
        .flatMap((tab: any) => tab.workspace.windows).filter((entry: any) => entry.agent_id || entry.is_pm)).toEqual([]);
      await actual.bringToFront();
      await gotoLiveGwt(actual, base!, { projectKey: key, enableTestBridge: true });
      const theme = testInfo.project.name.includes('light') ? 'light' : 'dark';
      await expect(actual.locator('html')).toHaveAttribute('data-theme', theme);
      await expect.poll(() => actual.evaluate(() => (window as any).__gwtPlaywrightMessages?.filter((entry: any) =>
        entry.payload.kind === 'project_execution_status').at(-1)?.payload.status.view_state)).toBe('open');
      const beforeIds = new Set(await actual.locator('.workspace-window[data-preset="shell"]').evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.id)));
      await sendLiveGwtEvent(actual, { kind: 'create_window', preset: 'shell' });
      const created = async () => (await actual.locator('.workspace-window[data-preset="shell"]').evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.id!))).filter(id => !beforeIds.has(id));
      await expect.poll(created).toHaveLength(1); const id = (await created())[0];
      await focusLiveWindow(actual, id);
      await expect.poll(() => actual.evaluate(id => (window as any).__gwtTerminalTestApi.bufferText(id).trim(), id), { timeout: 30_000 }).toMatch(/[^\n]+[>$#%]$/);
      await actual.screenshot({ path: testInfo.outputPath(`actual-view-admitted-${theme}.png`) });
      await sendLiveGwtEvent(actual, { kind: 'close_window', id });
    } catch (error) {
      await actual.screenshot({ path: testInfo.outputPath('cold-failure.png') }).catch(() => {});
      await testInfo.attach('cold-backend-messages', { body: JSON.stringify(await page.evaluate(() => (window as any).__coldEvents).catch(() => [])), contentType: 'application/json' });
      throw error;
    } finally {
      await actual.close();
      await page.evaluate(() => (window as any).__coldObserver?.close()).catch(() => {});
      if (key) await readLiveHubCatalog(page, base!, { send: { kind: 'preview_close_project', project_key: key, project_wide: true } }).catch(() => {});
      // Keep fixture history inside the owned isolated HOME until teardown.
    }
    expect(errors).toEqual([]);
  });
});
