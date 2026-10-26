import { readFile, realpath, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, relative, sep } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

test.afterEach(async ({ page }, info) => {
  if (info.status === info.expectedStatus || page.isClosed()) return;
  await info.attach("work-wire-failure", { contentType: "application/json", body: JSON.stringify(
    await page.evaluate(() => ({
      url: location.href,
      frames: ((window as any).__gwtPlaywrightMessages ?? [])
        .filter((frame: any) => frame.payload.kind.startsWith("active_work_projection"))
        .map((frame: any) => ({ sequence: frame.sequence, kind: frame.payload.kind,
          projection_id: frame.payload.projection?.id,
          agents: frame.payload.projection?.agents,
          fixture_works: frame.payload.projection?.active_works?.filter((work: any) =>
            work.agents?.some((agent: any) => agent.session_id === "issue-5404-shell")),
        })),
    }))),
  });
  await info.attach("work-ui-failure", { body: await page.screenshot(), contentType: "image/png" });
});

async function projectionAfter(page: Page, after: number, purpose: string) {
  // Current watcher changes use a bounded patch; ready hydration can replay
  // the full projection. Both carry authoritative current membership.
  return page.waitForFunction(({ after, purpose }) => {
    const frames = (window as any).__gwtPlaywrightMessages ?? [];
    return frames.find((frame: any) => frame.sequence > after
      && ["active_work_projection", "active_work_projection_patch"].includes(frame.payload.kind)
      && frame.payload.projection?.agents?.some((agent: any) =>
        agent.agent_id === "shell" && agent.title_summary === purpose))?.payload.projection;
  }, { after, purpose }, { timeout: 20_000 }).then(handle => handle.jsonValue()).catch(async error => {
    const frames = await page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .filter((frame: any) => frame.payload.kind.includes("work") || frame.payload.kind.includes("error"))
      .slice(-6).map((frame: any) => ({
        sequence: frame.sequence, kind: frame.payload.kind,
        projection_id: frame.payload.projection?.id,
        agents: frame.payload.projection?.agents,
        work_count: frame.payload.projection?.active_works?.length,
        error: frame.payload.error,
      })));
    throw new Error(`${String(error)}\nProject: ${page.url()}\nWork frames: ${JSON.stringify(frames)}`);
  });
}

test("Issue #5404 canonical Work remains responsive across ready replays and refreshes changed Current", async ({ page }, info) => {
  test.setTimeout(120_000);
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "requires scripts/verify-issue-5404.ps1");
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL!;
  const checkHome = await realpath(process.env.GWT_PLAYWRIGHT_CHECK_HOME!);
  expect(checkHome).not.toBe(await realpath(homedir()));
  const marker = JSON.parse(await readFile(join(checkHome, "issue-5404-isolated.json"), "utf8"));
  expect(await realpath(marker.project)).toBe(await realpath(process.env.GWT_PLAYWRIGHT_PROJECT_ROOT!));
  expect(new URL(base).origin).toBe(new URL(marker.url).origin);
  expect(marker.repo_hash).toMatch(/^[a-zA-Z0-9_-]+$/);
  const stateDir = join(checkHome, ".gwt/projects", marker.repo_hash, "project-state");
  const currentPath = join(stateDir, "current.json");
  // Mutations must stay in the fresh HOME, even if an input is a junction.
  for (const path of [stateDir, currentPath]) {
    const inside = relative(checkHome, await realpath(path));
    expect(inside !== "" && inside !== ".." && !inside.startsWith(`..${sep}`) && !isAbsolute(inside)).toBe(true);
  }
  expect(JSON.parse(await readFile(join(stateDir, "pm.json"), "utf8")).settings.auto_start).toBe(false);
  expect(JSON.parse(await readFile(join(stateDir, "issue-monitor.json"), "utf8")).enabled).toBe(false);

  await withLiveGwtBackendLock(base, info, async () => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    const initial = `Current projection ${theme}`;
    const changed = `Changed Current projection ${theme}`;
    const writeCurrent = async (purpose: string) => {
      const current = JSON.parse(await readFile(currentPath, "utf8"));
      const shell = current.agents.find((agent: any) => agent.session_id === "issue-5404-shell");
      expect(shell?.agent_id).toBe("shell");
      shell.title_summary = purpose;
      shell.current_focus = `${purpose} focus`;
      shell.updated_at = new Date().toISOString();
      current.updated_at = shell.updated_at;
      // Publish whole JSON bytes atomically so the watcher never sees a
      // truncated Current. No Session/ledger records are synthesized.
      const temporary = `${currentPath}.issue-5404-${theme}`;
      await writeFile(temporary, JSON.stringify(current));
      await rename(temporary, currentPath);
    };

    await writeCurrent(initial);
    await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
    await gotoLiveGwt(page, base, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await sendLiveGwtEvent(page, { kind: "retry_workspace_state_load" });
    const first = await projectionAfter(page, 0, initial);
    const work = first.active_works.find((work: any) => work.agents?.some((agent: any) =>
      agent.session_id === "issue-5404-shell"));
    expect(work?.id, "Current shell summary must create a canonical Work row").toBeTruthy();
    expect(work.work_summary).toBe(initial);

    await sendLiveGwtEvent(page, { kind: "create_window", preset: "issue",
      bounds: { x: 24, y: 24, width: 1200, height: 850 } });
    const surface = page.locator('.workspace-window[data-preset="issue"]').last();
    await expect(surface).toBeVisible();
    const id = (await surface.getAttribute("data-id"))!;
    try {
      await surface.locator(".issue-other-summary").click();
      const row = surface.locator(`.workspace-overview-row[data-workspace-id="${work.id}"]`);
      await expect(row).toContainText(initial);
      await row.click();
      await expect(row).toHaveAttribute("aria-selected", "true");

      const readyTimes: number[] = [];
      for (let replay = 0; replay < 5; replay += 1) {
        const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
        const started = performance.now();
        await sendLiveGwtEvent(page, { kind: "frontend_ready" });
        // Toggle real disclosure state while replaying, so liveness cannot
        // pass merely because the selected row was already selected.
        await surface.locator(".issue-other-summary").click();
        await expect(surface.locator(".issue-other-group")).not.toHaveAttribute("open", "");
        await surface.locator(".issue-other-summary").click();
        await expect(surface.locator(".issue-other-group")).toHaveAttribute("open", "");
        const replayed = await projectionAfter(page, cursor, initial);
        expect(replayed.active_works.some((item: any) => item.id === work.id)).toBe(true);
        await expect(row).toHaveAttribute("aria-selected", "true");
        readyTimes.push(performance.now() - started);
      }

      const cursor = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
      await writeCurrent(changed);
      await sendLiveGwtEvent(page, { kind: "retry_workspace_state_load" });
      const refreshed = await projectionAfter(page, cursor, changed);
      const refreshedWork = refreshed.active_works.find((item: any) => item.id === work.id);
      expect(refreshedWork.work_summary).toBe(changed);
      expect(refreshedWork.agents.find((agent: any) => agent.session_id === "issue-5404-shell").current_focus)
        .toBe(`${changed} focus`);
      await expect(row).toContainText(changed);
      await expect(row).toHaveAttribute("aria-selected", "true");
      await expect(page.locator(".workspace-state-notice")).toHaveCount(0);
      await info.attach(`work-projection-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
      await info.attach(`work-projection-replays-${theme}`, {
        body: JSON.stringify({ work_id: work.id, ready_roundtrip_ms: readyTimes, changed_focus: `${changed} focus` }),
        contentType: "application/json",
      });
    } finally {
      await sendLiveGwtEvent(page, { kind: "close_window", id });
    }
    expect(errors, "zero console/page errors").toEqual([]);
  });
});
