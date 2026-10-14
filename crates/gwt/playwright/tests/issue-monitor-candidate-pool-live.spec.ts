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
        request_id: 532101,
      });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? []).some(({ payload }: any) =>
        payload.kind === "issue_monitor_profiles" && payload.request_id === 532101
        && payload.profiles?.length === 1 && payload.profiles[0].agent_id === "claude"));
      const beforeIds = await page.locator(".workspace-window").evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.id));
      await sendLiveGwtEvent(page, {
        kind: "create_window", preset: "issue",
        bounds: { x: 60, y: 50, width: 1040, height: 820 },
      });
      const windowId = await page.waitForFunction((before) => {
        const node = [...document.querySelectorAll<HTMLElement>('.workspace-window[data-preset="settings"]')]
          .find(node => !before.includes(node.dataset.id));
        return node?.dataset.id;
      }, beforeIds).then(handle => handle.jsonValue());
      const surface = page.locator(`.workspace-window[data-id="${windowId}"]`);
      await expect(surface).toBeVisible();
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const pool = settings.locator('[data-settings-candidates]');
      const openCandidates = async () => {
        // The legacy Agents route reveals Candidates in the same Issues options.
        // It must focus the existing Settings window after reload.
        await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", {
          detail: { section: "agents" },
        })));
        await expect(pool).toBeVisible();
        await expect(settings.locator(".settings-issues-options")).toHaveAttribute("open", "");
        await expect(settings).toHaveCount(1);
        await expect(page.locator('.workspace-window[data-preset="issue"], .workspace-window[data-preset="issue_monitor"]')).toHaveCount(0);
        await expect(settings.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
      };
      const saveProfiles = async (action: () => Promise<unknown>, agentIds: string[]) => {
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        await action();
        // Only this save's fresh correlated readback can authorize the next
        // edit; an old status or a background profiles read is insufficient.
        await page.waitForFunction(({ cursor, agentIds }) => ((window as any).__gwtPlaywrightMessages ?? [])
          .some(({ sequence, payload }: any) => sequence > cursor
            && payload.kind === "issue_monitor_profiles" && Number.isInteger(payload.request_id)
            && JSON.stringify(payload.profiles?.map((profile: any) => profile.agent_id)) === JSON.stringify(agentIds)),
        { cursor, agentIds });
        await expect(pool).toHaveAttribute("data-saving", "false");
        await expect(pool).toHaveAttribute("data-dirty", "false");
      };
      await openCandidates();
      const rows = pool.locator("fieldset[data-agent-id]");
      await expect(rows).toHaveCount(1);
      await expect(pool.getByRole("button", { name: "Remove claude", exact: true })).toBeDisabled();
      await pool.getByLabel("Add candidate candidate", { exact: true }).fill("codex");
      // Keyboard focus must survive the input → button transition.
      await pool.getByLabel("Add candidate candidate", { exact: true }).press("Tab");
      await expect(pool.getByRole("button", { name: "Add candidate", exact: true })).toBeFocused();
      const statusCount = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status").length);
      await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
      await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status").length > before, statusCount);
      await expect(pool.getByLabel("Add candidate candidate", { exact: true })).toHaveValue("codex");
      await saveProfiles(() => pool.getByRole("button", { name: "Add candidate", exact: true }).press("Enter"), ["claude", "codex"]);
      await expect(rows).toHaveCount(2);
      await saveProfiles(() => pool.getByRole("button", { name: "Move up codex", exact: true }).click(), ["codex", "claude"]);
      await expect(rows.first()).toHaveAttribute("data-agent-id", "codex");
      await saveProfiles(() => pool.getByRole("button", { name: "Move down codex", exact: true }).click(), ["claude", "codex"]);
      await expect(rows.first()).toHaveAttribute("data-agent-id", "claude");
      await pool.getByLabel("Usage threshold (%) candidate", { exact: true }).fill("65");
      await pool.getByLabel("Usage threshold (%) candidate", { exact: true }).press("Tab");
      await pool.getByLabel("Prefer for codex", { exact: true }).fill("type:fix, label:bug");
      await saveProfiles(() => pool.getByRole("button", { name: "Save profiles", exact: true }).click(), ["claude", "codex"]);
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
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      await openCandidates();
      const reloadedPool = pool;
      await expect(reloadedPool.getByLabel("Usage threshold (%) candidate", { exact: true })).toHaveValue("65");
      await expect(reloadedPool.getByLabel("Prefer for codex", { exact: true })).toHaveValue("type:fix, label:bug");
      await expect(reloadedPool.getByLabel("Model claude", { exact: true })).toHaveValue("sonnet");
      await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
      await testInfo.attach(`candidate-pool-${testInfo.project.name}`, { body: await reloadedPool.screenshot(), contentType: "image/png" });
      await saveProfiles(() => reloadedPool.getByRole("button", { name: "Remove codex", exact: true }).click(), ["claude"]);
      await expect(reloadedPool.locator("fieldset[data-agent-id]")).toHaveCount(1);
      await expect(reloadedPool.getByRole("button", { name: "Remove claude", exact: true })).toBeDisabled();
      await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      expect(errors).toEqual([]);
    });
  });
});
