import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { readFileSync } from "node:fs";
import { createUiStateStore } from "../ui-state-store.js";
import { createAgentsSurface } from "../agents-surface.js";
import { createAgentRosterModel } from "../agent-activity-model.js";

const source = readFileSync(new URL("../agents-surface.js", import.meta.url), "utf8");
const styles = readFileSync(new URL("../styles/works-agents.css", import.meta.url), "utf8");
const members = [
  { member_id: "m1", name: "One", enabled: true, assignment: { session_id: "s1" }, provider: "Codex", model: "GPT-6", created_at: "2026-10-20T00:00:00Z", issue_count: 3, landed_count: 2, active_seconds: 7200, tokens: 5000, history: [
    { session_id: "s0", issue_number: 4000, title: "Earlier", result: "landed", started_at: "2026-10-20T00:00:00Z", ended_at: "2026-10-20T02:00:00Z" },
    { session_id: "s2", issue_number: 4600, title: "Later", result: null, started_at: "2026-10-22T00:00:00Z", ended_at: "2026-10-22T02:00:00Z" },
  ] },
  { member_id: "m2", name: "Two", enabled: true, provider: null, model: null, created_at: null, issue_count: null, landed_count: null, active_seconds: null, tokens: null, history: [] },
];
const current = { window_id: "one", session_id: "s1", runtime_incarnation: 4, issue_number: 4777, is_pm: false };
async function fixture({ onOpenWork = () => {}, onContinueSavedWork = () => {}, isContinuePending, schedule, switching } = {}) {
  const { document, window } = parseHTML("<main></main>");
  // linkedom does not track focus; keep that browser state local to this fixture.
  let activeElement = null;
  Object.defineProperty(document, "activeElement", { get: () => activeElement });
  const createElement = document.createElement.bind(document);
  document.createElement = (...args) => { const node = createElement(...args); node.focus = () => { activeElement = node; }; return node; };
  const frames = new Set(); const options = { scheduleFrame: fn => { frames.add(fn); return () => frames.delete(fn); } };
  const renamed = [], rosterController = createAgentRosterModel("project", { ...options, onRename: request => renamed.push(request) });
  const rosterModel = rosterController.model;
  rosterController.applyAgentRosterSnapshot({ project_key: "project", members: members.map(member => ({ ...member, agent_member_id: member.member_id })), revision: 3, capacity: 2, recommended_capacity: 6, past_contributors: [{ name: "Earlier author", kind: "implementation", issue_numbers: [4000], first_posted_at: "2026-10-20T00:00:00Z", last_posted_at: "2026-10-20T01:00:00Z", post_count: 4 }] });
  const scheduleModel = schedule ? createUiStateStore(schedule, options) : undefined;
  const switchModel = switching ? createUiStateStore(switching, options) : undefined;
  const activityModel = createUiStateStore({ agents: [current], count: 1 }, options);
  const windowCatalogModel = createUiStateStore({ catalog: { runtimeStates: {}, version: 1, appState: { active_tab_id: "p", tabs: [{ id: "p", workspace: { windows: [{ id: "one", session_id: "s1", agent_member_id: "m1", dynamic_title: "Current issue", status: "running" }] } }] } } }, options);
  const supportedAgentsModel = createUiStateStore({ agents: [{ id: "codex", name: "Codex" }, { id: "claude", name: "Claude Code" }] }, options);
  const customAgentsModel = createUiStateStore({ catalog: { loaded: true, agents: [{ id: "custom-cli", display_name: "Custom CLI" }] } }, options);
  const opened = [], rested = [], added = [], reassigned = [], refreshed = [], switched = [], saved = [], continued = [];
  const surface = createAgentsSurface({ document, rosterModel, activityModel, windowCatalogModel, supportedAgentsModel, customAgentsModel, scheduleModel, switchModel, onRenameMember: rosterController.editAgentMemberName, onRetryName: rosterController.retryAgentMemberName, isContinuePending, onContinueSavedWork: workId => { continued.push(workId); onContinueSavedWork(workId); }, onSwitch: action => { switched.push(action); switchModel.update(state => ({ ...state, pending: action })); }, onOpenSavedWork: intent => saved.push(intent), onReassign: action => reassigned.push(action), onRefreshSchedule: () => refreshed.push(true), onAddMember: (tool, name) => { added.push({ tool, name }); rosterModel.update(state => ({ ...state, error: null })); }, onRestMember: id => rested.push(id), onOpenWork: assignment => { opened.push(assignment); onOpenWork(assignment); } });
  for (const node of surface.element.querySelectorAll("select, input")) node.focus = () => { activeElement = node; };
  document.querySelector("main").append(surface.element);
  const flush = () => { for (const fn of [...frames]) { frames.delete(fn); fn(); } };
  return { surface, document, window, frames, flush, rosterModel, rosterController, renamed, supportedAgentsModel, customAgentsModel, activityModel, windowCatalogModel, scheduleModel, opened, rested, added, reassigned, refreshed, switchModel, switched, saved, continued };
}

test("Agents autosaves inline names while retaining native input, focus, invalid drafts and per-member selection drafts", async () => {
  const f = await fixture(), input = f.surface.element.querySelector('[aria-label="Display name"]');
  assert.ok(input, "selected registered member has an inline display name field");
  assert.equal(input.classList.contains("settings-input"), true);
  assert.match(styles, /\.roster-name-field\s*\{[^}]*flex-direction:\s*column[^}]*var\(--space-/s);
  assert.equal(input.value, "One");
  input.focus(); input.value = "  Maple  "; input.dispatchEvent(new f.window.Event("input"));
  assert.equal(f.renamed.length, 1); assert.equal(f.renamed[0].action.name, "Maple");
  f.flush(); assert.match(f.surface.element.querySelector(".roster-name-status").textContent, /Saving/);
  f.windowCatalogModel.update(state => ({ catalog: { ...state.catalog, version: 2 } }));
  f.rosterModel.update(state => ({ ...state, members: state.members.map(member => ({ ...member, tokens: 6000 })) }));
  f.supportedAgentsModel.update(state => ({ agents: [...state.agents] })); f.flush();
  assert.ok(f.surface.element.querySelector('[aria-label="Display name"]') === input && input.isConnected && f.document.activeElement === input);
  input.value = " "; input.dispatchEvent(new f.window.Event("input")); f.flush();
  assert.equal(input.getAttribute("aria-invalid"), "true"); assert.equal(input.value, " ");
  assert.match(f.surface.element.querySelector(".roster-name-status").textContent, /display name/i);
  f.surface.selectMember("m2"); f.surface.selectMember("m1");
  assert.ok(f.surface.element.querySelector('[aria-label="Display name"]') === input);
  assert.equal(input.value, " ", "switching selection retains each member's draft");
  const host = f.document.createElement("aside"); f.surface.setPreviewHost(host);
  assert.equal(host.querySelector('[aria-label="Display name"]').disabled, true);
  assert.equal(host.querySelector('input:not([disabled]), button:not([disabled])'), null);
  assert.equal(f.surface.element.querySelector('[data-action="save-name"], [data-action="edit-name"]'), null);
  f.surface.dispose();
});

test("Agents waits for IME completion and exposes correlated failure retry without losing the draft", async () => {
  const f = await fixture(), input = f.surface.element.querySelector('[aria-label="Display name"]');
  assert.ok(input, "inline name field is present");
  input.dispatchEvent(new f.window.Event("compositionstart")); input.value = "か";
  input.dispatchEvent(new f.window.Event("input")); assert.equal(f.renamed.length, 0);
  input.value = "楓"; input.dispatchEvent(new f.window.Event("compositionend"));
  input.dispatchEvent(new f.window.Event("input")); assert.equal(f.renamed.length, 1, "the final input event does not duplicate the write");
  f.rosterController.applyAgentRosterError({ project_key: "project", request_id: f.renamed[0].request_id, message: "Roster changed; retry" }); f.flush();
  assert.equal(input.value, "楓"); assert.equal(input.getAttribute("aria-invalid"), "true");
  const retry = f.surface.element.querySelector('[data-action="retry-name"]'); assert.equal(retry?.textContent, "Retry name");
  retry.click(); assert.equal(f.renamed.length, 2); assert.equal(f.renamed[1].action.name, "楓");
  f.rosterController.applyAgentRosterSnapshot({ project_key: "project", revision: 4,
    members: members.map(member => ({ ...member, agent_member_id: member.member_id, name: member.member_id === "m1" ? "楓" : member.name })) }, f.renamed[1].request_id); f.flush();
  assert.equal(f.surface.element.querySelector(".roster-name-status").textContent, "Saved");
  assert.equal(f.surface.element.querySelector(".roster-detail h3").textContent, "楓");
  assert.equal(input.getAttribute("aria-invalid"), "false"); f.surface.dispose();
});

test("selection changes never commit an interrupted IME draft through trailing native events", async () => {
  const f = await fixture(), input = f.surface.element.querySelector('[aria-label="Display name"]');
  input.dispatchEvent(new f.window.Event("compositionstart")); input.value = "み";
  input.dispatchEvent(new f.window.Event("input")); f.surface.selectMember("m2");
  input.dispatchEvent(new f.window.Event("compositionend")); input.dispatchEvent(new f.window.Event("input"));
  assert.equal(f.renamed.length, 0, "detaching an unfinished input does not submit a name");
  f.surface.selectMember("m1"); assert.equal(input.value, "み");
  input.value = "実"; input.dispatchEvent(new f.window.Event("input"));
  assert.equal(f.renamed.length, 1, "a later explicit completed edit can resume autosave"); f.surface.dispose();
});

test("Project window titles follow current member names only for an exact assigned Session", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  const definition = app.match(/      function windowDisplayTitle\(windowData\) \{[\s\S]*?\n      \}/)[0];
  const member = { member_id: "m1", name: "Maple", assignment: { session_id: "s1" } };
  const rosterModel = { read: () => ({ members: [member] }) };
  const title = new Function("rosterModel", "isAgentWindowPreset", "PM_WINDOW_TITLE", `${definition}; return windowDisplayTitle;`)(rosterModel, preset => preset === "agent", "Project Manager");
  const data = { preset: "agent", session_id: "s1", agent_name: "Historical name", dynamic_title: "Current issue" };
  assert.equal(title(data), "Maple — Current issue");
  assert.equal(title({ ...data, session_id: "s2", agent_member_id: "m1" }), "Historical name — Current issue");
  assert.equal(title({ ...data, is_pm: true }), "Historical name");
  assert.equal(data.agent_name, "Historical name", "current display never mutates Session or window history");
  const renderKeyFunctions = ["appendRenderKeyPart", "appendWindowPlacementRenderKey", "workspaceWindowsRenderKey"]
    .map(name => app.match(new RegExp(`      function ${name}\\([^]*?\\n      \\}`))[0]).join("\n");
  const renderKey = new Function("windowDisplayTitle", "worksHost", "appState", "allProjectWindowIds", "windowWorktreeForm",
    `${renderKeyFunctions}; return workspaceWindowsRenderKey;`)(title, null, { active_tab_id: "p" }, () => ["w"], () => "");
  const workspace = { windows: [{ ...data, id: "w" }] }, previousKey = renderKey(workspace);
  member.name = "Birch";
  assert.notEqual(renderKey(workspace), previousKey, "roster rename invalidates the existing title render key without mutating a window");
  assert.match(app, /applyAgentRosterSnapshot\(event\.roster, event\.request_id\)/);
  assert.match(app, /onRenameMember:\s*editAgentMemberName/);
});

test("Agents replaces the old terminal surface with the approved roster contract", () => {
  assert.match(source, /rosterModel/);
  assert.doesNotMatch(source, /agents-grid|agent-tile|All agents|Columns|openTab|localStorage|mountTerminal/);
});

test("Agents identifies the exact PM before the implementation pool without adding a member", async () => {
  const f = await fixture();
  const pm = { id: "pm", session_id: "pm-session", agent_name: "Yuu", is_pm: true, status: "running" };
  const withPm = name => {
    const catalog = f.windowCatalogModel.read().catalog, project = catalog.appState.tabs[0];
    f.windowCatalogModel.update(() => ({ catalog: { ...catalog, version: catalog.version + 1,
      appState: { ...catalog.appState, tabs: [{ ...project, workspace: {
        windows: [...project.workspace.windows.filter(window => window.id !== pm.id), { ...pm, agent_name: name }],
      } }] } } }));
  };
  withPm("Yuu");
  f.activityModel.update(() => ({ count: 2, agents: [current,
    { window_id: pm.id, session_id: pm.session_id, runtime_incarnation: 2, is_pm: true }] }));
  f.flush();
  const identity = f.surface.element.querySelector(".roster-pm-identity");
  assert.ok(identity && !identity.hidden);
  assert.equal(identity.getAttribute("role"), "group");
  assert.equal(identity.querySelector(".roster-pm-name").textContent, "Yuu");
  assert.equal(identity.querySelector(".window-role-badge").textContent, "PM");
  const children = [...f.surface.element.children];
  assert.ok(children.indexOf(identity) < children.indexOf(f.surface.element.querySelector(".roster-team")));
  assert.equal(identity.querySelectorAll("button").length, 0, "PM is not a selectable or restable member");
  assert.equal(f.surface.element.querySelector(".roster-count").textContent, "2");
  assert.equal(f.surface.element.querySelectorAll(".roster-list .roster-row").length, 2);
  assert.equal(f.surface.element.querySelector('.roster-row[aria-selected="true"]').dataset.memberId, "m1");
  withPm("Yuu updated"); f.flush();
  assert.equal(identity.querySelector(".roster-pm-name").textContent, "Yuu updated");
  f.activityModel.update(state => ({ ...state, snapshot: { project_key: "this-project" } }));
  f.windowCatalogModel.update(state => ({ catalog: { ...state.catalog, version: state.catalog.version + 1,
    appState: { ...state.catalog.appState, tabs: state.catalog.appState.tabs.map(tab => ({ ...tab, project_key: "another-project" })) } } }));
  f.flush();
  assert.equal(identity.hidden, true, "another project's catalog cannot supply the current PM's name");
  f.activityModel.update(() => ({ count: 1, agents: [current] })); f.flush();
  assert.equal(identity.hidden, true, "a stale PM catalog window cannot supply a live identity");
});

test("Agents keeps Team and Past tabs, capacity, member details, four real aggregates and reverse history", async () => {
  const f = await fixture();
  assert.deepEqual([...f.surface.element.querySelectorAll('[role="tab"]')].map(tab => tab.textContent), ["Team", "Past"]);
  assert.equal(f.surface.element.querySelector(".roster-count").textContent, "2");
  assert.match(f.surface.element.querySelector(".roster-capacity").textContent, /Recommended.*6/);
  assert.equal(f.surface.element.querySelectorAll(".roster-row").length, 2);
  assert.equal(f.surface.element.querySelectorAll(".roster-detail .roster-stat").length, 4);
  assert.match(f.surface.element.querySelector(".roster-detail").textContent, /Codex.*GPT-6.*2026-10-20/s);
  const history = [...f.surface.element.querySelectorAll(".roster-history li")];
  assert.match(history[0].textContent, /Now.*#4777.*Current issue/s);
  assert.match(history[1].textContent, /Later/); assert.match(history[2].textContent, /Earlier/);
  assert.equal(f.surface.selectMember("m2"), true);
  assert.ok(f.document.activeElement === f.surface.element.querySelector('.roster-row[data-member-id="m2"]'), "selection restores keyboard focus to the new selected row");
  assert.deepEqual([...f.surface.element.querySelectorAll(".roster-detail .roster-stat b")].map(node => node.textContent), ["—", "—", "—", "—"]);
  assert.equal(f.surface.element.querySelector(".roster-detail .roster-open"), null, "idle members have no current terminal link");
  assert.equal(f.surface.selectMember("missing"), false);
});

test("Agents current pill, row task and Now task open the exact assignment, while idle members only select", async () => {
  const f = await fixture();
  f.surface.element.querySelector('.roster-row[data-member-id="m1"] .roster-current').click();
  f.surface.element.querySelector(".roster-now button").click();
  const task = f.surface.element.querySelector('.roster-row[data-member-id="m1"] .roster-purpose');
  assert.equal(task.tagName, "BUTTON"); task.click();
  assert.deepEqual(f.opened, [current, current, current]);
  const row = task.closest(".roster-row"), pill = row.querySelector(".roster-current");
  task.focus();
  f.rosterModel.update(state => ({ ...state, members: state.members.map(member => member.member_id === "m1" ? { ...member, name: "Updated member", tokens: 6000 } : member) }));
  f.windowCatalogModel.update(state => ({ catalog: { ...state.catalog, version: 2, appState: { ...state.catalog.appState,
    tabs: [{ id: "p", workspace: { windows: [{ id: "one", session_id: "s1", dynamic_title: "Updated purpose" }] } }] } } }));
  f.flush();
  assert.ok(f.surface.element.querySelector('.roster-row[data-member-id="m1"]') === row, "ordinary frames retain the same member row");
  assert.ok(row.querySelector(".roster-purpose") === task);
  assert.ok(row.querySelector(".roster-current") === pill);
  assert.ok(task.isConnected && f.document.activeElement === task, "ordinary frames retain the native keyboard target");
  assert.equal(task.textContent, "Updated purpose");
  assert.equal(row.querySelector("h3").textContent, "Updated member");
  assert.equal(row.querySelector(".roster-row-stats .roster-stat:last-child b").textContent, "6K");
  f.rosterModel.update(state => ({ ...state, members: [...state.members].reverse() })); f.flush();
  assert.deepEqual([...row.parentElement.children].map(node => node.dataset.memberId), ["m2", "m1"]);
  f.rosterModel.update(state => ({ ...state, members: [...state.members].reverse() })); f.flush();
  assert.deepEqual([...row.parentElement.children].map(node => node.dataset.memberId), ["m1", "m2"]);
  assert.ok(row.querySelector(".roster-purpose") === task && task.isConnected, "reordered members retain exact native controls");
  const now = f.surface.element.querySelector(".roster-now button");
  const open = f.surface.element.querySelector(".roster-open");
  now.focus();
  f.rosterModel.update(state => ({ ...state, members: state.members.map(member => member.member_id === "m1" ? { ...member, tokens: 7000 } : member) }));
  f.flush();
  assert.ok(f.surface.element.querySelector(".roster-now button") === now && now.isConnected && f.document.activeElement === now,
    "ordinary telemetry retains the native current-work keyboard target");
  assert.ok(f.surface.element.querySelector(".roster-open") === open, "ordinary telemetry retains the native detail action");
  f.surface.selectMember("m2"); assert.equal(row.getAttribute("aria-selected"), "false");
  f.surface.selectMember("m1"); assert.equal(row.getAttribute("aria-selected"), "true");
  const successor = { ...current, runtime_incarnation: 5 };
  f.activityModel.update(() => ({ agents: [successor], count: 1 })); task.click(); pill.click(); now.click(); open.click();
  assert.equal(f.opened.length, 3, "captured buttons never redirect to a successor runtime");
  f.flush(); assert.equal(row.isConnected, false);
  f.surface.element.querySelector('.roster-row[data-member-id="m1"] .roster-purpose').click();
  assert.deepEqual(f.opened[3], successor);
  assert.equal(f.surface.element.querySelector('.roster-row[data-member-id="m2"] .roster-purpose').tagName, "SPAN");
  assert.equal(f.surface.element.querySelector('.roster-row[data-member-id="m2"] .roster-current'), null);
  f.activityModel.update(() => ({ agents: [], count: 0 }));
  f.surface.element.querySelector(".roster-now button").click();
  task.click();
  assert.equal(f.opened.length, 4, "links captured before the frame must recheck live membership");
  f.flush(); assert.equal(f.surface.element.querySelector(".roster-now"), null);
});

test("Issue #6362 Agents keeps census-proven work busy before catalog hydration and across project tabs", async () => {
  const f = await fixture();
  const catalog = f.windowCatalogModel.read().catalog;
  const project = catalog.appState.tabs[0];
  f.windowCatalogModel.update(() => ({ catalog: { ...catalog, version: 2,
    appState: { ...catalog.appState, tabs: [{ ...project, workspace: { windows: [] } }] } } }));
  f.flush();
  assert.match(f.surface.element.querySelector(".roster-capacity-note").textContent, /Busy 1 \/ Team 2/,
    "an observed child stays busy while its window catalog is hydrating");
  assert.doesNotMatch(f.surface.element.querySelector('.roster-row[data-member-id="m1"]').textContent,
    /Waiting for the next issue|Activity unavailable/);
  assert.equal(f.surface.element.querySelector('.roster-row[data-member-id="m1"] .roster-purpose').textContent, "Issue #4777");
  f.windowCatalogModel.update(() => ({ catalog: { ...catalog, version: 3,
    appState: { ...catalog.appState, tabs: [
      { ...project, workspace: { windows: [] } }, { ...project, id: "sibling" },
    ] } } }));
  f.flush();
  assert.match(f.surface.element.querySelector(".roster-capacity-note").textContent, /Busy 1 \/ Team 2/);
  const task = f.surface.element.querySelector('.roster-row[data-member-id="m1"] .roster-purpose');
  assert.equal(task.textContent, "Current issue", "window metadata enriches the same observed assignment across tabs");
  task.click();
  assert.deepEqual(f.opened, [current]);
  f.activityModel.update(state => ({ ...state, snapshot: { project_key: "this-project" } }));
  f.windowCatalogModel.update(() => ({ catalog: { ...catalog, version: 4,
    appState: { ...catalog.appState, tabs: [{ ...project, project_key: "another-project" }] } } }));
  f.flush();
  assert.match(f.surface.element.querySelector(".roster-capacity-note").textContent, /Busy 1 \/ Team 2/);
  assert.equal(f.surface.element.querySelector('.roster-row[data-member-id="m1"] .roster-purpose').textContent,
    "Issue #4777", "another project's catalog cannot enrich the observed assignment");
  f.surface.dispose();
});

const planned = () => ({ snapshot: { project_key: "project", roster_revision: 3, queue_revision: "queue-v1:one", schedule_revision: 7,
  members: [{ agent_member_id: "m1", issues: [{ issue_number: 5002, reason: "same_spec_continuation" }, { issue_number: 5003, reason: "queue_order" }, { issue_number: 5004, reason: "assigned_by_you" }], empty_reason: null },
    { agent_member_id: "m2", issues: [], empty_reason: "No queued issue" }] }, error: null, pending: false });

function chooseTarget(f, issue = 5002) {
  const entry = f.surface.element.querySelector(`.roster-plan[data-issue-number="${issue}"]`);
  assert.ok(entry, "canonical plan has a reassignment control");
  const select = entry.querySelector("select");
  select.querySelector('option[value="m2"]').selected = true;
  select.dispatchEvent(new f.window.Event("change"));
  return entry.querySelector("button");
}

test("Agents inserts canonical ordered plans and reasons before history without inventing an idle assignment", async () => {
  const f = await fixture({ schedule: planned() });
  const section = f.surface.element.querySelector(".roster-plans");
  assert.ok(section, "selected member shows canonical plans");
  assert.ok(section.previousElementSibling.querySelector(".roster-now"), "current work precedes future plans");
  assert.deepEqual([...section.querySelectorAll(".roster-plan")].map(node => node.dataset.issueNumber), ["5002", "5003", "5004"]);
  assert.deepEqual([...section.querySelectorAll(".roster-plan-reason")].map(node => node.textContent), ["Same SPEC continuation", "Queue order", "Assigned by you"]);
  assert.equal(section.nextElementSibling.className, "roster-history");
  assert.equal(section.querySelector("select").getAttribute("aria-label"), "Assign issue #5002 to member");
  assert.deepEqual([...section.querySelectorAll("select")[0].querySelectorAll("option")].map(node => node.value), ["", "m2"]);
  f.surface.selectMember("m2");
  assert.match(f.surface.element.querySelector(".roster-plans").textContent, /No queued issue/);
  assert.equal(f.surface.element.querySelector(".roster-plans select"), null);
  assert.equal(f.surface.element.querySelector(".roster-now"), null);
});

test("Agents reassigns once with captured revisions and waits for canonical readback", async () => {
  const f = await fixture({ schedule: planned() });
  const button = chooseTarget(f); button.click(); button.click();
  assert.deepEqual(f.reassigned, [{ kind: "reassign", issue_number: 5002, source_member_id: "m1", target_member_id: "m2", expected_roster_revision: 3, expected_queue_revision: "queue-v1:one", expected_schedule_revision: 7 }]);
  assert.match(f.surface.element.querySelector(".roster-plans").textContent, /Updating plans/);
  assert.equal(f.surface.element.querySelectorAll('.roster-plan[data-issue-number="5002"]').length, 1, "sending does not optimistically move the issue");
  f.scheduleModel.update(state => ({ ...state, pending: true })); f.flush();
  assert.equal(f.surface.element.querySelector(".roster-plan select").disabled, true);
  f.surface.element.querySelector('.roster-plans [data-action="refresh-plans"]').click();
  assert.deepEqual(f.refreshed, [true], "Refresh remains reachable after response loss");
  f.scheduleModel.update(state => ({ ...state, pending: false })); f.flush();
  assert.doesNotMatch(f.surface.element.querySelector(".roster-plans").textContent, /Updating plans/, "a canonical unchanged read releases the local response-loss guard");
  chooseTarget(f).click(); assert.equal(f.reassigned.length, 2);
  f.scheduleModel.update(state => ({ ...state, pending: false, snapshot: { ...state.snapshot, schedule_revision: 8, members: [
    { agent_member_id: "m1", issues: state.snapshot.members[0].issues.slice(1), empty_reason: null },
    { agent_member_id: "m2", issues: [state.snapshot.members[0].issues[0]], empty_reason: null },
  ] } })); f.flush();
  assert.equal(f.surface.element.querySelector('.roster-plan[data-issue-number="5002"]'), null);
  const updated = f.surface.element.querySelector(".roster-plans");
  assert.ok(updated.previousElementSibling.classList.contains("roster-current-work"), "changed plans remain after the retained current work");
  assert.equal(updated.nextElementSibling.className, "roster-history");
  f.surface.selectMember("m2");
  assert.equal(f.surface.element.querySelector('.roster-plan[data-issue-number="5002"]')?.dataset.issueNumber, "5002");
});

test("Agents refuses stale plan controls and preserves plans with Refresh recovery on errors", async () => {
  const f = await fixture({ schedule: planned() });
  const stale = chooseTarget(f);
  f.scheduleModel.update(state => ({ ...state, snapshot: { ...state.snapshot, schedule_revision: 8 } }));
  stale.click();
  assert.equal(f.reassigned.length, 0);
  assert.match(f.surface.element.querySelector(".roster-plans").textContent, /changed.*Refresh/i);
  f.scheduleModel.update(state => ({ ...state, error: "Readback unavailable", pending: false })); f.flush();
  assert.match(f.surface.element.querySelector(".roster-plans").textContent, /Readback unavailable/);
  assert.equal(f.surface.element.querySelector(".roster-plan button").disabled, true);
  assert.equal(f.surface.element.querySelectorAll(".roster-plan").length, 3);
  f.surface.element.querySelector('.roster-plans [data-action="refresh-plans"]').click();
  assert.deepEqual(f.refreshed, [true]);
  f.scheduleModel.update(() => planned()); f.rosterModel.update(state => ({ ...state, revision: 4 })); f.flush();
  assert.equal(f.surface.element.querySelector(".roster-plan button").disabled, true);
  f.scheduleModel.update(() => ({ snapshot: null, error: null, pending: false })); f.flush();
  assert.match(f.surface.element.querySelector(".roster-plans").textContent, /Plans unavailable/);
});

test("Agents planned controls use Operator styles and stay inert in split previews", async () => {
  const f = await fixture({ schedule: planned() }), host = f.document.createElement("aside");
  f.document.querySelector("main").append(host); f.surface.setPreviewHost(host);
  assert.ok(host.querySelector("select"));
  assert.equal(host.querySelector("select:not([disabled])"), null);
  assert.match(styles, /\.roster-plan-controls\s*\{[^}]*flex-wrap:\s*wrap/s);
  assert.match(styles, /\.agents-surface select\s*\{[^}]*var\(--color-/s);
  assert.doesNotMatch(styles, /#[0-9a-f]{3,8}\b|rgba?\(/i);
  assert.doesNotMatch(source, /role=["']dialog|modal-backdrop|position:\s*fixed/);
  f.surface.dispose();
});

test("Agents keeps a selected plan control and keyboard focus across unrelated catalog, usage and switch progress updates", async () => {
  const f = await fixture({ schedule: planned(), switching: switchingState() }); chooseTarget(f);
  const section = f.surface.element.querySelector(".roster-plans"), select = section.querySelector("select"), button = section.querySelector(".roster-plan button");
  select.focus();
  f.windowCatalogModel.update(state => ({ catalog: { ...state.catalog, version: 2 } }));
  f.rosterModel.update(state => ({ ...state, members: state.members.map(member => ({ ...member, tokens: 6000 })) })); f.flush();
  assert.ok(f.surface.element.querySelector(".roster-plans") === section, "unchanged plan controls remain attached while surrounding details refresh");
  assert.ok(section.querySelector("select") === select);
  assert.equal(select.value, "m2"); assert.equal(button.disabled, false);
  assert.ok(select.isConnected); assert.ok(f.document.activeElement === select);
  f.switchModel.update(state => ({ ...state, intents: [{ operation_id: "op", member_id: "m1", source_issue_number: 4777, target_issue_number: 5002, stage: "checkpointing" }] })); f.flush();
  assert.ok(f.surface.element.querySelector(".roster-plans select") === select, "switch progress preserves Reassign controls");
  assert.ok(f.document.activeElement === select); assert.equal(select.value, "m2");
  f.surface.dispose();
});

test("Agents add and rest actions target a member and Past shows measured contributor data", async () => {
  const f = await fixture();
  const name = f.surface.element.querySelector('[aria-label="Name for new member"]');
  name.value = "Maple"; name.dispatchEvent(new f.window.Event("input"));
  f.surface.element.querySelector('[data-action="add"]').click();
  f.surface.element.querySelector('.roster-detail [data-action="rest"]').click();
  assert.deepEqual(f.added, [{ tool: "codex", name: "Maple" }]); assert.deepEqual(f.rested, ["m1"]);
  f.surface.element.querySelector('[data-tab="past"]').click();
  assert.equal(f.surface.element.querySelector(".roster-team").hidden, true);
  assert.match(f.surface.element.querySelector(".roster-past").textContent, /Earlier author.*Implementation.*#4000.*2026-10-20.*4/s);
  assert.equal(f.surface.element.querySelectorAll(".roster-past .roster-stat").length, 4);
});

test("Agents batches shared updates, keeps stable member selection, and releases read-only previews", async () => {
  const f = await fixture(); const host = f.document.createElement("aside"); f.document.querySelector("main").append(host);
  f.surface.selectMember("m2"); f.surface.setPreviewHost(host);
  for (let i = 0; i < 8; i++) f.rosterModel.update(state => ({ ...state, members: state.members.map(m => m.member_id === "m2" ? { ...m, name: `Member ${i}` } : m) }));
  assert.equal(f.frames.size, 1); assert.match(f.surface.element.querySelector(".roster-detail").textContent, /Two/);
  f.flush(); assert.match(f.surface.element.querySelector(".roster-detail").textContent, /Member 7/);
  assert.match(host.textContent, /Member 7/);
  assert.equal(host.querySelector("button:not([disabled])"), null);
  f.rosterModel.update(state => ({ ...state, error: "Read unavailable", past_complete: false })); f.flush();
  assert.deepEqual([...f.surface.element.querySelectorAll(".roster-past-kpis .roster-stat b")].map(node => node.textContent), ["—", "—", "—", "—"]);
  assert.match(f.surface.element.textContent, /Read unavailable/);
  f.activityModel.update(() => ({ agents: [], count: null })); f.surface.selectMember("m1"); f.flush();
  assert.match(f.surface.element.querySelector(".roster-capacity-note").textContent, /Busy — \/ Team 2/);
  assert.match(f.surface.element.querySelector('.roster-row[data-member-id="m1"]').textContent, /Activity unavailable/);
  assert.equal(f.surface.element.querySelector('.roster-row[data-member-id="m1"] .roster-current, .roster-detail .roster-open'), null);
  assert.match(f.surface.element.querySelector(".roster-detail").textContent, /Activity unavailable/);
  assert.match(f.surface.element.querySelector('.roster-row[data-member-id="m2"]').textContent, /Waiting for the next issue/);
  f.surface.dispose(); assert.equal(host.childNodes.length, 0); assert.equal(f.frames.size, 0);
});


test("a resting busy member remains reachable from Works until its exact assignment ends", async () => {
  let workSurface;
  const f = await fixture({ onOpenWork: assignment => workSurface.openWork(assignment) });
  f.surface.selectMember("m2");
  f.rosterModel.update(state => ({ ...state, capacity: 1, members: [
    ...state.members.map(member => member.member_id === "m1" ? { ...member, enabled: false } : member),
    { member_id: "rested-idle", name: "Rested idle", enabled: false, assignment: null, history: [] },
  ] })); f.flush();
  const { createWorksSurface } = await import("../works-surface.js");
  workSurface = createWorksSurface({ document: f.document, windowCatalogModel: f.windowCatalogModel, activityModel: f.activityModel, rosterModel: f.rosterModel,
    windowDisplayTitle: data => data.dynamic_title || data.title || "Agent", mountTerminal() {}, mountPreview() { return () => {}; },
    onSelectMember: id => f.surface.selectMember(id) });
  f.document.querySelector("main").append(workSurface.element); workSurface.sync();
  workSurface.element.querySelector(".work-member").click();
  assert.equal(f.surface.element.querySelector('.roster-row[data-member-id="m1"]')?.getAttribute("aria-selected"), "true");
  assert.equal(f.surface.element.querySelector(".roster-count").textContent, "1");
  assert.match(f.surface.element.querySelector('.roster-row[data-member-id="m1"]').textContent, /Resting.*Finishing/);
  assert.equal(f.surface.element.querySelector('.roster-row[data-member-id="rested-idle"]'), null);
  assert.equal(f.surface.element.querySelector('.roster-detail [data-action="rest"]').disabled, true);
  f.surface.element.querySelector(".roster-now button").click();
  assert.deepEqual(f.opened, [current]); assert.equal(workSurface.isVisible("one"), true);
  f.activityModel.update(() => ({ agents: [], count: 0 })); f.flush();
  assert.equal(f.surface.element.querySelector('.roster-row[data-member-id="m1"]'), null);
  workSurface.dispose(); f.surface.dispose();
});


test("an observation timestamp alone preserves roster detail, controls and selection", async () => {
  const f = await fixture();
  const detail = f.surface.element.querySelector(".roster-detail").firstElementChild;
  const currentControl = f.surface.element.querySelector(".roster-now button");
  const row = f.surface.element.querySelector('.roster-row[data-member-id="m1"]');
  for (let i = 0; i < 8; i++) f.activityModel.update(state => ({ ...state, snapshot: { observed_at: `2026-10-25T00:00:0${i}Z` } }));
  assert.equal(f.frames.size, 0, "census watermarks do not schedule presentation work");
  f.flush();
  assert.ok(f.surface.element.querySelector(".roster-detail").firstElementChild === detail);
  assert.ok(f.surface.element.querySelector(".roster-now button") === currentControl);
  assert.ok(f.surface.element.querySelector('.roster-row[data-member-id="m1"]') === row);
  assert.equal(row.getAttribute("aria-selected"), "true");
  f.surface.dispose();
});


const switchingState = () => ({ snapshot: { project_key: "project", intents: [] }, intents: [], pending: null, error: null, disconnected: false });

test("Agents switches only the next planned issue from an exact live source and sends one stable request", async () => {
  const f = await fixture({ schedule: planned(), switching: switchingState() });
  const buttons = f.surface.element.querySelectorAll('[data-action="switch-next"]');
  assert.equal(buttons.length, 1);
  assert.equal(buttons[0].textContent, "Switch to next issue");
  assert.equal(buttons[0].closest(".roster-plan").dataset.issueNumber, "5002");
  buttons[0].click(); buttons[0].click();
  assert.equal(f.switched.length, 1);
  const action = f.switched[0];
  assert.match(action.operation_id, /^[0-9a-f-]{36}$/i);
  assert.deepEqual(Object.keys(action).sort(), ["expected_roster_revision", "kind", "member_id", "operation_id", "target_issue_number"]);
  assert.deepEqual({ ...action, operation_id: "id" }, { kind: "request", operation_id: "id", member_id: "m1", expected_roster_revision: 3, target_issue_number: 5002 });
  const status = f.surface.element.querySelector(".roster-switch-status");
  assert.equal(status.getAttribute("role"), "status");
  assert.equal(status.getAttribute("aria-live"), "polite");
  assert.match(status.textContent, /#4777.*#5002.*Awaiting/s);
  assert.equal(f.surface.element.querySelector('[data-action="switch-next"]').disabled, true);
  f.surface.selectMember("m2");
  assert.equal(f.surface.element.querySelector('[data-action="switch-next"]'), null, "an idle member never supplies a source");
  f.surface.dispose();
});

test("Agents rechecks captured live source and schedule authority before switching", async () => {
  const f = await fixture({ schedule: planned(), switching: switchingState() });
  let button = f.surface.element.querySelector('[data-action="switch-next"]');
  assert.ok(button);
  f.activityModel.update(() => ({ count: 1, agents: [{ ...current, runtime_incarnation: 5 }] }));
  button.click();
  assert.equal(f.switched.length, 0, "a new child incarnation cannot reuse a captured source");
  f.flush(); button = f.surface.element.querySelector('[data-action="switch-next"]');
  f.scheduleModel.update(state => ({ ...state, snapshot: { ...state.snapshot, schedule_revision: 8 } }));
  button.click();
  assert.equal(f.switched.length, 0);
  f.activityModel.update(() => ({ count: 1, agents: [{ ...current, is_pm: true }] })); f.flush();
  assert.equal(f.surface.element.querySelector('[data-action="switch-next"]'), null, "PM and stale window records are not sources");
  f.surface.dispose();
});

test("Agents announces switch stages, cancels before stop, and offers saved Works after failure", async () => {
  const f = await fixture({ schedule: planned(), switching: switchingState() });
  const intent = { operation_id: "op", member_id: "m1", source_issue_number: 4777, target_issue_number: 5002, stage: "checkpointing" };
  const progress = value => { f.switchModel.update(state => ({ ...state, intents: [value], pending: null })); f.flush(); };
  progress(intent);
  assert.match(f.surface.element.querySelector(".roster-switch-status").textContent, /#4777.*#5002.*Saving/s);
  const cancel = f.surface.element.querySelector('[data-action="cancel-switch"]');
  assert.ok(cancel); cancel.click(); cancel.click();
  assert.deepEqual(f.switched, [{ kind: "cancel", operation_id: "op" }]);
  progress({ ...intent, stage: "stopping_current" });
  assert.match(f.surface.element.querySelector(".roster-switch-status").textContent, /Stopping current/);
  assert.equal(f.surface.element.querySelector('[data-action="cancel-switch"]'), null);
  progress({ ...intent, stage: "launching_next" });
  assert.match(f.surface.element.querySelector(".roster-switch-status").textContent, /Launching next/);
  progress({ ...intent, stage: "failed", reason: "Launch failed; saved work is retained" });
  const recovery = f.surface.element.querySelector('[data-action="open-saved-work"]');
  assert.equal(recovery?.textContent, "Open saved work in Works"); recovery.click();
  assert.deepEqual(f.opened, [current], "a pre-stop failure focuses the exact still-live source in Works");
  assert.deepEqual(f.saved, []);
  f.switchModel.update(state => ({ ...state, disconnected: true })); f.flush();
  assert.match(f.surface.element.querySelector(".roster-switch-status").textContent, /Unknown.*Reconnect/s);
  f.switchModel.update(() => ({ ...switchingState(), intents: [{ ...intent, stage: "completed" }], notice: { member_id: "m1", message: "No saved request was observed. Request again to recheck the current member." } })); f.flush();
  const absent = f.surface.element.querySelector(".roster-switch-status");
  assert.equal(absent?.getAttribute("role"), "status");
  assert.equal(absent?.textContent, "No saved request was observed. Request again to recheck the current member.");
  assert.equal(f.surface.element.querySelector('[data-action="switch-next"]').disabled, false);
  f.surface.dispose();
});

test("Project app routes switch receipts and reads switches with Agents refresh without a poller", () => {
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.match(app, /case "agent_switch":\s*applyAgentSwitchSnapshot\(event\.snapshot\)/);
  assert.match(app, /case "agent_switch_error":\s*applyAgentSwitchError\(event\)/);
  assert.match(app, /send\(\{ kind: "agent_switch", action \}\)/);
  assert.match(app, /onRefreshSchedule:.*requestAgentSwitch\(\)/);
  assert.match(app, /if \(host && changed\).*requestAgentSwitch\(\)/);
  assert.match(app, /clearAgentSwitch\(\)/);
});


test("Agents continues only server-eligible saved work through the existing correlated dispatcher", async () => {
  const { createContinueWorkDispatcher, createLaunchPendingController } = await import("../launch-pending-controller.js");
  let dispatcher, pending;
  const f = await fixture({ schedule: planned(), switching: switchingState(), onContinueSavedWork: id => dispatcher.dispatch(id), isContinuePending: id => pending.isPending(`continue:${id}`) });
  const sent = [];
  pending = createLaunchPendingController({ onChange: () => f.surface.sync(), setTimeoutFn: () => 1, clearTimeoutFn() {} });
  dispatcher = createContinueWorkDispatcher({ launchPending: pending, send: action => sent.push(action) });
  const intent = { operation_id: "switch-operation", member_id: "m1", source_issue_number: 4777, target_issue_number: 5002, stage: "unknown", saved_work_id: "opaque-saved-work", can_resume_saved: false };
  const update = value => { f.switchModel.update(state => ({ ...state, intents: [value] })); f.flush(); };
  update(intent);
  assert.equal(f.surface.element.querySelector('[data-action="continue-saved-work"]'), null, "an opaque Work id alone cannot authorize continuation");
  update({ ...intent, saved_work_id: null, can_resume_saved: true });
  assert.equal(f.surface.element.querySelector('[data-action="continue-saved-work"]'), null);
  update({ ...intent, can_resume_saved: true });
  const button = f.surface.element.querySelector('[data-action="continue-saved-work"]');
  assert.equal(button?.textContent, "Continue saved work");
  button.click(); button.click();
  assert.deepEqual(f.continued, ["opaque-saved-work"]);
  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "continue_work"); assert.equal(sent[0].work_id, "opaque-saved-work");
  assert.match(sent[0].operation_id, /^continue-[0-9a-f-]{36}$/i);
  assert.equal(button.disabled, true);
  const outcome = { kind: "continue_work_outcome", operation_id: sent[0].operation_id, work_id: "opaque-saved-work", outcome: "failed", message: "Source authority changed" };
  assert.deepEqual(dispatcher.handleOutcome(outcome), outcome);
  assert.equal(button.disabled, false, "the real outcome settles existing continuation pending state");
  assert.match(f.surface.element.querySelector(".roster-switch-status").textContent, /Unknown/);
  assert.doesNotMatch(f.surface.element.querySelector(".roster-switch-status").textContent, /completed|continued successfully/i);
  update({ ...intent, stage: "launching_next", can_resume_saved: true });
  assert.ok(f.surface.element.querySelector('[data-action="continue-saved-work"]'), "saved source recovery does not require an automatic-exit receipt");
  const app = readFileSync(new URL("../app.js", import.meta.url), "utf8");
  assert.match(app, /onContinueSavedWork:.*continueWorkDispatcher\.dispatch\(workId\)/);
  f.surface.dispose();
});


test("Agents registers an explicit name with its selected catalog tool and preserves rejected drafts", async () => {
  const f = await fixture();
  const select = f.surface.element.querySelector('[aria-label="Tool for new member"]');
  const name = f.surface.element.querySelector('[aria-label="Name for new member"]');
  assert.ok(name, "registration exposes an accessible display name field");
  assert.equal(name.classList.contains("settings-input"), true, "registration reuses the tokenized shared input primitive");
  const register = f.surface.element.querySelector('[data-action="add"]');
  const input = value => { name.value = value; name.dispatchEvent(new f.window.Event("input")); };
  assert.equal(register.disabled, true, "empty names cannot register");
  input("  oNe  ");
  assert.equal(register.disabled, true, "normalized duplicates are rejected");
  assert.equal(name.getAttribute("aria-invalid"), "true");
  input("  Maple  ");
  assert.deepEqual([...select.options].map(option => option.value), ["codex", "claude", "custom-cli"]);
  select.querySelector('[value="custom-cli"]').selected = true;
  register.click();
  assert.deepEqual(f.added, [{ tool: "custom-cli", name: "Maple" }]);
  f.rosterModel.update(state => ({ ...state, error: "Agent roster changed; reload before retrying" })); f.flush();
  assert.equal(name.value, "  Maple  ", "backend rejection retains the draft");
  name.focus(); f.rosterModel.update(state => ({ ...state, revision: state.revision + 1, error: null })); f.flush();
  assert.ok(f.surface.element.querySelector('[aria-label="Name for new member"]') === name && f.document.activeElement === name);
  select.focus();
  assert.ok(f.surface.element.querySelector('[aria-label="Tool for new member"]') === select && f.document.activeElement === select);
  assert.equal(select.value, "custom-cli");
  const preview = f.document.createElement("aside"); f.surface.setPreviewHost(preview);
  assert.equal(preview.querySelector('[aria-label="Name for new member"]').disabled, true);
  assert.equal(register.disabled, false, "a rejected request can be retried");
  f.rosterModel.update(state => ({ ...state, revision: state.revision + 1,
    members: [...state.members, { member_id: "other-writer", name: "Maple", enabled: true }] })); f.flush();
  assert.equal(name.value, "  Maple  ", "a later read must not mistake another writer's member for the rejected registration");
  f.rosterModel.update(state => ({ ...state, error: "Agent roster changed; reload before retrying" })); f.flush();
  input("Cedar"); register.click();
  f.activityModel.update(state => ({ ...state, count: 2 })); f.flush();
  f.rosterModel.update(state => ({ ...state, revision: state.revision + 1, error: null,
    members: [...state.members, { member_id: "retry", name: "Cedar", enabled: true }] })); f.flush();
  assert.equal(name.value, "", "retry success clears the submitted draft after unrelated activity redraw");
});
