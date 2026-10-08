import { expect, test } from "@playwright/test";
import {
  gotoLiveGwt,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

test.describe("Issue Monitor candidate pool", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  test.setTimeout(90_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("adds, reorders, edits and removes candidates with persistent settings", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      page.on("console", (message) => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      await expect(page.locator("#close-project-button")).toBeVisible();
      // The live fixture has Monitor and PM disabled. This only edits profiles;
      // it never starts an agent, enables the Monitor, or creates a branch.
      await sendLiveGwtEvent(page, {
        kind: "issue_monitor_profiles_set",
        profiles: [{ agent_id: "claude", model: "sonnet", prefer_for: ["kind:spec"] }],
        usage_threshold_percent: 80,
      });
      const beforeIds = await page.locator(".workspace-window").evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.id));
      await sendLiveGwtEvent(page, {
        kind: "create_window", preset: "issue",
        bounds: { x: 60, y: 50, width: 1040, height: 820 },
      });
      const windowId = await page.waitForFunction((before) => {
        const node = [...document.querySelectorAll<HTMLElement>('.workspace-window[data-preset="issue"]')]
          .find(node => !before.includes(node.dataset.id));
        return node?.dataset.id;
      }, beforeIds).then(handle => handle.jsonValue());
      const surface = page.locator(`.workspace-window[data-id="${windowId}"]`);
      await expect(surface).toBeVisible();
      const pool = surface.locator(".knowledge-monitor-pool");
      await pool.locator("summary").click();
      const rows = pool.locator(".knowledge-monitor-candidate");
      await expect(rows).toHaveCount(1);
      await expect(pool.getByRole("button", { name: "Remove claude", exact: true })).toBeDisabled();
      await pool.getByLabel("Agent command").fill("codex");
      // Keyboard focus must survive the input → button transition.
      await pool.getByLabel("Agent command").press("Tab");
      await expect(pool.getByRole("button", { name: "Add candidate", exact: true })).toBeFocused();
      const statusCount = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status").length);
      await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
      await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status").length > before, statusCount);
      await expect(pool.getByLabel("Agent command")).toHaveValue("codex");
      await pool.getByRole("button", { name: "Add candidate", exact: true }).press("Enter");
      await expect(rows).toHaveCount(2);
      await expect(pool.locator("summary")).toContainText("Auto (2)");
      await pool.getByRole("button", { name: "Move up codex", exact: true }).click();
      await expect(rows.first()).toHaveAttribute("data-agent-id", "codex");
      await pool.getByRole("button", { name: "Move down codex", exact: true }).click();
      await expect(rows.first()).toHaveAttribute("data-agent-id", "claude");
      await pool.getByLabel("Usage threshold (%)", { exact: true }).fill("65");
      await pool.getByLabel("Usage threshold (%)", { exact: true }).press("Tab");
      await pool.getByLabel("Prefer for codex", { exact: true }).fill("type:fix, label:bug");
      await pool.getByLabel("Prefer for codex", { exact: true }).press("Tab");
      // Wait for the persisted status, not a sleep or merely the input's value.
      await expect.poll(() => page.evaluate(() => {
        const messages = (window as any).__gwtPlaywrightMessages ?? [];
        return messages.some(({ payload }: any) => payload.kind === "issue_monitor_status"
          && payload.status?.usage_threshold_percent === 65
          && payload.status?.launch_profile_candidates?.some((candidate: any) =>
            candidate.agent_id === "codex" && candidate.prefer_for?.includes("label:bug")));
      })).toBe(true);
      // Keep the real project route and re-install the standard test bridge.
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      const reloadedPool = surface.locator(".knowledge-monitor-pool");
      if (!(await reloadedPool.evaluate((node) => (node as HTMLDetailsElement).open))) {
        await reloadedPool.locator("summary").click();
      }
      await expect(reloadedPool.getByLabel("Usage threshold (%)", { exact: true })).toHaveValue("65");
      await expect(reloadedPool.getByLabel("Prefer for codex", { exact: true })).toHaveValue("type:fix, label:bug");
      await expect(reloadedPool.locator('[data-agent-id="claude"]')).toContainText("sonnet");
      // Held is a status projection; inject just that projection without changing
      // real quota state or waiting for a provider to exhaust its quota.
      await page.evaluate(() => {
        const socket = (window as any).__gwtPlaywrightSockets.find((candidate: WebSocket) => candidate.url.includes("repo_hash="));
        socket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({
        kind: "issue_monitor_status",
        status: { usage_threshold_percent: 65, launch_profile_candidates: [
          { index: 0, agent_id: "claude", summary: "claude / sonnet", prefer_for: ["kind:spec"], held_until: "2099-01-01T00:00:00Z" },
          { index: 1, agent_id: "codex", summary: "codex", prefer_for: ["type:fix", "label:bug"] },
        ] },
      }) }));
      });
      await expect(reloadedPool.getByText("Held", { exact: true })).toBeVisible();
      await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
      await testInfo.attach(`candidate-pool-${testInfo.project.name}`, { body: await reloadedPool.screenshot(), contentType: "image/png" });
      await reloadedPool.getByRole("button", { name: "Remove codex", exact: true }).click();
      await expect(reloadedPool.locator(".knowledge-monitor-candidate")).toHaveCount(1);
      await expect(reloadedPool.getByRole("button", { name: "Remove claude", exact: true })).toBeDisabled();
      await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      expect(errors).toEqual([]);
    });
  });
});
