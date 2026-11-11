/* SPEC #2970 Phase 11 / Issue #3784 — deterministic browser contract for the
 * compact provider-usage status summary and its shared detail popover.
 * Playwright runs this spec in both Chromium dark and light projects.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const liveUrl = process.env.GWT_PLAYWRIGHT_BASE_URL;

async function installUsageTheme(page: any, theme: string) {
  await page.addInitScript(({ theme, appOrigin }) => {
    // Init scripts can also run in the opaque document before app navigation.
    if (location.origin === appOrigin) localStorage.setItem("gwt:ui:theme", theme);
  }, { theme, appOrigin: new URL(liveUrl || APP_URL).origin });
}

test.describe("Provider usage status summary", () => {
  test.use({ viewport: { width: 1440, height: 900 } });
  test.beforeEach(async ({ page }, info) => {
    await installUsageTheme(page,
      info.project.name.includes("light") ? "light" : "dark");
  });

  test("Usage frame subscriptions batch bursts and advance with suspended animation frames", async ({ page }, info) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page, true);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);
    await page.getByRole("tab", { name: "Usage & Limits" }).click();
    const checkbox = page.getByRole("checkbox", { name: "Show Claude Code account usage" });
    await expect(checkbox).toBeChecked();
    const result = await page.evaluate(async () => {
      // Exercise the post-dispatch factory from the served checkout. Socket
      // coalescing alone must not mask repeated synchronous model updates.
      const modulePath = "/provider-usage-surface.js";
      const { createProviderUsageSurface } = await import(modulePath);
      const shell = (window as any).__operatorShell;
      const originalRender = shell.applyProviderUsage;
      const originalFrame = window.requestAnimationFrame;
      const renders: number[] = [];
      let workspaceRenders = 0;
      shell.applyProviderUsage = (snapshot: any) => {
        renders.push(snapshot.accounts[0].windows[0].used_percent);
        originalRender(snapshot);
      };
      const surface = createProviderUsageSurface({ send() {}, renderWorkspaceWindows: () => workspaceRenders++ });
      const usage = (value: number) => ({
        accounts: [{ provider: "claude_code", state: { kind: "ok" }, windows: [{ kind: "weekly", used_percent: value }] }],
        sessions: Array.from({ length: 5 }, (_, i) => ({ session_id: `frame-${i}`, value })), consumption: [],
      });
      const panel = document.querySelector<HTMLElement>('[data-settings-panel="usage"]')!;
      try {
        surface.renderUsagePanel(panel);
        const control = panel.querySelector("input")!;
        control.focus();
        for (let value = 1; value <= 50; value++) surface.applyProviderUsageUi(usage(value));
        const immediate = surface.usageForSession("frame-4").value;
        const beforeFrame = renders.length;
        await new Promise<void>(resolve => originalFrame(() => resolve()));
        const normal = { renders: [...renders], workspaceRenders, immediate, beforeFrame };
        window.requestAnimationFrame = () => -1;
        const start = performance.now();
        for (let value = 51; value <= 70; value++) surface.applyProviderUsageUi(usage(value));
        while (renders.length < 2 && performance.now() - start < 1000) {
          await new Promise(resolve => setTimeout(resolve, 100));
        }
        return { normal, renders, workspaceRenders, fallbackMs: performance.now() - start,
          focused: document.activeElement === control, checked: control.checked };
      } finally {
        window.requestAnimationFrame = originalFrame;
        shell.applyProviderUsage = originalRender;
      }
    });
    expect(result.normal).toEqual({ renders: [50], workspaceRenders: 1, immediate: 50, beforeFrame: 0 });
    expect(result.renders).toEqual([50, 70]);
    expect(result.workspaceRenders).toBe(2);
    expect(result.fallbackMs).toBeLessThan(1000);
    expect(result.focused && result.checked).toBe(true);
    await expect(checkbox).toBeFocused();
    await expectClaudeModels(page, "—", "—");
    await expect(page.locator('#op-strip-usage [data-provider="claude_code"]'))
      .toHaveAttribute("data-severity", "normal");
    await page.locator("#op-strip-usage").hover();
    await expect(page.locator("#provider-usage-popover")).toContainText(/Weekly\s*70%/);
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
    await page.screenshot({ path: info.outputPath("usage-frame.png"), fullPage: true });
    await info.attach("frame-results", { body: JSON.stringify(result), contentType: "application/json" });
    expect(errors).toEqual([]);
  });

  test("mounted Usage setting follows snapshots without replacing its focused control", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page, true);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);
    await page.getByRole("tab", { name: "Usage & Limits" }).click();
    const checkbox = page.getByRole("checkbox", { name: "Show Claude Code account usage" });
    await expect(checkbox).toBeChecked();
    await checkbox.focus();
    await emitProviderUsage(page, [{ provider: "claude_code", windows: [], state: { kind: "disabled" } }]);
    await expect(checkbox).not.toBeChecked();
    await expect(checkbox).toBeFocused();
    await emitProviderUsage(page, [{ provider: "claude_code", windows: [], state: { kind: "ok" } }]);
    await expect(checkbox).toBeChecked();
    await expect(checkbox).toBeFocused();
    expect(errors).toEqual([]);
  });

  test("labels providers, exposes severity, and opens one complete popover", async ({
    page,
  }, testInfo) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);

    const expectedTheme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", expectedTheme);

    const strip = page.locator("#op-strip-usage");
    await expect(strip).toBeVisible({ timeout: 10_000 });
    await expect(strip).toContainText("USAGE");
    await expect(strip).toContainText("CX 96%");
    await expectClaudeModels(page, "—", "—");
    await expect(strip).not.toContainText(/[⬡◇]/);

    const codex = strip.locator('[data-provider="codex"]');
    const claude = strip.locator('[data-provider="claude_code"]');
    await expect(codex).toHaveAttribute("data-severity", "danger");
    await expect(claude).toHaveAttribute("data-severity", "warning");
    await expect(strip).toHaveAttribute("aria-label", /Codex 96% danger/);
    await expect(strip).toHaveAttribute("aria-label", /Claude Code.*Fable weekly.*not provided.*Opus weekly.*not provided/);
    await expect(strip).toHaveAttribute("aria-label", /warning/);

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

    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);

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

  test("shows Claude Fable weekly and enabled usage credits (SPEC #2970 Phase 8)", async ({
    page,
  }, testInfo) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => {
      if (message.type() === "error") errors.push(message.text());
    });
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);
    const strip = page.locator("#op-strip-usage");
    await expect(strip).toBeVisible({ timeout: 10_000 });
    const resetsAt = "2031-01-02T03:04:05Z";
    const account = {
      provider: "claude_code", plan: "max", state: { kind: "ok" },
      windows: [{ kind: "fable_weekly", used_percent: 0, resets_at: resetsAt }],
    };
    const extraUsage = { is_enabled: true, used_credits: 1610, monthly_limit: 2000,
      utilization: 80.5, currency: "USD", disabled_reason: null };
    await emitProviderUsage(page, [{ ...account, extra_usage: extraUsage }]);
    await expectClaudeModels(page, "0%", "—");
    // Keep the detail open through updates without depending on the native
    // pointer staying over the trigger in a headed browser.
    await page.mouse.move(0, 0);
    await strip.focus();
    await strip.press("ArrowDown");
    await expect(page.locator("#provider-usage-popover")).toBeFocused();
    const card = page.locator('.op-usage-card[data-provider="claude_code"]');
    const fable = card.locator(".op-usage-win").filter({ hasText: "Fable weekly" });
    await expect(fable.locator(".op-usage-win__lbl")).toHaveText("Fable weekly");
    await expect(fable.locator(".op-usage-win__pct")).toHaveText("0%");
    await expect(fable.locator(".op-usage-bar__fill")).toHaveCSS("width", "0px");
    const resetLabel = await page.evaluate(iso => new Date(iso).toLocaleString(undefined, {
      month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit",
    }), resetsAt);
    await expect(fable.locator(".op-usage-win__reset")).toHaveText(`↻ ${resetLabel}`);
    const credits = card.locator('[data-usage-credits="true"]');
    await expect(credits).toBeVisible();
    await expect(credits.locator(".op-usage-win__lbl")).toHaveText("Usage credits");
    await expect(credits.locator(".op-usage-win__pct")).toHaveText("81%");
    await expect(credits.locator(".op-usage-bar__fill")).toHaveAttribute("style", "width: 81%;");
    await expect(credits.locator(".op-usage-bar__fill")).toHaveAttribute("data-severity", "warning");
    await expect(credits.locator(".op-usage-win__reset")).toHaveText("1610 / 2000 USD");
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      testInfo.project.name.includes("light") ? "light" : "dark");
    await page.screenshot({ path: testInfo.outputPath("claude-fable-usage-credits.png"), fullPage: true });

    await emitProviderUsage(page, [{ ...account, extra_usage: { ...extraUsage,
      used_credits: null, monthly_limit: null, utilization: null, currency: null } }]);
    await expect(credits.locator(".op-usage-win__reset")).toHaveText("— / —");
    await expect(credits.locator(".op-usage-win__pct")).toHaveText("—");
    await expect(credits.locator(".op-usage-bar")).toHaveCount(0);
    for (const extra of [undefined, null, { is_enabled: false, used_credits: null,
      monthly_limit: null, utilization: null, currency: null, disabled_reason: null }]) {
      await emitProviderUsage(page, [{ ...account, extra_usage: extra }]);
      await expect(credits).toHaveCount(0);
      await expect(fable.locator(".op-usage-win__pct")).toHaveText("0%");
    }
    expect(errors).toEqual([]);
  });

  test("keeps full Claude model windows separate from account limits (Issue #6638)", async ({ page }, info) => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);
    const account = {
      provider: "claude_code", state: { kind: "ok" }, limit_reached: false,
      windows: [
        { kind: "weekly", used_percent: 96 },
        { kind: "fable_weekly", used_percent: 100, resets_at: "2031-01-02T03:04:05Z" },
      ],
    };
    await emitProviderUsage(page, [account]);
    const strip = page.locator("#op-strip-usage");
    await expectClaudeModels(page, "100%", "—");
    await expect(strip.locator('[data-provider="claude_code"]')).toHaveAttribute("data-severity", "danger");
    await expect(strip).not.toHaveAttribute("data-limit", "true");
    await page.mouse.move(0, 0);
    await strip.focus();
    await strip.press("ArrowDown");
    const popover = page.locator("#provider-usage-popover");
    await expect(popover).toBeFocused();
    const card = popover.locator('.op-usage-card[data-provider="claude_code"]');
    await expect(card).toContainText(/Weekly\s*96%/);
    await expect(card).toContainText(/Fable weekly\s*100%/);
    await expect(card.locator(".op-usage-card__limit")).toHaveCount(0);
    await expect(card.locator(".op-usage-win").filter({ hasText: "Fable weekly" })
      .locator(".op-usage-win__reset")).not.toBeEmpty();
    await expect(page.locator("html")).toHaveAttribute("data-theme",
      info.project.name.includes("light") ? "light" : "dark");
    await page.screenshot({ path: info.outputPath("claude-model-limit.png"), fullPage: true });

    await emitProviderUsage(page, [{ ...account, limit_reached: true,
      windows: [{ kind: "five_hour", used_percent: 100 }, ...account.windows] }]);
    await expectClaudeModels(page, "100%", "—");
    await expect(strip.locator('[data-provider="claude_code"]')).toHaveAttribute("data-limit", "true");
    await expect(card.locator(".op-usage-card__limit")).toHaveText("Limit reached");
    await expect(card).toContainText(/Fable weekly\s*100%/);
    expect(errors).toEqual([]);
  });

  test("keeps both Claude weekly models independent of the active session and shared limit (SPEC #2970 Phase 11)", async ({ page }, info) => {
    const errors = captureUsageErrors(page);
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page, false, true);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);
    const strip = page.locator("#op-strip-usage");
    await expect(strip).toBeVisible();
    const account = {
      provider: "claude_code", state: { kind: "ok" }, limit_reached: false,
      windows: [
        { kind: "five_hour", used_percent: 30 },
        { kind: "weekly", used_percent: 40 },
        { kind: "opus_weekly", used_percent: 70 },
        { kind: "fable_weekly", used_percent: 20, resets_at: "2031-01-02T03:04:05Z" },
      ],
    };
    const codex = { provider: "codex", state: { kind: "ok" }, windows: [
      { kind: "five_hour", used_percent: 90 }, { kind: "weekly", used_percent: 10 },
    ] };
    const sessions = ["fable", "opus", "codex"].map(model => ({
      session_id: `usage-${model}-session`, provider: model === "codex" ? "codex" : "claude_code",
      project_path: "/fixture",
      model: model === "codex" ? "gpt-5" : `claude-${model}-5`,
      total_tokens: 1000, context_left_pct: 80, eligible: true, state: { kind: "ok" },
    }));
    await emitProviderUsage(page, [codex, account], sessions);
    await expectClaudeModels(page, "20%", "70%");
    await expect(strip.locator(".op-usage-sum")).toHaveCount(2);
    await expect(strip.locator(".op-usage-more")).toHaveCount(0);
    await expect(strip.locator('[data-provider="codex"]')).toHaveText("CX 90%");
    await expect(strip).toHaveAttribute("aria-label", /Claude Code.*Fable weekly 20%.*Opus weekly 70%/);
    await expect(page.locator("#provider-usage-popover")).toBeHidden();

    await page.getByRole("button", { name: "Works", exact: true }).click();
    for (const model of ["fable", "opus", "codex"]) {
      await page.locator(`.work-row[data-window-id="usage-${model}"] .work-open`).click();
      await expect(page.locator(`.works-terminal[data-window-id="usage-${model}"]`)).toBeVisible();
      await expect(page.locator(".works-full-meta")).toContainText(model === "codex" ? "gpt-5" : `claude-${model}-5`);
      await expectClaudeModels(page, "20%", "70%");
      await strip.focus();
      await expectClaudeModels(page, "20%", "70%");
      await page.keyboard.press("Escape");
      await page.locator('.works-full [data-action="back"]').click();
    }

    // A snapshot changes only Fable; reversing the input order must not change
    // the fixed Fable/Opus presentation or infer values from session models.
    account.windows[3].used_percent = 25;
    await emitProviderUsage(page, [codex, account], [...sessions].reverse());
    await expectClaudeModels(page, "25%", "70%");
    await page.mouse.move(0, 0);
    await strip.focus();
    await strip.press("ArrowDown");
    const popover = page.locator("#provider-usage-popover");
    await expect(popover).toBeFocused();
    const card = popover.locator('.op-usage-card[data-provider="claude_code"]');
    const fable = card.locator(".op-usage-win").filter({ hasText: "Fable weekly" });
    const opus = card.locator(".op-usage-win").filter({ hasText: "Opus weekly" });
    await expect(fable.locator(".op-usage-win__pct")).toHaveText("25%");
    await expect(opus.locator(".op-usage-win__pct")).toHaveText("70%");
    const resetLabel = await page.evaluate(iso => new Date(iso).toLocaleString(undefined, {
      month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit",
    }), account.windows[3].resets_at!);
    await expect(fable.locator(".op-usage-win__reset")).toHaveText(`↻ ${resetLabel}`);
    await expect(opus.locator(".op-usage-win__reset")).toBeEmpty();
    await expect(card.locator(".op-usage-card__limit")).toHaveCount(0);

    await emitProviderUsage(page, [codex, { ...account, windows: account.windows.map(window =>
      window.kind === "five_hour" ? { ...window, used_percent: 100 } :
        window.kind === "weekly" ? { ...window, used_percent: 95 } : window) }], sessions);
    await expectClaudeModels(page, "25%", "70%");
    await expect(strip.locator('[data-provider="claude_code"]')).toHaveAttribute("data-severity", "danger");
    await expect(strip).toHaveAttribute("aria-label", /(?:shared|account).*100%/i);
    await expect(card).toContainText(/5-hour\s*100%/);
    await expect(card.locator(".op-usage-card__limit")).toHaveCount(0);
    await expect(popover).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(popover).toBeHidden();
    await expect(strip).toBeFocused();
    expect(await strip.evaluate(element => getComputedStyle(element).outlineStyle)).not.toBe("none");
    await page.screenshot({ path: info.outputPath("claude-weekly-models.png"), fullPage: true });
    expect(errors).toEqual([]);
  });

  test("distinguishes Claude model zero, missing, degraded and stale values (SPEC #2970 Phase 11)", async ({ page }, info) => {
    const errors = captureUsageErrors(page);
    await page.goto("about:blank");
    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);
    const strip = page.locator("#op-strip-usage");
    const modelWindows = (fable: unknown, opus: unknown) => [
      { kind: "fable_weekly", used_percent: fable }, { kind: "opus_weekly", used_percent: opus },
    ];
    for (const [windows, fable, opus] of [
      [[{ kind: "fable_weekly", used_percent: 0 }], "0%", "—"],
      [[{ kind: "opus_weekly", used_percent: 0 }], "—", "0%"],
      [[], "—", "—"],
      [modelWindows(null, "invalid"), "—", "—"],
    ] as const) {
      await emitProviderUsage(page, [{ provider: "claude_code", state: { kind: "ok" }, windows }]);
      await expectClaudeModels(page, fable, opus);
      await expect(strip).toHaveAttribute("aria-label", /not provided/);
    }
    await page.mouse.move(0, 0);
    await strip.focus();
    await strip.press("ArrowDown");
    const popover = page.locator("#provider-usage-popover");
    const card = popover.locator('.op-usage-card[data-provider="claude_code"]');
    await expect(popover).toBeFocused();
    await expect(card.locator(".op-usage-win__pct")).toHaveText(["—", "—"]);
    await expect(card.locator(".op-usage-bar")).toHaveCount(0);
    await expect(card).not.toContainText(/0%|NaN/);
    await page.keyboard.press("Escape");
    for (const [kind, value] of [["disabled", "off"], ["no_data", "—"], ["unavailable", "n/a"]]) {
      await emitProviderUsage(page, [{ provider: "claude_code", state: { kind, reason: "fixture unavailable" },
        windows: modelWindows(20, 70) }]);
      await expectClaudeModels(page, value, value);
    }
    for (const [percent, severity] of [[79, "normal"], [80, "warning"], [94, "warning"], [95, "danger"]] as const) {
      await emitProviderUsage(page, [{ provider: "claude_code", state: { kind: "ok" },
        windows: modelWindows(percent, 70) }]);
      await expectClaudeModels(page, `${percent}%`, "70%");
      const claude = strip.locator('[data-provider="claude_code"]');
      await expect(claude.locator('[data-window-kind="fable_weekly"]')).toHaveAttribute("data-severity", severity);
      await expect(claude.locator('[data-window-kind="opus_weekly"]')).toHaveAttribute("data-severity", "normal");
      await expect(claude).not.toHaveAttribute("data-severity", "danger");
    }
    await emitProviderUsage(page, [{ provider: "claude_code", fetched_at: "2026-01-01T00:00:00Z",
      state: { kind: "stale", age_secs: 420 }, windows: modelWindows(20, 70) }]);
    await expectClaudeModels(page, "20%", "70%");
    await expect(strip.locator('[data-provider="claude_code"]')).toContainText(/stale/i);
    await expect(strip).toHaveAttribute("aria-label", /stale.*(?:420|7m|7 min)/i);
    await strip.focus();
    await strip.press("ArrowDown");
    await expect(popover).toBeFocused();
    await expect(card.locator(".op-usage-win__pct")).toHaveText(["20%", "70%"]);
    await expect(card.locator(".op-usage-card__reason")).toHaveText("stale 7m");
    await expect(page.locator("html")).toHaveAttribute("data-theme", info.project.name.includes("light") ? "light" : "dark");
    await page.screenshot({ path: info.outputPath("claude-weekly-stale.png"), fullPage: true });
    expect(errors).toEqual([]);
  });

  test("keeps both Claude models readable after narrow and 200% reflow (SPEC #2970 Phase 11)", async ({ page: initialPage, browser }, info) => {
    const errors = captureUsageErrors(initialPage);
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const openFixture = async (page: any) => {
      await installUsageTheme(page, theme);
      if (!liveUrl) await installEmbeddedRoutes(page);
      await installProviderUsageBackend(page);
      if (liveUrl) await gotoLiveGwt(page, liveUrl);
      else await page.goto(APP_URL);
    };
    await openFixture(initialPage);
    const accounts = [
      { provider: "codex", state: { kind: "ok" }, windows: [{ kind: "weekly", used_percent: 40 }] },
      { provider: "claude_code", state: { kind: "ok" }, windows: [
        { kind: "fable_weekly", used_percent: 20 }, { kind: "opus_weekly", used_percent: 70 },
      ] },
    ];
    for (const [width, height, zoom] of [[1440, 900, 1], [768, 900, 1], [360, 900, 1], [720, 450, 2]] as const) {
      // Supported context sizing preserves 1440×900 screenshot pixels at the
      // 200% equivalent of 720×450 CSS pixels and device scale factor 2.
      const context = zoom === 2 ? await browser.newContext({
        viewport: { width, height }, deviceScaleFactor: 2, colorScheme: theme,
      }) : null;
      try {
        const page = context ? await context.newPage() : initialPage;
        const pageErrors = context ? captureUsageErrors(page) : errors;
        if (context) await openFixture(page);
        else await page.setViewportSize({ width, height });
        if (zoom === 2) {
          expect(await page.evaluate(() => ({ width: innerWidth, height: innerHeight, scale: devicePixelRatio })))
            .toEqual({ width: 720, height: 450, scale: 2 });
        }
        const strip = page.locator("#op-strip-usage");
        await emitProviderUsage(page, accounts);
        await expectClaudeModels(page, "20%", "70%");
        await assertUsageLayout(page);
        accounts[1].windows[0].used_percent = 95;
        await emitProviderUsage(page, accounts);
        await expectClaudeModels(page, "95%", "70%");
        await assertUsageLayout(page);
        await page.evaluate(() => {
          (window as any).__emitWorkspaceStateNotice({ kind: "load_error",
            path: "/fixture/current.json", message: "Workspace persistence is unavailable. Retry to reload." });
        });
        await expect(page.locator("#app > .react-island > .workspace-state-notice")).toBeVisible();
        await expectClaudeModels(page, "95%", "70%");
        await assertUsageLayout(page);
        await strip.focus();
        await strip.press("ArrowDown");
        const popover = page.locator("#provider-usage-popover");
        await expect(popover).toBeVisible();
        await expect(popover).toBeFocused();
        const box = (await popover.boundingBox())!;
        expect(box.x).toBeGreaterThanOrEqual(0);
        expect(box.x + box.width).toBeLessThanOrEqual(width + 1);
        expect(box.y).toBeGreaterThanOrEqual(0);
        expect(box.y + box.height).toBeLessThanOrEqual(height + 1);
        await page.keyboard.press("Escape");
        await expect(strip).toBeFocused();
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        const screenshot = await page.screenshot({ path: info.outputPath(`claude-weekly-${width}-${zoom}x.png`), fullPage: true });
        if (zoom === 2) {
          expect(screenshot.readUInt32BE(16), "200% PNG width in physical pixels").toBe(1440);
          expect(screenshot.readUInt32BE(20), "200% PNG height in physical pixels").toBe(900);
        }
        await page.evaluate(() => (window as any).__emitWorkspaceStateNotice(null));
        await expect(page.locator(".workspace-state-notice")).toHaveCount(0);
        accounts[1].windows[0].used_percent = 20;
        expect(pageErrors).toEqual([]);
      } finally {
        await context?.close();
      }
    }
    expect(errors).toEqual([]);
  });

  test("keeps weekly 100% separate from a provider refusal (Issue #5037)", async ({
    page,
  }, testInfo) => {
    const pageErrors: string[] = [];
    const consoleErrors: string[] = [];
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });

    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);

    const expectedTheme = testInfo.project.name.includes("light") ? "light" : "dark";
    await expect(page.locator("html")).toHaveAttribute("data-theme", expectedTheme);

    const strip = page.locator("#op-strip-usage");
    await expect(strip).toBeVisible({ timeout: 10_000 });
    const account = {
      provider: "codex",
      plan: "pro",
      windows: [{ kind: "weekly", used_percent: 100, window_minutes: 10080 }],
      limit_reached: false,
      state: { kind: "ok" },
    };
    await emitProviderUsage(page, [account]);
    await expect(strip).toContainText("CX 100%");

    await strip.hover();
    const popover = page.locator("#provider-usage-popover");
    await expect(popover).toBeVisible();
    const codex = popover.locator('.op-usage-card[data-provider="codex"]');
    await expect(codex).toContainText(/Weekly\s*100%/);
    await expect(codex.locator(".op-usage-card__limit")).toHaveCount(0);

    await emitProviderUsage(page, [{ ...account, limit_reached: true }]);
    await expect(strip).toContainText("CX 100%");
    await expect(codex).toContainText(/Weekly\s*100%/);
    await expect(codex.locator(".op-usage-card__limit")).toBeVisible();
    await expect(codex.locator(".op-usage-card__limit")).toHaveText("Limit reached");
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

    if (!liveUrl) await installEmbeddedRoutes(page);
    await installProviderUsageBackend(page);
    if (liveUrl) await gotoLiveGwt(page, liveUrl);
    else await page.goto(APP_URL);

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
        provider: "custom_provider",
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
    await expect(strip.locator('[data-provider="custom_provider"]')).toHaveText("CU 95%");
    await expect(strip.locator(".op-usage-more")).toHaveText("+2");
    await expect(strip).toHaveAttribute("aria-label", /Codex 40% normal/);
    await expect(strip).toHaveAttribute("aria-label", /Claude Code.*Fable weekly.*not provided.*Opus weekly.*not provided/);
    await expect(strip).toHaveAttribute("aria-label", /CU 95% danger/);
    expect((await strip.boundingBox())!.width).toBeCloseTo(initialWidth, 1);

    await strip.hover();
    const popover = page.locator("#provider-usage-popover");
    await expect(popover).toBeVisible();
    await expect(popover).toContainText("Codex");
    await expect(popover).toContainText("Claude Code");
    await expect(popover).toContainText("custom_provider");
    await expect(popover).toContainText(/Weekly\s*40%/);
    await expect(popover).toContainText(/Weekly\s*70%/);
    await expect(popover).toContainText(/Weekly\s*95%/);
    await expect(popover).toContainText(/1\/4/);

    // Selection still counts actual providers. A critical Claude account
    // retains both models when it becomes the one visible provider.
    await emitProviderUsage(page, [
      { provider: "custom_provider", state: { kind: "ok" }, windows: [{ kind: "weekly", used_percent: 95 }] },
      { provider: "claude_code", state: { kind: "ok" }, windows: [
        { kind: "weekly", used_percent: 99 }, { kind: "opus_weekly", used_percent: 70 },
        { kind: "fable_weekly", used_percent: 20 },
      ] },
      { provider: "codex", state: { kind: "ok" }, windows: [{ kind: "weekly", used_percent: 40 }] },
    ]);
    await expect(strip.locator(".op-usage-sum")).toHaveCount(1);
    await expectClaudeModels(page, "20%", "70%");
    await expect(strip.locator(".op-usage-more")).toHaveText("+2");
    await expect(strip).toHaveAttribute("aria-label", /CU 95% danger.*Claude Code.*Fable weekly 20%.*Opus weekly 70%.*Codex 40% normal/);
    expect(pageErrors).toEqual([]);
    expect(consoleErrors).toEqual([]);
  });
});

function captureUsageErrors(page: any): string[] {
  const errors: string[] = [];
  page.on("pageerror", (error: Error) => errors.push(error.message));
  page.on("console", (message: any) => { if (message.type() === "error") errors.push(message.text()); });
  return errors;
}

async function expectClaudeModels(page: any, fable: string, opus: string): Promise<void> {
  const models = page.locator('#op-strip-usage [data-provider="claude_code"] .op-usage-model');
  await expect(models).toHaveCount(2);
  await expect(models.nth(0)).toHaveAttribute("data-window-kind", "fable_weekly");
  await expect(models.nth(1)).toHaveAttribute("data-window-kind", "opus_weekly");
  await expect(models.nth(0)).toHaveText(new RegExp(`^Fable\\s+(?:weekly|7d)\\s+${escapeRegExp(fable)}$`));
  await expect(models.nth(1)).toHaveText(new RegExp(`^Opus\\s+(?:weekly|7d)\\s+${escapeRegExp(opus)}$`));
  await expect(models.nth(0)).toBeVisible();
  await expect(models.nth(1)).toBeVisible();
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

async function assertUsageLayout(page: any): Promise<void> {
  const violations = await page.evaluate(() => {
    const problems: string[] = [];
    const strip = document.querySelector<HTMLElement>("#op-strip-usage")!;
    const models = [...strip.querySelectorAll<HTMLElement>(".op-usage-model")];
    const fits = (inner: DOMRect, outer: DOMRect) => inner.left >= outer.left - 1 &&
      inner.right <= outer.right + 1 && inner.top >= outer.top - 1 && inner.bottom <= outer.bottom + 1;
    const overlaps = (left: DOMRect, right: DOMRect) =>
      Math.min(left.right, right.right) - Math.max(left.left, right.left) > 1 &&
      Math.min(left.bottom, right.bottom) - Math.max(left.top, right.top) > 1;
    const viewport = new DOMRect(0, 0, window.innerWidth, window.innerHeight);
    for (const model of models) {
      const range = document.createRange();
      range.selectNodeContents(model);
      for (const text of range.getClientRects()) {
        if (!fits(text, viewport)) problems.push(`${model.dataset.windowKind}: outside viewport`);
        for (let ancestor: HTMLElement | null = model; ancestor; ancestor = ancestor.parentElement) {
          if ((ancestor === model || ancestor === strip || ancestor.classList.contains("op-status-strip")) &&
              !fits(text, ancestor.getBoundingClientRect())) problems.push(`${model.dataset.windowKind}: text escapes ${ancestor.className}`);
          const style = getComputedStyle(ancestor);
          if (/(hidden|clip|auto|scroll)/.test(`${style.overflowX} ${style.overflowY}`) &&
              !fits(text, ancestor.getBoundingClientRect())) problems.push(`${model.dataset.windowKind}: clipped by ${ancestor.className}`);
        }
      }
      if (getComputedStyle(model).textOverflow === "ellipsis") problems.push("model ellipsis");
    }
    if (overlaps(models[0].getBoundingClientRect(), models[1].getBoundingClientRect())) problems.push("models overlap");
    for (const neighbor of document.querySelectorAll<HTMLElement>(".op-status-strip__cell, .project-bar, #op-rail, .workspace-state-notice")) {
      if (neighbor === strip || !neighbor.getClientRects().length) continue;
      if (overlaps(strip.getBoundingClientRect(), neighbor.getBoundingClientRect())) problems.push(`overlap: ${neighbor.id || neighbor.className}`);
    }
    if (document.documentElement.scrollWidth > document.documentElement.clientWidth + 1 ||
        document.body.scrollWidth > document.documentElement.clientWidth + 1) problems.push("horizontal page overflow");
    return problems;
  });
  expect(violations, "both full model labels and values fit without clipping or overlap").toEqual([]);
}

async function emitProviderUsage(page: any, accounts: unknown[], sessions: unknown[] = []): Promise<void> {
  await page.evaluate(({ accounts, sessions }) => {
    (window as any).__emitProviderUsage(accounts, sessions);
  }, { accounts, sessions });
}

async function installProviderUsageBackend(page: any, withSettings = false, withAgents = false): Promise<void> {
  await page.addInitScript(({ withSettings, withAgents }) => {
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
            project_key: location.pathname.split("/")[2],
            kind: "git",
            workspace: {
              viewport: { x: 0, y: 0, zoom: 1 },
              windows: withSettings ? [{ id: "tab-1::settings", title: "Settings", preset: "settings",
                geometry: { x: 40, y: 40, width: 1100, height: 750 }, z_index: 1,
                status: "running", persist: true, minimized: false, maximized: false }] :
                withAgents ? ["fable", "opus", "codex"].map((model, index) => ({
                  id: `usage-${model}`, title: model, preset: "agent", agent_name: model,
                  agent_id: model === "codex" ? "codex" : "claude", session_id: `usage-${model}-session`,
                  geometry: { x: 100, y: 100, width: 720, height: 400 }, geometry_revision: 0,
                  z_index: index + 1, status: "idle", persist: true, minimized: false, maximized: false,
                })) : [],
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
          if (withAgents) this.emit({ kind: "active_work_projection", projection: {
            id: "usage-fixture", active_works: [], local_agent_activity: {
              project_key: location.pathname.split("/")[2], host_id: "usage-fixture",
              observed_at: new Date().toISOString(), agents: ["fable", "opus", "codex"].map(model => ({
                window_id: `usage-${model}`, session_id: `usage-${model}-session`,
                runtime_incarnation: 1, is_pm: false, issue_number: null,
              })),
            },
          } });
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

    (window as any).__emitProviderUsage = (accounts: unknown[], sessions: unknown[] = []) => {
      socketRef?.emit({
        kind: "provider_usage",
        accounts,
        sessions,
        consumption: [],
      });
    };
    (window as any).__emitWorkspaceStateNotice = (notice: unknown) => {
      socketRef?.emit({ kind: "workspace_state_notice", notice });
    };

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  }, { withSettings, withAgents });
}
