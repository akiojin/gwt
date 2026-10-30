// SPEC-1933 Phase: System Settings (Output Language) — DOM / CSS structure
//
// Settings ウィンドウのタブ化と System タブの Language select が
// Operator Design tokens を使い、つぎはぎ実装になっていないことを
// CSS とレンダラのソースパターンで検証する（kanban-structure と同じ手法）。
//
// SPEC-3064 Phase 3 (E4): the Settings window renderer moved from app.js to
// settings-surface.js. Renderer/source patterns are pinned against the
// extracted module; receive() dispatch case arms stay pinned to app.js.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import { createUiStateStore } from "../ui-state-store.js";
import { createInteractionGuard } from "../interaction-guard.js";
import { bindControlExplanations, setControlExplanation } from "../control-explanations.js";
import { renderCustomAgentEnvEditor } from "../custom-agent-env-editor.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const settingsSource = readFileSync(resolve(here, "../settings-surface.js"), "utf8");
const componentsCss = readFileSync(
  resolve(here, "../styles/components.css"),
  "utf8",
);

function extractFunctionSource(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `missing function ${name}`);
  const braceStart = source.indexOf("{", start);
  assert.notEqual(braceStart, -1, `missing body for ${name}`);
  let depth = 0;
  for (let i = braceStart; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "{") depth += 1;
    if (ch === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(start, i + 1);
    }
  }
  assert.fail(`unterminated function ${name}`);
}

function loadFunction(name) {
  return Function(`"use strict"; return (${extractFunctionSource(settingsSource, name)});`)();
}

function loadFunctionWithDeps(name, deps) {
  const dependencySources = deps
    .map((dep) => extractFunctionSource(settingsSource, dep))
    .join("\n");
  return Function(
    `"use strict"; ${dependencySources}; return (${extractFunctionSource(settingsSource, name)});`,
  )();
}

function createSettingsHarness({ scheduleFrame, onSend } = {}) {
  const { document, window } = parseHTML("<html><body><main></main></body></html>");
  const sent = [];
  const frames = new Set();
  const flush = () => { for (const callback of [...frames]) { frames.delete(callback); callback(); } };
  const frameScheduler = scheduleFrame || (callback => { frames.add(callback); return () => frames.delete(callback); });
  const rosterModel = createUiStateStore({ revision: 1, members: [
    { member_id: "member-codex", name: "Codex member", agent_id: "codex", enabled: true },
    { member_id: "member-custom", name: "Custom member", agent_id: "fixture-cli", enabled: true },
  ] }, { scheduleFrame: frameScheduler });
  const factory = Function("document", "window", "createInteractionGuard", "createUiStateStore", "setControlExplanation", "bindControlExplanations",
    "renderIndexSettingsPanel", "renderCustomAgentEnvEditor", `${settingsSource
      .replace(/^import .*;\r?\n/gm, "")
      .replace(/^export /gm, "")}\nreturn createSettingsSurface;`)(
    document, window, createInteractionGuard, createUiStateStore, setControlExplanation, bindControlExplanations, () => {}, renderCustomAgentEnvEditor,
  );
  const surface = factory({
    scheduleFrame: frameScheduler,
    rosterModel,
    send: message => { onSend?.(message); sent.push(message); },
    createNode(tag, className, text) {
      const node = document.createElement(tag);
      if (className) node.className = className;
      if (text) node.textContent = text;
      return node;
    },
    focusWindowLocally() {},
    activeProjectTab: () => ({ project_root: "/projects/A", title: "Project A" }),
    focusOrSpawnPreset() {}, renderUsagePanel() {},
    indexStatusByProjectRoot: new Map(), pmSettingsPanel: { mount() {} },
    mountIssues(_windowData, host) {
      host.innerHTML = '<div class="issue-bridge-root"><div class="knowledge-monitor-bar"></div><div class="workspace-toolbar"></div><button data-action="monitor-settings"></button></div>';
    },
  });
  const body = document.querySelector("main");
  return { surface, body, document, window, sent, flush };
}

test("Custom Agent replies commit immutable snapshots shared by all mounts and registration catalogs", () => {
  const frames = new Set();
  const flush = () => { for (const callback of [...frames]) { frames.delete(callback); callback(); } };
  const { surface, body, document, sent } = createSettingsHarness({ scheduleFrame(callback) {
    frames.add(callback);
    return () => frames.delete(callback);
  } });
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const second = document.createElement("main");
  document.body.appendChild(second);
  surface.renderSettingsWindow(second, { id: "settings-second", title: "Settings" });
  surface.applySupportedAgentList({ agents: [{ id: "codex", name: "Codex" }] });
  surface.applyIssueMonitorProfiles({ project_root: "/projects/A", profiles: [] });
  const initial = surface.customAgentsModel.read();
  surface.applyCustomAgentsReceiveEvent({ kind: "custom_agent_error", code: "unavailable", message: "Catalog unavailable" });
  assert.ok(body.querySelector('[data-settings-candidates] option[value="member-codex"]'),
    "registered candidate identities do not wait for a CLI catalog read");
  const agent = { id: "fixture-cli", display_name: "Fixture CLI", command: "fixture", env: { TOKEN: "***REDACTED***" } };
  surface.applyCustomAgentsReceiveEvent({ kind: "custom_agent_list", agents: [agent] });
  const loaded = surface.customAgentsModel.read();
  agent.env.TOKEN = "mutated";
  assert.equal(initial.catalog.loading, true);
  assert.deepEqual(initial.catalog.agents, []);
  assert.equal(loaded.catalog.agents[0].env.TOKEN, "***REDACTED***");
  assert.equal(loaded.catalog.loading, false);
  flush();
  for (const host of [body, second]) {
    assert.match(host.querySelector('[data-role="settings-scroll"]').textContent, /Fixture CLI/);
    assert.ok(host.querySelector('[data-settings-candidates] option[value="member-custom"]'));
  }
  const scrolls = [body, second].map(host => host.querySelector('[data-role="settings-scroll"]'));
  const renders = [0, 0];
  scrolls.forEach((scroll, index) => {
    const append = scroll.appendChild.bind(scroll);
    scroll.appendChild = node => {
      if (node.tagName === "BUTTON") renders[index]++;
      return append(node);
    };
  });
  for (let index = 0; index < 50; index++) {
    surface.applyCustomAgentsReceiveEvent({ kind: "custom_agent_list", agents: [
      { ...loaded.catalog.agents[0], display_name: `Burst ${index}` },
    ] });
  }
  assert.equal(surface.customAgentsModel.read().catalog.agents[0].display_name, "Burst 49");
  assert.match(body.querySelector('[data-settings-candidates]').textContent, /Custom member/,
    "candidate names remain registered identities while CLI display metadata changes");
  assert.deepEqual(renders, [0, 0], "only the Custom Agents list waits for the frame");
  assert.equal(frames.size, 1, "the burst reserves one frame");
  flush();
  assert.deepEqual(renders, [1, 1], "both mounts draw the latest shared snapshot once");
  for (const scroll of scrolls) assert.match(scroll.textContent, /Burst 49/);
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "ja" });
  flush();
  assert.deepEqual(renders, [1, 1], "an unrelated Settings store does not rebuild the list");
  body.querySelector('[aria-label="Edit agent environment"]').click();
  const draft = body.querySelector('[aria-label="Environment value"]');
  draft.value = "unsaved-draft";
  const reads = sent.filter(message => message.kind === "list_custom_agents").length;
  surface.resyncSettings();
  surface.resyncSettings();
  assert.ok(body.querySelector('[aria-label="Environment value"]') === draft,
    "recovery reads preserve the existing environment input");
  assert.equal(draft.value, "unsaved-draft");
  assert.equal(sent.filter(message => message.kind === "list_custom_agents").length, reads + 2);
  assert.equal(sent.filter(message => message.kind === "update_custom_agent").length, 0);
  body.querySelector('[aria-label="Cancel environment editing"]').click();
  body.querySelector('[aria-label="Edit agent environment"]').click();
  body.querySelector('[aria-label="Environment value"]').value = "saved-value";
  body.querySelector('[aria-label="Save environment changes"]').click();
  assert.equal(body.querySelector('[aria-label="Save environment changes"]') === null, true,
    "saving closes the editor synchronously before a second click can resend it");
  const saves = sent.filter(message => message.kind === "update_custom_agent");
  assert.equal(saves.length, 1);
  assert.equal(saves[0].agent.env.TOKEN, "saved-value");
  body.querySelector('[aria-label="Edit agent environment"]').click();
  const nextDraft = body.querySelector('[aria-label="Environment value"]');
  nextDraft.value = "fresh-unsaved-draft";
  flush();
  assert.equal(nextDraft.isConnected, true, "an already rendered save snapshot cannot replace a reopened editor");
  assert.equal(nextDraft.value, "fresh-unsaved-draft");
  body.querySelector('[aria-label="Cancel environment editing"]').click();
  surface.applyCustomAgentsReceiveEvent({ kind: "custom_agent_saved", agent: { ...agent, display_name: "Updated CLI", env: {} } });
  assert.equal(loaded.catalog.agents[0].display_name, "Fixture CLI", "past snapshot stays immutable");
  assert.equal(surface.customAgentsModel.read().catalog.agents.length, 1, "save replaces the existing agent");
  surface.applyCustomAgentsReceiveEvent({ kind: "custom_agent_saved", agent: { id: "other", command: "other" } });
  assert.equal(surface.customAgentsModel.read().catalog.agents.length, 2, "save inserts a new agent");
  surface.applyCustomAgentsReceiveEvent({ kind: "backend_connection_result", models: ["fixture-model"] });
  flush();
  for (const host of [body, second]) {
    assert.match(host.querySelector('[data-role="settings-scroll"]').textContent, /Updated CLI/);
    assert.match(host.querySelector('[data-role="settings-scroll"]').textContent, /returned 1 model/);
  }
  const detachedText = second.textContent;
  second.remove();
  surface.applyCustomAgentsReceiveEvent({ kind: "custom_agent_deleted", agent_id: "fixture-cli" });
  assert.equal(second.textContent, detachedText, "detached mount is no longer rendered");
  assert.ok(body.querySelector('[data-settings-candidates] option[value="member-custom"]'),
    "removing a CLI catalog entry does not replace or remove its registered member");
  surface.applyCustomAgentsReceiveEvent({ kind: "custom_agent_error", code: "invalid", message: "Fixture rejected" });
  assert.equal(surface.customAgentsModel.read().catalog.loading, false);
  const reopened = document.createElement("main");
  document.body.appendChild(reopened);
  surface.renderSettingsWindow(reopened, { id: "settings-reopened", title: "Settings" });
  assert.match(reopened.querySelector('[data-role="settings-scroll"]').textContent, /Fixture rejected/);
  assert.match(reopened.querySelector('[data-role="settings-scroll"]').textContent, /other/);
  flush();
  assert.match(body.querySelector('[data-role="settings-scroll"]').textContent, /Error \[invalid\]: Fixture rejected/);
  assert.equal(second.textContent, detachedText, "pending rendering excludes the detached mount");
});

test("Agent Backend replies commit immutable per-agent catalogs for all Settings mounts", () => {
  const frames = new Set();
  const flush = () => { for (const callback of [...frames]) { frames.delete(callback); callback(); } };
  const { surface, body, document, sent } = createSettingsHarness({ scheduleFrame(callback) {
    frames.add(callback);
    return () => frames.delete(callback);
  } });
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const second = document.createElement("main");
  document.body.appendChild(second);
  surface.renderSettingsWindow(second, { id: "settings-second", title: "Settings" });
  const panels = [body, second].map(host => host.querySelector('[data-settings-panel="agent-backends"]'));
  const renders = [0, 0];
  panels.forEach((panel, index) => {
    const append = panel.appendChild.bind(panel);
    panel.appendChild = node => {
      if (node.dataset.agent === "claudeCode") renders[index]++;
      return append(node);
    };
  });
  assert.equal(typeof surface.agentBackendsModel?.read, "function", "backend catalog must expose immutable snapshots");
  const initial = surface.agentBackendsModel.read();
  const profile = { id: "fixture", display_name: "Fixture backend", base_url: "https://example.invalid", model: "fixture-model" };
  surface.applyAgentBackendsReceiveEvent({ kind: "agent_backend_list", agent: "claudeCode", backends: [profile] });
  const loaded = surface.agentBackendsModel.read();
  profile.display_name = "mutated outside the model";
  assert.equal(loaded.backends.claudeCode[0].display_name, "Fixture backend");
  assert.deepEqual(initial.backends.claudeCode, []);
  surface.applyAgentBackendsReceiveEvent({ kind: "agent_backend_list", agent: "codex", backends: [{ id: "codex-fixture" }] });
  surface.applyAgentBackendsReceiveEvent({ kind: "agent_backend_saved", agent: "claudeCode", profile: { ...profile, display_name: "Updated backend" } });
  assert.equal(surface.agentBackendsModel.read().backends.claudeCode.length, 1);
  assert.equal(loaded.backends.claudeCode[0].display_name, "Fixture backend", "upsert must preserve previous snapshots");
  assert.equal(surface.customAgentsModel.read().statusMessage, 'Saved backend "fixture" for claudeCode.',
    "save feedback remains synchronous");
  assert.deepEqual(renders, [0, 0], "model updates defer only the Backend presentation");
  for (let index = 0; index < 50; index++) {
    surface.applyAgentBackendsReceiveEvent({ kind: "agent_backend_list", agent: "claudeCode",
      backends: [{ ...profile, display_name: "Updated backend", model: `burst-${index}` }] });
  }
  assert.equal(surface.agentBackendsModel.read().backends.claudeCode[0].model, "burst-49");
  assert.ok(frames.size > 0, "the burst waits for scheduled presentation");
  flush();
  assert.deepEqual(renders, [1, 1], "each mount draws the latest shared snapshot once");
  for (const host of [body, second]) {
    const panel = host.querySelector('[data-settings-panel="agent-backends"]');
    assert.match(panel.textContent, /Updated backend/);
    assert.match(panel.textContent, /codex-fixture/);
    assert.match(panel.textContent, /burst-49/);
  }
  const detachedText = second.textContent;
  second.remove();
  surface.applyAgentBackendsReceiveEvent({ kind: "agent_backend_deleted", agent: "claudeCode", id: "fixture" });
  assert.equal(second.textContent, detachedText, "detached mounts no longer render");
  assert.equal(surface.agentBackendsModel.read().backends.codex.length, 1, "delete preserves the other agent");
  assert.equal(surface.customAgentsModel.read().statusMessage, 'Deleted backend "fixture" for claudeCode.');
  surface.applyAgentBackendsReceiveEvent({ kind: "agent_backend_error", message: "Backend unavailable" });
  assert.equal(surface.customAgentsModel.read().statusMessage, "Backend unavailable", "errors keep the existing shared status channel");
  surface.resyncSettings();
  assert.ok(sent.some(message => message.kind === "list_agent_backends" && message.agent === "codex"));
  const reopened = document.createElement("main");
  document.body.appendChild(reopened);
  surface.renderSettingsWindow(reopened, { id: "settings-reopened", title: "Settings" });
  assert.match(reopened.querySelector('[data-settings-panel="agent-backends"]').textContent, /codex-fixture/);
  assert.match(reopened.querySelector('[data-settings-panel="agent-backends"]').textContent, /No Claude Code backend profiles/,
    "a late mount reads current state before the pending frame");
  flush();
  assert.deepEqual(renders, [2, 1], "a detached mount is excluded from pending rendering");
  assert.equal(second.textContent, detachedText);
  surface.applyAgentBackendsReceiveEvent({ kind: "agent_backend_error", message: "Still unavailable" });
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "ja" });
  flush();
  assert.deepEqual(renders, [2, 1], "shared status feedback and unrelated Settings updates do not redraw Backends");
  assert.match(body.querySelector('[data-settings-panel="agent-backends"]').textContent, /No Claude Code backend profiles/);
});

test("Update status batches snapshots while toggles and failed-save rollback remain synchronous", () => {
  const frames = new Set();
  const flush = () => { for (const callback of [...frames]) { frames.delete(callback); callback(); } };
  const { surface, body, document } = createSettingsHarness({ scheduleFrame(callback) {
    frames.add(callback);
    return () => frames.delete(callback);
  } });
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const second = document.createElement("main");
  document.body.appendChild(second);
  surface.renderSettingsWindow(second, { id: "settings-second", title: "Settings" });
  second.dataset.settingsProjectRoot = "/projects/B";
  const panels = [body, second].map(host => host.querySelector("[data-update-settings-status]"));
  const toggles = [body, second].map(host => host.querySelector('[aria-label="Auto-apply updates"]'));
  const renders = [0, 0];
  panels.forEach((panel, index) => {
    const replace = panel.replaceChildren.bind(panel);
    panel.replaceChildren = (...nodes) => { renders[index]++; return replace(...nodes); };
  });
  const status = { auto_apply_updates: true, auto_apply_updates_effective: true, update_drain: null };
  surface.applyGlobalUpdateSettings({ status, project_root: "/projects/A" });
  for (const toggle of toggles) {
    assert.equal(toggle.checked, true);
    assert.equal(toggle.disabled, false);
  }
  assert.deepEqual(renders, [0, 0], "only status rendering waits for a frame");
  flush();
  assert.deepEqual(renders, [1, 1]);
  renders.fill(0);
  for (let index = 1; index <= 50; index++) {
    surface.applyGlobalUpdateSettings({ project_root: "/projects/A", status: { ...status,
      update_drain: { version: `fixture-${index}`, blocking: [] },
    } });
  }
  assert.equal(frames.size, 1);
  assert.deepEqual(renders, [0, 0]);
  const detachedText = panels[1].textContent;
  const late = document.createElement("main");
  document.body.appendChild(late);
  surface.renderSettingsWindow(late, { id: "settings-late", title: "Settings" });
  const latePanel = late.querySelector("[data-update-settings-status]");
  assert.match(latePanel.textContent, /fixture-50/, "late mounts read the current snapshot synchronously");
  const lateNode = latePanel.firstChild;
  flush();
  assert.deepEqual(renders, [1, 0], "another project's drain cannot redraw this status");
  assert.match(panels[0].textContent, /fixture-50/);
  assert.equal(latePanel.firstChild, lateNode, "pending frames do not redraw a hydrated mount");
  assert.equal(panels[1].textContent, detachedText);
  toggles[0].checked = false;
  surface.applyGlobalUpdateSettings({ kind: "system_settings_error" });
  assert.equal(toggles[0].checked, true, "a same-value error restores the authoritative toggle immediately");
  flush();
  assert.deepEqual(renders, [1, 0], "same-value rollback does not rebuild status");
  second.remove();
  surface.applyGlobalUpdateSettings({ project_root: "/projects/B", status: { ...status,
    update_drain: { version: "detached", blocking: [] },
  } });
  flush();
  assert.deepEqual(renders, [1, 0], "detached and unrelated mounts are not rendered");
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "ja" });
  flush();
  assert.deepEqual(renders, [1, 0], "another Settings store does not render update status");
});

test("System replies advance one immutable snapshot for all mounts, including while guarded", () => {
  const { surface, body, document, flush } = createSettingsHarness();
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const second = document.createElement("main");
  document.body.appendChild(second);
  surface.renderSettingsWindow(second, { id: "settings-second", title: "Settings" });
  const initial = surface.systemSettingsModel.read();
  const resource = { enabled: true, preset: "custom", priority: "idle", cpu_limit_percent: 35, build_jobs: 2 };
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "en", board_provider: "slack", agent_resource: resource });
  flush();
  const loaded = surface.systemSettingsModel.read();
  resource.build_jobs = 99;
  assert.equal(loaded.agentResource.buildJobs, 2, "wire objects cannot mutate committed state");
  assert.equal(initial.loaded, false);
  for (const host of [body, second]) {
    assert.equal(host.querySelector('[id$="-settings-system-codex-hooks"]').checked, true);
    assert.equal(host.querySelector('[id$="-settings-system-agent-build-jobs"]').value, "2");
  }
  const select = body.querySelector('[id$="-settings-system-language"]');
  surface.systemSettingsInteractionGuard.activate();
  surface.applySystemSettingsEvent({ kind: "system_settings_updated", language: "ja", codex_trust_managed_hooks: false });
  flush();
  surface.applyAutostartStatus({ enabled: true });
  surface.applyBoardAuthStatus({ slack: true });
  flush();
  assert.equal(surface.systemSettingsModel.read().language, "ja", "guard delays DOM only");
  assert.equal(body.querySelector('[id$="-settings-system-language"]'), select);
  assert.equal(body.querySelector('[id$="-settings-system-codex-hooks"]').checked, true);
  assert.equal(loaded.language, "en", "an earlier snapshot stays immutable");
  surface.systemSettingsInteractionGuard.release();
  for (const host of [body, second]) {
    assert.equal(host.querySelector('[id$="-settings-system-codex-hooks"]').checked, false);
    assert.equal(host.querySelector("[data-role='system-settings-status']").textContent, "Saved system settings.");
  }
  const detached = second.innerHTML;
  second.remove();
  surface.applySystemSettingsEvent({ kind: "system_settings_error", message: "Denied" });
  flush();
  assert.equal(body.querySelector("[data-role='system-settings-status']").textContent, "Denied");
  assert.equal(second.innerHTML, detached, "removed mounts no longer render");
});

test("System snapshots batch each mounted panel while local controls and autostart stay synchronous", () => {
  const frames = new Set();
  const flush = () => { for (const callback of [...frames]) { frames.delete(callback); callback(); } };
  const { surface, body, document, window } = createSettingsHarness({
    scheduleFrame(callback) { frames.add(callback); return () => frames.delete(callback); },
    onSend(message) {
      if (message.kind === "update_autostart") {
        for (const host of [body, second]) assert.equal(host.querySelector('[id$="-settings-system-autostart"]').disabled, true);
      }
    },
  });
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const second = document.createElement("main");
  document.body.appendChild(second);
  surface.renderSettingsWindow(second, { id: "second", title: "Settings" });
  surface.applyAutostartStatus({ enabled: false });
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "en" });
  surface.resyncSettings();
  flush();
  const initial = surface.systemSettingsModel.read();
  const panels = [body, second].map(host => host.querySelector('[data-system-settings-content]'));
  const statuses = panels.map(panel => panel.querySelector('[data-role="system-settings-status"]'));
  const draws = [0, 0];
  let prototype = statuses[0];
  while (!Object.hasOwn(prototype, "textContent")) prototype = Object.getPrototypeOf(prototype);
  const descriptor = Object.getOwnPropertyDescriptor(prototype, "textContent");
  statuses.forEach((status, index) => Object.defineProperty(status, "textContent", {
    get() { return descriptor.get.call(this); },
    set(value) { draws[index]++; descriptor.set.call(this, value); },
  }));
  for (let index = 1; index <= 50; index++) {
    surface.applySystemSettingsEvent({ kind: "system_settings", language: index === 50 ? "ja" : "en",
      agent_resource: { build_jobs: index },
    });
  }
  assert.equal(surface.systemSettingsModel.read().agentResource.buildJobs, 50);
  assert.equal(initial.language, "en");
  assert.equal(initial.agentResource.buildJobs, null);
  assert.deepEqual(draws, [0, 0], "scalar patches must wait for the frame too");
  assert.equal(frames.size, 1);
  const late = document.createElement("main");
  document.body.appendChild(late);
  surface.renderSettingsWindow(late, { id: "late", title: "Settings" });
  assert.equal(late.querySelector('[id$="-settings-system-agent-build-jobs"]').value, "50");
  const lateNode = late.querySelector("[data-system-settings-content]").firstChild;
  flush();
  assert.equal(late.querySelector("[data-system-settings-content]").firstChild, lateNode);
  assert.deepEqual(draws, [1, 1]);
  for (const panel of panels) assert.equal(panel.querySelector('[id$="-settings-system-agent-build-jobs"]').value, "50");
  surface.applyIssueMonitorProfiles({ project_root: "/projects/A", profiles: [] });
  flush();
  assert.deepEqual(draws, [1, 1], "an unrelated store does not render System");
  const resource = body.querySelector('[id$="-settings-system-agent-resource-enabled"]');
  resource.checked = false;
  resource.dispatchEvent(new window.Event("change"));
  assert.equal(body.querySelector('[id$="-settings-system-agent-build-jobs"]').disabled, true);
  const syncDraws = [...draws];
  flush();
  assert.deepEqual(draws, syncDraws, "a frame does not repeat a synchronous local render");
  surface.applyAutostartStatus({ enabled: false });
  const autostart = body.querySelector('[id$="-settings-system-autostart"]');
  autostart.checked = true;
  autostart.dispatchEvent(new window.Event("change"));
  surface.applyAutostartError({ message: "Rejected" });
  assert.equal(autostart.checked, false);
  assert.equal(autostart.disabled, false);
  assert.equal(body.querySelector('[data-role="autostart-settings-status"]').textContent, "Rejected");
  const restoredDraws = [...draws];
  flush();
  assert.deepEqual(draws, restoredDraws);
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "en" });
  second.remove();
  flush();
  assert.equal(draws[1], restoredDraws[1], "a detached mount is not rendered");
});

test("System local resource edits use current model values and preserve wire validation", () => {
  const { surface, body, window, sent, flush } = createSettingsHarness();
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "ja", agent_resource: {
    enabled: true, preset: "custom", priority: "idle", cpu_limit_percent: 35, build_jobs: 2,
  } });
  flush();
  const previous = surface.systemSettingsModel.read();
  const jobs = body.querySelector('[id$="-settings-system-agent-build-jobs"]');
  const trust = body.querySelector('[id$="-settings-system-codex-hooks"]');
  surface.systemSettingsInteractionGuard.activate();
  jobs.value = "4";
  jobs.dispatchEvent(new window.Event("change"));
  assert.equal(body.querySelector("[data-role='system-settings-status']").textContent, "Saving…",
    "safe status feedback remains visible while destructive rendering is guarded");
  assert.deepEqual(sent.at(-1), { kind: "update_system_settings", language: "ja", agent_resource: {
    enabled: true, preset: "custom", priority: "idle", cpu_limit_percent: 35, build_jobs: 4,
  } });
  assert.equal(previous.agentResource.buildJobs, 2);
  surface.applySystemSettingsEvent({ kind: "system_settings_error", message: "Denied while focused" });
  flush();
  assert.equal(body.querySelector("[data-role='system-settings-status']").textContent, "Denied while focused");
  surface.systemSettingsInteractionGuard.release();
  assert.ok(body.querySelector('[id$="-settings-system-codex-hooks"]') === trust,
    "flushing a local input change must retain its neighboring click target");
  trust.checked = false;
  trust.dispatchEvent(new window.Event("change"));
  assert.deepEqual(sent.at(-1), { kind: "update_system_settings", language: "ja", codex_trust_managed_hooks: false });
  const cpu = body.querySelector('[id$="-settings-system-agent-cpu-limit"]');
  const count = sent.length;
  cpu.value = "101";
  cpu.dispatchEvent(new window.Event("change"));
  assert.equal(sent.length, count);
  assert.equal(surface.systemSettingsModel.read().agentResource.cpuLimitPercent, 35);
  assert.equal(surface.systemSettingsModel.read().statusKind, "error");
});

test("Board auth snapshots jointly render Settings and survive guarded System refreshes", () => {
  const { surface, body, document, flush } = createSettingsHarness();
  surface.applySystemSettingsEvent({ kind: "system_settings", board_provider: "slack" });
  flush();
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const second = document.createElement("main");
  document.body.appendChild(second);
  surface.renderSettingsWindow(second, { id: "settings-second", title: "Settings" });
  const previous = surface.systemSettingsModel.read();
  surface.applyBoardAuthStatus({ slack: true, slack_client_id: "client-a", message: "Connected" });
  flush();
  const signedIn = surface.systemSettingsModel.read();
  assert.ok(Object.isFrozen(signedIn.boardConfig));
  assert.equal(previous.boardAuth.slack, false, "later replies cannot mutate earlier snapshots");
  for (const host of [body, second]) {
    assert.equal(host.querySelector(".board-auth-status").dataset.signedIn, "true");
    assert.equal(host.querySelector('input[id$="-settings-board-slack-client-id"]').value, "client-a");
  }
  surface.systemSettingsInteractionGuard.activate();
  surface.applySystemSettingsEvent({ kind: "system_settings_updated", language: "ja" });
  flush();
  surface.applyBoardAuthStatus({ slack: false, slack_client_id: "client-b" });
  flush();
  assert.equal(surface.systemSettingsModel.read().boardAuth.slack, false,
    "the shared model updates while a dropdown is open");
  assert.equal(body.querySelector(".board-auth-status").dataset.signedIn, "true");
  surface.systemSettingsInteractionGuard.release();
  assert.equal(surface.systemSettingsModel.read().language, "ja", "the queued System reply still flushes");
  assert.equal(signedIn.boardAuth.slack, true);
  surface.renderSystemPanelInAllSettingsWindows();
  for (const host of [body, second]) {
    assert.equal(host.querySelector(".board-auth-status").dataset.signedIn, "false");
    assert.equal(host.querySelector('input[id$="-settings-board-slack-client-id"]').value, "client-b");
  }
  second.remove();
  surface.applyBoardAuthStatus({ slack: true });
  flush();
  assert.equal(second.querySelector(".board-auth-status").dataset.signedIn, "false",
    "detached mounts are removed from the subscription");
});

test("Autostart snapshots jointly render mounted Settings and remain immutable across replies", () => {
  const { surface, body, document, window } = createSettingsHarness();
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const second = document.createElement("main");
  document.body.appendChild(second);
  surface.renderSettingsWindow(second, { id: "settings-second", title: "Settings" });
  const snapshot = surface.systemSettingsModel.read();
  assert.ok(Object.isFrozen(snapshot));
  surface.applyAutostartStatus({ enabled: false });
  for (const host of [body, second]) {
    assert.equal(host.querySelector('input[id$="-settings-system-autostart"]').disabled, false);
  }
  const checkbox = body.querySelector('input[id$="-settings-system-autostart"]');
  checkbox.checked = true;
  checkbox.dispatchEvent(new window.Event("change"));
  const pending = surface.systemSettingsModel.read();
  assert.equal(pending.autostartPending, true);
  for (const host of [body, second]) {
    assert.equal(host.querySelector('input[id$="-settings-system-autostart"]').checked, true);
    assert.equal(host.querySelector('[data-role="autostart-settings-status"]').textContent, "Saving…");
  }
  surface.applyAutostartError({ message: "Could not save" });
  assert.equal(pending.autostartPending, true, "later replies cannot mutate an earlier snapshot");
  assert.equal(snapshot.autostartChecking, true);
  for (const host of [body, second]) {
    assert.equal(host.querySelector('input[id$="-settings-system-autostart"]').checked, false);
    assert.equal(host.querySelector('[data-role="autostart-settings-status"]').textContent, "Could not save");
  }
  second.remove();
  surface.systemSettingsInteractionGuard.activate();
  surface.applyAutostartStatus({ enabled: true }, "Saved login launch setting.");
  assert.equal(surface.systemSettingsModel.read().autostartEnabled, true,
    "the model advances even while destructive rendering waits");
  assert.equal(body.querySelector('input[id$="-settings-system-autostart"]').checked, false);
  surface.systemSettingsInteractionGuard.release();
  assert.equal(body.querySelector('input[id$="-settings-system-autostart"]').checked, true);
  assert.equal(body.querySelector('[data-role="autostart-settings-status"]').textContent, "Saved login launch setting.");
  surface.systemSettingsInteractionGuard.activate();
  surface.applySystemSettingsEvent({ kind: "system_settings_updated",
    language: "ja", codex_trust_managed_hooks: false, board_provider: "teams",
  });
  surface.renderSystemPanelInAllSettingsWindows();
  surface.applyAutostartStatus({ enabled: false });
  surface.systemSettingsInteractionGuard.release();
  assert.equal(surface.systemSettingsModel.read().language, "ja",
    "a subscribed refresh must not replace another System reply waiting in the shared guard");
  assert.equal(surface.systemSettingsModel.read().codexTrustManagedHooks, false);
  assert.equal(surface.systemSettingsModel.read().boardProvider, "teams");
  assert.equal(body.querySelector('input[id$="-settings-system-autostart"]').checked, false);
});

test("Issue #5531: mounted Settings recovers all lost reads without replaying writes or replacing input", () => {
  const { surface, body, document, sent } = createSettingsHarness();
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  assert.equal(sent.filter(message => message.kind === "get_autostart_status").length, 1);
  assert.equal(surface.systemSettingsModel.read().autostartChecking, true);
  assert.equal(body.querySelector('[data-role="autostart-settings-status"]').textContent,
    "Checking login launch status…");
  const secondBody = document.createElement("main");
  document.body.appendChild(secondBody);
  surface.renderSettingsWindow(secondBody, { id: "settings-second", title: "Settings" });
  assert.equal(sent.filter(message => message.kind === "get_autostart_status").length, 1,
    "another mount does not duplicate a pending status read");
  secondBody.remove();
  const draft = body.querySelector('[aria-label="Allowed labels"]');
  draft.value = "keep-my-draft";
  assert.equal(surface.customAgentsModel.read().catalog.loading, true);
  sent.length = 0;
  surface.resyncSettings();
  assert.deepEqual(sent, [
    { kind: "list_issue_monitor" }, { kind: "get_issue_monitor_profiles" },
    { kind: "agent_roster", action: { kind: "read" }, expected_revision: null },
    { kind: "list_supported_agents" }, { kind: "get_system_settings" },
    { kind: "get_update_settings" }, { kind: "get_board_auth_status" },
    { kind: "get_autostart_status" }, { kind: "list_custom_agents" },
    { kind: "list_agent_backends", agent: "claudeCode" },
    { kind: "list_agent_backends", agent: "codex" },
  ]);
  assert.equal(body.querySelector('[aria-label="Allowed labels"]'), draft);
  assert.equal(draft.value, "keep-my-draft");
  sent.length = 0;
  body.remove();
  surface.resyncSettings();
  assert.deepEqual(sent, [{ kind: "get_system_settings" }], "only shared Works presentation settings are read without a Settings mount");
});

test("Issue #5531: recovered System snapshots preserve unsaved provider and OAuth inputs", () => {
  const { surface, body, window, flush } = createSettingsHarness();
  surface.applySystemSettingsEvent({ kind: "system_settings", board_provider: "slack" });
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const secret = body.querySelector('input[type="password"]');
  const port = body.querySelector(".board-oauth-port-form input");
  secret.value = "unsaved-secret";
  secret.dispatchEvent(new window.Event("input", { bubbles: true }));
  port.value = "9876";
  port.dispatchEvent(new window.Event("input", { bubbles: true }));
  surface.resyncSettings();
  surface.applyBoardAuthStatus({ oauth_redirect_port: 8765 });
  flush();
  assert.equal(body.querySelector('input[type="password"]'), secret);
  assert.equal(secret.value, "unsaved-secret");
  assert.equal(body.querySelector(".board-oauth-port-form input"), port);
  assert.equal(port.value, "9876");
  body.querySelector(".board-config-form button").click();
  body.querySelector(".board-oauth-port-form button").click();
  surface.renderSystemPanelInAllSettingsWindows();
  assert.notEqual(body.querySelector('input[type="password"]'), secret,
    "explicit saves allow the next authoritative snapshot to render");
  assert.equal(body.querySelector('input[type="password"]').value, "");
});

test("Issue #5531: dirty provider drafts keep autostart pending and error state synchronized", () => {
  const { surface, body, window, sent } = createSettingsHarness();
  surface.applySystemSettingsEvent({ kind: "system_settings", board_provider: "slack" });
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  surface.applyAutostartStatus({ enabled: false });
  surface.applySystemSettingsEvent({ kind: "system_settings_updated" });
  surface.renderSystemPanelInAllSettingsWindows();
  const secret = body.querySelector('input[type="password"]');
  secret.value = "unsaved-secret";
  secret.dispatchEvent(new window.Event("input", { bubbles: true }));
  const port = body.querySelector(".board-oauth-port-form input");
  port.value = "9876";
  port.dispatchEvent(new window.Event("input", { bubbles: true }));
  const autostart = body.querySelector('input[id$="-settings-system-autostart"]');
  autostart.checked = true;
  autostart.dispatchEvent(new window.Event("change"));
  const pendingDisabled = autostart.disabled;
  const status = body.querySelector('[data-role="autostart-settings-status"]');
  assert.equal(status.textContent, "Saving…");
  sent.length = 0;
  surface.resyncSettings();
  assert.equal(sent.filter(message => message.kind === "get_autostart_status").length, 1);
  assert.equal(sent.some(message => message.kind === "update_autostart"), false,
    "recovery probes status without retrying the pending mutation");
  assert.equal(surface.systemSettingsModel.read().autostartPending, true);
  surface.applyAutostartError({ message: "Login launch could not be saved." });
  assert.equal(autostart.checked, false, "a failed write restores the authoritative checkbox value");
  assert.equal(autostart.disabled, false, "the error releases the pending checkbox");
  assert.equal(pendingDisabled, true, "the checkbox is disabled while its write is pending");
  assert.equal(status.textContent, "Login launch could not be saved.");
  assert.equal(status.dataset.kind, "error");
  assert.equal(body.querySelector('[data-role="system-settings-status"]').textContent,
    "Saved system settings.");
  surface.resyncSettings();
  assert.equal(autostart.disabled, true, "a recovery query disables the known checkbox until its reply");
  assert.equal(status.textContent, "Checking login launch status…");
  surface.applyAutostartStatus({ enabled: true });
  assert.equal(autostart.checked, true);
  assert.equal(autostart.disabled, false);
  assert.equal(status.textContent, "");
  assert.equal(status.dataset.kind, undefined);
  assert.equal(body.querySelector('input[type="password"]'), secret);
  assert.equal(secret.value, "unsaved-secret");
  assert.equal(port.value, "9876");
});

test("Supported Agents renders the catalog, identifiers, versions, and install actions", () => {
  const { document } = parseHTML("<html><body><section></section></body></html>");
  const panel = document.querySelector("section");
  const render = loadFunctionWithDeps("renderSupportedAgentsPanel", ["supportedAgentVersion"]);
  render(panel, [
    { id: "claude", name: "Claude Code", installed: true, installed_version: "2.1.0" },
    { id: "codex", name: "Codex", installed: true, installed_version: null },
    { id: "grok", name: "Grok Build", installed: false, installed_version: null, install_supported: true },
  ]);
  const rows = panel.querySelectorAll("[data-agent-id]");
  assert.equal(rows.length, 3);
  assert.match(rows[0].textContent, /Claude Code.*Installed.*2\.1\.0/);
  assert.equal(rows[1].children[2].textContent, "Unknown");
  assert.match(rows[1].children[2].title, /version unavailable/i);
  assert.match(rows[2].textContent, /Grok Build.*Not installed/);
  assert.equal(rows[2].children[2].textContent, "Not installed");
  assert.match(rows[0].textContent, /\(claude\)/);
  assert.equal(rows[2].querySelector("button")?.getAttribute("aria-label"), "Install Grok Build");
  assert.equal(panel.querySelectorAll("th[scope='col']").length, 5);
  render(panel, null);
  assert.match(panel.textContent, /Loading agent detection/);
  assert.equal(panel.querySelectorAll("[data-agent-id]").length, 0);
});

test("Issue #5594 normalizes the five reported CLI outputs without losing diagnostic details", () => {
  const normalize = loadFunction("supportedAgentVersion");
  const examples = [
    ["2.1.301 (Claude Code)", "2.1.301"],
    ["codex-cli 0.162.1", "0.162.1"],
    ["grok 1.0.46 (2765805b9442) [stable]", "1.0.46"],
    ["OpenClaw 2026.3.23-2 (7ffe7e4)", "2026.3.23-2"],
    ["Unknown (version unavailable)", null],
  ];
  for (const [raw, version] of examples) assert.equal(normalize(raw), version, raw);
  const { document } = parseHTML("<section></section>");
  const panel = document.querySelector("section");
  const render = loadFunctionWithDeps("renderSupportedAgentsPanel", ["supportedAgentVersion"]);
  render(panel, examples.map(([raw], index) => ({
    id: String(index), name: "Agent", installed: true, installed_version: raw,
    update_check_error: "Release channel cannot be verified. Run the native updater manually.",
  })));
  for (const [index, row] of [...panel.querySelectorAll("tbody tr")].entries()) {
    assert.equal(row.children[2].textContent, examples[index][1] || "Unknown");
    assert.equal(row.children[2].title, examples[index][0]);
    assert.equal(row.children[3].textContent, "Unknown");
    assert.match(row.children[3].title, /Release channel cannot be verified/);
  }
});

test("Supported Agents sends maintenance requests and exposes failures and automatic update conditions", () => {
  const { document, Event } = parseHTML("<html><body><section></section></body></html>");
  const panel = document.querySelector("section");
  const render = loadFunctionWithDeps("renderSupportedAgentsPanel", ["supportedAgentVersion"]);
  const messages = [];
  const agents = [
    { id: "opencode", name: "OpenCode", installed: false, install_supported: true },
    { id: "grok", name: "Grok Build", installed: true, installed_version: "1.0.0", available_version: "2.0.0", update_available: true },
  ];
  const options = { send: (message) => messages.push(message), autoUpdate: false };
  render(panel, agents, options);
  const install = panel.querySelector('[aria-label="Install OpenCode"]');
  assert.ok(install, "a missing supported agent needs an Install action");
  assert.equal(install.textContent, "Install");
  assert.equal(panel.querySelector('[aria-label="Update Grok Build"]').textContent, "Update");
  install.click();
  panel.querySelector('[aria-label="Update Grok Build"]').click();
  const automatic = panel.querySelector('[aria-label="Automatically update agents"]');
  assert.equal(automatic.checked, false);
  automatic.checked = true;
  automatic.dispatchEvent(new Event("change"));
  assert.deepEqual(messages, [
    { kind: "maintain_supported_agent", agent_id: "opencode", action: "install" },
    { kind: "maintain_supported_agent", agent_id: "grok", action: "update" },
    { kind: "set_agent_auto_update", enabled: true },
  ]);
  assert.match(panel.textContent, /next.*startup/i);
  assert.match(panel.textContent, /installed agents with a verified update route/i);
  assert.match(panel.textContent, /agent windows/i);
  const supportedAgentsModel = createUiStateStore({
    agents, autoUpdate: false, maintenanceByAgent: {},
  });
  const handlers = Function("supportedAgentsModel", "send", `
    ${extractFunctionSource(settingsSource, "supportedAgentPanelOptions")}
    ${extractFunctionSource(settingsSource, "applySupportedAgentList")}
    ${extractFunctionSource(settingsSource, "applySupportedAgentMaintenance")}
    return { supportedAgentPanelOptions, applySupportedAgentList, applySupportedAgentMaintenance };
  `)(supportedAgentsModel, message => messages.push(message));
  let renders = 0;
  supportedAgentsModel.subscribe(state => state, state => {
    renders += 1;
    render(panel, state.agents, handlers.supportedAgentPanelOptions());
  });
  handlers.applySupportedAgentMaintenance({ agent_id: "opencode", pending: true, message: "Installing OpenCode…" });
  assert.equal(panel.querySelector('[aria-label="Install OpenCode"]').disabled, true);
  assert.equal(panel.querySelector('[aria-label="Update Grok Build"]').disabled, true);
  handlers.applySupportedAgentList({ agents, auto_update: false, maintenance_pending: false });
  assert.equal(panel.querySelector('[aria-label="Install OpenCode"]').disabled, false,
    "an authoritative snapshot recovers a missed completion notification");
  handlers.applySupportedAgentMaintenance({ agent_id: "",
    pending: false, success: false, message: "Cannot save automatic updates. Check configuration permissions and retry.",
  });
  assert.equal(panel.querySelector('[aria-label="Automatically update agents"]').checked, false);
  assert.match(panel.querySelector('[role="alert"]').textContent, /Cannot save automatic updates/);
  const retry = panel.querySelector('[aria-label="Automatically update agents"]');
  retry.checked = true;
  const rendersBeforeRetry = renders;
  retry.dispatchEvent(new Event("change"));
  assert.equal(renders, rendersBeforeRetry + 1, "synchronous model subscribers see the local draft");
  assert.equal(supportedAgentsModel.read().autoUpdate, false, "confirmed settings wait for the backend");
  assert.equal(supportedAgentsModel.read().autoUpdateDraft, true);
  assert.equal(retry.checked, true);
  handlers.applySupportedAgentList({ agents, auto_update: true, maintenance_pending: false });
  assert.ok(!panel.querySelector('[role="alert"]'), "a successful retry clears the prior setting-save error");
  assert.equal(panel.querySelector('[aria-label="Automatically update agents"]').checked, true);
  handlers.applySupportedAgentMaintenance({ agent_id: "opencode", pending: false, success: false, message: "Registry unavailable. Check your connection and retry." });
  assert.match(panel.querySelector('[role="alert"]').textContent, /Check your connection and retry/);
  assert.equal(panel.querySelector('[role="alert"]').title, "Registry unavailable. Check your connection and retry.");
  assert.equal(panel.querySelector('[aria-label="Install OpenCode"]').disabled, false);
  agents[1].installed_version = "grok 2.0.0";
  agents[1].update_available = false;
  agents[1].up_to_date = true;
  handlers.applySupportedAgentList({ agents, auto_update: true, maintenance_pending: false });
  assert.match(panel.querySelector('[data-agent-id="grok"]').textContent, /Latest/);
  assert.equal(panel.querySelector('[aria-label="Update Grok Build"]'), null);
});

test("Supported Agents batches catalogs while local operations and authoritative recovery stay current", () => {
  const frames = new Set();
  const flush = () => { for (const callback of [...frames]) { frames.delete(callback); callback(); } };
  const { surface, body, document, window, sent } = createSettingsHarness({
    scheduleFrame(callback) { frames.add(callback); return () => frames.delete(callback); },
    onSend(message) {
      if (message.kind === "maintain_supported_agent") {
        for (const host of [body, second]) {
          assert.equal(host.querySelector('[aria-label="Install OpenCode"]').disabled, true,
            "all mounts disable maintenance actions before sending");
        }
      }
    },
  });
  surface.renderSettingsWindow(body, { id: "supported-first", title: "Settings" });
  const second = document.createElement("main");
  document.body.appendChild(second);
  surface.renderSettingsWindow(second, { id: "supported-second", title: "Settings" });
  surface.applyCustomAgentsReceiveEvent({ kind: "custom_agent_list", agents: [] });
  surface.applyIssueMonitorProfiles({ project_root: "/projects/A", profiles: [] });
  const agents = [{ id: "opencode", name: "OpenCode", installed: false, install_supported: true }];
  surface.applySupportedAgentList({ agents, auto_update: false });
  flush();
  const panels = [body, second].map(host => host.querySelector('[data-settings-panel="supported-agents"]'));
  const tables = panels.map(panel => panel.querySelector("table"));
  const draws = [0, 0];
  panels.forEach((panel, index) => {
    const append = panel.appendChild.bind(panel);
    panel.appendChild = node => { if (node.tagName === "TABLE") draws[index]++; return append(node); };
  });
  for (let index = 0; index < 50; index++) {
    surface.applySupportedAgentList({ agents: [{ ...agents[0], name: `OpenCode ${index}` }], auto_update: false });
  }
  assert.deepEqual(draws, [0, 0], "catalog burst leaves DOM unchanged until the frame");
  assert.equal(panels[0].querySelector("table"), tables[0]);
  assert.equal(frames.size, 1);
  assert.equal(surface.supportedAgentsModel.read().agents[0].name, "OpenCode 49",
    "registration catalog readers observe the latest immutable tool snapshot before paint");
  assert.match(body.querySelector('[data-settings-candidates]').textContent, /Codex member/,
    "candidate membership stays independent of provider catalog metadata");
  // The pending snapshot must not overwrite a later local checkbox edit.
  const automatic = panels[0].querySelector('[aria-label="Automatically update agents"]');
  automatic.checked = true;
  automatic.dispatchEvent(new window.Event("change"));
  assert.deepEqual(draws, [0, 0], "local toggle commits without synchronous DOM reconstruction");
  flush();
  assert.deepEqual(draws, [1, 1]);
  for (const panel of panels) {
    assert.match(panel.textContent, /OpenCode 49/);
    assert.equal(panel.querySelector('[aria-label="Automatically update agents"]').checked, true);
  }
  assert.equal(panels[0].querySelector('[aria-label="Automatically update agents"]'), automatic);
  surface.applySupportedAgentMaintenance({ agent_id: "", pending: false, success: false, message: "Save denied" });
  flush();
  assert.equal(automatic.checked, false, "global failure alone rolls back the unconfirmed toggle");
  assert.match(panels[0].querySelector('[role="alert"]').textContent, /Save denied/);
  surface.applySupportedAgentList({ agents, auto_update: false });
  // A late mount draws current state synchronously and does not skip pending siblings.
  const late = document.createElement("main");
  document.body.appendChild(late);
  surface.renderSettingsWindow(late, { id: "supported-late", title: "Settings" });
  const lateTable = late.querySelector('[data-settings-panel="supported-agents"] table');
  assert.match(lateTable.textContent, /OpenCode/);
  flush();
  assert.equal(late.querySelector('[data-settings-panel="supported-agents"] table'), lateTable);
  assert.equal(panels[0].querySelector('[role="alert"]'), null);
  panels[0].querySelector('[aria-label="Install OpenCode"]').click();
  const before = [...draws];
  flush();
  assert.deepEqual(draws, before, "the frame skips the snapshot already rendered before maintenance send");
  assert.deepEqual(sent.filter(message => message.kind === "maintain_supported_agent"), [
    { kind: "maintain_supported_agent", agent_id: "opencode", action: "install" },
  ]);
  second.remove();
  const detachedDraws = draws[1];
  surface.applySupportedAgentList({ agents, auto_update: true });
  flush();
  assert.equal(draws[1], detachedDraws);
  assert.equal(automatic.checked, true);
  const beforeUnrelated = [...draws];
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "ja" });
  flush();
  assert.deepEqual(draws, beforeUnrelated);
});

test("Supported Agents uses the shared Settings tab and refreshes from its backend snapshot", () => {
  assert.match(settingsSource, /buildSettingsTab\("supported-agents",\s*"Supported Agents"/);
  assert.match(settingsSource, /linkSettingsPanel\(panelSupportedAgents, windowData\.id, "supported-agents"\)/);
  assert.match(settingsSource, /send\(\{\s*kind:\s*"list_supported_agents"\s*\}\)/);
  assert.match(appSource, /case "supported_agent_list":[\s\S]{0,200}applySupportedAgentList\(event\)/);
  assert.match(appSource, /case "supported_agent_maintenance":[\s\S]{0,200}applySupportedAgentMaintenance\(event\)/);
  assert.match(extractFunctionSource(appSource, "handleSocketOpen"), /requestSupportedAgentRefresh\(\)/);
  const messages = [];
  const bodies = new Set();
  const refresh = Function("settingsWindowBodies", "purgeDetachedSettingsBodies", "send", `
    ${extractFunctionSource(settingsSource, "requestSupportedAgentRefresh")}
    return requestSupportedAgentRefresh;
  `)(bodies, () => {}, message => messages.push(message));
  refresh();
  assert.equal(messages.length, 0, "reconnect without Settings does not probe agent metadata");
  bodies.add({});
  refresh();
  assert.deepEqual(messages, [{ kind: "list_supported_agents" }]);
});

test("Supported Agents table uses Operator tokens in both themes", () => {
  const table = componentsCss.match(/:root\[data-theme\]\s+\.settings-supported-agents\s*\{([^}]*)\}/);
  assert.ok(table, "Supported Agents table must use the shared dual-theme scope");
  assert.match(table[1], /var\(--font-body\)/);
  assert.match(table[1], /var\(--color-text\)/);
  const cells = componentsCss.match(/\.settings-supported-agents td\s*\{([^}]*)\}/);
  assert.ok(cells, "Supported Agents cells must have token-based spacing");
  assert.match(cells[1], /var\(--space-/);
  assert.match(cells[1], /var\(--color-border\)/);
  assert.doesNotMatch(table[1] + cells[1], /#[\da-f]{3,8}\b|\brgba?\(/i);
  assert.match(table[1], /table-layout:\s*fixed/);
  assert.match(cells[1], /white-space:\s*nowrap/);
  assert.match(cells[1], /text-overflow:\s*ellipsis/);
  const button = componentsCss.match(/\.settings-supported-agents \.wizard-button\s*\{([^}]*)\}/);
  assert.ok(button, "action buttons need a shared width and one-line labels");
  assert.match(button[1], /width:/);
  assert.match(button[1], /white-space:\s*nowrap/);
  assert.doesNotMatch(button[1], /#[\da-f]{3,8}\b|\brgba?\(/i);
});

test("two Settings System panels keep labels associated with controls in their own window", () => {
  const { document } = parseHTML(`<main><section id="settings-left-system"></section><section id="settings-right-system"></section></main>`);
  const scopeSettingsControlIds = loadFunction("scopeSettingsControlIds");
  const panels = [...document.querySelectorAll("section")];
  for (const panel of panels) {
    panel.innerHTML = '<label for="settings-system-language">Output Language</label><select id="settings-system-language"><option>English</option></select>';
    scopeSettingsControlIds(panel);
    const target = panel.querySelector("label").getAttribute("for");
    assert.equal(document.getElementById(target), panel.querySelector("select"));
  }
  assert.notEqual(panels[0].querySelector("select").id, panels[1].querySelector("select").id);
});

test("components.css declares the settings tab and panel surfaces under :root[data-theme]", () => {
  // Every Settings primitive must live inside the dual-theme scope so it
  // honors Dark Operator / Light Drafting alongside the rest of the shell.
  for (const klass of [
    ".settings-root",
    ".settings-toolbar",
    ".settings-heading",
    ".settings-tabs",
    ".settings-tab",
    ".settings-panel",
    ".settings-section",
    ".settings-select",
    ".settings-checkbox-label",
    ".settings-checkbox",
    ".settings-help",
    ".settings-status",
  ]) {
    const pattern = new RegExp(
      `:root\\[data-theme\\]\\s+${klass.replace(".", "\\.")}\\b`,
    );
    assert.match(
      componentsCss,
      pattern,
      `expected components.css to define ${klass} under :root[data-theme]`,
    );
  }
});

test("settings-tab uses Operator color/space tokens (no hardcoded colors)", () => {
  // Pull the .settings-tab declaration block and verify it threads design
  // tokens, not raw color literals — this is the contract that prevents
  // patchwork-style additions from drifting away from the design system.
  const block = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-tab\s*\{([\s\S]*?)\}/,
  );
  assert.ok(block, "expected .settings-tab block in components.css");
  const body = block[1];
  assert.match(body, /var\(--color-surface-elevated\)/);
  assert.match(body, /var\(--color-border\)/);
  assert.match(body, /var\(--color-text\)/);
  assert.match(body, /var\(--font-body\)/);
  assert.match(body, /var\(--radius-md\)/);
  // No raw hex / rgb literals snuck in.
  assert.doesNotMatch(body, /#[0-9a-fA-F]{3,8}\b/, "no hex color literals");
  assert.doesNotMatch(body, /\brgb\(/, "no raw rgb() literals");
});

test("Settings keeps grouped navigation beside a flexible content pane", () => {
  const toolbarBlock = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-toolbar\s*\{([\s\S]*?)\}/,
  );
  assert.ok(toolbarBlock, "expected .settings-toolbar block in components.css");
  assert.match(
    toolbarBlock[1],
    /flex-wrap:\s*wrap/,
    "settings toolbar must wrap instead of clipping the tab strip",
  );

  const tabsBlock = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-tabs\s*\{([\s\S]*?)\}/,
  );
  assert.ok(tabsBlock, "expected .settings-tabs block in components.css");
  assert.match(tabsBlock[1], /flex-direction:\s*column/);
  assert.match(tabsBlock[1], /min-width:\s*0/);
  assert.match(tabsBlock[1], /overflow-y:\s*auto/);
  assert.match(componentsCss, /\.settings-layout\s*\{[^}]*grid-template-columns:[^}]*minmax\(0, 1fr\)/);
  assert.match(componentsCss, /\.settings-field\s*\{[^}]*align-items:\s*center/);
  assert.match(componentsCss, /\.settings-body\s*\{[^}]*overflow:\s*auto/);
  assert.match(componentsCss, /@media\s*\(max-width:\s*600px\)[\s\S]*?\.settings-layout\s*\{[^}]*grid-template-columns:\s*minmax\(0, 1fr\)/);
});

test("settings-tab[aria-selected=true] uses --color-state-active tint", () => {
  // The active state must inherit the Operator accent so the tab strip
  // picks up the same accent as Project Bar / Sidebar Layers / etc.
  const block = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-tab\[aria-selected="true"\]\s*\{([\s\S]*?)\}/,
  );
  assert.ok(
    block,
    "expected .settings-tab[aria-selected=true] block in components.css",
  );
  assert.match(block[1], /var\(--color-state-active\)/);
});

test("settings-select uses --color-focus-ring on focus-visible", () => {
  // Focus visibility must reuse the same accent as the rest of the app.
  const block = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-select:focus-visible\s*\{([\s\S]*?)\}/,
  );
  assert.ok(block, "expected .settings-select:focus-visible block");
  assert.match(block[1], /var\(--color-focus-ring\)/);
});

test("renderSettingsWindow builds a tablist toolbar with System, Project Manager, and Custom Agents tabs", () => {
  // The renderer must declare role="tablist" so assistive tech sees the tab
  // group, and emit both data-settings-tab values that switchSettingsTab
  // expects.
  const renderSource = extractFunctionSource(settingsSource, "renderSettingsWindow");
  assert.match(renderSource, /toolbar\.className\s*=\s*"settings-toolbar"/);
  assert.match(renderSource, /tabs\.setAttribute\("role",\s*"tablist"\)/);
  assert.match(renderSource, /buildSettingsTab\("system",\s*"System"/);
  assert.match(
    renderSource,
    /buildSettingsTab\(\s*"project-manager",\s*"Project Manager"/,
  );
  assert.match(
    renderSource,
    /buildSettingsTab\("custom-agents",\s*"Custom Agents"/,
  );
});

test("Project Manager tab mounts the shared PM editor controller", () => {
  const { document } = parseHTML('<!doctype html><body><div id="settings-body"></div></body>');
  const settingsBody = document.getElementById("settings-body");
  const mounted = [];
  const pmSettingsPanel = {
    mount(node) {
      mounted.push(node);
    },
  };
  const mountProjectManagerSettingsPanel = loadFunction(
    "mountProjectManagerSettingsPanel",
  );

  mountProjectManagerSettingsPanel(document, settingsBody, pmSettingsPanel);

  assert.equal(mounted.length, 1);
  assert.equal(mounted[0].dataset.settingsPanel, "project-manager");
  assert.equal(mounted[0].parentElement, settingsBody);

  const renderSource = extractFunctionSource(settingsSource, "renderSettingsWindow");
  const mountSource = extractFunctionSource(
    settingsSource,
    "mountProjectManagerSettingsPanel",
  );
  assert.match(
    renderSource,
    /mountProjectManagerSettingsPanel\(\s*document,\s*bodyEl,\s*pmSettingsPanel,\s*windowData\.id,\s*\)/,
    "renderSettingsWindow must mount the shared PM editor at its render boundary",
  );
  assert.doesNotMatch(
    `${renderSource}\n${mountSource}`,
    /set_pm_(?:auto_start|launch_profile|loop_interval)/,
    "Settings render and mount helpers must not duplicate PM event construction",
  );
});

test("Project Manager tab and panel expose an explicit accessible relationship", () => {
  const { document } = parseHTML('<!doctype html><body><div id="settings-body"></div></body>');
  const settingsBody = document.getElementById("settings-body");
  const mountProjectManagerSettingsPanel = loadFunction(
    "mountProjectManagerSettingsPanel",
  );

  const panel = mountProjectManagerSettingsPanel(
    document,
    settingsBody,
    null,
    "window-7",
  );

  assert.equal(panel.id, "settings-window-7-panel-project-manager");
  assert.equal(
    panel.getAttribute("aria-labelledby"),
    "settings-window-7-tab-project-manager",
  );

  const tabSource = extractFunctionSource(settingsSource, "buildSettingsTab");
  assert.match(tabSource, /btn\.id\s*=\s*settingsTabId\(windowId,\s*id\)/);
  assert.match(
    tabSource,
    /btn\.setAttribute\("aria-controls",\s*settingsPanelId\(windowId,\s*id\)\)/,
  );
});

test("settings:open routes to the Project Manager tab and frames an existing Settings window", () => {
  const { document } = parseHTML(`<!doctype html><body>
    <div id="settings-body">
      <button class="settings-tab active" data-settings-tab="system" aria-selected="true"></button>
      <button class="settings-tab" data-settings-tab="project-manager" aria-selected="false"></button>
      <section class="settings-panel" data-settings-panel="system"></section>
      <section class="settings-panel hidden" data-settings-panel="project-manager"></section>
    </div>
  </body>`);
  const settingsBody = document.getElementById("settings-body");
  const framed = [];
  const installSettingsOpenHandler = loadFunctionWithDeps(
    "installSettingsOpenHandler",
    ["switchSettingsTab"],
  );

  installSettingsOpenHandler({
    document,
    settingsWindowBodies: new Set([settingsBody]),
    focusOrSpawnPreset: (preset) => framed.push(preset),
    setPendingTarget: () => assert.fail("an existing Settings window must not pend"),
  });
  document.dispatchEvent(
    new document.defaultView.CustomEvent("settings:open", {
      detail: { target: "project-manager" },
    }),
  );

  assert.equal(
    settingsBody.querySelector('[data-settings-tab="project-manager"]')
      .getAttribute("aria-selected"),
    "true",
  );
  assert.equal(
    settingsBody.querySelector('[data-settings-panel="project-manager"]')
      .classList.contains("hidden"),
    false,
  );
  assert.equal(
    settingsBody.querySelector('[data-settings-panel="system"]')
      .classList.contains("hidden"),
    true,
  );
  assert.deepEqual(framed, ["settings"]);
});

test("System tab Language select offers Auto / English / 日本語", () => {
  // SPEC-1933 NFR-005: Language select labels are the single approved
  // exception to the English-only UI rule. Verify all three options exist
  // and use the expected raw values.
  for (const opt of [
    /value:\s*"auto",\s*text:\s*"Auto \(OS locale\)"/,
    /value:\s*"en",\s*text:\s*"English"/,
    /value:\s*"ja",\s*text:\s*"日本語"/,
  ]) {
    assert.match(
      settingsSource,
      opt,
      `Language select missing option: ${opt}`,
    );
  }
});

test("renderSystemPanel sends update_system_settings on select change", () => {
  // The select listener must propagate the new raw value to the backend
  // via the WebSocket protocol (SPEC-1933 FR-007). Verify both the kind
  // and the payload field name.
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"update_system_settings",\s*language:\s*next\s*\}\)/,
    "expected select onChange to send update_system_settings",
  );
});

test("renderSystemPanel exposes Codex managed hook trust opt-in", () => {
  assert.match(
    settingsSource,
    /trustText\.textContent\s*=\s*"Trust gwt-managed Codex hooks"/,
    "expected System tab Codex hook trust checkbox label",
  );
  assert.match(
    settingsSource,
    /codexTrustManagedHooks:\s*true/,
    "expected Codex hook trust UI state to default on",
  );
  assert.match(
    settingsSource,
    /trustCheckbox\.checked\s*=\s*state\.codexTrustManagedHooks\s*!==\s*false/,
    "expected unchecked only when config explicitly disables Codex hook trust",
  );
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"update_system_settings",\s*language:\s*systemSettingsModel\.read\(\)\.language\s*\|\|\s*"auto",\s*codex_trust_managed_hooks:\s*next,\s*\}\)/,
    "expected checkbox onChange to send codex_trust_managed_hooks",
  );
});

test("System tab Board provider select offers Local/Slack/Teams as selectable (SPEC-2963)", () => {
  // SPEC-2963 Phase 5: slack/teams are now real, selectable options (sign-in
  // gated rather than disabled "coming soon").
  assert.match(
    settingsSource,
    /id\s*=\s*"settings-system-board-provider"/,
    "expected a Board provider select with the canonical id",
  );
  for (const provider of ["local", "slack", "teams"]) {
    assert.match(
      settingsSource,
      new RegExp(`value:\\s*"${provider}"`),
      `${provider} must be a Board provider option`,
    );
  }
});

test("System tab exposes remote provider sign-in affordance + auth status (SPEC-2963)", () => {
  assert.match(settingsSource, /"board-auth-status"/, "expected an auth status element");
  assert.match(
    settingsSource,
    /kind:\s*"board_provider_sign_in",\s*provider:\s*selectedProvider/,
    "Sign in button must send board_provider_sign_in",
  );
  assert.match(
    settingsSource,
    /kind:\s*"board_provider_sign_out",\s*provider:\s*selectedProvider/,
    "Sign out button must send board_provider_sign_out",
  );
  assert.match(
    settingsSource,
    /kind:\s*"get_board_auth_status"/,
    "panel must fetch board auth status",
  );
  assert.match(
    appSource,
    /case\s+"board_auth_status":/,
    "dispatch must handle board_auth_status",
  );
});

test("System tab exposes remote provider config form that saves via update_board_provider_config (SPEC-2963)", () => {
  // FR-006: client_id / default_channel / tenant_id / secret editable from the
  // UI instead of hand-editing config.toml.
  assert.match(
    settingsSource,
    /board-config-form/,
    "expected a provider config form container",
  );
  assert.match(
    settingsSource,
    /"settings-board-slack-client-id"/,
    "Slack client id input must exist",
  );
  assert.match(
    settingsSource,
    /"settings-board-slack-secret"/,
    "Slack client secret input must exist",
  );
  assert.match(
    settingsSource,
    /"settings-board-teams-tenant-id"/,
    "Teams tenant id input must exist",
  );
  assert.match(
    settingsSource,
    /kind:\s*"update_board_provider_config"/,
    "Save button must send update_board_provider_config",
  );
  // The secret must only be sent when the user typed one (empty box keeps the
  // stored secret), so it is conditional rather than always included.
  assert.match(
    settingsSource,
    /secretInput\.value\.length\s*>\s*0/,
    "secret must be sent only when the user typed a value",
  );
  // Dispatch must store the non-secret config view for prefill.
  assert.match(
    settingsSource,
    /slackClientId:\s*event\.slack_client_id/,
    "dispatch must prefill slack client id from board_auth_status",
  );
  // The secret field clears after Save by design; an explicit saved-state note
  // must make persistence obvious instead of looking like data loss.
  assert.match(
    settingsSource,
    /board-secret-state/,
    "a client-secret saved-state indicator must exist",
  );
  assert.match(
    settingsSource,
    /A client secret is saved/,
    "the saved-state indicator must confirm the secret persisted",
  );
});

test("System tab accepts a Teams channel link without exposing Team ID fields (SPEC-2963 Phase 9)", () => {
  assert.match(
    settingsSource,
    /function\s+parseTeamsChannelLink\(/,
    "expected a helper that parses a copied Teams channel link",
  );
  assert.match(
    settingsSource,
    /function\s+composeTeamsDefaultChannel\(/,
    "expected the parsed Teams link to save as team_id/channel_id",
  );
  assert.match(
    settingsSource,
    /function\s+formatTeamsChannelLink\(/,
    "expected saved team_id/channel_id config to render back into the link field",
  );
  assert.match(
    settingsSource,
    /"settings-board-teams-channel-link"/,
    "Teams channel link paste input must exist",
  );
  assert.doesNotMatch(
    settingsSource,
    /"settings-board-teams-team-id"/,
    "Teams Team ID must not be exposed as a separate input",
  );
  assert.doesNotMatch(
    settingsSource,
    /"settings-board-teams-channel-id"/,
    "Teams Channel ID must not be exposed as a separate input",
  );
  assert.match(
    settingsSource,
    /const\s+parsedTeamsChannel\s*=\s*parseTeamsChannelLink\(\s*teamsChannelLinkValue\s*,?\s*\)/,
    "Save must parse the Teams channel link",
  );
  assert.match(
    settingsSource,
    /default_channel:\s*nextTeamsDefaultChannel/,
    "Save payload must use the parsed or preserved Teams channel",
  );
  assert.match(
    settingsSource,
    /let\s+nextTeamsDefaultChannel\s*=\s*cfg\.teamsDefaultChannel\s*\|\|\s*""/,
    "saving client/tenant settings without a new link must preserve the existing channel",
  );
  assert.match(
    settingsSource,
    /formatTeamsChannelLink\(\s*cfg\.teamsDefaultChannel,\s*cfg\.teamsTenantId,?\s*\)/,
    "saved channel must remain visible in the Teams channel link field after rerender",
  );
});

test("Teams channel helpers parse and rehydrate channel links without losing the saved channel (SPEC-2963 Phase 9)", () => {
  const parseTeamsChannelLink = loadFunction("parseTeamsChannelLink");
  const formatTeamsChannelLink = loadFunctionWithDeps("formatTeamsChannelLink", [
    "parseTeamsDefaultChannel",
  ]);
  const composeTeamsDefaultChannel = loadFunction("composeTeamsDefaultChannel");
  const teamId = "0abf2b52-ada6-40ee-96ba-c8010f5be083";
  const channelId = "19:04685a0564cd4f7eb712f8152167360a@thread.skype";
  const tenantId = "0ff7b59c-80f2-4925-99ca-f6d55b11d31c";
  const defaultChannel = `${teamId}/${channelId}`;

  assert.deepEqual(
    parseTeamsChannelLink(
      `https://teams.microsoft.com/l/channel/${encodeURIComponent(channelId)}/gwt-test?groupId=${teamId}&tenantId=${tenantId}`,
    ),
    { teamId, channelId },
  );
  assert.equal(composeTeamsDefaultChannel(teamId, channelId), defaultChannel);
  assert.deepEqual(parseTeamsChannelLink(formatTeamsChannelLink(defaultChannel, tenantId)), {
    teamId,
    channelId,
  });
  assert.equal(parseTeamsChannelLink("not a Teams link"), null);
});

test("System tab exposes an editable OAuth callback port + redirect URL hint (SPEC-2963 FR-005)", () => {
  // The OAuth redirect must use a fixed, registerable loopback port; the UI
  // exposes it (default 8765) and shows the exact URL to register.
  assert.match(
    settingsSource,
    /"settings-board-oauth-port"/,
    "OAuth callback port input must exist",
  );
  assert.match(
    settingsSource,
    /127\.0\.0\.1:\$\{[^}]+\}\/oauth\/callback/,
    "the redirect URL hint must show http://127.0.0.1:<port>/oauth/callback",
  );
  assert.match(
    settingsSource,
    /kind:\s*"update_board_oauth_port"/,
    "Save port must send update_board_oauth_port",
  );
  assert.match(
    settingsSource,
    /oauthRedirectPort:\s*event\.oauth_redirect_port/,
    "dispatch must prefill the OAuth port from board_auth_status",
  );
});

test("renderSystemPanel sends update_system_settings with board_provider on change (SPEC-2959)", () => {
  assert.match(
    settingsSource,
    /kind:\s*"update_system_settings",[\s\S]*?board_provider:\s*next/,
    "Board provider select onChange must send update_system_settings with board_provider",
  );
});

test("System tab exposes Launch GWT at login autostart control", () => {
  assert.match(
    settingsSource,
    /autostartEnabled:\s*false/,
    "expected System settings state to track autostart enabled status",
  );
  assert.match(
    settingsSource,
    /autostartPreviousEnabled:\s*false/,
    "expected System settings state to retain the previous autostart value for failed updates",
  );
  assert.match(
    settingsSource,
    /autostartEnabled:\s*state\.autostartPending\s*\?\s*state\.autostartPreviousEnabled\s*===\s*true/,
    "expected autostart update errors to restore the last confirmed value",
  );
  assert.match(
    settingsSource,
    /autostartText\.textContent\s*=\s*"Launch GWT at login"/,
    "expected System tab autostart checkbox label",
  );
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"update_autostart",\s*enabled:\s*next\s*\}\)/,
    "expected autostart checkbox onChange to send update_autostart",
  );
});

test("autostart checking and failures stay local and preserve the last confirmed state", () => {
  const { document, Event } = parseHTML("<html><body></body></html>");
  const state = {
    autostartEnabled: false, autostartPreviousEnabled: false,
    autostartLoaded: false, autostartPending: false, autostartChecking: true,
    statusMessage: "Saved system settings.", statusKind: "success",
  };
  const model = createUiStateStore(state);
  const messages = [];
  const createDiv = className => {
    const element = document.createElement("div");
    element.className = className;
    return element;
  };
  const renderSource = extractFunctionSource(settingsSource, "renderSystemPanel");
  const autostartSource = renderSource.slice(
    renderSource.indexOf('const autostartSection ='),
    renderSource.indexOf('panel.appendChild(section)'),
  );
  const { render, applyAutostartStatus, applyAutostartError } = Function(
    "document", "systemSettingsModel", "createDiv", "send", `
      ${extractFunctionSource(settingsSource, "applyAutostartStatus")}
      ${extractFunctionSource(settingsSource, "applyAutostartError")}
      ${extractFunctionSource(settingsSource, "syncAutostartCheckbox")}
      ${extractFunctionSource(settingsSource, "renderAutostartStatus")}
      function render() { const state = systemSettingsModel.read(); ${autostartSource}; return autostartSection; }
      return { render, applyAutostartStatus, applyAutostartError };
    `,
  )(document, model, createDiv, message => messages.push(message));
  const toggle = section => section.querySelector('input[type="checkbox"]');
  let section = render();
  assert.equal(toggle(section).disabled, true, "unknown autostart cannot be changed");
  assert.match(section.textContent, /Checking login launch status/);

  applyAutostartError({ message: "Status query timed out." });
  section = render();
  assert.equal(toggle(section).disabled, true, "failed initial status remains unknown");
  assert.match(section.textContent, /unavailable.*Status query timed out/i);
  assert.doesNotMatch(section.textContent, /Checking login launch status/);

  applyAutostartStatus({ enabled: true, mechanism: "fixture", install_path: "/fixture/gwt" });
  model.update(state => ({ ...state, autostartPreviousEnabled: false, autostartChecking: true }));
  applyAutostartError({ message: "Refresh query unavailable." });
  assert.equal(model.read().autostartEnabled, true, "refresh failure preserves the last confirmed value");
  section = render();
  assert.equal(toggle(section).disabled, false);
  assert.equal(toggle(section).checked, true);
  toggle(section).checked = false;
  toggle(section).dispatchEvent(new Event("change"));
  assert.equal(toggle(render()).disabled, true, "saving disables only the autostart control");
  assert.deepEqual(messages, [{ kind: "update_autostart", enabled: false }]);
  applyAutostartError({ message: "Update denied." });
  section = render();
  assert.equal(toggle(section).checked, true, "failed update restores the known value");
  assert.equal(toggle(section).disabled, false, "known status allows retry");
  assert.match(section.textContent, /Update denied/);
  applyAutostartStatus({ enabled: false }, "Saved login launch setting.");
  assert.match(render().textContent, /Saved login launch setting/);
  assert.equal(state.statusMessage, "Saved system settings.");
  assert.equal(state.statusKind, "success");
  const requestSource = extractFunctionSource(settingsSource, "requestSettingsSnapshots");
  assert.match(requestSource, /const shouldRequestAutostartStatus = recovering \|\| \(!autostart\.autostartPending\s*&& !autostart\.autostartChecking\)/,
    "reopening Settings must preserve an in-flight query or update");
  assert.match(requestSource, /if \(shouldRequestAutostartStatus\)\s*\{\s*systemSettingsModel\.update\([\s\S]{0,100}autostartChecking: true/,
    "only a new query starts checking");
  assert.match(requestSource, /if \(shouldRequestAutostartStatus\)\s*\{[\s\S]*?send\(\{ kind: "get_autostart_status" \}\)/,
    "use the same decision when sending the query");
});

test("renderSettingsWindow requests current settings via get_system_settings", () => {
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"get_system_settings"\s*\}\)/,
    "expected renderSettingsWindow to send get_system_settings on open",
  );
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"get_autostart_status"\s*\}\)/,
    "expected renderSettingsWindow to send get_autostart_status on open",
  );
});

test("Frontend dispatches all system_settings and autostart backend events", () => {
  // The dispatch must handle the three reply variants from
  // crates/gwt/src/system_settings.rs: SystemSettings (load),
  // SystemSettingsUpdated (save success), SystemSettingsError (failure).
  for (const kind of [
    /case\s+"system_settings":/,
    /case\s+"system_settings_updated":/,
    /case\s+"system_settings_error":/,
    /case\s+"autostart_status":/,
    /case\s+"autostart_error":/,
  ]) {
    assert.match(
      appSource,
      kind,
      `expected backend event dispatch case: ${kind}`,
    );
  }
});

test("app.js consumes #about hash by opening the About GWT version surface", () => {
  assert.match(
    appSource,
    /window\.addEventListener\(\s*"hashchange",\s*consumeAboutHash/,
    "expected app.js to listen for #about hash changes",
  );
  assert.match(
    appSource,
    /window\.location\.hash\s*===\s*"#about"/,
    "expected app.js to detect #about hash exactly",
  );
  assert.match(
    appSource,
    /releaseNotesWindow\.openAbout\(versionState\.current\s*\|\|\s*null\)/,
    "expected #about to open the About GWT version surface",
  );
});

test("Custom Agents panel keeps the data-role='settings-scroll' hook for the legacy renderer", () => {
  // renderSettingsAgentList queries [data-role='settings-scroll'] to find
  // its target. Moving it onto the Custom Agents panel preserves the
  // existing list/edit flow without rewriting that surface.
  assert.match(
    settingsSource,
    /panelAgents\.dataset\.role\s*=\s*"settings-scroll"/,
    "Custom Agents panel must carry data-role='settings-scroll'",
  );
});

test("Custom Agents panel wires the env editor to update_custom_agent", () => {
  assert.match(
    settingsSource,
    /from "\/custom-agent-env-editor\.js"/,
    "settings surface must import the custom agent env editor module",
  );
  assert.match(
    settingsSource,
    /renderCustomAgentEnvEditor\(\{/,
    "renderSettingsAgentList must mount the env editor",
  );
  assert.match(
    settingsSource,
    /kind:\s*"update_custom_agent",\s*agent:\s*updatedAgent/,
    "env editor save must send update_custom_agent with the edited agent",
  );
});

test("switchSettingsTab toggles aria-selected and hidden together", () => {
  // The active / hidden contract: aria-selected on the tab and the .hidden
  // class on every panel except the matching one must change in lockstep.
  // Without this, screen readers and the visual state can desync.
  assert.match(
    settingsSource,
    /aria-selected/,
    "switchSettingsTab must update aria-selected",
  );
  assert.match(
    settingsSource,
    /panel\.classList\.toggle\(\s*"hidden"/,
    "switchSettingsTab must toggle hidden on non-active panels",
  );
});

test("renderSystemPanel exposes agent process-tree resource controls (#3813)", () => {
  assert.match(
    settingsSource,
    /agentResource:\s*\{\s*enabled:\s*true,\s*preset:\s*"automatic",\s*priority:\s*"below-normal",\s*cpuLimitPercent:\s*null,\s*buildJobs:\s*null,?\s*\}/,
    "expected agent resource UI state to default to enabled / automatic preset",
  );
  for (const id of [
    "settings-system-agent-resource-enabled",
    "settings-system-agent-preset",
    "settings-system-agent-priority",
    "settings-system-agent-cpu-limit",
    "settings-system-agent-build-jobs",
  ]) {
    assert.match(
      settingsSource,
      new RegExp(`id\\s*=\\s*"${id}"`),
      `expected agent resource control with id ${id}`,
    );
  }
  assert.match(
    settingsSource,
    /kind:\s*"update_system_settings",\s*language:\s*systemSettingsModel\.read\(\)\.language\s*\|\|\s*"auto",\s*agent_resource:/,
    "expected agent resource controls to send the complete agent_resource object",
  );
  assert.match(
    settingsSource,
    /function parseOptionalPositiveInteger\(/,
    "expected empty numeric inputs to resolve to automatic (null) mode",
  );
  assert.match(
    settingsSource,
    /normalizeAgentResource\(event\.agent_resource, state\.agentResource\)/,
    "expected model commits to normalize agent resource state",
  );
  assert.match(
    settingsSource,
    /presetSelect\.className\s*=\s*"settings-select"/,
    "expected the preset select to reuse the shared settings-select primitive",
  );
  for (const [value, label] of [
    ["automatic", "Automatic (recommended)"],
    ["gui-responsiveness", "Prioritize GUI responsiveness"],
    ["build-speed", "Prioritize build speed"],
    ["custom", "Custom"],
  ]) {
    const escapedLabel = label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    assert.match(
      settingsSource,
      new RegExp(`value:\\s*"${value}",\\s*text:\\s*"${escapedLabel}"`),
      `expected preset option ${value} labelled "${label}"`,
    );
  }
  assert.match(
    settingsSource,
    /customSection\.hidden\s*=\s*agentResource\.preset\s*!==\s*"custom"/,
    "expected numeric / priority controls to be shown only for the Custom preset",
  );
  assert.match(
    componentsCss,
    /\.settings-section\[hidden\]\s*\{\s*display:\s*none;/,
    "expected the hidden attribute to win over the flex display of settings sections",
  );
  assert.doesNotMatch(
    settingsSource,
    /Cargo build jobs/,
    "expected the parallelism control to use generic build vocabulary, not cargo",
  );
  assert.match(
    settingsSource,
    /prioritySelect\.className\s*=\s*"settings-select"/,
    "expected the priority select to reuse the shared settings-select primitive",
  );
  assert.match(
    settingsSource,
    /cpuInput\.className\s*=\s*"settings-input"/,
    "expected numeric inputs to reuse the shared settings-input primitive",
  );
});

test("app.js delegates complete System events to the shared model (#3813)", () => {
  assert.match(appSource, /case "system_settings":\s*case "system_settings_updated":\s*applySystemSettingsEvent\(event\);/);
  assert.match(settingsSource, /agentResource: normalizeAgentResource\(event\.agent_resource, state\.agentResource\)/);
});


test("Works output lines use the System writer and keep invalid or rejected drafts out of the shared setting", () => {
  const { surface, body, window, sent, flush } = createSettingsHarness();
  surface.renderSettingsWindow(body, { id: "settings", title: "Settings" });
  const input = () => body.querySelector('[aria-label="Works output lines"]');
  assert.ok(input(), "System settings exposes the saved Works line count");
  assert.equal(input().value, "3");
  assert.equal(input().getAttribute("min"), "3"); assert.equal(input().getAttribute("max"), "10"); assert.equal(input().getAttribute("step"), "1");
  surface.applySystemSettingsEvent({ kind: "system_settings", language: "ja" }); flush();
  assert.equal(surface.systemSettingsModel.read().worksPreviewLines, 3);
  input().value = "5"; input().dispatchEvent(new window.Event("change"));
  assert.deepEqual(sent.at(-1), { kind: "update_system_settings", language: "ja", works_preview_lines: 5 });
  assert.equal(surface.systemSettingsModel.read().worksPreviewLines, 3, "only saved backend values reach Works");
  surface.applySystemSettingsEvent({ kind: "system_settings_updated", works_preview_lines: 5 }); flush();
  assert.equal(input().value, "5");
  assert.equal(surface.systemSettingsModel.read().worksPreviewLines, 5);
  const count = sent.length;
  for (const invalid of ["", "2", "11", "4.5"]) {
    input().value = invalid; input().dispatchEvent(new window.Event("change")); flush();
    assert.equal(sent.length, count);
    assert.equal(surface.systemSettingsModel.read().worksPreviewLines, 5);
    assert.match(body.textContent, /whole number from 3 to 10/);
  }
  input().value = "10"; input().dispatchEvent(new window.Event("change"));
  surface.applySystemSettingsEvent({ kind: "system_settings_error", message: "Storage unavailable" }); flush();
  assert.equal(input().value, "5"); assert.equal(surface.systemSettingsModel.read().worksPreviewLines, 5);
  assert.match(body.textContent, /Storage unavailable/);
});

test("Works loads its saved output line count on reconnect without a Settings mount", () => {
  const { surface, sent } = createSettingsHarness();
  surface.resyncSettings();
  assert.deepEqual(sent, [{ kind: "get_system_settings" }]);
});
