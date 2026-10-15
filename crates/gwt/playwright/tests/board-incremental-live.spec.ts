import { spawn } from "node:child_process";
import { readFile, realpath } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

async function boardFrames(page: Page, id: string, after = 0) {
  return page.evaluate(({ id, after }) => ((window as any).__gwtPlaywrightMessages ?? [])
    .filter((row: any) => row.sequence > after && row.payload.kind === "board_entries" && row.payload.id === id),
  { id, after });
}

test("Issue #5402 live Board sends one new entry, retains existing entries, and renders Markdown without duplicate notifications", async ({ page }, info) => {
  test.setTimeout(120_000);
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL, "requires scripts/verify-issue-5402.ps1");
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL!;
  const checkHome = process.env.GWT_PLAYWRIGHT_CHECK_HOME!;
  expect(await realpath(checkHome)).not.toBe(await realpath(homedir()));
  const marker = JSON.parse(await readFile(join(checkHome, "issue-5402-isolated.json"), "utf8"));
  expect(new URL(base).origin).toBe(new URL(marker.url).origin);
  const state = join(checkHome, ".gwt/projects", marker.repo_hash, "project-state");
  expect(JSON.parse(await readFile(join(state, "pm.json"), "utf8")).settings.auto_start).toBe(false);
  expect(JSON.parse(await readFile(join(state, "issue-monitor.json"), "utf8")).enabled).toBe(false);

  const post = (body: string, mentions: string[] = []) => new Promise<void>((resolve, reject) => {
    const env = { ...process.env, HOME: checkHome, USERPROFILE: checkHome };
    for (const name of Object.keys(env)) if (/^(GWT_|CODEX_|CLAUDE_)/.test(name)) delete env[name];
    const child = spawn(join(marker.project, "target/debug", process.platform === "win32" ? "gwtd.exe" : "gwtd"), [], {
      cwd: marker.project, env, windowsHide: true,
    });
    let stdout = "", stderr = "";
    child.stdout.on("data", chunk => { stdout += chunk; });
    child.stderr.on("data", chunk => { stderr += chunk; });
    child.once("error", reject);
    child.once("close", code => {
      if (code) return reject(new Error(stderr || stdout));
      try {
        const envelope = JSON.parse(stdout);
        expect(envelope.ok, stdout).toBe(true);
        resolve();
      } catch (error) { reject(error); }
    });
    child.stdin.end(JSON.stringify({ schema_version: 1, operation: "board.post",
      params: { kind: "status", body, broadcast: true, mentions } }));
  });

  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const token = `issue-5402-${theme}-${Date.now()}`;
  await post(`${token} retained entry`);
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  await gotoLiveGwt(page, base, { enableTestBridge: true });
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await sendLiveGwtEvent(page, { kind: "create_window", preset: "board",
    bounds: { x: 30, y: 30, width: 1200, height: 900 } });
  const board = page.locator('.workspace-window[data-preset="board"]').last();
  await expect(board).toBeVisible();
  const id = (await board.getAttribute("data-id"))!;
  const cards = board.locator("[data-board-entry-id]");
  await expect(cards.filter({ hasText: `${token} retained entry` })).toHaveCount(1);
  await expect(board.locator(".board-status")).not.toContainText("Loading");
  // The first watcher notification may initialize its scoped projection with
  // a full snapshot. Measure the following steady-state notification.
  await post(`${token} warmup entry`);
  await expect(cards.filter({ hasText: `${token} warmup entry` })).toHaveCount(1);
  const initialIds = await cards.evaluateAll(elements => elements.map(element => element.getAttribute("data-board-entry-id")));
  const initialMentions = Number((await board.locator(".board-for-you-filter").innerText()).match(/\((\d+)\)/)?.[1] ?? 0);
  const expectedMentionLabel = `For you (${initialMentions + 1})`;

  const markdown = `${token} **incremental Markdown** and \`inline code\``;
  await post(markdown, ["user:you"]);
  const newCard = cards.filter({ hasText: token }).filter({ hasText: "incremental Markdown" });
  await expect(newCard).toHaveCount(1);
  await expect(newCard.locator(".board-message-body strong")).toHaveText("incremental Markdown");
  await expect(newCard.locator(".board-message-body code")).toHaveText("inline code");
  const newId = await newCard.getAttribute("data-board-entry-id");
  await expect.poll(async () => (await boardFrames(page, id))
    .filter((frame: any) => frame.payload.entries.some((entry: any) => entry.id === newId)).length).toBeGreaterThan(0);
  const deltaFrames = (await boardFrames(page, id))
    .filter((frame: any) => frame.payload.entries.some((entry: any) => entry.id === newId));
  for (const frame of deltaFrames) expect(frame.payload.entries.map((entry: any) => entry.id)).toEqual([newId]);
  await expect(cards).toHaveCount(initialIds.length + 1);
  expect(await cards.evaluateAll(elements => elements.map(element => element.getAttribute("data-board-entry-id"))))
    .toEqual([...initialIds, newId]);
  await expect(board.locator(".board-for-you-filter")).toHaveText(expectedMentionLabel);

  // A full refresh may repeat the already-delivered entry; it must not append
  // a second card or increment the mention count again.
  const beforeRefresh = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence);
  await board.getByRole("button", { name: "Refresh board", exact: true }).click();
  await expect.poll(async () => (await boardFrames(page, id, beforeRefresh)).length).toBeGreaterThan(0);
  await expect(cards).toHaveCount(initialIds.length + 1);
  await expect(newCard).toHaveCount(1);
  await expect(board.locator(".board-for-you-filter")).toHaveText(expectedMentionLabel);
  await info.attach(`board-incremental-${theme}`, { body: await page.screenshot(), contentType: "image/png" });
  await info.attach(`board-delta-${theme}`, { body: JSON.stringify(deltaFrames), contentType: "application/json" });
  await sendLiveGwtEvent(page, { kind: "close_window", id });
  expect(errors, "zero console/page errors").toEqual([]);
});
