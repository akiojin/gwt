import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { existsSync, readFileSync } from "node:fs";
import { createUiStateStore } from "../ui-state-store.js";

const moduleUrl = new URL("../works-surface.js", import.meta.url);
const source = existsSync(moduleUrl) ? readFileSync(moduleUrl, "utf8") : "";
const windows = [
  { id: "one", preset: "codex", agent_name: "One", dynamic_title: "First issue", session_id: "s1", agent_member_id: "m1", status: "running" },
  { id: "pm", title: "Claude Code", preset: "agent", agent_id: "claude", agent_name: "Manager", purpose_title: "Project management", is_pm: true, session_id: "sp", status: "waiting" },
  { id: "old", preset: "codex", agent_name: "Old", session_id: "old-session", status: "running" },
];
const live = windows.slice(0, 2).map((w, i) => ({ window_id: w.id, session_id: w.session_id, runtime_incarnation: i + 1, is_pm: !!w.is_pm, issue_number: w.is_pm ? null : 4777,
  current_task: w.is_pm ? "Coordinate the current project work" : null }));
function catalog(wins, version = 1) { return { version, runtimeStates: {}, appState: { active_tab_id: "p", tabs: [{ id: "p", workspace: { windows: wins } }] } }; }
async function fixture(telemetry = null, initialCatalog = catalog(windows), previewAvailable = () => true) {
  const { createWorksSurface } = await import(moduleUrl);
  const { document, window } = parseHTML("<main></main>");
  const frames = new Set();
  const options = { scheduleFrame: fn => { frames.add(fn); return () => frames.delete(fn); } };
  const windowCatalogModel = createUiStateStore({ catalog: initialCatalog }, options);
  const activityModel = createUiStateStore({ agents: live, count: 2 }, options);
  const rosterModel = createUiStateStore({ members: [{ member_id: "m1", name: "One", enabled: true, assignment: { session_id: "s1" } }] }, options);
  const runtimes = new Map(), mounts = [], previews = new Set(), previewMounts = [], selected = [], focused = [];
  const systemSettingsModel = createUiStateStore({ worksPreviewLines: 3 }, options);
  const surface = createWorksSurface({ document, windowCatalogModel, activityModel, rosterModel, systemSettingsModel,
    windowDisplayTitle: w => `${w.agent_name || w.title || "Agent"}${w.dynamic_title ? ` — ${w.dynamic_title}` : ""}`,
    mountTerminal(id, root, options) { let input = runtimes.get(id); if (!input) { input = document.createElement("textarea"); runtimes.set(id, input); } root.appendChild(input); mounts.push({ id, root, readOnly: options?.readOnly }); },
    mountPreview(id, root, options) { if (!previewAvailable()) return () => {}; const pre = document.createElement("pre"); pre.textContent = `${id} output`; root.appendChild(pre); previews.add(pre); previewMounts.push({ id, root, lines: options?.tailLines }); return () => { previews.delete(pre); pre.remove(); }; },
    onFocus: id => focused.push(id), onLayout() {}, onSelectMember: id => selected.push(id), getTelemetry: () => telemetry,
  });
  document.querySelector("main").append(surface.element); surface.sync();
  const flush = () => { for (const frame of [...frames]) { frames.delete(frame); frame(); } };
  return { surface, document, window, windowCatalogModel, activityModel, rosterModel, frames, flush, runtimes, mounts, previews, previewMounts, systemSettingsModel, selected, focused };
}

test("Works implements the approved list and native terminal API", () => {
  assert.match(source, /export function createWorksSurface/);
  assert.doesNotMatch(source, /openTab|agents-grid|localStorage|addEventListener\(["']keydown["'],\s*[^\n]*Escape/);
});

test("Works joins exact local-live sessions, places PM first and keeps unknown telemetry fields", async () => {
  const { surface, document, selected, activityModel, flush } = await fixture();
  const rows = [...surface.element.querySelectorAll(".work-row")];
  assert.deepEqual(rows.map(row => row.dataset.windowId), ["pm", "one"]);
  assert.equal(surface.contains("old"), false);
  assert.equal(surface.element.querySelector('[data-count="running"]').textContent, "1");
  assert.equal(surface.element.querySelector('[data-count="waiting"]').textContent, "1");
  assert.match(surface.element.textContent, /Today.*Input.*Cache.*Output.*Landed PRs/s);
  assert.equal(surface.element.querySelector('[data-metric="today-cost"]').textContent, "—");
  assert.equal(surface.element.querySelector('[data-metric="today-total"]')?.textContent, "—");
  assert.match(surface.element.querySelector('[data-availability="today"]')?.textContent || "", /No data/);
  assert.match(surface.element.querySelector('[data-availability="fleet"]')?.textContent || "", /No data/);
  assert.match(surface.element.querySelector(".works-source-note")?.textContent || "", /Cost.*per-session USD.*Landed PRs.*daily merge data/s);
  assert.match(rows[1].textContent, /Context remaining.*Branch.*Verification.*Last action/s);
  assert.equal(rows[0].querySelector(".work-member"), null, "PM is a role, not a fictional member link");
  assert.equal(rows[0].querySelector(".work-name").textContent, "Manager");
  assert.equal(rows[0].querySelector(".window-role-badge").textContent, "PM");
  assert.equal(rows[0].querySelector(".work-purpose").textContent, "Coordinate the current project work");
  assert.equal(rows[0].querySelector(".work-identity").textContent, "PM · claude");
  rows[1].querySelector(".work-member").click();
  assert.deepEqual(selected, ["m1"]);
  assert.equal(document.querySelectorAll(".work-runtime-cache textarea").length, 2, "list hosts fit the existing native runtimes without accepting input");
  assert.ok(document.querySelector(".work-runtime-cache").hasAttribute("inert"));
  activityModel.update(() => ({ agents: [], count: null })); flush();
  assert.equal(surface.element.querySelector(".works-empty").textContent, "Local agent activity is unavailable.");
  assert.equal(surface.element.querySelector('[data-count="running"]').textContent, "—");
  surface.dispose();
});

test("Works keeps stable member order after PM when census arrival order changes", async () => {
  const f = await fixture();
  const two = { id: "two", preset: "claude", agent_name: "Two", session_id: "s2", status: "idle" };
  f.rosterModel.update(state => ({ ...state, members: [...state.members, { member_id: "m2", name: "Two", enabled: true, assignment: { session_id: "s2" } }] }));
  f.windowCatalogModel.update(() => ({ catalog: catalog([...windows, two], 2) }));
  f.activityModel.update(() => ({ agents: [{ window_id: "two", session_id: "s2", runtime_incarnation: 3 }, ...live], count: 3 })); f.flush();
  assert.deepEqual([...f.surface.element.querySelectorAll(".work-row")].map(row => row.dataset.windowId), ["pm", "one", "two"]);
  f.surface.dispose();
});

test("Works retains live rows across partial catalogs without granting terminal access", async () => {
  const f = await fixture(null, catalog([]));
  f.activityModel.update(() => ({ agents: [live[0]], count: 1 })); f.flush();
  const rows = () => [...f.surface.element.querySelectorAll(".work-row")];
  assert.equal(rows().length, 1, "one observed launch stays visible before the first catalog");
  assert.equal(f.surface.element.querySelector(".works-empty").hidden, true);
  assert.equal(rows()[0].querySelector(".work-open").textContent, "Syncing terminal…");
  assert.equal(f.mounts.length, 0);
  assert.equal(f.previews.size, 0);
  assert.equal(f.surface.openWork(live[0]), false);
  assert.ok(rows().every(row => row.querySelector(".work-open").disabled));

  f.windowCatalogModel.update(() => ({ catalog: catalog(windows, 2) })); f.flush();
  assert.equal(f.surface.openWork(live[0]), true);
  const mountCount = f.mounts.length;
  const next = { ...live[0], session_id: "next-session", runtime_incarnation: 5 };
  const previewsBeforeGap = f.previews.size;
  const previewHost = f.document.createElement("aside"); f.document.querySelector("main").append(previewHost);
  f.activityModel.update(() => ({ agents: [next], count: 1 }));
  // Public navigation can run before the queued render has refreshed entries.
  f.surface.setPreviewHost(previewHost);
  assert.equal(f.previews.size, previewsBeforeGap, "full previews must consult current model authority");
  f.surface.showList();
  f.surface.setPreviewHost(null);
  f.surface.setPreviewHost(previewHost);
  assert.equal(f.mounts.length, mountCount, "list navigation must consult current model authority");
  assert.equal(f.previews.size, previewsBeforeGap, "split previews must consult current model authority");
  assert.ok(f.runtimes.get("one").closest("[inert]"), "the selected terminal must park in its inert cache");
  f.flush();
  assert.equal(rows().length, 1, "activity-first replacement must not disappear");
  assert.equal(f.surface.element.querySelector(".works-empty").hidden, true);
  assert.equal(rows()[0].querySelector(".work-open").textContent, "Syncing terminal…");
  assert.equal(f.surface.element.querySelector(".works-full").hidden, true);
  assert.equal(f.surface.openWork(live[0]), false);
  assert.equal(f.surface.openWork(next), false);
  assert.equal(f.mounts.length, mountCount, "no terminal mount may use a mismatched catalog");
  assert.equal(f.previews.size, 0);

  const nextWindow = { ...windows[0], session_id: next.session_id };
  const otherTab = { ...catalog([], 3), appState: { active_tab_id: "empty", tabs: [
    { id: "empty", workspace: { windows: [] } },
    { id: "background", workspace: { windows: [nextWindow] } },
  ] } };
  f.windowCatalogModel.update(() => ({ catalog: otherTab })); f.flush();
  assert.equal(f.surface.openWork(next), true, "a matching live window on another tab remains usable");
  const retainedRow = rows()[0], mountsBeforeGap = f.mounts.length;
  f.windowCatalogModel.update(() => ({ catalog: { ...otherTab, appState: { tabs: [] } } })); f.flush();
  assert.equal(rows()[0], retainedRow, "catalog resync must retain the same live row");
  assert.equal(f.surface.element.querySelector(".works-full").hidden, true);
  assert.equal(f.mounts.length, mountsBeforeGap);
  assert.equal(f.previews.size, 0);
  assert.equal(f.surface.openWork(next), false);

  f.windowCatalogModel.update(() => ({ catalog: catalog([{ ...nextWindow, session_id: "newer-session" }], 5) })); f.flush();
  assert.equal(rows().length, 1, "catalog-first replacement must not disappear");
  const newer = { ...next, session_id: "newer-session", runtime_incarnation: 6 };
  f.activityModel.update(() => ({ agents: [newer], count: 1 })); f.flush();
  assert.equal(f.surface.openWork(next), false);
  assert.equal(f.surface.openWork(newer), true);
  f.activityModel.update(() => ({ agents: [newer], count: 1, snapshot: { project_key: "this-project" } }));
  const foreign = catalog([{ ...nextWindow, session_id: newer.session_id }], 6);
  foreign.appState.tabs[0].project_key = "another-project";
  f.windowCatalogModel.update(() => ({ catalog: foreign })); f.flush();
  assert.equal(rows().length, 1);
  assert.equal(f.surface.openWork(newer), false, "another project's catalog cannot authorize a terminal");
  foreign.appState.tabs[0].project_key = "this-project";
  f.windowCatalogModel.update(() => ({ catalog: foreign })); f.flush();
  assert.equal(f.surface.openWork(newer), true);
  f.activityModel.update(() => ({ agents: [], count: 0 })); f.flush();
  assert.equal(rows().length, 0, "only live activity, never a retained catalog, owns membership");
  assert.equal(f.surface.openWork(newer), false);
  f.surface.dispose();
});

test("Works full view reuses one runtime, preserves Esc and returns to the list with presentation controls", async () => {
  const { surface, document, window, runtimes, mounts } = await fixture();
  const nativeRoot = surface.element.querySelector('.works-terminal[data-window-id="one"]');
  assert.equal(mounts.find(mount => mount.id === "one")?.readOnly, true);
  const row = surface.element.querySelector('[data-window-id="one"]');
  const enter = new window.Event("keydown", { bubbles: true, cancelable: true });
  Object.defineProperty(enter, "key", { value: "Enter" }); row.dispatchEvent(enter);
  const input = runtimes.get("one");
  assert.ok(input && surface.isVisible("one"));
  assert.ok(input.parentElement === nativeRoot);
  assert.ok(nativeRoot.parentElement === surface.element.querySelector(".works-terminal-host"));
  assert.equal(mounts.at(-1).readOnly, false);
  assert.ok(surface.element.querySelector('.works-terminal[data-window-id="pm"]').closest("[hidden]") === null,
    "unselected native caches keep a layout box for newly arriving output and visible Issue mirrors");
  assert.equal(surface.isVisible("pm"), true, "the inert cache remains eligible for fit and output parsing");
  const esc = new window.Event("keydown", { bubbles: true, cancelable: true });
  Object.defineProperty(esc, "key", { value: "Escape" }); input.dispatchEvent(esc);
  assert.equal(esc.defaultPrevented, false);
  assert.equal(surface.isVisible("one"), true);
  assert.equal(document.querySelectorAll(".works-full input, .works-full form").length, 0);
  surface.element.querySelector('[data-action="back"]').click();
  assert.equal(surface.element.querySelector(".works-full").hidden, true);
  assert.equal(surface.isVisible("one"), true, "list runtime remains eligible for fit");
  assert.ok(nativeRoot.parentElement.classList.contains("work-runtime-cache"));
  assert.equal(mounts.at(-1).readOnly, true);
  assert.equal(surface.openWork({ ...live[0], session_id: "wrong" }), false);
  assert.equal(surface.openWork(live[0]), true);
  assert.equal(runtimes.get("one"), input);
  surface.element.querySelector('[data-action="previous"]').click();
  assert.equal(surface.isVisible("pm"), true);
  surface.element.querySelector('[data-action="next"]').click();
  assert.equal(surface.isVisible("one"), true);
  assert.ok(mounts.filter(mount => mount.id === "one").every(mount => mount.root === nativeRoot), "one native root survives list/full/previous/next");
  assert.equal(surface.element.querySelector('[aria-label*="Stop"], [aria-label*="Close"]'), null);
});

test("Works batches bursts, resolves synchronous navigation and rejects ended or rebound assignments", async () => {
  const f = await fixture();
  const retainedPreview = f.surface.element.querySelector('[data-window-id="one"] pre');
  const retainedName = f.surface.element.querySelector('[data-window-id="one"] .work-member');
  const retainedRow = retainedName.closest(".work-row"), removedRows = [];
  const observer = new f.window.MutationObserver(records => { for (const record of records) removedRows.push(...record.removedNodes); });
  observer.observe(retainedRow.parentElement, { childList: true });
  for (let i = 0; i < 10; i++) f.windowCatalogModel.update(() => ({ catalog: catalog(windows.map(w => w.id === "one" ? { ...w, dynamic_title: `Purpose ${i}` } : w), i + 2) }));
  assert.equal(f.frames.size, 1);
  assert.match(f.surface.element.textContent, /First issue/);
  f.surface.openWork("one");
  assert.match(f.surface.element.textContent, /Purpose 9/);
  f.flush();
  assert.equal(f.surface.element.querySelector('[data-window-id="one"] pre'), retainedPreview);
  assert.ok(f.surface.element.querySelector('[data-window-id="one"] .work-member') === retainedName, "rendering preserves a focused member control");
  await Promise.resolve();
  assert.equal(removedRows.includes(retainedRow), false, "unchanged list order must not detach a keyboard target"); observer.disconnect();
  f.activityModel.update(() => ({ agents: [live[1]], count: 1 })); f.flush();
  assert.equal(f.surface.isVisible("one"), false);
  assert.equal(f.surface.openWork(live[0]), false);
  assert.equal(f.surface.element.querySelector(".works-overview").hidden, false);
  f.activityModel.update(() => ({ agents: [{ ...live[0], runtime_incarnation: 9 }, live[1]], count: 2 }));
  assert.equal(f.surface.openWork(live[0]), false, "incarnation mismatch must not open a replacement");
  f.surface.dispose();
  assert.equal(f.frames.size, 0);
  assert.equal(f.previews.size, 0);
});

test("Works distinguishes measured zero from unavailable tokens and thirty-minute graph gaps", async () => {
  const values = [...Array(29).fill(null), 0];
  const telemetry = { today: { input_tokens: 0, cache_tokens: 0, output_tokens: 0, cost_usd: 0, landed_pr_count: 0 }, fleet_tokens_per_minute: values, by_window: { one: { input_tokens: 0, cache_tokens: 0, output_tokens: 0, context_remaining_percent: 0, ahead_of_develop: 0, tokens_per_minute: values } } };
  telemetry.availability = { today: { state: "measured", detail: "Measured: all project session tokens are available." },
    fleet: { state: "partial", detail: "Partial: some live agent minutes are missing." } };
  const f = await fixture(telemetry);
  assert.equal(f.surface.element.querySelector('[data-metric="today-total"]')?.textContent, "0");
  assert.equal(f.surface.element.querySelector('[data-metric="today-cost"]').textContent, "$0.00");
  assert.equal(f.surface.element.querySelector('[data-availability="today"]')?.dataset.state, "measured");
  assert.equal(f.surface.element.querySelector('[data-availability="fleet"]').textContent, telemetry.availability.fleet.detail);
  assert.equal(f.surface.element.querySelector(".works-source-note").hidden, true);
  assert.equal(f.surface.element.querySelector('[data-metric="fleet-now"]').textContent, "0");
  const graph = f.surface.element.querySelector('[data-window-id="one"] svg');
  assert.equal(graph.dataset.knownCount, "1");
  assert.match(graph.getAttribute("aria-label"), /29 unavailable/);
  assert.equal(graph.querySelectorAll("circle").length, 1);
  assert.equal(f.surface.element.querySelector('[data-window-id="one"] [data-stat="context"]').textContent, "0%");
  assert.equal(f.surface.element.querySelector('[data-window-id="pm"] svg').dataset.knownCount, "0");
  telemetry.today = { ...telemetry.today, input_tokens: 12, cache_tokens: 10, output_tokens: 3 }; f.surface.sync();
  assert.equal(f.surface.element.querySelector('[data-metric="today-total"]').textContent, "15", "cached input is already included in input and must not be added twice");
  const row = f.surface.element.querySelector('[data-window-id="one"]');
  telemetry.today = { input_tokens: null, cache_tokens: null, output_tokens: null };
  telemetry.fleet_tokens_per_minute = Array(30).fill(null);
  for (const state of ["unavailable", "partial"]) {
    telemetry.availability = { today: { state, detail: `${state}: project session measurements are missing.` },
      fleet: { state, detail: `${state}: live agent measurements are missing.` } };
    f.surface.sync();
    assert.equal(f.surface.element.querySelector('[data-availability="today"]').textContent, telemetry.availability.today.detail);
    assert.equal(f.surface.element.querySelector('[data-availability="fleet"]').dataset.state, state);
    assert.equal(f.surface.element.querySelector('[data-metric="today-total"]').textContent, "—");
    assert.equal(f.surface.element.querySelector('[data-metric="fleet-now"]').textContent, "—");
  }
  telemetry.today = { input_tokens: 0, cache_tokens: 0, output_tokens: 0 };
  telemetry.fleet_tokens_per_minute = Array(30).fill(0);
  telemetry.availability = { today: { state: "measured", detail: "Measured project tokens." }, fleet: { state: "measured", detail: "Measured live agent minutes." } };
  f.surface.sync();
  assert.equal(f.surface.element.querySelector('[data-metric="today-total"]').textContent, "0");
  assert.equal(f.surface.element.querySelector('[data-metric="fleet-now"]').textContent, "0");
  assert.equal(f.surface.element.querySelector('[data-window-id="one"]'), row);
  f.surface.dispose();
});

test("Works retains native input and fixed names when runtime state and purpose change", async () => {
  const f = await fixture(); f.surface.openWork("one"); const input = f.runtimes.get("one");
  const nativeRoot = input.parentElement, removedRoots = [];
  const observer = new f.window.MutationObserver(records => { for (const record of records) removedRoots.push(...record.removedNodes); });
  observer.observe(nativeRoot.parentElement, { childList: true });
  f.windowCatalogModel.update(() => ({ catalog: { ...catalog(windows.map(w => w.id === "one" ? { ...w, dynamic_title: "New purpose" } : w), 2), runtimeStates: { one: "error" } } }));
  f.flush(); await Promise.resolve();
  assert.equal(removedRoots.includes(nativeRoot), false, "metadata rendering must not detach a focused native root");
  observer.disconnect();
  assert.equal(f.surface.element.querySelector(".works-full-name").textContent, "One");
  assert.match(f.surface.element.querySelector(".works-full-meta").textContent, /New purpose.*Error.*Input unavailable/);
  assert.equal(f.runtimes.get("one"), input);
  assert.equal(f.surface.element.querySelector('[data-count="error"]').textContent, "1");
  f.windowCatalogModel.update(() => ({ catalog: { ...catalog(windows.map(w => w.id === "one" ? { ...w, status: "interrupted" } : w), 3), runtimeStates: { one: "running" } } }));
  f.flush();
  assert.match(f.surface.element.querySelector(".works-full-meta").textContent, /Running.*Interactive/,
    "a current runtime recovery supersedes the earlier interrupted catalog status");
  f.surface.dispose();
});

test("Works does not reclaim a terminal while its surface is hidden", async () => {
  const f = await fixture(); f.surface.openWork("one");
  const external = f.document.createElement("aside"); f.document.querySelector("main").append(external);
  external.append(f.runtimes.get("one")); f.surface.element.hidden = true;
  const mounts = f.mounts.length;
  f.windowCatalogModel.update(() => ({ catalog: catalog(windows, 2) })); f.flush(); f.surface.sync();
  assert.ok(f.runtimes.get("one").parentElement === external, "inactive updates leave the current runtime host alone");
  assert.equal(f.mounts.length, mounts);
  f.surface.element.hidden = false; f.surface.sync();
  assert.ok(f.surface.element.querySelector(".works-full").contains(f.runtimes.get("one")));
  f.surface.dispose();
});

test("Works read-only split preview follows selection and releases its output subscriptions", async () => {
  const f = await fixture(); const host = f.document.createElement("aside"); f.document.querySelector("main").append(host);
  f.surface.setPreviewHost(host);
  assert.equal(host.querySelectorAll(".work-row").length, 2);
  const output = host.querySelector('[data-window-id="one"] .terminal-text-preview, [data-window-id="one"] pre');
  const mounts = f.previewMounts.length;
  for (let index = 0; index < 3; index++) f.surface.sync();
  assert.ok(host.querySelector('[data-window-id="one"] pre') === output,
    "same-identity activation must retain the already displayed preview");
  f.windowCatalogModel.update(() => ({ catalog: catalog(windows.map(window =>
    window.id === "one" ? { ...window, dynamic_title: "Updated purpose" } : window), 2) })); f.flush();
  assert.match(host.textContent, /Updated purpose/);
  assert.ok(host.querySelector('[data-window-id="one"] pre') === output,
    "metadata updates must preserve the subscribed output node");
  assert.equal(f.previewMounts.length, mounts, "retained output must keep its existing subscription");
  f.surface.openWork("one");
  assert.match(host.textContent, /Read-only preview/);
  assert.equal(host.querySelector("textarea"), null);
  const fullOutput = host.querySelector(".works-preview-output pre");
  f.surface.sync();
  assert.ok(host.querySelector(".works-preview-output pre") === fullOutput);
  f.surface.setPreviewHost(null); assert.equal(host.childNodes.length, 0);
  f.surface.dispose(); assert.equal(f.previews.size, 0);
  let ready = false;
  const cold = await fixture(null, catalog(windows), () => ready);
  const coldHost = cold.document.createElement("aside"); cold.document.querySelector("main").append(coldHost);
  cold.surface.setPreviewHost(coldHost);
  assert.equal(coldHost.querySelectorAll("pre").length, 0);
  ready = true; cold.surface.sync();
  assert.match(coldHost.textContent, /one output/, "an unavailable first mount must recover for the same identity");
  cold.surface.dispose(); assert.equal(cold.previews.size, 0);
});

test("Works and Agents CSS uses Operator tokens, responsive wrapping and an in-surface full view", () => {
  const css = readFileSync(new URL("../styles/works-agents.css", import.meta.url), "utf8");
  assert.doesNotMatch(css, /#[0-9a-f]{3,8}\b|rgba?\(|position:\s*fixed/i);
  assert.match(css, /var\(--font-mono\)/);
  assert.match(css, /height:\s*calc\(var\(--works-preview-lines,\s*3\)/);
  assert.match(css, /@media\s*\(max-width:\s*[^)]+\)/);
  assert.match(css, /prefers-reduced-motion/);
  assert.match(css, /data-state="waiting"[^}]*\.work-spark[^}]*--color-state-needs-input/s);
  assert.match(css, /\.works-full[\s\S]*min-height:\s*0/);
  assert.match(css, /\.works-availability[^}]*var\(--type-xs\)[^}]*overflow-wrap:\s*anywhere/s);
});


test("Works output line settings update both lists without replacing the native terminal", async () => {
  const f = await fixture();
  const host = f.document.createElement("aside"); f.document.querySelector("main").append(host);
  f.surface.setPreviewHost(host);
  const row = f.surface.element.querySelector('.work-row[data-window-id="one"]');
  const native = f.runtimes.get("one");
  assert.ok(f.previewMounts.every(mount => mount.lines === 3));
  row.querySelector(".work-tail pre").dispatchEvent(new f.window.Event("click", { bubbles: true }));
  assert.equal(f.surface.element.querySelector(".works-full").hidden, true, "reading or selecting output does not open its terminal");
  for (const lines of [5, 10]) {
    f.systemSettingsModel.update(state => ({ ...state, worksPreviewLines: lines })); f.flush();
    assert.equal(f.surface.element.style.getPropertyValue("--works-preview-lines"), String(lines));
    assert.equal(host.querySelector(".works-surface").style.getPropertyValue("--works-preview-lines"), String(lines));
    for (const root of [f.surface.element, host]) {
      const tail = root.querySelector('.work-row[data-window-id="one"] .work-tail');
      assert.equal(f.previewMounts.filter(mount => mount.root === tail).at(-1).lines, lines);
      assert.equal(tail.querySelectorAll("pre").length, 1, "old text subscription is disposed");
    }
    assert.equal(f.surface.element.querySelector('.work-row[data-window-id="one"]'), row);
    assert.equal(f.runtimes.get("one"), native);
    assert.equal(f.surface.element.querySelectorAll(".work-row").length, 2);
    assert.match(f.surface.element.textContent, /Today.*Input.*Cache.*Output.*Landed PRs/s);
  }
  f.surface.openWork("one");
  f.systemSettingsModel.update(state => ({ ...state, worksPreviewLines: 3 })); f.flush();
  assert.ok(f.surface.element.querySelector(".works-terminal-host").contains(native));
  f.surface.dispose();
  assert.equal(f.previews.size, 0);
  assert.equal(f.frames.size, 0);
});
