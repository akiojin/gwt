import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { parse } from "acorn";
import { existsSync, readFileSync } from "node:fs";
import { createUiStateStore } from "../ui-state-store.js";
import { createAgentActivityModel } from "../agent-activity-model.js";

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
async function fixture(telemetry = null, initialCatalog = catalog(windows), previewAvailable = () => true, observed = false, initialActivity = { agents: live, count: 2 }) {
  const { createWorksSurface } = await import(moduleUrl);
  const { document, window } = parseHTML("<main></main>");
  const frames = new Set();
  const options = { scheduleFrame: fn => { frames.add(fn); return () => frames.delete(fn); } };
  const windowCatalogModel = createUiStateStore({ catalog: initialCatalog }, options);
  const activity = observed ? createAgentActivityModel("project") : null;
  if (activity) activity.applyAgentActivitySnapshot(activitySnapshot(live));
  const activityModel = activity?.model || createUiStateStore(initialActivity, options);
  const rosterModel = createUiStateStore({ members: [{ member_id: "m1", name: "One", enabled: true, assignment: { session_id: "s1" } }] }, options);
  const revocations = [];
  const runtimes = new Map(), mounts = [], layouts = [], previews = new Set(), previewMounts = [], selected = [], focused = [];
  const systemSettingsModel = createUiStateStore({ worksPreviewLines: 3 }, options);
  const summaryModel = createUiStateStore({ running: 0, block: 0, idle: 0, error: 0 }, options);
  const surface = createWorksSurface({ projectKey: observed ? "project" : undefined, document, windowCatalogModel, activityModel, rosterModel, systemSettingsModel, summaryModel,
    windowDisplayTitle: w => `${w.agent_name || w.title || "Agent"}${w.dynamic_title ? ` — ${w.dynamic_title}` : ""}`,
    mountTerminal(id, root, options) { let input = runtimes.get(id); if (!input) { input = document.createElement("textarea"); input.className = "xterm"; runtimes.set(id, input); } root.appendChild(input); mounts.push({ id, root, readOnly: options?.readOnly }); },
    onRevokeTerminal(id, root) { revocations.push({ id, root }); },
    mountPreview(id, root, options) { if (!previewAvailable()) return () => {}; const pre = document.createElement("pre"); pre.textContent = `${id} output`; root.appendChild(pre); previews.add(pre); previewMounts.push({ id, root, lines: options?.tailLines }); return () => { previews.delete(pre); pre.remove(); }; },
    onFocus: id => focused.push(id), onLayout() { layouts.push(true); }, onSelectMember: id => selected.push(id), getTelemetry: () => telemetry,
  });
  document.querySelector("main").append(surface.element); surface.sync();
  const flush = () => { for (const frame of [...frames]) { frames.delete(frame); frame(); } };
  return { activity, revocations, surface, document, window, windowCatalogModel, activityModel, rosterModel, summaryModel, frames, flush, runtimes, mounts, layouts, previews, previewMounts, systemSettingsModel, selected, focused };
}


function activitySnapshot(agents, tick = 0, host_id = "host") {
  return { project_key: "project", host_id, observed_at: new Date(Date.UTC(2026, 10, 3, 0, 0, tick)).toISOString(), agents };
}
function projectCatalog(wins = windows, version = 1) {
  const value = catalog(wins, version); value.appState.tabs[0].project_key = "project"; return value;
}

test("Works preserves same-identity full presentation across unavailable activity and clear, revoking input before render", async () => {
  const f = await fixture(null, projectCatalog(), undefined, true);
  f.surface.openWork(live[0]);
  const row = f.surface.element.querySelector('.work-row[data-window-id="one"]');
  const root = f.surface.element.querySelector('.works-terminal[data-window-id="one"]');
  const preview = row.querySelector("pre"), input = f.runtimes.get("one");
  input.value = "draft";
  const assertRejectedOpenPreservesFull = () => {
    const mounts = f.mounts.length, focused = f.focused.length;
    for (const target of [live[0], { ...live[0], hostId: "foreign-host" }]) {
      assert.equal(f.surface.openWork(target), false);
      assert.equal(f.surface.element.querySelector(".works-full").hidden, false, "rejected open preserves stale full selection");
      assert.ok(root.parentElement === f.surface.element.querySelector(".works-terminal-host"));
      assert.ok(row.querySelector("pre") === preview);
      assert.equal(input.value, "draft");
    }
    assert.equal(f.mounts.length, mounts, "rejected open cannot mount a stale target");
    assert.equal(f.focused.length, focused, "rejected open cannot focus a stale target");
  };
  for (const [index, lose] of [() => f.activity.applyAgentActivitySnapshot(activitySnapshot(null, 1)), () => f.activity.clearAgentActivity()].entries()) {
    const mounts = f.mounts.length, revocations = f.revocations.length;
    lose();
    assert.ok(f.revocations.length > revocations, "all unavailable owned entries are revoked synchronously before a frame");
    assert.ok(f.revocations.at(-1).root === root);
    assert.equal(f.surface.contains(live[0]), false);
    f.surface.sync();
    assert.ok(f.surface.element.querySelector('.work-row[data-window-id="one"]') === row, "row remains identical");
    assert.ok(f.surface.element.querySelector('.works-terminal[data-window-id="one"]') === root, "root remains identical");
    assert.equal(f.surface.element.querySelector(".works-full").hidden, false);
    assert.match(f.surface.element.querySelector(".works-full-meta").textContent, /Input unavailable/);
    assert.ok(row.querySelector("pre") === preview, "existing output subscription survives temporary loss");
    assert.equal(f.mounts.length, mounts, "retained display never mounts a stale terminal");
    assert.equal(input.value, "draft");
    assertRejectedOpenPreservesFull();
    let unsubscribe;
    const recovered = new Promise(resolve => {
      let initial = true;
      unsubscribe = f.activityModel.subscribeFrame(state => state, () => {
        if (initial) { initial = false; return; }
        resolve();
      });
    });
    f.activity.applyAgentActivitySnapshot(activitySnapshot(live, index + 2));
    await recovered; unsubscribe();
    assert.equal(f.surface.contains(live[0]), true);
    assert.ok(f.mounts.filter(mount => mount.id === "one").at(-1).root === root);
    assert.equal(f.mounts.filter(mount => mount.id === "one").at(-1).readOnly, false);
  }
  const revocations = f.revocations.length;
  f.windowCatalogModel.update(() => ({ catalog: projectCatalog([], 2) }));
  assert.ok(f.revocations.length > revocations, "catalog loss revokes before its frame");
  f.surface.sync();
  assert.equal(f.surface.element.querySelector(".works-full").hidden, false);
  assert.ok(f.surface.element.querySelector('.works-terminal[data-window-id="one"]') === root);
  assert.ok(row.querySelector("pre") === preview);
  assertRejectedOpenPreservesFull();
  f.windowCatalogModel.update(() => ({ catalog: projectCatalog([], 3) }));
  assert.ok(f.revocations.filter(event => event.id === "one").every(event => event.root === root), "repeated reconciliation targets only the same captured root; binding idempotence is measured by the native fixture");
  f.windowCatalogModel.update(() => ({ catalog: projectCatalog(windows, 4) })); f.flush();
  assert.equal(f.surface.element.querySelector(".works-full").hidden, false);
  assert.ok(f.mounts.filter(mount => mount.id === "one").at(-1).root === root);
  assert.equal(f.mounts.filter(mount => mount.id === "one").at(-1).readOnly, false);
  f.activity.applyAgentActivitySnapshot(activitySnapshot([], 4)); f.surface.sync();
  assert.equal(f.surface.element.querySelectorAll(".work-row").length, 0);
  assert.equal(f.surface.element.querySelector(".works-full").hidden, true);
  f.surface.dispose(); f.activityModel.dispose();
});

test("Works discards the old full root for conflicting catalog and foreign host", async () => {
  for (const change of [
    f => f.windowCatalogModel.update(() => ({ catalog: projectCatalog([{ ...windows[0], session_id: "replacement" }], 2) })),
    f => f.activity.applyAgentActivitySnapshot(activitySnapshot(live, 1, "foreign-host")),
  ]) {
    const f = await fixture(null, projectCatalog(), undefined, true);
    f.surface.openWork(live[0]);
    const root = f.surface.element.querySelector('.works-terminal[data-window-id="one"]');
    const mounts = f.mounts.length;
    change(f);
    f.surface.element.querySelector('[data-action="next"]').click();
    assert.equal(f.mounts.length, mounts, "old selection cannot navigate into a conflicting identity");
    assert.ok(f.revocations.at(-1)?.root === root, "conflict revokes only the old selected root before render");
    f.surface.sync();
    assert.equal(root.isConnected, false, "identity conflict cannot retain the old root");
    assert.equal(f.surface.element.querySelector(".works-full").hidden, true);
    f.surface.dispose(); f.activityModel.dispose();
  }
});

test("Works implements the approved list and native terminal API", () => {
  assert.match(source, /export function createWorksSurface/);
  assert.doesNotMatch(source, /openTab|agents-grid|localStorage|addEventListener\(["']keydown["'],\s*[^\n]*Escape/);
});

test("Works joins exact local-live sessions, places PM first and keeps unknown telemetry fields", async () => {
  const { surface, document, selected, activityModel, summaryModel, flush } = await fixture();
  const rows = [...surface.element.querySelectorAll(".work-row")];
  assert.deepEqual(rows.map(row => row.dataset.windowId), ["pm", "one"]);
  assert.equal(surface.contains("old"), false);
  // SPEC #5016 AC-HS1: the heading reads the server summary, not the listed rows.
  summaryModel.update(() => ({ running: 5, block: 2, idle: 1, error: 0 })); flush();
  assert.deepEqual(["running", "block", "idle", "error"].map(key => surface.element.querySelector(`[data-count="${key}"]`).textContent), ["5", "2", "1", "0"]);
  assert.match(surface.element.querySelector(".works-counts").textContent, /RUNNING 5\s*BLOCK 2\s*IDLE 1\s*ERROR 0/);
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
  assert.equal(surface.element.querySelector('[data-count="running"]').textContent, "5", "an unavailable census does not hide the server summary");
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
  assert.equal(f.surface.element.querySelector(".works-full").hidden, false, "same-identity catalog gap retains full presentation");
  assert.equal(f.mounts.length, mountsBeforeGap);
  assert.ok(f.previews.size > 0, "catalog gap retains existing output subscription");
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
  const layouts = f.layouts.length;
  const observer = new f.window.MutationObserver(records => { for (const record of records) removedRoots.push(...record.removedNodes); });
  observer.observe(nativeRoot.parentElement, { childList: true });
  f.windowCatalogModel.update(() => ({ catalog: { ...catalog(windows.map(w => w.id === "one" ? { ...w, dynamic_title: "New purpose" } : w), 2), runtimeStates: { one: "error" } } }));
  f.flush(); await Promise.resolve();
  assert.equal(f.layouts.length, layouts + 1, "selected header changes request layout after model rendering");
  assert.equal(removedRoots.includes(nativeRoot), false, "metadata rendering must not detach a focused native root");
  observer.disconnect();
  assert.equal(f.surface.element.querySelector(".works-full-name").textContent, "One");
  assert.match(f.surface.element.querySelector(".works-full-meta").textContent, /New purpose.*Error.*Input unavailable/);
  assert.equal(f.runtimes.get("one"), input);
  assert.equal(f.surface.element.querySelector('[data-count="error"]').textContent, "0", "a row state change never recounts the heading");
  for (const [status, label] of [["stopped", "Stopped"], ["exited", "Stopped"], ["interrupted", "Interrupted"]]) {
    f.windowCatalogModel.update(() => ({ catalog: { ...catalog(windows, 3), runtimeStates: { one: status } } }));
    f.flush();
    assert.match(f.surface.element.querySelector(".works-full-meta").textContent, new RegExp(`${label}.*Input unavailable`));
    assert.equal(f.surface.element.querySelector('[data-window-id="one"] .work-state').textContent, label);
    assert.equal(f.runtimes.get("one"), input);
  }
  f.windowCatalogModel.update(() => ({ catalog: { ...catalog(windows.map(w => w.id === "one" ? { ...w, status: "interrupted" } : w), 3), runtimeStates: { one: "running" } } }));
  f.flush();
  assert.match(f.surface.element.querySelector(".works-full-meta").textContent, /Running.*Interactive/,
    "a current runtime recovery supersedes the earlier interrupted catalog status");
  f.surface.dispose();
});

test("Works retains the selected stopped transcript without adding it to live activity", async () => {
  const started = { ...windows[0], runtime_started_at_ms: 123 };
  const stopped = { ...started, status: "stopped", persist: false };
  const f = await fixture(null, catalog([started, windows[1]]));
  assert.equal(f.surface.openWork(live[0]), true);
  const input = f.runtimes.get("one"), nativeRoot = input.parentElement;
  const row = f.surface.element.querySelector('.work-row[data-window-id="one"]');
  f.windowCatalogModel.update(() => ({ catalog: catalog([stopped, windows[1]], 2) }));
  f.activityModel.update(() => ({ agents: [live[1]], count: 1 }));
  assert.ok(f.revocations.some(revocation => revocation.id === "one" && revocation.root === nativeRoot), "STOP revokes native input before its display frame");
  f.flush();
  assert.equal(f.surface.element.querySelector(".works-full").hidden, false);
  assert.equal(f.surface.element.querySelector('.work-row[data-window-id="one"]'), row);
  assert.equal(input.parentElement, nativeRoot);
  assert.equal(f.runtimes.get("one"), input);
  assert.match(f.surface.element.querySelector(".works-full-meta").textContent, /Stopped/);
  assert.equal(f.mounts.filter(mount => mount.id === "one").at(-1).readOnly, true);
  assert.equal(f.surface.openWork(live[0]), true, "the same physical transcript keeps its exact assignment");
  assert.equal(f.surface.element.querySelector(".works-active").textContent, "Active 1");
  assert.equal(f.surface.element.querySelector('[data-count="running"]').textContent, "0");

  f.windowCatalogModel.update(() => ({ catalog: catalog([{ ...stopped, runtime_started_at_ms: 456 }, windows[1]], 3) })); f.flush();
  assert.equal(f.surface.openWork(live[0]), false, "a later same-Session process cannot inherit the stopped incarnation");
  const replacement = { ...live[0], runtime_incarnation: 9 };
  f.activityModel.update(() => ({ agents: [replacement, live[1]], count: 2 })); f.flush();
  assert.equal(f.surface.element.querySelectorAll('.work-row[data-window-id="one"]').length, 1);
  assert.equal(f.surface.openWork(live[0]), false);
  assert.equal(f.surface.openWork(replacement), true, "live membership takes precedence over the retained catalog row");
  const rebound = { ...replacement, session_id: "new-session", runtime_incarnation: 10 };
  f.activityModel.update(() => ({ agents: [rebound, live[1]], count: 2 })); f.flush();
  assert.equal(f.surface.contains("one"), false, "a mismatched live Session cannot fall back to the old stopped terminal");
  f.surface.dispose();
});

test("Works opens a fresh retained STOP snapshot until its canonical window is removed", async () => {
  const stopped = { ...windows[0], status: "stopped", persist: false, runtime_started_at_ms: 123 };
  const f = await fixture(null, catalog([stopped,
    { ...stopped, id: "legacy", persist: true }, { ...stopped, id: "shell", preset: "shell" },
  ]), undefined, false, { agents: [], count: 0 });
  assert.deepEqual([...f.surface.element.querySelectorAll(".work-row")].map(row => row.dataset.windowId), ["one"]);
  assert.equal(f.surface.openWork("one"), true);
  assert.equal(f.surface.openWork(live[0]), false, "a fresh snapshot must not invent a physical incarnation");
  assert.equal(f.surface.element.querySelector(".works-active").textContent, "Active 0");
  assert.equal(f.surface.element.querySelector('[data-count="running"]').textContent, "0");
  assert.equal(f.surface.element.querySelector(".works-empty").hidden, true);
  f.windowCatalogModel.update(() => ({ catalog: catalog([], 2) })); f.flush();
  assert.equal(f.surface.contains("one"), false);
  assert.equal(f.surface.element.querySelectorAll(".work-row").length, 0);
  assert.equal(f.surface.element.querySelector(".works-full").hidden, true);
  f.surface.dispose();
});

test("Works telemetry refresh preserves the terminal host without requesting layout", async () => {
  const telemetry = { today: { input_tokens: 1, output_tokens: 2 } };
  const f = await fixture(telemetry);
  f.surface.openWork("one");
  const input = f.runtimes.get("one"), host = input.parentElement;
  const mounts = f.mounts.length, layouts = f.layouts.length;
  telemetry.today.input_tokens = 10;
  f.surface.sync(false);
  assert.equal(f.surface.element.querySelector('[data-metric="today-total"]').textContent, "12");
  assert.equal(input.parentElement, host);
  assert.equal(f.mounts.length, mounts, "telemetry must not rehost the live terminal");
  assert.equal(f.layouts.length, layouts, "telemetry must not trigger persisted terminal fits");
  f.surface.sync();
  assert.ok(f.mounts.length > mounts, "explicit host synchronization still reclaims runtimes");
  assert.equal(f.layouts.length, layouts + 1, "explicit host synchronization still requests layout");
  const external = f.document.createElement("aside"); f.document.querySelector("main").append(external);
  const progress = f.document.createElement("div"); progress.className = "attachment-progress"; progress.hidden = true;
  host.append(progress);
  external.append(input);
  const beforeReclaim = f.mounts.length, beforeLayout = f.layouts.length;
  f.surface.sync(false);
  assert.ok(input.parentElement === host, "telemetry reclaims a selected native terminal even with retained attachment progress");
  assert.ok(progress.parentElement === host, "reclaim preserves attachment progress");
  assert.equal(f.mounts.length, beforeReclaim + 1, "only the selected host missing its xterm is mounted again");
  assert.equal(f.layouts.length, beforeLayout, "reclaim avoids a global persisted-layout pass");
  f.surface.sync(false);
  assert.equal(f.mounts.length, beforeReclaim + 1, "an unchanged host returns to the ordinary no-mount fast path");
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.match(app, /worksTelemetryModel\.subscribeFrame\(state => state, \(\) => worksSurface\.sync\(false\)\)/);
  f.surface.dispose();
});

test("Works reuses unchanged sparklines but redraws changed buckets and shared scales", async () => {
  const values = () => Array(30).fill(1);
  const telemetry = { fleet_tokens_per_minute: values(), by_window: {
    one: { tokens_per_minute: values() }, pm: { tokens_per_minute: values() },
  } };
  const f = await fixture(telemetry);
  const graphs = [...f.surface.element.querySelectorAll("svg")];
  const mounts = f.mounts.length, layouts = f.layouts.length;
  let clears = 0, nodes = 0;
  const create = f.document.createElementNS.bind(f.document);
  f.document.createElementNS = (...args) => { nodes++; return create(...args); };
  for (const graph of graphs) {
    const replace = graph.replaceChildren.bind(graph);
    graph.replaceChildren = (...args) => { clears++; return replace(...args); };
  }
  for (let tick = 0; tick < 20; tick++) {
    telemetry.fleet_tokens_per_minute = values();
    telemetry.by_window.one.tokens_per_minute = values();
    telemetry.by_window.pm.tokens_per_minute = values();
    f.activityModel.update(state => ({ ...state, snapshot: { observed_at: tick } }));
    f.flush();
  }
  assert.deepEqual({ clears, nodes, mounts: f.mounts.length - mounts, layouts: f.layouts.length - layouts, pending: f.frames.size },
    { clears: 0, nodes: 0, mounts: 0, layouts: 0, pending: 0 }, "same data must not rebuild the SVG trees");
  const row = id => f.surface.element.querySelector(`[data-window-id="${id}"] svg`);
  const beforeOne = row("one").innerHTML, beforePm = row("pm").innerHTML;
  telemetry.by_window.one.tokens_per_minute[0] = 2;
  f.surface.sync(false);
  assert.notEqual(row("one").innerHTML, beforeOne, "changed measurements redraw");
  assert.notEqual(row("pm").innerHTML, beforePm, "an unchanged row follows the shared scale");
  assert.equal(clears, 2, "the fleet graph is independent of the row scale");
  telemetry.by_window.one.tokens_per_minute[0] = null;
  f.surface.sync(false);
  assert.equal(row("one").dataset.knownCount, "29", "missing buckets remain visible as gaps");
  assert.equal(row("one").querySelectorAll("circle").length, 29);
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
  assert.match(css, /\.agents-surface \.settings-status\s*\{[^}]*overflow-wrap:\s*anywhere/s);
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
    assert.ok(f.surface.element.querySelector('.work-row[data-window-id="one"]') === row, "row remains identical");
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

test("Works app revocation downgrades only its existing exact-root runtime synchronously", () => {
  const appSource = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const ast = parse(appSource, { ecmaVersion: "latest", sourceType: "module" });
  const definitions = {};
  function visit(node) {
    if (!node || typeof node !== "object") return;
    if (node.type === "FunctionDeclaration" && ["applyTerminalReadOnly", "reparentTerminalRuntime", "sameNativeSession", "capturedWorksOwner", "revokeWorksTerminal"].includes(node.id?.name)) {
      definitions[node.id.name] = appSource.slice(node.start, node.end);
    }
    if (node.type === "Property" && node.key?.name === "onRevokeTerminal") definitions.callback = appSource.slice(node.value.start, node.value.end);
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) value.forEach(visit);
      else if (value && typeof value === "object") visit(value);
    }
  }
  visit(ast);
  assert.ok(definitions.callback, "the app must provide the narrow revocation adapter");
  const { document } = parseHTML('<main><div class="root"><div class="native"></div></div><div class="replacement"></div></main>');
  const root = document.querySelector(".root"), native = document.querySelector(".native"), replacement = document.querySelector(".replacement");
  const events = [], frames = [];
  const runtime = { terminalContainer: root, readOnly: false, terminal: { element: native, options: { disableStdin: false } },
    containerBindingsCleanup() { events.push("cleanup"); } };
  const identity = { projectKey: "project", window_id: "one", session_id: "s1", hostId: "host", runtime_incarnation: 1 };
  runtime.nativeIdentity = identity;
  const terminalMap = new Map([["one", runtime]]);
  const previous = new Map([["one", { runtime, root, container: null, identity }]]);
  const surface = { nativeIdentity: (_id, candidate) => candidate === root ? identity : null };
  const revoke = new Function("terminalMap", "worksPreviousHosts", "worksSurface", "windowMap", "attachTerminalContainerBindings", "scheduleInitialFitHandshake", "requestAnimationFrame", "scheduleTerminalFit",
    Object.entries(definitions).filter(([key]) => key !== "callback").map(([, value]) => value).join("\n") + `\nreturn ${definitions.callback};`)(terminalMap, previous, surface, new Map(),
    (_id, host, _terminal, options) => { assert.ok(host === root); events.push(options.readOnly ? "readonly bindings" : "interactive bindings"); return () => {}; },
    () => events.push("handshake"), callback => frames.push(callback), () => events.push("fit"));
  revoke("missing", root, identity);
  revoke("one", replacement, identity);
  assert.equal(runtime.readOnly, false, "another root cannot downgrade a replacement runtime");
  assert.equal(events.length, 0);
  assert.equal(terminalMap.size, 1, "missing runtimes are never created");
  revoke("one", root, identity);
  assert.equal(runtime.readOnly, true);
  assert.equal(runtime.terminal.options.disableStdin, true);
  assert.ok(runtime.terminalContainer === root && native.parentElement === root);
  assert.deepEqual(events, ["cleanup", "readonly bindings", "handshake"]);
  assert.equal(frames.length, 1, "fit remains scheduled, after synchronous authority revocation");
  revoke("one", root, identity);
  assert.deepEqual(events, ["cleanup", "readonly bindings", "handshake"], "repeated revocation is idempotent");
  assert.equal(frames.length, 1);
});

async function nativeLifetimeFixture(outputBatcher = null) {
  const appSource = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const definitions = {}, adapters = {};
  const names = ["renderWorkspace", "renderAppState", "publishWindowCatalog", "allProjectWindowIdSet", "workspaceWindowIdSet", "applyTerminalReadOnly", "reparentTerminalRuntime", "createTerminalRuntime", "disposeTerminalRuntime", "settleWorksTerminal", "nativeSessionIdentity", "sameNativeSession", "currentNativeWindow", "recordWorksPreviousHost", "setWorksHost", "capturedWorksOwner", "revokeWorksTerminal"];
  function visit(node) {
    if (!node || typeof node !== "object") return;
    if (node.type === "FunctionDeclaration" && names.includes(node.id?.name)) definitions[node.id.name] = appSource.slice(node.start, node.end);
    if (node.type === "Property" && ["onRevokeTerminal", "onReleaseTerminal", "mountTerminal"].includes(node.key?.name)
      && (node.key.name !== "mountTerminal" || appSource.slice(node.value.start, node.value.end).includes("worksPreviousHosts"))) adapters[node.key.name] = appSource.slice(node.value.start, node.value.end);
    for (const value of Object.values(node)) if (Array.isArray(value)) value.forEach(visit); else if (value && typeof value === "object") visit(value);
  }
  visit(parse(appSource, { ecmaVersion: "latest", sourceType: "module" }));
  const { createWorksSurface } = await import(moduleUrl);
  const { createWorkspaceRenderSync } = await import("../issue-render-sync.js");
  const { classifyProjectWindowVisibility } = await import("../terminal-viewport-reflow.js");
  const { document } = parseHTML('<main class="surface-area"><article class="shell"><div class="window-body"><div class="terminal-root"></div></div></article></main>');
  const shell = document.querySelector(".shell"), original = document.querySelector(".terminal-root");
  const events = [], failures = [], frames = [], previous = new Map();
  const initial = projectCatalog([windows[0]]).appState;
  const catalog = createUiStateStore({ catalog: { appState: initial, runtimeStates: {}, version: 1, focusedId: null } });
  const activity = createAgentActivityModel("project"); activity.applyAgentActivitySnapshot(activitySnapshot([live[0]]));
  class NativeTerminal {
    constructor(options = {}) { this.options = options; this.element = document.createElement("div"); this.buffer = { active: { baseY: 30, viewportY: 7 } }; this.disposed = false; }
    loadAddon() {} open(root) { root.append(this.element); } onData(callback) { this.input = callback; }
    dispose() { this.disposed = true; this.element.remove(); events.push("dispose"); }
  }
  const terminal = new NativeTerminal(); original.append(terminal.element);
  const runtime = { terminal, terminalContainer: original, readOnly: false, isReady: true, activationFrame: null,
    nativeIdentity: { projectKey: "project", window_id: "one", session_id: "s1", hostId: "host" }, cleanup() { events.push("cleanup"); } };
  const terminalMap = new Map([["one", runtime]]), windowMap = new Map([["one", shell]]), noop = () => {};
  const env = { document, routeProjectKey: "project", appState: initial, worksHost: document.querySelector("main"), worksSurface: null,
    worksPreviousHosts: previous, terminalMap, windowMap, windowCatalogModel: catalog, agentActivityModel: activity.model, focusedId: null,
    reconcilingWindowCatalog: false, workspaceRenderSync: createWorkspaceRenderSync({ onDegraded: errors => failures.push(...errors.map(item => String(item.error))) }),
    traceMeasure: (_kind, _meta, run) => run(), UI_TRACE_EVENT: { renderWorkspace: "workspace", renderAppState: "app" },
    agentKanbanPendingPlacement: { consumePlacementMessage: () => null }, workspaceWindowsRenderKey: value => JSON.stringify(value),
    classifyProjectWindowVisibility, send: noop, applyVisibilityTransition: noop, activateTerminalOnReveal: noop,
    disposePmChat: noop, removePmWindowState: noop, cancelAnimationFrame: noop, terminalViewportRefreshScheduler: { clear: noop },
    agentAttentionToaster: { forgetWindow: noop }, alertsToasts: { dismiss: noop }, settingsNavigationModel: createUiStateStore({}),
    terminalOutputBatcher: outputBatcher || { clear: id => events.push(`clear:${id}`) }, disposeProfileWindow: noop, disposeIndexSearchWindow: noop,
    clearKnowledgeBridgeState: noop, workspaceOverviewSurface: { deleteState: noop }, clearBranchCleanupForWindow: noop,
    pmSettingsPanel: { unmount: noop }, disposeSettingsBody: noop, ensureWindow: noop, recomputeOperatorTelemetry: noop,
    cleanupTextPreviews: noop, resumeKnowledgeEventRefresh: noop, attachTerminalContainerBindings: (_id, _root, _terminal, options) => { events.push(options.readOnly ? "readonly" : "interactive"); return noop; },
    scheduleInitialFitHandshake: noop, requestAnimationFrame: callback => frames.push(callback), scheduleTerminalFit: noop,
    dismissOperatorBriefing: noop, mergeProjectCatalog: state => state, splitSurfaces: { close: noop }, connectSocket: noop,
    applyStatus: noop, detailMap: new Map(), setVersionState: noop, versionState: {}, activeProjectTab: () => initial.tabs[0],
    renderProjectPicker: noop, renderProjectOnboarding: noop, syncCurrentProjectWorkspaceIds: noop, deriveCurrentProjectWorkspaceIds: () => [],
    requestAgentRoster: noop, activeWorkspace: () => initial.tabs[0].workspace,
    window: {}, requestSocketSnapshots: () => events.push("resync"), workspaceWindowById: () => windows[0],
    Terminal: NativeTerminal, FitAddon: class {}, XTERM_THEME_DARK: {}, resolveTerminalFontFamily: () => "mono", isBlinkBrowser: () => false,
    installTerminalViewportRefreshHandlers: () => noop, createTerminalSnapshotWriteCoordinator: () => ({}),
  };
  for (const name of ["renderedTerminalPackets", "decoderMap", "windowRuntimeStateMap", "windowRuntimeStateSinceMap", "renderedWindowElementKeys", "renderedRuntimeStatusKeys", "renderedAgentKanbanBodyKeys", "renderedIssuePreviewBodyKeys", "renderedIssueWindowHeaderKeys", "pendingSnapshotMap", "branchListStateMap", "boardStateMap", "logStateMap"]) env[name] = new Map();
  const factory = new Function(...Object.keys(env), Object.values(definitions).join("\n") + `\nreturn { adapters: {${Object.entries(adapters).map(([key,value]) => `${key}:${value}`).join(",")}}, bind(surface){worksSurface=surface;}, workspace(next){appState=next;return renderWorkspace(next.tabs[0].workspace);}, create:createTerminalRuntime, leave(){setWorksHost(null);}, enter(){setWorksHost(document.querySelector("main"));} };`);
  const api = factory(...Object.values(env));
  const surface = createWorksSurface({ document, projectKey: "project", windowCatalogModel: catalog, activityModel: activity.model,
    rosterModel: createUiStateStore({ members: [] }), windowDisplayTitle: () => "One", ...api.adapters, mountPreview: () => noop });
  api.bind(surface); document.querySelector("main").append(surface.element); surface.sync(); surface.openWork(live[0]); events.length = 0;
  return { surface, api, runtime, terminalMap, windowMap, previous, activity, catalog, initial, original, shell, events, failures, env,
    dispose() { surface.dispose(); activity.model.dispose(); catalog.dispose(); } };
}

test("Works owned native preserves its input mode when another host requests the same Session", async () => {
  const f = await nativeLifetimeFixture();
  const root = f.runtime.terminalContainer, terminal = f.runtime.terminal;
  try {
    assert.ok(f.api.create("one", f.original) === f.runtime);
    assert.ok(f.runtime.terminalContainer === root && f.runtime.terminal === terminal);
    assert.equal(f.runtime.readOnly, false, "another host cannot downgrade the current interactive owner");
    assert.equal(terminal.options.disableStdin, false);
    f.activity.clearAgentActivity();
    assert.equal(f.runtime.readOnly, true);
    assert.ok(f.api.create("one", f.original) === f.runtime);
    assert.ok(f.runtime.terminalContainer === root && f.runtime.terminal === terminal);
    assert.equal(f.runtime.readOnly, true, "another host cannot enable an owner after authority loss");
    assert.equal(terminal.options.disableStdin, true);
  } finally { f.dispose(); }
});

test("Works actual native helpers retain an omitted owner and release it on measured removal without a later workspace", async () => {
  const f = await nativeLifetimeFixture();
  const root = f.runtime.terminalContainer;
  const gap = projectCatalog([]).appState;
  f.api.leave();
  assert.ok(f.runtime.terminalContainer === f.original, "fresh departure first returns the native normally");
  for (const name of ["decoderMap", "pendingSnapshotMap", "renderedTerminalPackets"]) f.env[name].set("one", "owned old work");
  f.api.workspace(gap);
  assert.deepEqual(f.failures, []);
  assert.ok(f.terminalMap.get("one") === f.runtime, "temporary catalog omission retains native runtime before coherent publish");
  assert.equal(f.events.filter(event => event === "clear:one").length, 0, "retained native keeps its bounded output queue");
  assert.equal(f.runtime.readOnly, true);
  assert.equal(f.runtime.terminal.disposed, false);
  for (const name of ["decoderMap", "pendingSnapshotMap", "renderedTerminalPackets"]) assert.equal(f.env[name].get("one"), "owned old work", `${name} follows retained native lifetime`);
  f.api.enter(); f.surface.sync();
  assert.ok(f.runtime.terminalContainer === root, "stale reentry displays the same native in the original Works root");
  assert.equal(f.runtime.terminal.buffer.active.viewportY, 7);
  f.activity.applyAgentActivitySnapshot(activitySnapshot([], 1));
  assert.equal(f.terminalMap.has("one"), false, "authoritative census removal releases a shell-less native synchronously");
  assert.equal(f.runtime.terminal.disposed, true);
  assert.equal(f.events.filter(event => event === "clear:one").length, 1, "exact native disposal clears its bounded output queue");
  for (const name of ["decoderMap", "pendingSnapshotMap", "renderedTerminalPackets"]) assert.equal(f.env[name].has("one"), false, `${name} is released with the exact native`);
  assert.equal(f.events.filter(event => event === "dispose").length, 1);
  f.dispose(); assert.equal(f.events.filter(event => event === "dispose").length, 1);
  const unloading = await nativeLifetimeFixture();
  unloading.api.workspace(gap);
  unloading.dispose();
  assert.equal(unloading.terminalMap.has("one"), false, "unload releases a retained native without a windowMap owner");
  assert.equal(unloading.runtime.terminal.disposed, true);
});

test("Works actual native release returns a same-Session diagnostic owner readonly and never reuses it for a different Session", async () => {
  const f = await nativeLifetimeFixture();
  f.api.leave();
  f.activity.applyAgentActivitySnapshot(activitySnapshot([], 1));
  assert.ok(f.runtime.terminalContainer === f.original, "ended Works returns existing native to its exact diagnostic shell");
  assert.equal(f.runtime.readOnly, true);
  assert.equal(f.runtime.terminal.disposed, false);
  const next = projectCatalog([{ ...windows[0], session_id: "new-session" }]).appState;
  f.api.workspace(next);
  assert.equal(f.runtime.terminal.disposed, true, "known incoming Session is fenced before any later reuse call");
  const replacementRoot = f.shell.ownerDocument.createElement("div");
  const replacement = f.api.create("one", replacementRoot, { nativeHost: true });
  assert.ok(replacement !== f.runtime, "same window id cannot reuse another Session's native runtime");
  assert.equal(f.runtime.terminal.disposed, true);
  assert.ok(f.events.includes("resync"));
  f.dispose();
});


test("Works ordinary surface departure keeps unavailable native ownership readonly and restores only fresh exact hosts", async () => {
  const f = await nativeLifetimeFixture();
  const root = f.runtime.terminalContainer, terminal = f.runtime.terminal;
  const record = f.previous.get("one");
  f.activity.clearAgentActivity();
  f.api.leave();
  assert.ok(f.runtime.terminalContainer === root, "unknown departure cannot steal the retained root");
  assert.equal(f.runtime.readOnly, true, "saved previous interactive mode is not current authority");
  assert.ok(f.previous.get("one") === record, "retained owner keeps its exact return provenance");
  f.activity.applyAgentActivitySnapshot(activitySnapshot([live[0]], 1));
  assert.equal(f.runtime.readOnly, true, "hidden recovery cannot enable a retained root");
  f.api.enter();
  assert.ok(f.runtime.terminal === terminal && f.runtime.terminalContainer === root);
  assert.equal(f.runtime.readOnly, false);
  f.api.leave();
  assert.ok(f.runtime.terminalContainer === f.original, "fresh exact normal departure returns to the current shell");
  assert.equal(f.runtime.readOnly, false);
  assert.ok(f.previous.get("one") === record, "fresh return keeps the surviving entry association");
  f.api.enter(); f.surface.showList(); f.api.leave();
  f.activity.clearAgentActivity();
  assert.ok(f.runtime.terminalContainer === root, "proof loss also reclaims a normally returned list native");
  assert.equal(f.runtime.readOnly, true);
  const bindingCount = f.events.filter(event => event === "readonly").length;
  f.activity.clearAgentActivity();
  assert.equal(f.events.filter(event => event === "readonly").length, bindingCount, "repeated loss has no binding effects");
  f.activity.applyAgentActivitySnapshot(activitySnapshot([live[0]], 2));
  f.api.enter(); f.api.leave();
  assert.ok(f.runtime.terminalContainer === f.original);
  f.activity.clearAgentActivity();
  assert.ok(f.runtime.terminalContainer === root, "an earlier revoked flag cannot suppress reclaim after another fresh list return");
  assert.equal(f.runtime.readOnly, true);
  f.dispose();
});

test("Works departure and old release cannot touch or erase a replacement owner", async () => {
  const f = await nativeLifetimeFixture();
  const record = f.previous.get("one");
  const replacementRoot = f.shell.ownerDocument.createElement("div");
  const replacement = { ...f.runtime, terminalContainer: replacementRoot, readOnly: false };
  f.terminalMap.set("one", replacement);
  f.api.leave();
  assert.ok(replacement.terminalContainer === replacementRoot);
  assert.equal(replacement.readOnly, false);
  assert.ok(f.previous.get("one") === record, "unsettled evidence is never erased by a whole-map clear");
  replacement.terminalContainer = record.root;
  f.dispose();
  assert.ok(f.terminalMap.get("one") === replacement, "captured-runtime mismatch protects even a reused DOM root");
});


test("Works removed shell without a native clears old queued bytes before same-id new Session creation", async () => {
  const { createTerminalOutputBatcher } = await import("../terminal-output-buffer.js");
  const writes = [];
  const batcher = createTerminalOutputBatcher({ schedule() {}, write: (_id, text) => writes.push(text) });
  const f = await nativeLifetimeFixture(batcher);
  // Contained/unmounted shells legitimately have no native; seed that boundary
  // while exercising the actual app remove_window and createRuntime helpers.
  f.terminalMap.delete("one"); f.previous.clear(); f.runtime.terminal.dispose();
  f.env.pendingSnapshotMap.set("one", "old snapshot");
  f.env.decoderMap.set("one", "old decoder");
  batcher.enqueue("one", "old batched output");
  f.api.workspace(projectCatalog([]).appState);
  assert.deepEqual(f.failures, []);
  for (const name of ["pendingSnapshotMap", "decoderMap"]) {
    assert.equal(f.env[name].has("one"), false, `${name} must not outlive a removed shell without a native`);
  }
  assert.equal(batcher.pendingCount("one"), 0);
  f.api.workspace(projectCatalog([{ ...windows[0], session_id: "new-session" }]).appState);
  const root = f.shell.ownerDocument.createElement("div");
  const fresh = f.api.create("one", root, { nativeHost: true });
  assert.equal(fresh.nativeIdentity.session_id, "new-session");
  assert.equal(batcher.pendingCount("one"), 0);
  assert.equal(f.env.pendingSnapshotMap.has("one"), false);
  batcher.flushNow();
  assert.deepEqual(writes, [], "the new Session cannot receive the old id-keyed batch");
  f.dispose();
});
