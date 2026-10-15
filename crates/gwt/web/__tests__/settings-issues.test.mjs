import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

const source = readFileSync(new URL("../settings-surface.js", import.meta.url), "utf8");
function load(name, bindings = {}) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `missing ${name}`);
  const open = source.indexOf("{", source.indexOf(")", start));
  let depth = 1, end = open + 1;
  while (depth && end < source.length) {
    if (source[end] === "{") depth++;
    if (source[end] === "}") depth--;
    end++;
  }
  return Function(...Object.keys(bindings), `return (${source.slice(start, end)})`)(...Object.values(bindings));
}

test("Issues Settings keeps labels and values together and sends existing project controls", () => {
  const { document, Event } = parseHTML("<html><body><section></section></body></html>");
  const panel = document.querySelector("section"), sent = [];
  load("renderIssuesSettingsPanel")(panel, {
    project: { project_root: "/projects/A", name: "Project A" },
    status: { enabled: true, autonomous_mode: false, max_active_agents: 4,
      max_active_agents_override: 4, terminal_queue_auto_refill: false,
      terminal_queue_auto_refill_limit: 3, allowed_labels: ["auto-merge"] },
    send: message => sent.push(message),
  });
  assert.match(panel.textContent, /Project settings.*Project A/);
  const change = (name, value) => {
    const control = panel.querySelector(`[aria-label="${name}"]`);
    assert.ok(control, name);
    if (control.type === "checkbox") control.checked = value;
    else control.value = value;
    control.dispatchEvent(new Event("change"));
  };
  change("Max active", "6");
  const mode = panel.querySelector('[aria-label="Max active mode"]');
  assert.equal(mode.value, "manual");
  mode.querySelector('[value="auto"]').selected = true;
  mode.dispatchEvent(new Event("change"));
  change("Auto-refill", true);
  change("Refill limit", "5");
  change("Allowed labels", "auto-merge, bug");
  assert.deepEqual(sent, [
    { kind: "set_issue_monitor_max_active_agents", max_active_agents: 6 },
    { kind: "set_issue_monitor_max_active_agents", max_active_agents: null },
    { kind: "set_issue_monitor_auto_refill", enabled: true, limit: 3 },
    { kind: "set_issue_monitor_auto_refill", enabled: true, limit: 5 },
    { kind: "set_issue_monitor_allowed_labels", allowed_labels: ["auto-merge", "bug"] },
  ]);
  assert.equal(panel.querySelector('[aria-label="Autonomous"]'), null, "the summary owns Autonomous once");
});

test("Auto capacity is read-only until Manual is selected", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderIssuesSettingsPanel")(panel, { project: { project_root: "/A" },
    status: { max_active_agents: 4, max_active_agents_override: null }, send: m => sent.push(m) });
  const max = panel.querySelector('[aria-label="Max active"]');
  const mode = panel.querySelector('[aria-label="Max active mode"]');
  assert.equal(max.disabled, true);
  assert.equal(mode.value, "auto");
  mode.querySelector('[value="manual"]').selected = true;
  mode.dispatchEvent(new Event("change"));
  assert.deepEqual(sent, [{ kind: "set_issue_monitor_max_active_agents", max_active_agents: 4 }]);
});

test("Manual starts at one when Auto has no available capacity", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderIssuesSettingsPanel")(panel, { project: { project_root: "/A" },
    status: { max_active_agents: 0, max_active_agents_override: null }, send: m => sent.push(m) });
  const max = panel.querySelector('[aria-label="Max active"]');
  const mode = panel.querySelector('[aria-label="Max active mode"]');
  assert.equal(max.value, "0", "Auto still displays its effective capacity");
  mode.querySelector('[value="manual"]').selected = true;
  mode.dispatchEvent(new Event("change"));
  assert.equal(max.disabled, false);
  assert.equal(max.value, "1");
  assert.deepEqual(sent, [{ kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 }]);
});

test("Agents Settings edits full profiles while preserving runtime fields and project isolation", () => {
  const { document, Event } = parseHTML("<html><body><section id=a></section><section id=b></section></body></html>");
  const a = document.getElementById("a"), b = document.getElementById("b"), sent = [];
  const profiles = [
    { agent_id: "codex", model: "gpt-6", reasoning: "high", runtime_target: "Docker", docker_service: "dev", prefer_for: ["type:fix"] },
    { agent_id: "claude", model: "opus", reasoning: "medium", prefer_for: [] },
  ];
  const render = load("renderAgentsSettingsPanel");
  render(a, { project: { project_root: "/A", name: "A" }, profiles, send: m => sent.push(m) });
  render(b, { project: { project_root: "/B", name: "B" }, profiles: [{ agent_id: "codex", model: "B model" }], send: () => assert.fail("wrong project") });
  const model = a.querySelector('[aria-label="Model codex"]');
  assert.equal(model.value, "gpt-6");
  assert.deepEqual([...a.querySelector('[aria-label="Runtime codex"]').options].map(o => o.value), ["Host", "Docker"]);
  model.value = "new model";
  model.dispatchEvent(new Event("input", { bubbles: true }));
  assert.equal(a.dataset.dirty, "true", "incoming status must not destroy unsaved profile edits");
  a.querySelector('[data-action="save-profiles"]').click();
  assert.equal(sent[0].profiles[0].model, "new model");
  assert.equal(sent[0].profiles[0].docker_service, "dev");
  assert.deepEqual(sent[0].profiles[0].prefer_for, ["type:fix"]);
  assert.equal(b.querySelector('[aria-label="Model codex"]').value, "B model");
  a.querySelector('[aria-label="Move down codex"]').click();
  assert.deepEqual(sent.at(-1).profiles.map(p => p.agent_id), ["claude", "codex"]);
  assert.match(a.textContent, /Candidates/);
  assert.ok(a.querySelector('[aria-label="Prefer for codex"]'));
  assert.ok(a.querySelector('[data-action="add-candidate"]'));
});

test("Candidates use the known catalog, including custom agents, and explain empty or invalid choices", () => {
  const { document } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderAgentsSettingsPanel")(panel, {
    profiles: [{ agent_id: "ClaudeCode" }],
    availableAgents: [{ id: "claude", name: "Claude Code" }, { id: "codex", name: "Codex" }, { id: "team-agent", name: "Team agent" }],
    send: message => sent.push(message),
  });
  const picker = panel.querySelector('[aria-label="Add candidate candidate"]');
  assert.equal(picker.localName, "select");
  assert.deepEqual([...picker.options].map(option => option.value), ["", "codex", "team-agent"]);
  const add = panel.querySelector('[data-action="add-candidate"]');
  add.click();
  assert.match(panel.querySelector('[role="alert"]').textContent, /Select an agent/);
  picker.querySelector('[value="team-agent"]').selected = true;
  add.click();
  assert.deepEqual(sent[0].profiles.map(profile => profile.agent_id), ["ClaudeCode", "team-agent"]);
  panel.querySelector('[aria-label="Agent ClaudeCode"]').value = "Codex";
  picker.querySelector('[value="codex"]').selected = true;
  add.click();
  assert.match(panel.querySelector('[role="alert"]').textContent, /already in the candidate pool/);
  assert.equal(sent.length, 1, "a legacy alias in the draft is already the same provider");
  assert.ok(panel.querySelector(".settings-status"), "feedback uses the shared Settings primitive");
});

test("Candidates disable addition with an accessible reason when the catalog is exhausted", () => {
  const { document } = parseHTML("<section></section>");
  const panel = document.querySelector("section");
  load("renderAgentsSettingsPanel")(panel, {
    profiles: [{ agent_id: "Codex" }], availableAgents: [{ id: "codex", name: "Codex" }],
    send: () => assert.fail("no available candidate"),
  });
  assert.equal(panel.querySelector('[data-action="add-candidate"]').disabled, true);
  assert.match(panel.querySelector('[role="status"]').textContent, /All known agents.*candidate pool/);
});

test("a rejected candidate save shows its reason inline and preserves the draft", () => {
  const { document } = parseHTML('<div id="settings"><section data-settings-candidates data-saving="true" data-dirty="true" data-pending-request="1"><input value="draft model"><p data-candidate-status></p></section></div>');
  const body = document.getElementById("settings");
  body.dataset.settingsProjectRoot = "/A";
  const apply = load("renderCandidateSaveError");
  const panel = body.querySelector("section");
  apply(panel, { request_id: 2, message: "Unrelated save rejected" });
  assert.equal(panel.dataset.saving, "true", "a stale failure must not cancel this save");
  panel.dataset.saving = "false"; // Editing while the save is in flight keeps the pending request.
  apply(panel, { request_id: 1, message: "Candidate save rejected" });
  assert.equal(panel.dataset.saving, "false");
  assert.equal(panel.dataset.dirty, "true");
  assert.equal(panel.querySelector("input").value, "draft model");
  assert.equal(panel.querySelector('[role="alert"]').textContent, "Candidate save rejected");
  assert.match(readFileSync(new URL("../app.js", import.meta.url), "utf8"), /case "issue_monitor_profiles_write_failed":\s*applyIssueMonitorProfilesError\(event\)/);
});

test("section-addressed Settings selects and focuses the requested project panel", () => {
  const { document, CustomEvent } = parseHTML('<html><body><div id="settings"><button class="settings-tab" data-settings-tab="issues"></button><section class="settings-panel" data-settings-panel="issues"></section></div></body></html>');
  const body = document.getElementById("settings");
  body.dataset.settingsProjectRoot = "/A";
  body.dataset.settingsWindowId = "settings-A";
  const unbound = body.cloneNode(true);
  delete unbound.dataset.settingsProjectRoot;
  document.body.prepend(unbound);
  const panel = body.querySelector("section");
  let selectedSection;
  body.addEventListener("settings:section-changed", event => { selectedSection = event.detail; });
  let focused = 0, scrollOptions, spawned = 0;
  panel.focus = () => focused++;
  panel.scrollIntoView = options => { scrollOptions = options; };
  const start = source.indexOf("export function switchSettingsTab");
  const end = source.indexOf("export function createSettingsSurface", start);
  const install = Function(`${source.slice(start, end).replaceAll("export ", "")}; return installSettingsOpenHandler;`)();
  install({ document, settingsWindowBodies: new Set([unbound, body]), activeProjectTab: () => ({ project_root: "/A" }), focusOrSpawnPreset: () => spawned++, setPendingTarget: () => assert.fail("existing window") });
  document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "issues", project_root: "/A" } }));
  assert.equal(body.querySelector("button").getAttribute("aria-selected"), "true");
  assert.equal(focused, 1);
  assert.deepEqual(scrollOptions, { block: "start" });
  assert.equal(spawned, 1);
  assert.deepEqual(selectedSection, { windowId: "settings-A", section: "issues" });
});

test("Settings declares app/project scope and the global update condition using existing primitives", () => {
  assert.match(source, /buildSettingsTab\("issues", "Issues"/);
  assert.match(source, /settings-issues-options/);
  assert.match(source, /mountIssues\(windowData, issuesHost\)/);
  assert.match(source, /aria-orientation.*vertical/);
  assert.match(source, /Application settings/);
  assert.match(source, /Auto-apply updates/);
  assert.match(source, /at least one open project.*Autonomous/);
  assert.match(source, /set_auto_apply_updates/);
  assert.doesNotMatch(source, /set_issue_monitor_auto_apply_updates/);
  assert.doesNotMatch(source, /style\.cssText\s*=/);
});

test("System update settings show saved and effective values and the blocking drain", () => {
  const { document } = parseHTML('<html><body><section></section></body></html>');
  const panel = document.querySelector("section");
  const render = load("renderUpdateSettingsStatus");
  render(panel, { auto_apply_updates: null, auto_apply_updates_effective: false,
    update_drain: { version: "9.120.0", blocking: [
      { kind: "active_pane", label: "Fix #4271", state: "Running" },
      { kind: "pending_acquire_claim", issue_number: 12 },
      { kind: "active_execution", worktree: "/work/issue-13" },
      { kind: "held_verification_lease", lease_id: "verification-14" },
    ] } });
  assert.match(panel.textContent, /Saved: Default \(Off\).*Effective: Off/);
  assert.match(panel.textContent, /Update 9\.120\.0 pending/);
  assert.equal(panel.querySelectorAll("li").length, 4);
  for (const expected of ["Fix #4271", "#12", "/work/issue-13", "verification-14"]) {
    assert.ok(panel.textContent.includes(expected), expected);
  }
  render(panel, { auto_apply_updates: true, auto_apply_updates_effective: true, update_drain: null });
  assert.match(panel.textContent, /Saved: On.*Effective: On.*No update pending/);
  assert.equal(panel.querySelectorAll("li").length, 0);
});

test("failed update preference save restores the last authoritative toggle value", () => {
  let state = { auto_apply_updates: true, auto_apply_updates_effective: true, loaded: true, drains: {} };
  const { document } = parseHTML('<input type="checkbox">');
  const toggle = document.querySelector("input");
  toggle.checked = false;
  const globalUpdateModel = { update(reducer) {
    state = reducer(state);
    toggle.checked = state.auto_apply_updates_effective;
  } };
  load("applyGlobalUpdateSettings", { globalUpdateModel })({ kind: "system_settings_error" });
  assert.equal(toggle.checked, true, "failed OFF must not look saved");
  assert.equal(state.auto_apply_updates, true);
  assert.match(readFileSync(new URL("../app.js", import.meta.url), "utf8"), /case "system_settings_error":\s*applyGlobalUpdateSettings\(event\)/);
});
