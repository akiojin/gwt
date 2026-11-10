import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const base = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const checkHome = process.env.GWT_PLAYWRIGHT_CHECK_HOME ?? "";

test.use({ viewport: { width: 1600, height: 1100 } });

test("Issue #4224 reauthentication needs human attention while quota remains queued", async ({ page }, info) => {
  test.skip(!base || !checkHome, "requires the isolated checkout browser-check runner");
  test.setTimeout(90_000);
  await withLiveGwtBackendLock(base, info, async () => {
    const marker = JSON.parse(await readFile(join(checkHome, "issue-4224-isolated.json"), "utf8"));
    expect(await realpath(checkHome)).not.toBe(await realpath(homedir()));
    expect(new URL(base).origin).toBe(new URL(marker.url).origin);
    const preferences = join(checkHome, ".gwt/projects", marker.repo_hash, "project-state");
    expect(JSON.parse(await readFile(join(preferences, "pm.json"), "utf8")).settings.auto_start).toBe(false);
    expect(JSON.parse(await readFile(join(preferences, "issue-monitor.json"), "utf8")).enabled).toBe(false);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.includes("light") ? "light" : "dark";
    await page.addInitScript(theme => {
      localStorage.setItem("gwt:ui:theme", theme);
      const send = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data) {
        if (typeof data === "string") {
          const event = JSON.parse(data);
          // The real checkout serves and creates the Issue window. Its existing
          // display seam supplies only these two diagnostic Inbox projections;
          // Rust tests own live-screen classification and Monitor transitions.
          if (event.kind === "list_issue_monitor") return;
          if (event.kind === "load_knowledge_bridge" && event.knowledge_kind === "issue") {
            (window as any).__providerAuthFixtureLoad = event;
            queueMicrotask(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
              ...event, kind: "knowledge_entries", entries: [], selected_number: null, refresh_enabled: true,
            } })));
            return;
          }
        }
        return send.call(this, data);
      };
    }, theme);
    await gotoLiveGwt(page, base, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue", bounds: { x: 40, y: 40, width: 1400, height: 900 } });
    const issueWindow = page.locator('.workspace-window[data-preset="issue"]').last();
    const surface = issueWindow.locator('.issue-bridge-root');
    await expect(surface).toBeVisible();
    await page.waitForFunction(() => (window as any).__providerAuthFixtureLoad);
    const id = await issueWindow.getAttribute("data-id");
    const authReason = "Claude Code reauthentication required; sign in to the provider CLI, then retry the Issue.";
    const quotaReason = "Codex usage limit reached — resumes after 2026-10-23T03:00:00Z";
    const items = [
      { issue: { number: 422401, title: "Provider reauthentication", state: "open", labels: [] },
        state: "needs_human", error_message: authReason, retry_hold_reason: authReason, retry_not_before: null },
      { issue: { number: 422402, title: "Provider quota", state: "open", labels: [] },
        state: "queued", error_message: quotaReason, retry_hold_reason: quotaReason,
        retry_not_before: "2026-10-23T03:00:00Z" },
    ];
    await page.evaluate(items => {
      const inject = (detail: unknown) => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail }));
      inject({ kind: "issue_monitor_status", status: { enabled: false, state: "disabled", active_count: 0,
        max_active_agents: 1, terminal_queue: [{ number: 422402, queued_by: "operator" }], queue_len: 1 } });
      inject({ kind: "issue_monitor_inbox", items });
    }, items);
    const auth = surface.locator('[data-queue-column="backlog"] .knowledge-row[data-issue-number="422401"]');
    await expect(auth).toBeVisible();
    await expect(auth.locator(".knowledge-row-badge")).toHaveText("Needs human");
    await expect(auth.locator('[data-key="reason"]')).toHaveText(authReason);
    const quota = surface.locator('[data-queue-column="queued"] .knowledge-row[data-issue-number="422402"]');
    await expect(quota).toBeVisible();
    await expect(quota.locator(".knowledge-row-badge")).toHaveText("Queued");
    await expect(quota).not.toContainText("re-authenticate");
    await info.attach(`provider-auth-notice-${theme}`, { body: await page.screenshot({ fullPage: true }), contentType: "image/png" });
    await info.attach("provider-auth-inbox-projections", { body: JSON.stringify({ theme, items,
      scope: "Live checkout assets and window; deterministic Inbox display projections. Runtime transition evidence is in Rust tests." }, null, 2),
      contentType: "application/json" });
    await sendLiveGwtEvent(page, { kind: "close_window", id });
    expect(errors, "zero console/page errors").toEqual([]);
  });
});
