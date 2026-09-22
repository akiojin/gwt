/* SPEC-2013 Phase 8 — the top-toolbar Open Project split-button (Issue #2684)
 * was retired. The consolidated `Projects ▾` switcher now owns switching
 * (OPEN / RECENT) plus project intake (Open Folder… / Clone from GitHub…),
 * and the picker overlay only shows when no tab is open.
 *
 * The fixture pattern follows `tests/index-status.spec.ts`: serve the
 * embedded frontend via `installEmbeddedRoutes` and replace WebSocket with a
 * deterministic backend that emits one workspace_state with a single tab.
 */
import { expect, test } from "@playwright/test";
import {
  APP_URL,
  installEmbeddedRoutes,
} from "./_helpers/embedded-frontend";

const browserErrors = new WeakMap<
  object,
  { consoleErrors: string[]; pageErrors: string[] }
>();

test.describe("Projects ▾ consolidated chrome", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test.beforeEach(({ page }) => {
    const errors = { consoleErrors: [] as string[], pageErrors: [] as string[] };
    browserErrors.set(page, errors);
    page.on("console", (message) => {
      if (message.type() === "error") {
        errors.consoleErrors.push(message.text());
      }
    });
    page.on("pageerror", (error) => errors.pageErrors.push(error.message));
  });

  test.afterEach(({ page }) => {
    const errors = browserErrors.get(page);
    expect(errors?.consoleErrors ?? []).toEqual([]);
    expect(errors?.pageErrors ?? []).toEqual([]);
  });

  test("the Open Project split-button is removed from the top bar", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    await expect(page.locator("#open-project-group")).toHaveCount(0);
    await expect(page.locator("#open-project-menu")).toHaveCount(0);
    await expect(page.locator(".split-button-group")).toHaveCount(0);
    await expect(page.locator("#project-switcher-button")).toBeVisible();
  });

  test("Projects ▾ lists OPEN + RECENT and the Open Folder / Clone actions", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });
    // Picker overlay must NOT be visible while a tab is active.
    await expect(page.locator("#project-picker")).not.toHaveClass(/visible/);

    await page.locator("#project-switcher-button").click();
    const panel = page.locator("#project-switcher-panel");
    await expect(panel).toHaveClass(/open/);

    await expect(panel.getByText("Open Projects")).toBeVisible();
    await expect(panel.getByText("Fixture Project")).toBeVisible();
    await expect(panel.getByText("Recent A")).toBeVisible();
    await expect(
      panel.locator("[data-action='open-folder']"),
    ).toHaveText(/Open Folder/);
    await expect(
      panel.locator("[data-action='clone-from-github']"),
    ).toHaveText(/Clone from GitHub/);
  });

  test("Clone from GitHub action opens the clone modal", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    await page.locator("#project-switcher-button").click();
    await page.locator("[data-action='clone-from-github']").click();

    await expect(page.locator("#clone-project-modal")).toHaveClass(/open/);
    await expect(page.locator("#clone-project-modal")).not.toHaveAttribute(
      "aria-hidden",
      "true",
    );
  });

  test("Clone modal stays interactive above the zero-tab project picker", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page, { activeTabKind: null });
    await page.goto(APP_URL);

    await expect(page.locator("#project-picker")).toHaveClass(/visible/);
    await page.locator("#picker-clone-project").click();
    await expect(page.locator("#clone-project-modal")).toHaveClass(/open/);

    await expectCloneModalAbove(page, "#project-picker");
  });

  test("Projects popover can open Clone above non-git onboarding", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page, { activeTabKind: "non_repo" });
    await page.goto(APP_URL);

    await expect(page.locator("#project-onboarding")).toHaveClass(/visible/);
    await page.locator("#project-switcher-button").click();
    await page
      .locator("[data-action='clone-from-github']")
      .click({ timeout: 2_000 });
    await expect(page.locator("#clone-project-modal")).toHaveClass(/open/);

    await expectCloneModalAbove(page, "#project-onboarding");
  });

  test("Clone modal stays interactive above the command palette", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({ timeout: 10_000 });
    await page.locator("#project-switcher-button").click();
    await page.locator("[data-action='clone-from-github']").click();
    await expect(page.locator("#clone-project-modal")).toHaveClass(/open/);

    await page.evaluate(() => {
      const palette = document.getElementById("op-palette-backdrop");
      palette?.setAttribute("data-open", "true");
      palette?.setAttribute("aria-hidden", "false");
    });
    await expectCloneModalAbove(page, "#op-palette-backdrop");
  });

  test("Command palette keeps aria-modal ownership above the Projects popover", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    const panel = page.locator("#project-switcher-panel");
    const palette = page.locator("#op-palette-backdrop");
    await page.locator("#project-switcher-button").click();
    await expect(panel).toBeVisible();
    await page.locator("body").dispatchEvent("keydown", {
      key: "k",
      code: "KeyK",
      metaKey: true,
      bubbles: true,
    });
    await expect(palette).toHaveAttribute("data-open", "true");
    await expect(panel).toBeVisible();

    const result = await page.evaluate(() => {
      const palette = document.getElementById("op-palette-backdrop");
      const panel = document.getElementById("project-switcher-panel");
      if (!(palette instanceof HTMLElement) || !(panel instanceof HTMLElement)) {
        return null;
      }
      const rect = panel.getBoundingClientRect();
      const hit = document.elementFromPoint(
        rect.left + rect.width / 2,
        rect.top + rect.height / 2,
      );
      return {
        paletteZ: Number(getComputedStyle(palette).zIndex),
        panelZ: Number(getComputedStyle(panel).zIndex),
        paletteOwnsHit: hit !== null && palette.contains(hit),
      };
    });
    expect(result).not.toBeNull();
    expect(result!.paletteZ).toBeGreaterThan(result!.panelZ);
    expect(result!.paletteOwnsHit).toBe(true);
  });

  test("Escape closes the Projects switcher", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installWorkspaceFixture(page);
    await page.goto(APP_URL);

    await expect(page.locator(".project-tab")).toBeVisible({
      timeout: 10_000,
    });

    const panel = page.locator("#project-switcher-panel");
    await page.locator("#project-switcher-button").click();
    await expect(panel).toHaveClass(/open/);

    await page.keyboard.press("Escape");
    await expect(panel).not.toHaveClass(/open/);
    await expect(page.locator("#project-switcher-button")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });
});

async function installWorkspaceFixture(
  page: any,
  options: {
    recentProjects?: Array<{ title: string; path: string; kind: string }>;
    activeTabKind?: string | null;
  } = {},
): Promise<void> {
  const recentProjects = options.recentProjects ?? [
    { title: "Recent A", path: "/recent/a", kind: "git" },
  ];
  const activeTabKind =
    options.activeTabKind === undefined ? "git" : options.activeTabKind;
  await page.addInitScript((fixture: any) => {
    const tabs = fixture.activeTabKind === null
      ? []
      : [
          {
            id: "tab-1",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: fixture.activeTabKind,
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [],
            },
          },
        ];
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs,
        active_tab_id: tabs.length > 0 ? "tab-1" : null,
        recent_projects: fixture.recentProjects,
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

      send(raw: string): void {
        let message: any;
        try {
          message = JSON.parse(raw);
        } catch (e) {
          return;
        }
        if (message.kind === "frontend_ready") {
          (this as any).emit(workspaceState);
        }
      }

      close(): void {
        (this as any).readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: any): void {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  }, { recentProjects, activeTabKind });
}

async function expectCloneModalAbove(
  page: any,
  coveredSelector: string,
): Promise<void> {
  await expect(page.locator(coveredSelector)).toBeVisible();
  const result = await page.evaluate((selector: string) => {
    const modal = document.getElementById("clone-project-modal");
    const dialog = modal?.querySelector(".modal-shell");
    const covered = document.querySelector(selector);
    if (
      !(modal instanceof HTMLElement) ||
      !(dialog instanceof HTMLElement) ||
      !(covered instanceof HTMLElement)
    ) {
      return null;
    }
    const rect = dialog.getBoundingClientRect();
    const centerX = rect.left + rect.width / 2;
    const centerY = rect.top + rect.height / 2;
    const coveredRect = covered.getBoundingClientRect();
    const hit = document.elementFromPoint(
      centerX,
      centerY,
    );
    return {
      modalZ: Number(getComputedStyle(modal).zIndex),
      coveredZ: Number(getComputedStyle(covered).zIndex),
      coveredContainsDialogCenter:
        coveredRect.left <= centerX &&
        centerX <= coveredRect.right &&
        coveredRect.top <= centerY &&
        centerY <= coveredRect.bottom,
      dialogOwnsHit: hit !== null && dialog.contains(hit),
      hitDescription: hit
        ? `${hit.tagName.toLowerCase()}#${hit.id}.${Array.from(hit.classList).join(".")}`
        : "none",
    };
  }, coveredSelector);

  expect(result).not.toBeNull();
  expect(result!.modalZ).toBeGreaterThan(result!.coveredZ);
  expect(result!.coveredContainsDialogCenter).toBe(true);
  expect(
    result!.dialogOwnsHit,
    `dialog center hit ${result!.hitDescription}`,
  ).toBe(true);
}
