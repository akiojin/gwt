import { mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { gotoLiveGwt } from "./_helpers/live-gwt";

test("Runtime hook refresh preserves Board controls and fences unrelated events", async ({ page }, info) => {
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base, "requires an isolated checkout server");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  await gotoLiveGwt(page, base!);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await page.evaluate(() => document.dispatchEvent(new CustomEvent("op:command", { detail: { id: "open-board" } })));
  const board = page.locator('.workspace-window[data-preset="board"]').last();
  await expect(board).toBeVisible();
  await expect(board.locator(".board-status")).not.toContainText("Loading");
  const id = (await board.getAttribute("data-id"))!;
  // Controlled inbound frames exercise the app receiver, store and real Board.
  // Capture load requests so backend replies cannot race the loading assertions.
  const result = await page.evaluate(id => {
    const root = document.querySelector(`.workspace-window[data-id="${id}"]`)!;
    const sent: any[] = [];
    const originalSend = WebSocket.prototype.send;
    WebSocket.prototype.send = function (data) {
      const event = typeof data === "string" ? JSON.parse(data) : null;
      if (event?.kind === "load_board") { sent.push(event); return; }
      return originalSend.call(this, data);
    };
    const inject = (detail: any) => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail }));
    const receive = () => inject({ kind: "board_entries", id,
      entries: [{ id: "hook-post", author: "Fixture", body: "Retained entry", created_at: "2026-10-20T00:00:00Z" }], has_more_before: false });
    receive();
    const card = root.querySelector('[data-board-entry-id="hook-post"]');
    const input = root.querySelector(".board-textarea") as HTMLTextAreaElement;
    input.focus(); input.value = "Unsent draft"; input.dispatchEvent(new Event("input")); input.setSelectionRange(2, 5);
    const hook = (event: any) => inject({ kind: "runtime_hook_event", event });
    try {
      hook(null); hook({ kind: "other" });
      hook({ kind: "coordination_event", project_root: "unrelated-project" });
      const ignored = sent.length;
      hook({ kind: "coordination_event" });
      const loading = root.querySelector(".board-status")!.textContent;
      hook({ kind: "coordination_event" });
      const whileLoading = sent.length;
      receive(); hook({ kind: "coordination_event" }); receive();
      return { ignored, loading, whileLoading, loads: sent.map(event => event.id),
        sameCard: !!card && card === root.querySelector('[data-board-entry-id="hook-post"]'),
        sameInput: input === root.querySelector(".board-textarea"), focused: document.activeElement === input,
        draft: input.value, selection: [input.selectionStart, input.selectionEnd] };
    } finally { WebSocket.prototype.send = originalSend; }
  }, id);
  expect(result).toEqual({ ignored: 0, loading: expect.stringContaining("Loading"), whileLoading: 1,
    loads: [id, id], sameCard: true, sameInput: true, focused: true,
    draft: "Unsent draft", selection: [2, 5] });
  await page.screenshot({ path: info.outputPath("runtime-hook-board.png") });
  expect(errors, "zero console/page errors").toEqual([]);
});

test("Issue #5403 Board retains 1,000 entries across 10,000 rendered pushes and pages history without rebuilding cards", async ({ page }, info) => {
  test.setTimeout(360_000);
  const base = process.env.GWT_PLAYWRIGHT_BASE_URL;
  test.skip(!base, "requires scripts/verify-issue-5403.ps1");
  const checkHome = process.env.GWT_PLAYWRIGHT_CHECK_HOME!;
  expect(await realpath(checkHome)).not.toBe(await realpath(homedir()));
  const marker = JSON.parse(await readFile(join(checkHome, "issue-5403-isolated.json"), "utf8"));
  expect(new URL(base!).origin).toBe(new URL(marker.url).origin);
  const statePath = join(checkHome, ".gwt/projects", marker.repo_hash, "project-state");
  expect(JSON.parse(await readFile(join(statePath, "pm.json"), "utf8")).settings.auto_start).toBe(false);
  expect(JSON.parse(await readFile(join(statePath, "issue-monitor.json"), "utf8")).enabled).toBe(false);
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  const theme = info.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  await gotoLiveGwt(page, base!);
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

  // Exercise the checkout's served production surface, mounted with its real
  // markup/CSS. Deterministic incoming events avoid 10,000 disk/remote posts.
  await page.evaluate(async () => {
    const { createBoardLogsSurface } = await import("/board-logs-surface.js");
    const root = document.createElement("section");
    root.className = "workspace-window";
    root.id = "retention-fixture";
    root.style.cssText = "position:fixed;inset:20px;z-index:1000;display:flex;flex-direction:column";
    const body = document.createElement("div");
    body.className = "window-body";
    root.append(body);
    document.body.append(root);
    let cardsBuilt = 0;
    const sent: any[] = [];
    const createNode = (tag: string, className: string, text?: string) => {
      const node = document.createElement(tag);
      node.className = className;
      if (text !== undefined) node.textContent = text;
      return node;
    };
    const surface = createBoardLogsSurface({
      send: (event: any) => sent.push(event), createNode,
      createKnowledgeMarkdownBody: (entry: any, className: string) => {
        cardsBuilt++;
        return createNode("div", className, entry.body);
      },
      windowMap: new Map([["retention", root]]),
      activeWorkspace: () => ({ windows: [] }),
      activeProjectTab: () => ({ project_root: "fixture" }),
      getActiveWorkProjection: () => ({ active_works: [] }),
      sendWindowFocus: () => {}, focusWindowLocally: () => {},
    });
    surface.mountBoardWindow({ id: "retention" }, body);
    const state = surface.ensureBoardState("retention");
    const entry = (index: number) => ({ id: `post-${index}`, author: "Fixture", author_kind: "agent",
      body: `Update ${index}`, created_at: new Date(Date.UTC(2026, 9, 15) + index * 1000).toISOString() });
    const receive = (entries: any[]) => surface.applyBoardLogsReceiveEvent({
      kind: "board_entries", id: "retention", entries, has_more_before: false,
    });
    (window as any).boardRetention = { surface, state, entry, receive, sent, root, get cardsBuilt() { return cardsBuilt; } };
  });
  const metrics = await page.evaluate(async () => {
    const f = (window as any).boardRetention;
    const times: number[] = [];
    const started = performance.now();
    let maxRetained = 0, maxCards = 0;
    for (let index = 0; index < 10_000; index++) {
      const before = performance.now();
      f.receive([f.entry(index)]);
      times.push(performance.now() - before);
      maxRetained = Math.max(maxRetained, f.state.entries.length);
      maxCards = Math.max(maxCards, f.root.querySelectorAll("[data-board-entry-id]").length);
      if (maxRetained > 1000 || maxCards > 1000) throw new Error(`unbounded at push ${index}`);
      if (index % 100 === 0) await new Promise(resolve => requestAnimationFrame(resolve));
    }
    const sorted = times.slice().sort((a, b) => a - b);
    return { maxRetained, maxCards, cardsBuilt: f.cardsBuilt, totalMs: performance.now() - started,
      p95Ms: sorted[Math.floor(sorted.length * 0.95)], maxMs: sorted.at(-1),
      first: f.state.entries[0].id, last: f.state.entries.at(-1).id };
  });
  expect(metrics.maxRetained).toBe(1000);
  expect(metrics.maxCards).toBe(1000);
  expect(metrics.cardsBuilt, "exactly one new card per push").toBe(10_000);
  expect(metrics.first).toBe("post-9000");
  expect(metrics.last).toBe("post-9999");
  expect(metrics.p95Ms, "95% of pushes render within a 100ms interaction budget").toBeLessThan(100);
  expect(metrics.totalMs, "10,000 real renders stay under 120 seconds").toBeLessThan(120_000);

  const behavior = await page.evaluate(() => {
    const f = (window as any).boardRetention;
    const card = () => f.root.querySelector('[data-board-entry-id="post-9500"]');
    const retained = card();
    const input = f.root.querySelector(".board-textarea");
    input.focus(); input.value = "Unsent draft"; input.dispatchEvent(new Event("input"));
    input.setSelectionRange(3, 7);
    const scroller = f.root.querySelector(".board-timeline-scroll");
    scroller.scrollTop = retained.offsetTop;
    const beforeTop = retained.getBoundingClientRect().top;
    f.receive([f.entry(10_000)]);
    const preserved = { identity: retained === card(), focus: document.activeElement === input,
      draft: input.value, selection: [input.selectionStart, input.selectionEnd],
      scrollShift: Math.abs(retained.getBoundingClientRect().top - beforeTop) };
    // Repeated snapshots preserve nodes; edits invalidate only affected cards.
    f.receive([f.entry(9500)]);
    const repeatIdentity = retained === card();
    f.receive([{ ...f.entry(9500), body: "Edited body" }]);
    const edit = card().textContent.includes("Edited body") && retained !== card();
    f.surface.requestOlderBoardEntries("retention");
    const cursor = f.sent.at(-1).before_entry_id;
    f.surface.applyBoardLogsReceiveEvent({ kind: "board_history_page", id: "retention",
      entries: Array.from({ length: 50 }, (_, i) => f.entry(8951 + i)), has_more_before: true });
    const history = { first: f.state.entries[0].id, count: f.state.entries.length,
      latestVisible: !!f.root.querySelector(".board-load-latest") };
    f.receive([f.entry(10_001)]);
    const historyStable = f.state.entries.at(-1).id === "post-9950";
    f.root.querySelector(".board-load-latest").click();
    const latestRequest = f.sent.at(-1).kind;
    f.receive(Array.from({ length: 50 }, (_, i) => f.entry(9952 + i)));
    const latest = { first: f.state.entries[0].id, last: f.state.entries.at(-1).id,
      buttonGone: !f.root.querySelector(".board-load-latest") };
    return { preserved, repeatIdentity, edit, cursor, history, historyStable, latestRequest, latest };
  });
  expect(behavior.preserved.identity).toBe(true);
  expect(behavior.preserved.focus).toBe(true);
  expect(behavior.preserved.draft).toBe("Unsent draft");
  expect(behavior.preserved.selection).toEqual([3, 7]);
  expect(behavior.preserved.scrollShift).toBeLessThan(2);
  expect(behavior.repeatIdentity).toBe(true);
  expect(behavior.edit).toBe(true);
  expect(behavior.cursor).toBe("post-9001");
  expect(behavior.history).toEqual({ first: "post-8951", count: 1000, latestVisible: true });
  expect(behavior.historyStable).toBe(true);
  expect(behavior.latestRequest).toBe("load_board");
  expect(behavior.latest).toEqual({ first: "post-9952", last: "post-10001", buttonGone: true });
  console.log(`Board ${theme}: ${JSON.stringify(metrics)}`);
  const artifacts = join(process.cwd(), "target", "issue-5403-headed");
  await mkdir(artifacts, { recursive: true });
  const evidence = join(artifacts, `board-retention-${theme}.json`);
  await writeFile(evidence, JSON.stringify({ metrics, behavior }, null, 2));
  await info.attach(`board-retention-${theme}.json`, { path: evidence, contentType: "application/json" });
  const screenshot = join(artifacts, `board-retention-${theme}.png`);
  await page.screenshot({ path: screenshot });
  await info.attach(`board-retention-${theme}.png`, { path: screenshot, contentType: "image/png" });
  expect(errors, "zero console/page errors").toEqual([]);
});
