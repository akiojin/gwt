/* Issue #4911 AC-9 — `＋` / `−` / reorder / save of the Issue Monitor Agent
 * Settings sets against a real backend, in both themes. The save only writes
 * the candidate pool; it never starts an agent, enables the Monitor, or
 * creates a branch.
 */
import { readFile, realpath, rm, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, relative } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import {
  gotoLiveGwt,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const UPDATE_FIXTURE = process.env.GWT_E2E_AGENT_UPDATE_FIXTURE_DIR ?? "";

test.describe("Issue Monitor Agent Settings sets", () => {
  test.skip(!BASE, "requires an isolated checkout gwt instance");
  // Windows status publication can wait for the disabled Monitor's 60s scan.
  test.setTimeout(180_000);
  test.use({ viewport: { width: 1440, height: 1000 } });

  test("adds, reorders, removes and saves sets in launch order", async ({ page }, testInfo) => {
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

      // Start from one saved candidate, written the way the JSON operation
      // writes it, and wait until the backend reports it.
      await sendLiveGwtEvent(page, {
        kind: "issue_monitor_profiles_set",
        profiles: [{ agent_id: "codex" }],
        request_id: 491101,
      });
      const seeded = await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .find(({ payload }: any) => ["issue_monitor_profiles", "issue_monitor_profiles_write_failed"].includes(payload.kind)
          && payload.request_id === 491101)?.payload).then(handle => handle.jsonValue());
      expect(seeded.kind, seeded.message).toBe("issue_monitor_profiles");
      expect(seeded.profiles.length).toBeGreaterThan(0);
      expect(seeded.profiles.every((profile: any) => profile.agent_id === "codex")).toBe(true);

      // A fresh browser-check project can have only one enabled member.
      // Produce the migrated roster, then register two more people through
      // the UI without expanding the already-bound one-candidate pool.
      const sequence = () => page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      const freshRoster = async (cursor: number, revision = -1) => page.waitForFunction(({ cursor, revision }) =>
        ((window as any).__gwtPlaywrightMessages ?? []).find(({ sequence, payload }: any) =>
          sequence > cursor && payload.kind === "agent_roster" && payload.roster.revision > revision)?.payload.roster,
      { cursor, revision }).then(handle => handle.jsonValue());
      const rosterCursor = await sequence();
      await page.locator('.op-rail [data-surface="agents"]').click();
      await sendLiveGwtEvent(page, { kind: "agent_roster", action: { kind: "read" }, expected_revision: null });
      let roster = await freshRoster(rosterCursor);
      const registeredIds = new Set(roster.members.map((member: any) => member.agent_member_id));
      expect(registeredIds.size).toBe(roster.members.length);
      expect(roster.members.every((member: any) => member.agent_member_id && member.agent_id)).toBe(true);
      // Legacy import can expand existing people on a shared theme server.
      // Bind one explicit owner before adding people, keeping this form at1.
      const owner = roster.members.find((member: any) => member.enabled && member.agent_id === "codex");
      expect(owner).toBeTruthy();
      await sendLiveGwtEvent(page, { kind: "issue_monitor_profiles_set", request_id: 491102,
        profiles: [{ agent_member_id: owner.agent_member_id, agent_id: "codex" }] });
      const bound = await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
        .find(({ payload }: any) => ["issue_monitor_profiles", "issue_monitor_profiles_write_failed"].includes(payload.kind)
          && payload.request_id === 491102)?.payload).then(handle => handle.jsonValue());
      expect(bound.kind, bound.message).toBe("issue_monitor_profiles");
      expect(bound.profiles.map((profile: any) => profile.agent_member_id)).toEqual([owner.agent_member_id]);
      await expectSavedAgents(page, ["codex"]);
      const initialCapacity = roster.capacity;
      const newMembers: string[] = [];
      for (let index = 0; index < 2; index += 1) {
        await page.getByLabel("Tool for new member", { exact: true }).selectOption("codex");
        const cursor = await sequence();
        await page.getByLabel("Name for new member", { exact: true }).fill(`Settings member ${index}`);
        await page.locator('.roster-capacity [data-action="add"]').click();
        const next = await freshRoster(cursor, roster.revision);
        const addedMembers = next.members.filter((member: any) => !registeredIds.has(member.agent_member_id));
        expect(addedMembers).toHaveLength(1);
        const member = addedMembers[0];
        expect(member.agent_member_id).toBeTruthy();
        expect(member.agent_id).toBe("codex");
        expect(member.enabled).toBe(true);
        expect(next.capacity).toBe(roster.capacity + 1);
        registeredIds.add(member.agent_member_id);
        newMembers.push(member.agent_member_id);
        roster = next;
      }
      expect(roster.capacity).toBe(initialCapacity + 2);
      expect(new Set(newMembers).size).toBe(2);
      const profilesCursor = await sequence();
      await sendLiveGwtEvent(page, { kind: "get_issue_monitor_profiles" });
      const singlePool = await page.waitForFunction(cursor => ((window as any).__gwtPlaywrightMessages ?? [])
        .find(({ sequence, payload }: any) => sequence > cursor && payload.kind === "issue_monitor_profiles")?.payload.profiles,
      profilesCursor).then(handle => handle.jsonValue());
      expect(singlePool).toHaveLength(1);
      expect(singlePool[0].agent_member_id).toBeTruthy();
      expect(singlePool[0].agent_id).toBe("codex");
      expect(registeredIds.has(singlePool[0].agent_member_id)).toBe(true);
      expect(newMembers).not.toContain(singlePool[0].agent_member_id);

      await sendLiveGwtEvent(page, { kind: "issue_monitor_configure_profile" });
      const modal = page.locator("#wizard-modal");
      // The open is queued behind the status reads above on a busy backend.
      await expect(modal).toHaveClass(/open/, { timeout: 30_000 });
      const sets = modal.locator(".launch-agent-set");
      await expect(sets).toHaveCount(1);
      await expect(sets.first()).toHaveAttribute("data-agent-id", "codex");
      const firstMember = await sets.first().getAttribute("data-agent-member-id");
      expect(firstMember).toBeTruthy();
      expect(firstMember).toBe(singlePool[0].agent_member_id);
      await expect(modal.getByLabel("Agent", { exact: true })).toHaveValue(firstMember!);
      // AC-6: the last set stays, and the form says why.
      await expect(
        modal.getByRole("button", { name: "Remove Agent Settings 1", exact: true }),
      ).toBeDisabled();
      await expect(modal.locator(".launch-agent-sets__footer")).toContainText(
        "At least one Agent Settings set is required",
      );

      // AC-1: `＋` adds a set, open in the same form.
      await modal.getByRole("button", { name: "Add Agent Settings" }).click();
      await expect(sets).toHaveCount(2);
      await expect(sets.nth(1)).toHaveClass(/is-open/);
      await expect(sets.nth(1).locator(".launch-section").first()).toContainText("Launch");
      const added = await sets.nth(1).getAttribute("data-agent-member-id");
      const addedTool = await sets.nth(1).getAttribute("data-agent-id");
      expect(added).toBeTruthy();
      expect(added).not.toBe(firstMember);
      expect(addedTool).toBe("codex");
      await expect(modal.getByLabel("Agent", { exact: true })).toHaveValue(added!);
      await expect(sets.nth(0)).toContainText("Codex");

      // AC-3: the order of the sets is the launch order.
      await modal.getByRole("button", { name: "Move Agent Settings 2 up", exact: true }).click();
      await expect(sets.nth(0)).toHaveAttribute("data-agent-member-id", added!);
      await expect(sets.nth(0)).toHaveClass(/is-open/);
      await expect(sets.nth(1)).toHaveAttribute("data-agent-member-id", firstMember!);

      // AC-2: `−` removes a set.
      await modal.getByRole("button", { name: "Add Agent Settings" }).click();
      await expect(sets).toHaveCount(3);
      await modal.getByRole("button", { name: "Remove Agent Settings 3", exact: true }).click();
      await expect(sets).toHaveCount(2);
      await expect(sets.nth(1)).toHaveClass(/is-open/);
      await expect(sets.nth(1)).toHaveAttribute("data-agent-member-id", firstMember!);
      await modal.getByRole("button", { name: "Edit Agent Settings 1", exact: true }).click();
      await expect(sets.nth(0)).toHaveClass(/is-open/);
      await expect(modal.locator(".launch-agent-sets__order")).toContainText("auto (2):");

      await expect(page.locator("html")).toHaveAttribute(
        "data-theme",
        testInfo.project.name.endsWith("light") ? "light" : "dark",
      );
      await testInfo.attach(`agent-settings-sets-live-${testInfo.project.name}`, {
        body: await modal.screenshot({ path: testInfo.outputPath("agent-settings-members.png") }),
        contentType: "image/png",
      });

      // Settings → Runtime → Confirm → Save.
      const submit = page.locator("#wizard-submit-button");
      await submit.click();
      await expect(modal.locator(".launch-section", { hasText: "Runtime" })).toBeVisible();
      await expect(submit).toBeEnabled();
      await submit.click();
      await expect(submit).toHaveText("Save settings");
      await expect(modal.locator(".launch-agent-sets__order")).toContainText("auto (2):");
      await submit.click();
      // Windows publishes the saved status after its Issue Scan completes.
      await expect(modal).not.toHaveClass(/open/, { timeout: 90_000 });

      // AC-5: the saved pool reads back in the same order through the status
      // the JSON operation reports, and the reopened form agrees with it.
      await expectSavedAgents(page, [addedTool!, "codex"]);
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      const readbackCursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      await sendLiveGwtEvent(page, { kind: "get_issue_monitor_profiles" });
      const profiles = await page.waitForFunction(cursor => ((window as any).__gwtPlaywrightMessages ?? [])
        .find(({ sequence, payload }: any) => sequence > cursor && payload.kind === "issue_monitor_profiles")?.payload.profiles, readbackCursor)
        .then(handle => handle.jsonValue());
      expect(profiles.map((profile: any) => profile.agent_member_id)).toEqual([added, firstMember]);
      await sendLiveGwtEvent(page, { kind: "issue_monitor_configure_profile" });
      await expect(modal).toHaveClass(/open/, { timeout: 30_000 });
      await expect(sets).toHaveCount(2);
      await expect(sets.nth(0)).toHaveAttribute("data-agent-member-id", added!);
      await expect(sets.nth(1)).toHaveAttribute("data-agent-member-id", firstMember!);
      await page.locator("#wizard-cancel-button").click();
      await expect(modal).not.toHaveClass(/open/);
      expect(errors).toEqual([]);
    });
  });

  for (const [agentId, displayName, model] of [
    ["codex", "Codex", "gpt-6-astra"],
    ["claude", "Claude Code", "sonnet"],
  ]) {
    test(`updates ${displayName} inside the saved Agent Settings form`, async ({ page }, testInfo) => {
      // An ordinary live server may resolve the real installer. This case
      // requires the dedicated fake CLI fixture and verifies its server origin.
      test.skip(!UPDATE_FIXTURE, "requires the isolated fake agent update fixture");
      await withLiveGwtBackendLock(BASE, testInfo, async () => {
        const directory = await realpath(UPDATE_FIXTURE);
        const marker = JSON.parse(await readFile(join(directory, "fixture.json"), "utf8"));
        const home = await realpath(marker.home);
        expect(home).not.toBe(await realpath(homedir()));
        const fixtureRelative = relative(home, directory);
        expect(isAbsolute(fixtureRelative) || fixtureRelative.split(/[\\/]/).includes("..")).toBe(false);
        expect(new URL(marker.url).origin).toBe(new URL(BASE).origin);
        expect(marker.repo_hash).toMatch(/^[a-zA-Z0-9_-]+$/);
        const preferences = join(home, ".gwt", "projects", marker.repo_hash, "project-state", "issue-monitor.json");
        const versionFile = join(directory, `${agentId}-version.txt`);
        const startedFile = join(directory, `${agentId}-update-started.json`);
        const releaseFile = join(directory, `${agentId}-update-release`);
        const completedFile = join(directory, `${agentId}-update-complete.json`);
        const previousVersion = (await readFile(versionFile, "utf8")).trim();
        expect(previousVersion).toMatch(/^\d+\.\d+\.\d+$/);
        const versionParts = previousVersion.split(".");
        const nextVersion = `${versionParts[0]}.${versionParts[1]}.${Number(versionParts[2]) + 1}`;
        await Promise.all([startedFile, releaseFile, completedFile].map(file => rm(file, { force: true })));

        const errors: string[] = [];
        page.on("pageerror", error => errors.push(error.message));
        page.on("console", message => {
          if (message.type() === "error") errors.push(message.text());
        });
        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
        await expect(page.locator("#close-project-button")).toBeVisible();
        const otherAgent = agentId === "codex" ? "claude" : "codex";
        await sendLiveGwtEvent(page, {
          kind: "issue_monitor_profiles_set",
          profiles: [{ agent_id: agentId, model, reasoning: "high" }, { agent_id: otherAgent }],
        });
        await expectSavedAgents(page, [agentId, otherAgent]);
        await sendLiveGwtEvent(page, { kind: "issue_monitor_configure_profile" });
        const modal = page.locator("#wizard-modal");
        await expect(modal).toHaveClass(/open/, { timeout: 30_000 });
        await expect(modal.locator(".launch-agent-set").first()).toHaveAttribute("data-agent-id", agentId);
        const setup = modal.locator(`.launch-agent-setup[data-agent-id="${agentId}"]`);
        const update = setup.getByRole("button", { name: `Update ${displayName}`, exact: true });
        await expect(update).toBeEnabled();
        const before = await liveWizardSnapshot(page);
        await expect(modal.locator("#wizard-error")).not.toBeVisible();
        expect(before).toMatchObject({
          error: null,
          selected_launch_target: "agent",
          selected_agent_id: agentId,
          selected_model: model,
          selected_reasoning: "high",
        });
        const savedPreferences = JSON.parse(await readFile(preferences, "utf8"));
        const cursor = await page.evaluate(() => Number((window as any).__gwtPlaywrightMessageSequence));

        try {
          await update.click();
          await expect.poll(() => readFile(startedFile, "utf8").catch(() => "")).not.toBe("");
          await expect(modal).toHaveClass(/open/);
          await expect(update).toBeDisabled();
          const status = setup.getByRole("status");
          await expect(status).toBeVisible();
          await expect(status).toHaveAttribute("aria-live", "polite");
          await expect(status).toContainText(/updating/i);
          await expect.poll(() => liveWizardSnapshot(page)).toMatchObject({ agent_setup: { pending: true } });
          await writeFile(releaseFile, "release\n");
          await expect.poll(() => readFile(completedFile, "utf8").catch(() => "")).not.toBe("");
          await expect(update).toBeEnabled();
          await expect(status).toContainText(nextVersion);
          await expect(modal).toHaveClass(/open/);
          await expect(modal.locator("#wizard-error")).not.toBeVisible();
          await expect.poll(() => liveWizardSnapshot(page)).toMatchObject({ agent_setup: { pending: false } });
          const snapshots = await page.evaluate(after => ((window as any).__gwtPlaywrightMessages ?? [])
            .filter((entry: any) => entry.sequence > after && entry.payload.kind === "launch_wizard_state")
            .map((entry: any) => entry.payload.wizard), cursor);
          expect(snapshots.length).toBeGreaterThan(0);
          for (const snapshot of snapshots) {
            expect(snapshot, "Update keeps the same open settings form").not.toBeNull();
            expect(snapshot).toMatchObject({
              error: null,
              selected_launch_target: before.selected_launch_target,
              selected_agent_id: before.selected_agent_id,
              selected_model: before.selected_model,
              selected_reasoning: before.selected_reasoning,
              selected_runtime_target: before.selected_runtime_target,
              phase: before.phase,
              issue_monitor_pool: before.issue_monitor_pool,
            });
          }
          // Runtime observations may change while the disabled Monitor runs.
          // The update must preserve the saved configuration, including pool order.
          const updatedPreferences = JSON.parse(await readFile(preferences, "utf8"));
          for (const key of ["launch_profile", "launch_profiles", "launch_auto", "enabled"]) {
            expect(updatedPreferences[key], `Update preserves ${key}`).toEqual(savedPreferences[key]);
          }
          await expect(page.locator("html")).toHaveAttribute("data-theme", testInfo.project.name.endsWith("light") ? "light" : "dark");
          await testInfo.attach(`agent-update-${agentId}-${testInfo.project.name}`, {
            body: await modal.screenshot(), contentType: "image/png",
          });
        } finally {
          // Release the fake CLI even when a progress assertion fails.
          await writeFile(releaseFile, "release\n");
          const cancel = page.locator("#wizard-cancel-button");
          if (await cancel.isVisible()) await cancel.click();
        }
        await expect(modal).not.toHaveClass(/open/);
        expect(errors).toEqual([]);
      });
    });
  }
});

async function liveWizardSnapshot(page: Page): Promise<any> {
  return page.evaluate(() => {
    const states = ((window as any).__gwtPlaywrightMessages ?? [])
      .filter(({ payload }: any) => payload.kind === "launch_wizard_state");
    return states[states.length - 1]?.payload.wizard;
  });
}

// Waits until the newest `issue_monitor_status` — what `issue.monitor.profiles`
// reads — lists the candidates in `expected` order. Profile edits and saves
// publish this status themselves; repeated list requests queue expensive
// Windows scans ahead of the next form action.
async function expectSavedAgents(page: any, expected: string[]): Promise<void> {
  await expect.poll(async () => {
    return page.evaluate(() => {
      const statuses = ((window as any).__gwtPlaywrightMessages ?? [])
        .filter(({ payload }: any) => payload.kind === "issue_monitor_status");
      const latest = statuses[statuses.length - 1]?.payload?.status;
      return (latest?.launch_profile_candidates ?? []).map((candidate: any) => candidate.agent_id);
    });
  }, { timeout: 90_000, intervals: [1_000, 2_000, 5_000] }).toEqual(expected);
}
