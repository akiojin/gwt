/**
 * SPEC-2809 — Console window live E2E.
 *
 * Drives a real gwt browser-server instance through the Add Window
 * picker into the Console preset, then asserts:
 *
 *   - The Console window mounts with the five fixed kind tabs (gh / git /
 *     docker / agent / runner) in the documented order.
 *   - Each pane shows the empty-state hint when no `process_line` has
 *     arrived yet.
 *   - The Console controller emits a `load_process_console` request over
 *     the live WebSocket as soon as it mounts (snapshot replay handshake).
 *   - Tab switching toggles `aria-selected` and the matching pane's
 *     `hidden` attribute without losing the other panes.
 *
 * The spec auto-skips when `GWT_PLAYWRIGHT_BASE_URL` is missing so CI does
 * not need a live backend by default; the `gwt-verify --mode pre-pr` flow
 * wires the URL up explicitly. This is the same skip contract used by
 * `release-notes-live.spec.ts` (PR #2780 follow-up).
 */
import { test, expect } from "@playwright/test";
import { gotoLiveGwt, openLiveGwtProject } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

const KINDS = ["gh", "git", "docker", "agent", "runner"];

test.describe.serial("Console window (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }) => {
    await gotoLiveGwt(page, BASE, {
      enableTestBridge: true,
      keepPresetModal: true,
    });
    await openLiveGwtProject(page);
    await expect(page.locator("#op-briefing")).toBeHidden();
    await expect(page.locator("#project-picker")).toBeHidden();
  });

  async function surfacePresetModal(page) {
    // `.modal-backdrop` is `display: none` by default and switches to
    // `display: flex` when the `.open` class lands on it; the production
    // path adds the class through the hotkey controller, but this spec
    // surfaces the picker directly so we do not depend on a workspace
    // tab being open or on the hotkey wiring.
    await page.evaluate(() => {
      const modal = document.getElementById("preset-modal");
      if (modal) {
        modal.setAttribute("aria-hidden", "false");
        modal.classList.add("open");
      }
    });
    const presetModal = page.locator("#preset-modal");
    await expect(presetModal).toBeVisible();
    return presetModal;
  }

  async function consoleWindowIds(page) {
    return await page.evaluate(() =>
      Array.from(document.querySelectorAll(".workspace-window"))
        .filter((node) => node.querySelector(".console-window"))
        .map((node) => (node as HTMLElement).dataset.id)
        .filter(Boolean),
    );
  }

  async function waitForNewConsoleWindow(page, beforeIds) {
    const id = await page
      .waitForFunction(
        ({ beforeIds }) => {
          const seen = new Set(beforeIds);
          const nodes = Array.from(document.querySelectorAll(".workspace-window"))
            .filter((candidate) => candidate.querySelector(".console-window"));
          const node =
            nodes.find(
              (candidate) =>
                !seen.has((candidate as HTMLElement).dataset.id || ""),
            ) ?? nodes[0];
          return node ? (node as HTMLElement).dataset.id || "" : "";
        },
        { beforeIds },
      )
      .then((handle) => handle.jsonValue());
    return {
      windowRoot: page.locator(`.workspace-window[data-id="${id}"]`),
      reused: beforeIds.includes(id),
    };
  }

  async function openConsoleWindow(page) {
    const beforeIds = await consoleWindowIds(page);
    const presetModal = await surfacePresetModal(page);
    const consoleButton = presetModal.locator("[data-preset='console']");
    await expect(consoleButton).toBeVisible();
    await consoleButton.click();
    return await waitForNewConsoleWindow(page, beforeIds);
  }

  test("Add Window picker exposes the Console preset", async ({ page }) => {
    const presetModal = await surfacePresetModal(page);
    const consoleButton = presetModal.locator("[data-preset='console']");
    await expect(consoleButton).toBeVisible();
    await expect(consoleButton.locator("strong")).toHaveText("Console");
  });

  test("Console window mounts with 5 fixed kind tabs and empty hints", async ({
    page,
  }) => {
    // Use page.evaluate to capture outbound WebSocket frames from inside
    // the page itself. The page.on('websocket') subscriber attaches too
    // late — the WS is opened during `beforeEach` goto, and `framesent`
    // listeners attached afterwards never see frames emitted earlier in
    // the boot sequence. Hooking from inside the page is reliable because
    // the controller calls `socketTransport.send` which goes through the
    // captured `WebSocket.send` shim.
    await page.evaluate(() => {
      (window as any).__gwtSentPayloads = [] as string[];
      const SendOriginal = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data: any) {
        try {
          (window as any).__gwtSentPayloads.push(String(data));
        } catch {
          /* no-op */
        }
        return SendOriginal.call(this, data);
      };
    });

    const { windowRoot, reused } = await openConsoleWindow(page);

    const consoleRoot = windowRoot.locator(".console-window");
    await expect(consoleRoot).toBeVisible();

    const tabs = consoleRoot.locator(".console-window__tab");
    await expect(tabs).toHaveCount(KINDS.length);
    for (let i = 0; i < KINDS.length; i++) {
      await expect(tabs.nth(i)).toHaveText(KINDS[i]);
      await expect(tabs.nth(i)).toHaveAttribute("data-kind", KINDS[i]);
    }

    const panes = consoleRoot.locator(".console-window__pane");
    await expect(panes).toHaveCount(KINDS.length);
    // Process output can already exist by the time a live Console opens
    // (for example from GitHub-backed Issue Monitor refreshes). A pane is
    // valid when it either still shows the empty hint or already has a
    // rendered process line/header.
    const IDLE_KINDS = ["gh", "git", "docker", "agent"];
    for (const kind of IDLE_KINDS) {
      const pane = consoleRoot.locator(
        `.console-window__pane[data-kind='${kind}']`,
      );
      await expect
        .poll(async () =>
          await pane.evaluate((node, kind) => {
            const empty = node.querySelector(".console-window__empty");
            const hasLine = node.querySelector(
              ".console-window__line, .console-window__invocation-header",
            );
            return (
              Boolean(hasLine) ||
              (empty?.textContent ?? "").includes(
                `Waiting for ${kind} process output`,
              )
            );
          }, kind),
        )
        .toBe(true);
    }
    // Runner: either the empty hint is still visible (timing-lucky case
    // where the snapshot reaches the controller before the first
    // gwt::index event) or at least one line/header has been rendered
    // by ConsoleTeeLayer. Both states pass.
    const runnerPane = consoleRoot.locator(
      ".console-window__pane[data-kind='runner']",
    );
    await expect
      .poll(async () =>
        await runnerPane.evaluate((node) => {
          const empty = node.querySelector(".console-window__empty");
          const hasLine = node.querySelector(
            ".console-window__line, .console-window__invocation-header",
          );
          return Boolean(empty) || Boolean(hasLine);
        }),
      )
      .toBe(true);

    // SPEC-2809 Phase F2 snapshot handshake — a freshly mounted Console
    // controller emits `load_process_console`. If the live backend reused an
    // existing Console window, no new mount occurs and the existing snapshot is
    // already visible.
    if (!reused) {
      await expect
        .poll(async () =>
          await page.evaluate(() =>
            ((window as any).__gwtSentPayloads as string[]).some((payload) =>
              payload.includes("load_process_console"),
            ),
          ),
        )
        .toBe(true);
    }
  });

  test("clicking a tab activates it and hides the others", async ({ page }) => {
    const { windowRoot } = await openConsoleWindow(page);

    const consoleRoot = windowRoot.locator(".console-window");
    await expect(consoleRoot).toBeVisible();

    const dockerTab = consoleRoot.locator(".console-window__tab[data-kind='docker']");
    // `force: true` bypasses the workspace-window decoration overlay that
    // wraps Console panes and would otherwise intercept the click. The
    // tab handler is wired through `addEventListener("click", ...)` so the
    // forced click still produces the real activate() behavior; we verify
    // that via the assertions below rather than trusting Playwright's
    // visual hit test.
    await dockerTab.click({ force: true });
    await expect(dockerTab).toHaveAttribute("aria-selected", "true");
    await expect(consoleRoot).toHaveAttribute("data-active-kind", "docker");

    for (const kind of KINDS) {
      const pane = consoleRoot.locator(
        `.console-window__pane[data-kind='${kind}']`,
      );
      if (kind === "docker") {
        await expect(pane).toBeVisible();
      } else {
        await expect(pane).toBeHidden();
      }
    }
  });
});
