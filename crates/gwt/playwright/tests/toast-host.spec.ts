/* SPEC #3206 — real-browser E2E for the shared toast-host primitive and the
 * v2 notification center.
 *
 * v1 routed the bottom-right `alerts` region (completion / attention /
 * board-mention) through `createToastStack`; the first describe below keeps
 * that regression coverage. v2 retired the top-right autonomous `log` region
 * in favour of the operator-rail bell + unread badge + history drawer, so the
 * second describe boots the full frontend against a fixture WebSocket and
 * asserts what only a real browser proves: the retired region is absent, the
 * bell/badge/drawer wiring works end to end (click, Esc, ×, clear-all), the
 * history scrolls inside the drawer, and FR-017 Issue-window errors are read
 * only in the notification center — never rendered in the Issue window.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("toast-host alerts region (real browser, SPEC #3206 P1)", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      class NoopSocket {
        constructor() {}
        send() {}
        close() {}
        addEventListener() {}
        removeEventListener() {}
      }
      // @ts-ignore
      window.WebSocket = NoopSocket;
    });
    await installEmbeddedRoutes(page);
    await page.goto(APP_URL);
    await page.evaluate(async () => {
      const mod = await import("/toast-host.js");
      document.body.replaceChildren();
      const host = document.createElement("div");
      host.id = "operator-notice-stack";
      host.className = "operator-notice-stack";
      document.body.appendChild(host);
      (window as any).__activated = [];
      const stack = (mod as any).createToastStack({
        document,
        className: "toast-alerts",
        ariaRole: "status",
        ariaLive: "polite",
        animateDismiss: true,
        levels: ["neutral", "info", "warn", "error", "done"],
        defaultLevel: "neutral",
      });
      stack.mount(host);
      (window as any).__alerts = stack;
    });
  });

  test("the three former systems share ONE bottom-right stack (no overlap)", async ({ page }) => {
    await page.evaluate(() => {
      const s = (window as any).__alerts;
      s.push({ id: "agent-completion", level: "neutral", title: "Done", message: "ok", dismissible: false });
      s.push({ id: "attention-w1", level: "warn", title: "Needs input", message: "y", dismissible: true });
      s.push({ id: "board-mention", level: "info", title: "Board reply", dismissible: false });
    });

    // All three live in the SAME single container (no per-system fixed offsets).
    expect(await page.locator(".toast-alerts").count()).toBe(1);
    const items = page.locator(".toast-alerts__list .toast-alerts__item");
    await expect(items).toHaveCount(3);
    await expect(items.first()).toContainText("Board reply"); // newest on top

    const layout = await page.evaluate(() => {
      const host = getComputedStyle(
        document.querySelector(".operator-notice-stack") as HTMLElement,
      );
      const alerts = getComputedStyle(
        document.querySelector(".toast-alerts") as HTMLElement,
      );
      return { hostPosition: host.position, alertsPosition: alerts.position };
    });
    expect(layout.hostPosition).toBe("fixed");
    expect(layout.alertsPosition).not.toBe("fixed");
  });

  test("sticky alerts and the update CTA remain visible and operable without overlap", async ({
    page,
  }) => {
    await page.evaluate(async () => {
      const mod = await import("/update-cta.js");
      (window as any).__updateSent = [];
      const controller = (mod as any).createUpdateCtaController({
        document,
        send: (message: unknown) => (window as any).__updateSent.push(message),
        setVersionState: () => {},
      });
      controller.showAvailable("9.99.0");
      (window as any).__updateController = controller;
      (window as any).__alerts.push({
        id: "sticky-error",
        level: "error",
        title: "Background task failed",
        message: "Open the affected work item for details.",
        timeoutMs: 0,
        dismissible: true,
        onActivate: () => (window as any).__activated.push("sticky-error"),
      });
    });

    const host = page.locator("#operator-notice-stack");
    const alerts = page.locator(".toast-alerts");
    const shell = page.locator("#update-cta-shell");
    await expect(host).toBeVisible();
    await expect(alerts).toHaveAttribute("role", "status");
    await expect(shell).toBeVisible();

    const ownership = await page.evaluate(() => {
      const hostNode = document.getElementById("operator-notice-stack")!;
      const alertsNode = document.querySelector(".toast-alerts")!;
      const shellNode = document.getElementById("update-cta-shell")!;
      return {
        children: Array.from(hostNode.children).map((node) => node.className),
        alertsParentIsHost: alertsNode.parentElement === hostNode,
        shellParentIsHost: shellNode.parentElement === hostNode,
        ctaInsideAlertsLiveRegion: alertsNode.contains(shellNode),
        hostRole: hostNode.getAttribute("role"),
        hostAriaLive: hostNode.getAttribute("aria-live"),
      };
    });
    expect(ownership.alertsParentIsHost).toBe(true);
    expect(ownership.shellParentIsHost).toBe(true);
    expect(ownership.children).toEqual(["toast-alerts", "update-cta-shell"]);
    expect(ownership.ctaInsideAlertsLiveRegion).toBe(false);
    expect(ownership.hostRole).toBeNull();
    expect(ownership.hostAriaLive).toBeNull();

    await page.evaluate(() => {
      const alerts = (window as any).__alerts;
      alerts.push({
        id: "queued-warning",
        level: "warn",
        title: "Needs input",
        timeoutMs: 0,
        dismissible: true,
      });
      alerts.push({
        id: "queued-info",
        level: "info",
        title: "Board reply",
        timeoutMs: 0,
        dismissible: true,
      });
      alerts.push({
        id: "queued-warning",
        level: "warn",
        title: "Needs input (updated)",
        timeoutMs: 0,
        dismissible: true,
      });
    });
    const alertItems = page.locator(".toast-alerts__item");
    await expect(alertItems).toHaveCount(3);
    await expect(page.locator('#update-cta')).toBeVisible();

    for (const viewport of [
      { width: 1280, height: 900 },
      { width: 390, height: 844 },
    ]) {
      await page.setViewportSize(viewport);
      const geometry = await page.evaluate(() => {
        const alertBoxes = Array.from(document.querySelectorAll(".toast-alerts__item"))
          .map((node) => node.getBoundingClientRect());
        const ctaBox = document
          .getElementById("update-cta-shell")!
          .getBoundingClientRect();
        const overlapAreas = alertBoxes.map((alertBox) => {
          const overlapWidth = Math.max(
            0,
            Math.min(alertBox.right, ctaBox.right) - Math.max(alertBox.left, ctaBox.left),
          );
          const overlapHeight = Math.max(
            0,
            Math.min(alertBox.bottom, ctaBox.bottom) - Math.max(alertBox.top, ctaBox.top),
          );
          return overlapWidth * overlapHeight;
        });
        return {
          maxOverlapArea: Math.max(...overlapAreas),
          scrollWidth: document.documentElement.scrollWidth,
          viewportWidth: window.innerWidth,
          maxAlertBottom: Math.max(...alertBoxes.map((box) => box.bottom)),
          ctaTop: ctaBox.top,
        };
      });
      expect(geometry.maxOverlapArea).toBe(0);
      expect(geometry.maxAlertBottom).toBeLessThanOrEqual(geometry.ctaTop);
      expect(geometry.scrollWidth).toBeLessThanOrEqual(geometry.viewportWidth);
    }

    const stickyAlert = page.locator(
      '.toast-alerts__item[data-toast-id="sticky-error"]',
    );
    await stickyAlert.focus();
    await expect(stickyAlert).toBeFocused();
    expect(await stickyAlert.evaluate((node) => node.matches(":focus-visible"))).toBe(true);
    await page.locator("#update-cta").focus();
    await expect(page.locator("#update-cta")).toBeFocused();
    expect(
      await page.locator("#update-cta").evaluate((node) => node.matches(":focus-visible")),
    ).toBe(true);

    await page.evaluate(() => (window as any).__alerts.dismiss("queued-info"));
    await expect(alertItems).toHaveCount(2);
    await expect(page.locator("#update-cta")).toBeVisible();

    await stickyAlert.click();
    expect(await page.evaluate(() => (window as any).__activated)).toContain("sticky-error");
    await expect(page.locator('.toast-alerts__item[data-toast-id="sticky-error"]')).toHaveCount(0);

    await page.evaluate(() =>
      (window as any).__alerts.push({
        id: "dismiss-error",
        level: "error",
        title: "Dismiss me",
        timeoutMs: 0,
        dismissible: true,
      }),
    );
    await page.locator('.toast-alerts__item[data-toast-id="dismiss-error"] .toast-alerts__dismiss').click();
    await expect(page.locator('.toast-alerts__item[data-toast-id="dismiss-error"]')).toHaveCount(0);

    await page.locator("[data-update-cta-dismiss]").click();
    await expect(shell).toHaveCount(0);
    await page.evaluate(() => (window as any).__updateController.showAvailable("9.99.0"));
    await page.locator("#update-cta").click();
    expect(await page.evaluate(() => (window as any).__updateSent)).toContainEqual({
      kind: "apply_update_start",
    });
    await expect(page.locator('#update-modal[data-state="downloading"]')).toBeVisible();
  });

  test("dedup by id replaces; onActivate jumps then dismisses", async ({ page }) => {
    await page.evaluate(() => {
      const s = (window as any).__alerts;
      s.push({ id: "attention-w1", level: "warn", title: "first" });
      s.push({ id: "attention-w1", level: "error", title: "second", dismissible: true });
    });
    const attention = page.locator('.toast-alerts__item[data-toast-id="attention-w1"]');
    await expect(attention).toHaveCount(1, { timeout: 1000 });
    await expect(attention).toContainText("second");
    await expect(attention).toHaveAttribute("data-level", "error");

    await page.evaluate(() =>
      (window as any).__alerts.push({
        id: "agent-completion",
        title: "Done",
        dismissible: false,
        onActivate: () => (window as any).__activated.push("completion"),
      }),
    );
    await page.locator('.toast-alerts__item[data-toast-id="agent-completion"]').click();
    expect(await page.evaluate(() => (window as any).__activated)).toContain("completion");
  });
});

/* SPEC #3206 v2 — notification center. Boots the full frontend against a
 * fixture WebSocket (one Issue window, monitor status) and drives backend
 * events through the app's `__gwt_test_inject` hook, which feeds receive()
 * exactly like a WebSocket frame. */
async function installNotificationCenterBackend(page, theme: "dark" | "light") {
  await page.addInitScript((selectedTheme: string) => {
    localStorage.setItem("gwt:ui:theme", selectedTheme);
    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-issue",
            title: "Fixture Project",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [
                {
                  id: "issue-kanban",
                  title: "Issue",
                  preset: "issue",
                  geometry: { x: 40, y: 60, width: 1100, height: 700 },
                  z_index: 1,
                  status: "running",
                  minimized: false,
                  maximized: false,
                  pre_maximize_geometry: null,
                  persist: true,
                  purpose_title: null,
                  dynamic_title: null,
                  dynamic_title_detail: null,
                  agent_id: null,
                  agent_color: null,
                  tab_group_id: null,
                  tab_group_active: false,
                },
              ],
            },
          },
        ],
        active_tab_id: "tab-issue",
        recent_projects: [],
      },
    };
    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;
      readyState = 0;
      url: string;
      constructor(url: string) {
        super();
        this.url = url;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }
      emit(payload: unknown) {
        this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
      }
      send(raw: string) {
        const message = JSON.parse(raw);
        if (message.kind === "frontend_ready") {
          this.emit(workspaceState);
          return;
        }
        if (message.kind === "list_issue_monitor") {
          this.emit({
            kind: "issue_monitor_status",
            status: {
              enabled: false,
              state: "disabled",
              queue_len: 0,
              active_count: 0,
              max_active_agents: 1,
              total_candidates: 1,
              autonomous_mode: false,
              launch_profile_source: "saved",
              launch_profile_summary: "codex / host",
            },
          });
          return;
        }
        if (message.kind === "load_knowledge_bridge") {
          this.emit({
            kind: "knowledge_entries",
            id: message.id,
            knowledge_kind: message.knowledge_kind,
            request_id: message.request_id,
            entries: [
              {
                number: 3206,
                title: "Notification center",
                state: "open",
                meta: "fixture",
                labels: ["gwt-spec"],
                linked_branch_count: 0,
                match_score: 100,
                phase: "implementation",
                has_unknown_phase: false,
                is_spec: true,
                monitor_state: "queued",
                queue_position: 1,
                exclusion_reason: null,
              },
            ],
            selected_number: null,
            empty_message: "",
            refresh_enabled: true,
          });
        }
      }
      close() {
        this.readyState = FixtureWebSocket.CLOSED;
      }
    }
    // @ts-ignore
    window.WebSocket = FixtureWebSocket;
  }, theme);
}

function inject(page, payload: Record<string, unknown>) {
  return page.evaluate((detail) => {
    window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail }));
  }, payload);
}

test.describe("notification center (real browser, SPEC #3206 v2)", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  let pageErrors: string[] = [];

  test.beforeEach(async ({ page }, testInfo) => {
    pageErrors = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") pageErrors.push(message.text());
    });
    const theme = testInfo.project.name.includes("light") ? "light" : "dark";
    await installNotificationCenterBackend(page, theme);
    await installEmbeddedRoutes(page);
    await page.goto(APP_URL);
    await expect(page.locator("#op-notifications-button")).toBeVisible();
    await expect(page.locator(".surface-knowledge .issue-bridge-root")).toBeVisible();
  });

  test.afterEach(() => {
    expect(pageErrors).toEqual([]);
  });

  test("the top-right log region is gone; autonomous events land in the bell badge and the drawer (newest on top, scrollable)", async ({
    page,
  }) => {
    const bell = page.locator("#op-notifications-button");
    const badge = bell.locator(".op-rail__badge");
    await expect(badge).toBeHidden();

    for (let i = 0; i < 12; i += 1) {
      await inject(page, {
        kind: "issue_monitor_toast",
        level: i === 11 ? "error" : i % 2 ? "success" : "info",
        message: `autonomous event ${i}`,
        issue_number: 3000 + i,
      });
    }

    // FR-012 / Sc 6: no floating top-right region anywhere in the page.
    await expect(page.locator(".autonomous-notifications")).toHaveCount(0);
    // FR-009 / Sc 5: unread badge with error emphasis while the drawer is closed.
    await expect(badge).toBeVisible();
    await expect(badge).toHaveText("12");
    await expect(badge).toHaveAttribute("data-has-error", "true");
    await expect(bell).toHaveAttribute("aria-label", /12 unread, includes errors/);

    await bell.click();
    const drawer = page.locator("#notification-center");
    await expect(drawer).toHaveAttribute("data-open", "true");
    await expect(drawer).toBeVisible();
    await expect(bell).toHaveAttribute("aria-expanded", "true");
    // FR-014: opening reads everything.
    await expect(badge).toBeHidden();

    const rows = drawer.locator(".notification-center__item");
    await expect(rows).toHaveCount(12);
    await expect(rows.first()).toContainText("Issue Monitor #3011");
    await expect(rows.first()).toContainText("autonomous event 11");
    await expect(rows.first()).toHaveAttribute("data-level", "error");
    await expect(rows.last()).toContainText("#3000");

    // The history scrolls inside the drawer body instead of growing the page.
    const scroll = await page.evaluate(() => {
      const body = document.querySelector("#notification-center .notification-center__body") as HTMLElement;
      const drawerNode = document.getElementById("notification-center") as HTMLElement;
      const styles = getComputedStyle(body);
      return {
        overflowY: styles.overflowY,
        drawerPosition: getComputedStyle(drawerNode).position,
        drawerZ: getComputedStyle(drawerNode).zIndex,
        noticeStackZ: getComputedStyle(document.getElementById("operator-notice-stack") as HTMLElement).zIndex,
        pageScrollHeight: document.documentElement.scrollHeight,
        viewportHeight: window.innerHeight,
      };
    });
    expect(scroll.overflowY).toBe("auto");
    expect(scroll.drawerPosition).toBe("fixed");
    // FR-015: persistent drawer never outranks the transient notice stack.
    expect(Number(scroll.drawerZ)).toBeLessThan(Number(scroll.noticeStackZ));
    expect(scroll.pageScrollHeight).toBeLessThanOrEqual(scroll.viewportHeight);

    // Esc closes and returns aria-expanded.
    await page.keyboard.press("Escape");
    await expect(drawer).toHaveAttribute("data-open", "false");
    await expect(bell).toHaveAttribute("aria-expanded", "false");
  });

  test("per-row dismiss and clear-all work on real clicks; the backdrop closes the drawer", async ({ page }) => {
    await inject(page, { kind: "issue_monitor_toast", level: "info", message: "first", issue_number: 1 });
    await inject(page, { kind: "issue_monitor_toast", level: "warn", message: "second", issue_number: 2 });
    await page.locator("#op-notifications-button").click();
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator(".notification-center__item");
    await expect(rows).toHaveCount(2);

    await rows.first().locator(".notification-center__dismiss").click();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText("first");

    await drawer.locator(".notification-center__clear").click();
    await expect(rows).toHaveCount(0);
    await expect(drawer.locator(".notification-center__empty")).toBeVisible();

    await page.locator("#notification-center-backdrop").click({ position: { x: 20, y: 20 } });
    await expect(drawer).toHaveAttribute("data-open", "false");
  });

  test("FR-017: an Issue Monitor error is read only in the notification center, never in the Issue window", async ({
    page,
  }) => {
    const issueWindow = page.locator(".surface-knowledge .issue-bridge-root");
    // User ruling 2026-09-04: errors are read in ONE place. The Issue window
    // carries neither the old red banner nor any indicator of its own.
    await expect(issueWindow.locator(".knowledge-monitor-error")).toHaveCount(0);
    await expect(issueWindow.locator(".surface-error-indicator")).toHaveCount(0);

    const status = (lastError: string | null) => ({
      kind: "issue_monitor_status",
      status: {
        enabled: true,
        state: lastError ? "error" : "idle",
        queue_len: 0,
        active_count: 0,
        max_active_agents: 1,
        total_candidates: 1,
        autonomous_mode: false,
        launch_profile_source: "saved",
        launch_profile_summary: "codex / host",
        last_error: lastError,
      },
    });

    await inject(page, status("issue #3785: gh issue list: github_rate_limited"));
    const badge = page.locator("#op-notifications-button .op-rail__badge");
    await expect(badge).toHaveText("1");
    await expect(badge).toHaveAttribute("data-has-error", "true");
    // Still nothing rendered inside the Issue window.
    await expect(issueWindow.locator(".surface-error-indicator")).toHaveCount(0);
    await expect(issueWindow.locator(".knowledge-status.error")).toHaveCount(0);

    // A changed error text is a new occurrence of the same key: one row, x2.
    await inject(page, status("issue #3785: gh issue list: github_rate_limited (retry)"));
    await page.locator("#op-notifications-button").click();
    const drawer = page.locator("#notification-center");
    await expect(drawer).toHaveAttribute("data-open", "true");
    const row = drawer.locator('.notification-center__item[data-error-key="issue-monitor:last_error"]');
    await expect(row).toHaveCount(1);
    await expect(row).toHaveAttribute("data-level", "error");
    await expect(row.locator(".notification-center__count")).toHaveText("\u00d72");
    await expect(row).toContainText("(retry)");
    await page.keyboard.press("Escape");

    // Recovery: the row falls to read/resolved and the badge clears.
    await inject(page, status(null));
    await expect(row).toHaveAttribute("data-resolved", "true");
    await expect(badge).toBeHidden();

    const rendered = await page.evaluate(() => {
      const rowNode = document.querySelector('.notification-center__item[data-error-key="issue-monitor:last_error"]') as HTMLElement;
      return {
        theme: document.documentElement.getAttribute("data-theme"),
        rim: getComputedStyle(rowNode).borderLeftColor,
      };
    });
    expect(rendered.rim).not.toBe("");
    expect(["dark", "light", null]).toContain(rendered.theme);
  });
});
