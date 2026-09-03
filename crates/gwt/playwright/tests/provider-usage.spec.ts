/* SPEC #2970 Phase 10 / Issue #3784 — deterministic browser contract for the
 * compact provider-usage status summary and its shared detail popover.
 * Playwright runs this spec in both Chromium dark and light projects.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Provider usage status summary", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("labels providers, exposes severity, and opens one complete popover", async ({
    page,
  }, testInfo) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    await page.goto(APP_URL);

    const expectedTheme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", expectedTheme);

    const strip = page.locator("#op-strip-usage");
    await expect(strip).toBeVisible({ timeout: 10_000 });
    await expect(strip).toContainText("USAGE");
    await expect(strip).toContainText("CX 96%");
    await expect(strip).toContainText("CC 80%");
    await expect(strip).not.toContainText(/[⬡◇]/);

    const codex = strip.locator('[data-provider="codex"]');
    const claude = strip.locator('[data-provider="claude_code"]');
    await expect(codex).toHaveAttribute("data-severity", "danger");
    await expect(claude).toHaveAttribute("data-severity", "warning");
    await expect(strip).toHaveAttribute(
      "aria-label",
      "Provider usage: Codex 96% danger, Claude Code 80% warning",
    );

    const contrastRatios = await strip.evaluate((element) => {
      const parseRgb = (value: string) =>
        (value.match(/[\d.]+/g) || []).slice(0, 3).map(Number);
      const luminance = (rgb: number[]) => {
        const channels = rgb.map((value) => {
          const normalized = value / 255;
          return normalized <= 0.04045
            ? normalized / 12.92
            : ((normalized + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
      };
      const background = luminance(
        parseRgb(getComputedStyle(element.closest(".op-status-strip")!).backgroundColor),
      );
      return [...element.querySelectorAll<HTMLElement>(".op-usage-sum")].map(
        (summary) => {
          const foreground = luminance(parseRgb(getComputedStyle(summary).color));
          return (
            (Math.max(foreground, background) + 0.05) /
            (Math.min(foreground, background) + 0.05)
          );
        },
      );
    });
    expect(contrastRatios.every((ratio) => ratio >= 4.5)).toBe(true);

    await strip.hover();
    const popover = page.locator("#provider-usage-popover");
    await expect(popover).toBeVisible();
    await expect(popover).toHaveAttribute("role", "region");
    await expect(popover).toHaveAttribute("aria-label", "Usage & Limits");
    await expect(popover).toContainText("Codex");
    await expect(popover).toContainText(/5-hour\s*96%/);
    await expect(popover).toContainText(/Weekly\s*29%/);
    await expect(popover).toContainText("Claude Code");
    await expect(popover).toContainText(/Weekly\s*80%/);
    await expect(popover).toContainText(/1\/2/);
    await expect(strip).toHaveAttribute("aria-expanded", "true");

    // Issue #3862 — the Claude card lists its sessions (tokens + context
    // remaining) inside the same popover, most-consumed context first.
    const claudeSessions = popover.locator(
      '.op-usage-card[data-provider="claude_code"] .op-usage-sess',
    );
    await expect(claudeSessions).toBeVisible();
    await expect(claudeSessions).toContainText(/Sessions \(2\)/);
    const sessionRows = claudeSessions.locator(".op-usage-sess__row");
    await expect(sessionRows).toHaveCount(2);
    await expect(sessionRows.nth(0)).toContainText("claude-sonnet-5");
    await expect(sessionRows.nth(0)).toContainText("24k");
    await expect(sessionRows.nth(0)).toContainText("12%");
    await expect(sessionRows.nth(0).locator(".op-usage-sess__ctx")).toHaveAttribute(
      "data-severity",
      "warning",
    );
    await expect(sessionRows.nth(1)).toContainText("claude-fable-5-1");
    await expect(sessionRows.nth(1)).toContainText("1.2M");
    await expect(sessionRows.nth(1)).toContainText("58%");
    await expect(
      popover.locator('.op-usage-card[data-provider="codex"] .op-usage-sess'),
    ).toHaveCount(0);
    const popoverBox = await popover.boundingBox();
    const viewport = page.viewportSize();
    expect(popoverBox).not.toBeNull();
    expect(viewport).not.toBeNull();
    expect(popoverBox!.x).toBeGreaterThanOrEqual(0);
    expect(popoverBox!.y).toBeGreaterThanOrEqual(0);
    expect(popoverBox!.x + popoverBox!.width).toBeLessThanOrEqual(viewport!.width);
    expect(popoverBox!.y + popoverBox!.height).toBeLessThanOrEqual(viewport!.height);

    await page.locator(".project-bar").hover();
    await expect(popover).toBeHidden();

    await strip.focus();
    await page.keyboard.press("ArrowDown");
    await expect(popover).toBeVisible();
    await expect(popover).toBeFocused();
    await emitProviderUsage(page, [
      {
        provider: "codex",
        windows: [{ kind: "weekly", used_percent: 96 }],
        state: { kind: "ok" },
      },
    ]);
    await expect(popover).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(popover).toBeHidden();
    await expect(strip).toBeFocused();

    // Click is the non-hover fallback used by pointer and touch activation.
    await strip.click();
    await expect(popover).toBeVisible();
    await expect(page.locator("#provider-usage-popover")).toHaveCount(1);
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });

  test("labels an unknown-length window by its reported minutes (Issue #3860)", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    await page.goto(APP_URL);

    const strip = page.locator("#op-strip-usage");
    await expect(strip).toBeVisible({ timeout: 10_000 });
    await emitProviderUsage(page, [
      {
        provider: "codex",
        plan: "pro",
        windows: [
          { kind: "weekly", used_percent: 5, window_minutes: 10080 },
          { kind: "unknown", used_percent: 40, window_minutes: 1440 },
          { kind: "unknown", used_percent: 12 },
        ],
        state: { kind: "ok" },
      },
    ]);
    await expect(strip).toContainText("CX 40%");

    await strip.hover();
    const popover = page.locator("#provider-usage-popover");
    await expect(popover).toBeVisible();
    await expect(popover).toContainText(/Weekly\s*5%/);
    await expect(popover).toContainText(/Unknown \(1-day\)\s*40%/);
    await expect(popover).toContainText(/Unknown\s*12%/);
    const weekly = popover.locator(".op-usage-win__lbl", { hasText: "Weekly" });
    await expect(weekly).toHaveAttribute("data-window-minutes", "10080");
    await expect(weekly).toHaveAttribute("title", "Window length: 7 days");
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });

  test("keeps a stable width and compacts three providers to the critical one", async ({
    page,
  }) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    await page.goto(APP_URL);

    const strip = page.locator("#op-strip-usage");
    await expect(strip).toBeVisible({ timeout: 10_000 });
    const initialWidth = (await strip.boundingBox())!.width;

    await emitProviderUsage(page, [
      {
        provider: "codex",
        windows: [{ kind: "weekly", used_percent: 9 }],
        state: { kind: "ok" },
      },
      {
        provider: "claude_code",
        windows: [{ kind: "weekly", used_percent: 79 }],
        state: { kind: "ok" },
      },
    ]);
    await expect(strip.locator(".op-usage-sum")).toHaveCount(2);
    await expect(strip.locator('[data-provider="codex"]')).toHaveAttribute(
      "data-severity",
      "normal",
    );
    const normalContrast = await strip
      .locator('[data-provider="codex"]')
      .evaluate((summary) => {
        const parseRgb = (value: string) =>
          (value.match(/[\d.]+/g) || []).slice(0, 3).map(Number);
        const luminance = (rgb: number[]) => {
          const channels = rgb.map((value) => {
            const normalized = value / 255;
            return normalized <= 0.04045
              ? normalized / 12.92
              : ((normalized + 0.055) / 1.055) ** 2.4;
          });
          return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
        };
        const foreground = luminance(parseRgb(getComputedStyle(summary).color));
        const background = luminance(
          parseRgb(
            getComputedStyle(summary.closest(".op-status-strip")!).backgroundColor,
          ),
        );
        return (
          (Math.max(foreground, background) + 0.05) /
          (Math.min(foreground, background) + 0.05)
        );
      });
    expect(normalContrast).toBeGreaterThanOrEqual(4.5);
    expect((await strip.boundingBox())!.width).toBeCloseTo(initialWidth, 1);

    await emitProviderUsage(page, [
      {
        provider: "codex",
        windows: [
          {
            kind: "weekly",
            used_percent: 40,
            resets_at: "2031-01-02T03:04:05Z",
          },
        ],
        state: { kind: "ok" },
      },
      {
        provider: "claude_code",
        windows: [
          {
            kind: "weekly",
            used_percent: 70,
            resets_at: "2031-01-03T03:04:05Z",
          },
        ],
        state: { kind: "ok" },
      },
      {
        provider: "gemini",
        windows: [
          {
            kind: "weekly",
            used_percent: 95,
            resets_at: "2031-01-04T03:04:05Z",
          },
        ],
        state: { kind: "ok" },
      },
    ]);
    await expect(strip.locator(".op-usage-sum")).toHaveCount(1);
    await expect(strip.locator('[data-provider="gemini"]')).toHaveText("GE 95%");
    await expect(strip.locator(".op-usage-more")).toHaveText("+2");
    await expect(strip).toHaveAttribute(
      "aria-label",
      "Provider usage: Codex 40% normal, Claude Code 70% normal, GE 95% danger",
    );
    expect((await strip.boundingBox())!.width).toBeCloseTo(initialWidth, 1);

    await strip.hover();
    const popover = page.locator("#provider-usage-popover");
    await expect(popover).toBeVisible();
    await expect(popover).toContainText("Codex");
    await expect(popover).toContainText("Claude Code");
    await expect(popover).toContainText("gemini");
    await expect(popover).toContainText(/Weekly\s*40%/);
    await expect(popover).toContainText(/Weekly\s*70%/);
    await expect(popover).toContainText(/Weekly\s*95%/);
    await expect(popover).toContainText(/1\/4/);
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });
});

async function emitProviderUsage(page: any, accounts: unknown[]): Promise<void> {
  await page.evaluate((nextAccounts) => {
    (window as any).__emitProviderUsage(nextAccounts);
  }, accounts);
}

async function installProviderUsageBackend(page: any): Promise<void> {
  await page.addInitScript(() => {
    try {
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    } catch {
      /* no-op */
    }

    const workspaceState = {
      kind: "workspace_state",
      workspace: {
        app_version: "playwright",
        tabs: [
          {
            id: "tab-1",
            title: "Usage Fixture",
            project_root: "/fixture",
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: [],
            },
          },
        ],
        active_tab_id: "tab-1",
        recent_projects: [],
      },
    };
    const usageSnapshot = {
      kind: "provider_usage",
      accounts: [
        {
          provider: "codex",
          plan: "pro",
          windows: [
            {
              kind: "five_hour",
              used_percent: 96,
              resets_at: "2030-01-02T03:04:05Z",
            },
            {
              kind: "weekly",
              used_percent: 29,
              resets_at: "2030-01-06T03:04:05Z",
            },
          ],
          state: { kind: "ok" },
        },
        {
          provider: "claude_code",
          plan: "max",
          windows: [
            {
              kind: "weekly",
              used_percent: 80,
              resets_at: "2030-01-07T03:04:05Z",
            },
          ],
          state: { kind: "ok" },
        },
      ],
      // Issue #3862 — per-session tokens / context ride the same snapshot and
      // must be readable from the same popover as the account windows.
      sessions: [
        {
          session_id: "sess-claude-fable",
          provider: "claude_code",
          model: "claude-fable-5-1",
          input_tokens: 1000000,
          output_tokens: 234567,
          total_tokens: 1234567,
          context_used_tokens: 420000,
          context_limit_tokens: 1000000,
          context_left_pct: 58,
          limit_reached: false,
          eligible: true,
          state: { kind: "ok" },
        },
        {
          session_id: "sess-claude-sonnet",
          provider: "claude_code",
          model: "claude-sonnet-5",
          input_tokens: 20000,
          output_tokens: 4000,
          total_tokens: 24000,
          context_used_tokens: 176000,
          context_limit_tokens: 200000,
          context_left_pct: 12,
          limit_reached: false,
          eligible: true,
          state: { kind: "ok" },
        },
      ],
      consumption: [],
    };

    let socketRef: FixtureWebSocket | null = null;

    class FixtureWebSocket extends EventTarget {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url: string) {
        super();
        (this as any).url = url;
        (this as any).readyState = FixtureWebSocket.CONNECTING;
        socketRef = this;
        setTimeout(() => {
          (this as any).readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
        }, 0);
      }

      send(raw: string) {
        let message: any = null;
        try {
          message = JSON.parse(raw);
        } catch {
          return;
        }
        if (message?.kind === "frontend_ready") {
          this.emit(workspaceState);
          this.emit(usageSnapshot);
        }
      }

      close() {
        (this as any).readyState = FixtureWebSocket.CLOSED;
        this.dispatchEvent(new CloseEvent("close"));
      }

      emit(payload: unknown) {
        setTimeout(() => {
          this.dispatchEvent(
            new MessageEvent("message", { data: JSON.stringify(payload) }),
          );
        }, 0);
      }
    }

    (window as any).__emitProviderUsage = (accounts: unknown[]) => {
      socketRef?.emit({
        kind: "provider_usage",
        accounts,
        sessions: [],
        consumption: [],
      });
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
