import { test, expect } from "@playwright/test";
import { readFile, realpath, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { gotoLiveGwt, withLiveGwtBackendLock, sendLiveGwtEvent } from "./_helpers/live-gwt";

test("Hook Center audits and regenerates the selected project's managed configuration", async ({ page }, info) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  const root = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT;
  test.skip(!base || !root || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check isolated checkout and HOME");
  await withLiveGwtBackendLock(base!, info, async () => {
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await gotoLiveGwt(page, base!, { enableTestBridge: true });
    await page.waitForFunction(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .some(({ payload }: any) => payload.kind === "workspace_state"));
    const theme = info.project.name.endsWith("light") ? "light" : "dark";
    await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
    await page.evaluate(() => document.dispatchEvent(new CustomEvent("settings:open", {
      detail: { section: "hooks" },
    })));
    const settings = page.locator('.workspace-window[data-preset="settings"]');
    const panel = settings.locator('[data-settings-panel="hooks"]');
    await expect(panel).toBeVisible();
    await expect(settings.getByRole("tab", { name: "Hook Center", exact: true }))
      .toHaveAttribute("aria-selected", "true");
    await expect(panel).toHaveAttribute("data-hook-audit-status", "loaded");
    const latest = () => page.evaluate(() => ((window as any).__gwtPlaywrightMessages ?? [])
      .findLast(({ payload }: any) => payload.kind === "hook_doctor")?.payload);
    const audit = await latest();
    expect(audit.error).toBeNull();
    expect(audit.project_root).toBe(root);
    expect(audit.report.repair).toBeNull();
    await expect(panel).toContainText(`Project: ${root}`);
    await expect(panel).toContainText(`Asset root: ${audit.report.configuration_audit.asset_root}`);
    await expect(panel.locator('[data-role="hook-health"]'))
      .toHaveAttribute("data-status", audit.report.health.status);
    await expect(panel).toContainText("Readable JSON does not imply managed or trusted content");
    await expect(panel).toContainText("Displaying a profile path does not enable profiling");
    await expect(panel).toContainText("existing sessions are not upgraded");
    const originals = new Map<string, { bytes: Buffer; mtime: number }>();
    for (const config of audit.report.configuration_audit.configs) {
      await expect(panel).toContainText(config.path);
      if (config.state === "present") originals.set(config.path, {
        bytes: await readFile(config.path), mtime: (await stat(config.path)).mtimeMs,
      });
      for (const event of config.event_binaries) {
        await expect(panel).toContainText(`${event.event} · ${event.fallback_binary ?? "Unknown"}`);
      }
    }
    if (audit.report.codex_hook_audit) {
      await expect(panel).toContainText(`Discovery mode: ${audit.report.codex_hook_audit.discovery_mode}`);
      for (const path of audit.report.codex_hook_audit.expected_hook_paths)
        await expect(panel).toContainText(path);
    }
    await panel.getByRole("button", { name: "Refresh audit", exact: true }).click();
    await expect.poll(async () => (await latest())?.request_id).toBeGreaterThan(audit.request_id);
    await expect(panel).toHaveAttribute("data-hook-audit-status", "loaded");
    for (const [path, original] of originals) {
      expect(await readFile(path)).toEqual(original.bytes);
      expect((await stat(path)).mtimeMs).toBe(original.mtime);
    }
    const beforeReconnect = await latest();
    const closed = await page.evaluate(() => {
      const connections = ((window as any).__gwtPlaywrightSockets ?? []) as WebSocket[];
      const connection = connections.findLast(socket => socket.readyState === WebSocket.OPEN
        && new URL(socket.url).searchParams.has("repo_hash"));
      if (!connection) return false;
      connection.close();
      return true;
    });
    expect(closed).toBe(true);
    await expect.poll(async () => (await latest())?.request_id).toBeGreaterThan(beforeReconnect.request_id);
    await expect(panel).toHaveAttribute("data-hook-audit-status", "loaded");
    expect((await latest()).report.repair).toBeNull();
    // Alter one worktree-local managed command, never the shared asset root.
    const localConfigPath = join(root!, ".codex", "hooks.json");
    expect(await realpath(localConfigPath)).toBe(join(await realpath(root!), ".codex", "hooks.json"));
    const localOriginal = originals.get(localConfigPath);
    expect(localOriginal, "the audited local Codex config must have saved expected bytes").toBeDefined();
    const originalText = localOriginal!.bytes.toString("utf8");
    const localConfig = JSON.parse(originalText);
    const managedStop = localConfig.hooks.Stop.flatMap((group: any) => group.hooks || [])
      .find((hook: any) => hook.type === "command" && typeof hook.command === "string" && hook.command.includes(" hook event Stop"));
    expect(managedStop, "only a recognized managed Stop command may receive drift").toBeDefined();
    const commandLiteral = JSON.stringify(managedStop.command);
    expect(originalText).toContain(commandLiteral);
    const drift = originalText.replace(commandLiteral, JSON.stringify(`${managedStop.command} `));
    expect(drift).not.toBe(originalText);
    expect(JSON.parse(drift).hooks.Stop).toBeDefined();
    try {
      await writeFile(localConfigPath, drift);
      expect(await readFile(localConfigPath, "utf8")).toBe(drift);
      const beforeRepair = await latest();
      await panel.getByRole("button", { name: "Regenerate managed hooks", exact: true }).click();
      await expect.poll(async () => (await latest())?.request_id).toBeGreaterThan(beforeRepair.request_id);
      await expect(panel).toHaveAttribute("data-hook-audit-status", "loaded");
      const repaired = await latest();
      expect(repaired.error).toBeNull();
      expect(repaired.report.repair.repaired).toBe(true);
      expect(await readFile(localConfigPath)).toEqual(localOriginal!.bytes);
      await expect(panel).toContainText("Last repair: Managed configuration regenerated");
      await expect(panel).toContainText("Regeneration does not confirm trust, launch success, or adoption by running sessions");
      await panel.getByRole("button", { name: "Refresh audit", exact: true }).click();
      await expect.poll(async () => (await latest())?.request_id).toBeGreaterThan(repaired.request_id);
      await expect(panel).toHaveAttribute("data-hook-audit-status", "loaded");
      expect((await latest()).report.repair).toBeNull();
      await expect(panel).toContainText("Latest audit: Read-only");
      await expect(panel).toContainText("Last repair: Managed configuration regenerated");
    } finally {
      await writeFile(localConfigPath, localOriginal!.bytes);
    }
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    for (const name of ["Configuration and binaries", "Codex discovery and trust", "Managed configuration repair"]) {
      const heading = panel.getByRole("heading", { name, exact: true });
      await heading.scrollIntoViewIfNeeded();
      await expect(heading).toBeInViewport();
    }
    const screenshot = info.outputPath(`hook-center-${theme}.png`);
    await page.screenshot({ path: screenshot });
    await info.attach(`hook-center-${theme}`, { path: screenshot, contentType: "image/png" });
    expect(errors).toEqual([]);
    // Release the fixed surface and close its disposable Settings window.
    await page.locator('button[data-surface="agents"]').click();
    const id = await settings.getAttribute("data-id");
    await sendLiveGwtEvent(page, { kind: "close_window", id });
    await expect(settings).toHaveCount(0);
  });
});
