import { expect, test } from "@playwright/test";
import { appendFile, readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import {
  gotoLiveGwt,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const CUSTOM_ID = "fixture-candidate";

test.afterEach(async ({ page }) => {
  if (!BASE || page.isClosed()) return;
  // Move away from Settings before closing its fixed live view.
  await page.locator('button[data-surface="agents"]').click();
  const settings = page.locator('.workspace-window[data-preset="settings"]');
  const ids = await settings.evaluateAll(windows => windows.map(window => (window as HTMLElement).dataset.id));
  const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
  for (const id of ids) await sendLiveGwtEvent(page, { kind: "close_window", id });
  await sendLiveGwtEvent(page, { kind: "list_windows" });
  // Wait for the backend's close/readback sequence before checking its DOM projection.
  await page.waitForFunction(({ cursor, ids }) => ((window as any).__gwtPlaywrightMessages ?? [])
    .some(({ sequence, payload }: any) => sequence > cursor && payload.kind === "window_list"
      && Array.isArray(payload.windows) && !payload.windows.some((window: any) => ids.includes(window.id))), { cursor, ids });
  await expect(settings).toHaveCount(0);
});

async function seedCustomCandidate(): Promise<void> {
  const isolatedHome = process.env.GWT_PLAYWRIGHT_CHECK_HOME;
  expect(isolatedHome, "requires browser-check's isolated HOME").toBeTruthy();
  expect(await realpath(isolatedHome!)).not.toBe(await realpath(homedir()));
  const config = join(isolatedHome!, ".gwt/config.toml");
  const source = await readFile(config, "utf8").catch(error => {
    if (error.code !== "ENOENT") throw error;
    return "";
  });
  if (!source.includes(`[tools.customCodingAgents.${CUSTOM_ID}]`)) {
    await appendFile(config, `\n[tools.customCodingAgents.${CUSTOM_ID}]\nid = "${CUSTOM_ID}"\ndisplayName = "Fixture Candidate"\nagentType = "command"\ncommand = "fixture-candidate-cli"\n`);
  }
}

test.describe("Issue Monitor candidate pool", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("adds, reorders, edits and removes candidates with persistent settings", async ({ page }, testInfo) => {
    await withLiveGwtBackendLock(BASE, testInfo, async () => {
      await seedCustomCandidate();
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      page.on("console", (message) => {
        if (message.type() === "error") errors.push(message.text());
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      await expect(page.locator("#close-project-button")).toBeVisible();
      // Own the required roster even when this case runs alone with capacity1.
      const sequence = () => page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      const freshRoster = async (cursor: number, revision = -1) => page.waitForFunction(({ cursor, revision }) =>
        ((window as any).__gwtPlaywrightMessages ?? []).find(({ sequence, payload }: any) =>
          sequence > cursor && payload.kind === "agent_roster" && payload.roster.revision > revision)?.payload.roster,
      { cursor, revision }).then(handle => handle.jsonValue());
      const rosterCursor = await sequence();
      await page.locator('.op-rail [data-surface="agents"]').click();
      await sendLiveGwtEvent(page, { kind: "agent_roster", action: { kind: "read" }, expected_revision: null });
      let roster = await freshRoster(rosterCursor);
      const tool = page.getByLabel("Tool for new member", { exact: true });
      const register = async (agentId: string) => {
        await expect(tool.locator(`option[value="${agentId}"]`)).toHaveCount(1);
        await tool.selectOption(agentId);
        const beforeIds = new Set(roster.members.map((member: any) => member.agent_member_id));
        const cursor = await sequence();
        await page.getByLabel("Name for new member", { exact: true }).fill(`Candidate ${roster.revision}`);
        await page.locator('.roster-capacity [data-action="add"]').click();
        const next = await freshRoster(cursor, roster.revision);
        const added = next.members.filter((member: any) => !beforeIds.has(member.agent_member_id));
        expect(added).toHaveLength(1);
        expect(added[0].agent_member_id).toBeTruthy();
        expect(added[0].agent_id).toBe(agentId);
        expect(added[0].enabled).toBe(true);
        expect(next.capacity).toBe(roster.capacity + 1);
        roster = next;
        return added[0];
      };
      while (roster.members.filter((member: any) => member.enabled && member.agent_id === "codex").length < 2) await register("codex");
      const claudeMember = roster.members.find((member: any) => member.enabled && member.agent_id === "claude") ?? await register("claude");
      // Custom registration remains an actual UI action. Bind the saved pool
      // to the existing Claude member so migration cannot rest this new person.
      const customMember = await register(CUSTOM_ID);
      const customRow = page.locator(`.roster-row[data-member-id="${customMember.agent_member_id}"]`);
      await expect(customRow).toBeVisible();
      await customRow.click();
      await expect(page.locator(".roster-detail")).toContainText(CUSTOM_ID);
      await page.screenshot({ path: testInfo.outputPath("registered-member-tools.png") });
      // The live fixture has Monitor and PM disabled. This only edits profiles;
      // it never starts an agent, enables the Monitor, or creates a branch.
      await sendLiveGwtEvent(page, {
        kind: "issue_monitor_profiles_set",
        profiles: [{ agent_member_id: claudeMember.agent_member_id, agent_id: "claude", model: "sonnet", prefer_for: ["kind:spec"] }],
        usage_threshold_percent: 80,
        request_id: 532101,
      });
      const seeded = await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? []).find(({ payload }: any) =>
        ["issue_monitor_profiles", "issue_monitor_profiles_write_failed"].includes(payload.kind)
        && payload.request_id === 532101)?.payload).then(handle => handle.jsonValue());
      expect(seeded.kind, seeded.message).toBe("issue_monitor_profiles");
      expect(seeded.profiles.map((profile: any) => profile.agent_id)).toEqual(["claude"]);
      const claudeId = seeded.profiles[0].agent_member_id;
      expect(claudeId).toBe(claudeMember.agent_member_id);
      const claudeName = claudeMember.name;
      const codexMembers = roster.members.filter((member: any) => member.enabled && member.agent_id === "codex");
      expect(codexMembers.length).toBeGreaterThanOrEqual(2);
      const [codexMember, secondCodex] = codexMembers;
      const codexId = codexMember.agent_member_id, codexName = codexMember.name;

      const settings = page.locator('.workspace-window[data-preset="settings"]');
      const pool = settings.locator('[data-settings-candidates]');
      const openCandidates = async () => {
        const issueHosts = page.locator('.workspace-window[data-preset="issue"], .workspace-window[data-preset="issue_monitor"]');
        const issueIds = await issueHosts.evaluateAll(windows => windows.map(window => window.getAttribute("data-id")).sort());
        // The legacy Agents route reveals Candidates in execution settings.
        // It must focus the existing Settings window after reload.
        await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", {
          detail: { section: "agents" },
        })));
        await expect(pool).toBeVisible();
        await expect(settings.locator(".issue-bridge-root")).toHaveCount(0);
        await expect(settings).toHaveCount(1);
        expect(await issueHosts.evaluateAll(windows => windows.map(window => window.getAttribute("data-id")).sort())).toEqual(issueIds);
        await expect(settings.locator('[data-settings-tab="issues"]')).toHaveAttribute("aria-selected", "true");
      };
      const saveProfiles = async (action: () => Promise<unknown>, agentIds: string[], dirty = false, memberIds?: string[]) => {
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        await action();
        // Only this save's fresh correlated readback can authorize the next
        // edit; an old status or a background profiles read is insufficient.
        const result = await page.waitForFunction(({ cursor, agentIds }) => ((window as any).__gwtPlaywrightMessages ?? [])
          .find(({ sequence, payload }: any) => sequence > cursor && Number.isInteger(payload.request_id)
            && (payload.kind === "issue_monitor_profiles_write_failed"
              || payload.kind === "issue_monitor_profiles"
                && JSON.stringify(payload.profiles?.map((profile: any) => profile.agent_id)) === JSON.stringify(agentIds)))?.payload,
        { cursor, agentIds }).then(handle => handle.jsonValue());
        expect(result.kind, result.message).toBe("issue_monitor_profiles");
        expect(result.profiles.every((profile: any) => profile.agent_member_id)).toBe(true);
        if (memberIds) expect(result.profiles.map((profile: any) => profile.agent_member_id)).toEqual(memberIds);
        await expect(pool).toHaveAttribute("data-saving", "false");
        await expect(pool).toHaveAttribute("data-dirty", String(dirty));
      };
      await openCandidates();
      const windowId = await settings.getAttribute("data-id");
      await expect(settings).toBeVisible();
      await expect(pool.getByRole("button", { name: "Save profiles", exact: true })).toHaveCount(0);
      await expect(pool.locator('[data-profile-field="agent_id"]')).toHaveCount(0);
      const rows = pool.locator("fieldset[data-agent-member-id]");
      await expect(rows).toHaveCount(1);
      await expect(pool.getByRole("button", { name: `Remove ${claudeName}`, exact: true })).toBeDisabled();
      const candidate = pool.getByLabel("Add candidate candidate", { exact: true });
      await expect(candidate).toHaveJSProperty("tagName", "SELECT");
      try {
        await expect(candidate).toBeEnabled({ timeout: 30_000 });
      } catch (error) {
        await testInfo.attach("candidate-catalog-state", { contentType: "application/json", body: JSON.stringify(await page.evaluate(() => ({
          active: document.activeElement?.getAttribute("aria-label"),
          dirty: document.querySelector<HTMLElement>("[data-settings-candidates]")?.dataset.dirty,
          catalogs: ((window as any).__gwtPlaywrightMessages ?? [])
            .filter(({ payload }: any) => ["supported_agent_list", "custom_agent_list", "system_settings_error", "custom_agent_error"].includes(payload.kind))
            .map(({ sequence, payload }: any) => ({ sequence, kind: payload.kind, message: payload.message,
              agents: payload.agents?.map((agent: any) => ({ id: agent.id, name: agent.name || agent.display_name })) })),
        }))) });
        throw error;
      }
      await expect(candidate).toHaveValue("");
      await pool.getByRole("button", { name: "Add candidate", exact: true }).click();
      await expect(pool.getByRole("alert")).toHaveText("Select a registered member to add.");
      await expect(candidate.locator(`option[value="${claudeId}"]`)).toHaveCount(0);
      await expect(candidate.locator(`option[value="${customMember.agent_member_id}"]`)).toHaveText(`${customMember.name} (${CUSTOM_ID})`);
      await candidate.focus();
      await candidate.selectOption(codexId);
      // Keyboard focus must survive the select → button transition.
      await candidate.focus();
      await pool.getByLabel("Add candidate candidate", { exact: true }).press("Tab");
      await expect(pool.getByRole("button", { name: "Add candidate", exact: true })).toBeFocused();
      const statusCount = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status").length);
      await sendLiveGwtEvent(page, { kind: "list_issue_monitor" });
      await page.waitForFunction(before => ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status").length > before, statusCount);
      await expect(pool.getByLabel("Add candidate candidate", { exact: true })).toHaveValue(codexId);
      await saveProfiles(() => pool.getByRole("button", { name: "Add candidate", exact: true }).press("Enter"), ["claude", "codex"]);
      await expect(rows).toHaveCount(2);
      await expect(candidate.locator(`option[value="${codexId}"]`)).toHaveCount(0);
      await candidate.selectOption(secondCodex.agent_member_id);
      await saveProfiles(() => pool.getByRole("button", { name: "Add candidate", exact: true }).click(), ["claude", "codex", "codex"], false,
        [claudeId, codexId, secondCodex.agent_member_id]);
      await expect(pool.getByLabel(`Model ${secondCodex.name}`, { exact: true })).toHaveJSProperty("tagName", "SELECT");
      await expect(pool.getByLabel(`Tool ${secondCodex.name}`, { exact: true })).toHaveText("Codex");
      await saveProfiles(() => pool.getByRole("button", { name: `Remove ${secondCodex.name}`, exact: true }).click(), ["claude", "codex"], false, [claudeId, codexId]);

      await saveProfiles(() => pool.getByRole("button", { name: `Move up ${codexName}`, exact: true }).click(), ["codex", "claude"]);
      await expect(rows.first()).toHaveAttribute("data-agent-id", "codex");
      await saveProfiles(() => pool.getByRole("button", { name: `Move down ${codexName}`, exact: true }).click(), ["claude", "codex"]);
      await expect(rows.first()).toHaveAttribute("data-agent-id", "claude");
      // A rejected backend save must explain the failure in this Settings
      // panel and retain the user's draft for correction and retry.
      // Hold only the real correlated response while the next field is being
      // typed, so cursor preservation is tested without timing assumptions.
      await page.evaluate(() => {
        const held: Array<{ socket: WebSocket; data: string }> = [];
        const sockets = (window as any).__gwtPlaywrightSockets as WebSocket[];
        const hold = (event: MessageEvent) => {
          const payload = JSON.parse(event.data);
          if (payload.kind === "issue_monitor_profiles" && Number.isInteger(payload.request_id)) {
            event.stopImmediatePropagation();
            held.push({ socket: event.currentTarget as WebSocket, data: event.data });
          }
        };
        sockets.forEach(socket => socket.addEventListener("message", hold, true));
        (window as any).candidateHeldResponses = held;
        (window as any).releaseCandidateResponses = () => {
          sockets.forEach(socket => socket.removeEventListener("message", hold, true));
          held.forEach(({ socket, data }) => socket.dispatchEvent(new MessageEvent("message", { data })));
        };
      });
      await saveProfiles(async () => {
        const responseCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        await pool.getByLabel(`Model ${codexName}`, { exact: true }).selectOption("gpt-6-astra");
        await pool.getByLabel(`Model ${codexName}`, { exact: true }).press("Tab");
        await pool.getByLabel(`Prefer for ${codexName}`, { exact: true }).fill("type:");
        await pool.getByLabel(`Prefer for ${codexName}`, { exact: true }).evaluate((input: HTMLInputElement) => input.setSelectionRange(3, 3));
        await page.waitForFunction(cursor => (window as any).candidateHeldResponses.length > 0
          || ((window as any).__gwtPlaywrightMessages ?? []).some(({ sequence, payload }: any) =>
            sequence > cursor && payload.kind === "issue_monitor_profiles_write_failed"), responseCursor);
        await page.evaluate(() => (window as any).releaseCandidateResponses());
      }, ["claude", "codex"], true);
      await expect(pool.getByLabel(`Prefer for ${codexName}`, { exact: true })).toBeFocused();
      await expect(pool.getByLabel(`Prefer for ${codexName}`, { exact: true })).toHaveValue("type:");
      await expect(pool.getByLabel(`Prefer for ${codexName}`, { exact: true })).toHaveJSProperty("selectionStart", 3);
      await pool.getByLabel(`Prefer for ${codexName}`, { exact: true }).fill("invalid-preference");
      await pool.getByLabel(`Prefer for ${codexName}`, { exact: true }).press("Tab");
      await expect(pool.getByRole("alert")).toContainText("prefer_for");
      await expect(pool.getByLabel(`Prefer for ${codexName}`, { exact: true })).toHaveAttribute("aria-invalid", "true");
      await expect(pool.locator('.settings-field:has([data-profile-field="prefer_for"]) + [role="alert"]')).toBeVisible();
      await expect(pool).toHaveAttribute("data-saving", "false");
      await expect(pool.getByLabel(`Model ${codexName}`, { exact: true })).toHaveValue("gpt-6-astra");
      await expect(pool.getByLabel(`Prefer for ${codexName}`, { exact: true })).toHaveValue("invalid-preference");
      const rejectedReadCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      await sendLiveGwtEvent(page, { kind: "get_issue_monitor_profiles" });
      await page.waitForFunction(cursor => ((window as any).__gwtPlaywrightMessages ?? []).some(({ sequence, payload }: any) =>
        sequence > cursor && payload.kind === "issue_monitor_profiles"
        && payload.profiles?.some((profile: any) => profile.agent_id === "codex"
          && profile.model === "gpt-6-astra" && (profile.prefer_for || []).length === 0)), rejectedReadCursor);
      await pool.getByLabel(`Prefer for ${codexName}`, { exact: true }).fill("type:fix, label:bug");
      await saveProfiles(() => pool.getByLabel(`Prefer for ${codexName}`, { exact: true }).press("Tab"), ["claude", "codex"]);
      const fastMode = pool.getByLabel(`Fast mode ${codexName}`, { exact: true });
      await fastMode.focus();
      await saveProfiles(() => fastMode.press("Space"), ["claude", "codex"]);
      await expect(fastMode).toBeFocused();
      await saveProfiles(() => fastMode.press("Space"), ["claude", "codex"]);
      // Wait for the persisted status, not a sleep or merely the input's value.
      await expect.poll(() => page.evaluate(() => {
        const messages = (window as any).__gwtPlaywrightMessages ?? [];
        return messages.some(({ payload }: any) => payload.kind === "issue_monitor_status"
          && !Object.hasOwn(payload.status, "usage_threshold_percent")
          && payload.status?.launch_profile_candidates?.some((candidate: any) =>
            candidate.agent_id === "codex" && candidate.prefer_for?.includes("label:bug")));
      })).toBe(true);
      // Keep the real project route and re-install the standard test bridge.
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      // The empty bootstrap frame can precede the persisted window snapshot.
      await expect(settings).toHaveAttribute("data-id", windowId!);
      await openCandidates();
      const reloadedPool = pool;
      await expect(reloadedPool.getByLabel("Usage threshold (%) candidate", { exact: true })).toHaveCount(0);
      await expect(reloadedPool.getByLabel(`Prefer for ${codexName}`, { exact: true })).toHaveValue("type:fix, label:bug");
      await expect(reloadedPool.getByLabel(`Model ${claudeName}`, { exact: true })).toHaveValue("sonnet");
      await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
      await testInfo.attach(`candidate-pool-${testInfo.project.name}`, { body: await reloadedPool.screenshot({ path: testInfo.outputPath("candidate-members.png") }), contentType: "image/png" });
      await saveProfiles(() => reloadedPool.getByRole("button", { name: `Remove ${codexName}`, exact: true }).click(), ["claude"]);
      await expect(reloadedPool.locator("fieldset[data-agent-id]")).toHaveCount(1);
      await expect(reloadedPool.getByRole("button", { name: `Remove ${claudeName}`, exact: true })).toBeDisabled();
      await expect(candidate).toBeEnabled({ timeout: 30_000 });
      await candidate.focus();
      await candidate.selectOption(customMember.agent_member_id);
      await expect(candidate).toHaveValue(customMember.agent_member_id);
      await saveProfiles(() => reloadedPool.getByRole("button", { name: "Add candidate", exact: true }).click(), ["claude", CUSTOM_ID]);
      await expect(reloadedPool.locator(`fieldset[data-agent-id="${CUSTOM_ID}"]`)).toBeVisible();
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      await expect(settings).toHaveAttribute("data-id", windowId!);
      await openCandidates();
      await expect(reloadedPool.locator(`fieldset[data-agent-id="${CUSTOM_ID}"]`)).toBeVisible();
      await saveProfiles(() => reloadedPool.getByRole("button", { name: `Remove ${customMember.name}`, exact: true }).click(), ["claude"]);
      // Commit, then close immediately without awaiting the save response.
      await reloadedPool.getByLabel(`Model ${claudeName}`, { exact: true }).selectOption("opus");
      await reloadedPool.getByLabel(`Model ${claudeName}`, { exact: true }).press("Tab");
      await page.locator('button[data-surface="agents"]').click();
      await sendLiveGwtEvent(page, { kind: "close_window", id: windowId });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .some(({ payload }: any) => payload.kind === "workspace_state"));
      await openCandidates();
      await expect(pool.getByLabel(`Model ${claudeName}`, { exact: true })).toHaveValue("opus");
      expect(errors).toEqual([]);
    });
  });
});
