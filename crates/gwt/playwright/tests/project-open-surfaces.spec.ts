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

  test("active Project Ctrl+O exposes pending, cancellation and manual path fallback", async ({ page }) => {
    await bootProjectOpenFixture(page, [fixtureTab("git")]);
    await page.keyboard.press('Control+o');
    const modal = page.locator('#open-project-path-modal');
    await expect(modal).toBeVisible();
    await expect(modal).toHaveClass('modal-backdrop open');
    await expect(modal.locator('.modal-shell > .modal-header')).toBeVisible();
    await expect(modal.locator('.modal-shell > .modal-body')).toBeVisible();
    await expect(modal.locator('.modal-shell > .modal-footer')).toBeVisible();
    await expect(modal.getByRole('dialog')).toHaveAttribute('aria-modal', 'true');
    await expect(modal.getByRole('status')).toContainText('Choose a folder');
    await page.evaluate(() => (window as any).__gwtProjectOpenFixture.reply({ kind: 'picker_started', purpose: 'open', request_id: 1 }));
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
    await page.evaluate((request_id) => (window as any).__gwtProjectOpenFixture.reply({ kind: 'project_open_error', request_id, message: 'Folder does not exist' }), firstRequestId);
    await expect(modal.getByRole('status')).toHaveText('Folder does not exist');
    await expect(modal.locator('[data-open-project-path]')).toHaveValue('E:\\repos\\manual');
    await modal.locator('[data-open-project-path]').fill('E:\\repos\\valid');
    await modal.locator('[data-open-project-submit]').click();
    const secondRequestId = await page.evaluate(() => (window as any).__gwtProjectOpenFixture.recordedSends.map(JSON.parse).filter((message: any) => message.kind === 'reopen_recent_project').at(-1).request_id);
    expect(secondRequestId).not.toBe(firstRequestId);
    await page.evaluate((request_id) => (window as any).__gwtProjectOpenFixture.reply({ kind: 'project_opened', request_id, project_key: '0123456789abcdef', title: 'Superseded Project' }), firstRequestId);
    await expect(modal).toBeVisible();
    await page.evaluate((request_id) => (window as any).__gwtProjectOpenFixture.reply({ kind: 'project_opened', request_id, project_key: '0123456789abcdef', title: 'Valid Project' }), secondRequestId);
    await expect(modal).toBeHidden();
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
        if (!new URL(url).searchParams.get('repo_hash')) hubSocket = this;
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
      value: { recordedSends, reply: (event: unknown) => hubSocket?.emit(event) },
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
