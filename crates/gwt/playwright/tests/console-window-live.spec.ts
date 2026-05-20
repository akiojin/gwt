/**
 * SPEC-2809 — Console window live E2E.
 *
 * Drives a real `gwt --headless --no-open` instance through the Add Window
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

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

const KINDS = ["gh", "git", "docker", "agent", "runner"];

test.describe.serial("Console window (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");

  test.beforeEach(async ({ page }) => {
    // Same splash-bypass pattern as release-notes-live.spec.ts (Issue
    // tracked separately): force the briefing overlay hidden before the
    // first frame so the rest of the test never collides with it.
    await page.addInitScript(() => {
      try {
        window.sessionStorage.setItem("gwt:ui:briefing", "1");
      } catch {
        /* no-op */
      }
    });
    await page.goto(BASE);
    // Hide the splash + every backdrop EXCEPT the Add Window picker, which
    // this spec must interact with. release-notes-live.spec.ts hides every
    // modal-backdrop because it only clicks the title bar; here we need the
    // preset modal to be reachable.
    await page.addStyleTag({
      content: `
        #op-briefing { display: none !important; pointer-events: none !important; }
        .modal-backdrop:not(#preset-modal) { display: none !important; pointer-events: none !important; }
      `,
    });
    await page.evaluate(() => {
      const overlay = document.getElementById("op-briefing");
      if (overlay) overlay.hidden = true;
    });
    await expect(page.locator("#op-briefing")).toBeHidden();
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

  async function openConsoleWindow(page) {
    const presetModal = await surfacePresetModal(page);
    const consoleButton = presetModal.locator("[data-preset='console']");
    await expect(consoleButton).toBeVisible();
    await consoleButton.click();
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

    await openConsoleWindow(page);

    const consoleRoot = page.locator(".console-window").first();
    await expect(consoleRoot).toBeVisible();

    const tabs = consoleRoot.locator(".console-window__tab");
    await expect(tabs).toHaveCount(KINDS.length);
    for (let i = 0; i < KINDS.length; i++) {
      await expect(tabs.nth(i)).toHaveText(KINDS[i]);
      await expect(tabs.nth(i)).toHaveAttribute("data-kind", KINDS[i]);
    }

    const panes = consoleRoot.locator(".console-window__pane");
    await expect(panes).toHaveCount(KINDS.length);
    for (const kind of KINDS) {
      const hint = consoleRoot.locator(
        `.console-window__pane[data-kind='${kind}'] .console-window__empty`,
      );
      await expect(hint).toHaveText(new RegExp(`Waiting for ${kind} process output`));
    }

    // SPEC-2809 Phase F2 snapshot handshake — the controller must emit
    // `load_process_console` once it mounts so historical lines are
    // visible immediately on open.
    await expect
      .poll(async () =>
        await page.evaluate(() =>
          ((window as any).__gwtSentPayloads as string[]).some((payload) =>
            payload.includes("load_process_console"),
          ),
        ),
      )
      .toBe(true);
  });

  test("clicking a tab activates it and hides the others", async ({ page }) => {
    await openConsoleWindow(page);

    // Use the most recently mounted Console window; tests share the
    // backend session so any earlier instance can linger in the DOM.
    const consoleRoot = page.locator(".console-window").last();
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
