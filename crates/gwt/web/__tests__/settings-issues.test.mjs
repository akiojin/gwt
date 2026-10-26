import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";
import { bindControlExplanations, setControlExplanation } from "../control-explanations.js";
import { createUiStateStore } from "../ui-state-store.js";
import { createInteractionGuard } from "../interaction-guard.js";
import { renderCustomAgentEnvEditor } from "../custom-agent-env-editor.js";

const source = readFileSync(new URL("../settings-surface.js", import.meta.url), "utf8");
const candidateChoices = {
  agents: [
    { id: "codex", name: "Codex", is_custom: false, default_model: "gpt-6-astra", supports_fast_mode: true, supports_skip_permissions: true,
      session_modes: ["Normal", "Continue", "Resume"], models: [
        { value: "gpt-6-astra", label: "Astra", default_reasoning: "medium", reasoning_options: [{ value: "medium", label: "Medium" }, { value: "high", label: "High" }] },
        { value: "gpt-6-luna", label: "Luna", default_reasoning: "medium", reasoning_options: [{ value: "medium", label: "Medium" }] },
      ] },
    { id: "claude", name: "Claude Code", is_custom: false, default_model: "", default_reasoning: "auto", supports_fast_mode: true, supports_skip_permissions: true,
      session_modes: ["Normal", "Continue", "Resume"], models: [
        { value: "", label: "Default", default_reasoning: "auto", reasoning_options: [{ value: "auto", label: "Auto" }] },
        { value: "haiku", label: "Haiku", default_reasoning: null, reasoning_options: [] },
      ] },
    { id: "hermes", name: "Hermes Agent", is_custom: false, default_model: "", models: [], model_suggestions: ["fixture-model", "other-model"],
      session_modes: ["Normal"], supports_model_input: true },
    { id: "custom", name: "Configured custom", is_custom: true, models: [], session_modes: ["Normal"], supports_model_input: false },
  ], docker_services: ["dev", "worker"], default_docker_service: "dev",
};

const candidateMembers = ["codex", "claude", "hermes", "custom"].map(agent_id => ({
  member_id: `m-${agent_id}`, agent_member_id: `m-${agent_id}`, agent_id, name: agent_id, enabled: true,
}));

function selectValue(control, value) {
  let option = [...control.options].find(option => option.value === value);
  if (!option) {
    option = control.ownerDocument.createElement("option");
    option.value = value;
    control.appendChild(option);
  }
  option.selected = true;
}
function load(name, bindings = {}) {
  bindings = { setControlExplanation, ...bindings };
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `missing ${name}`);
  const open = source.indexOf("{", source.indexOf(")", start));
  let depth = 1, end = open + 1;
  while (depth && end < source.length) {
    if (source[end] === "{") depth++;
    if (source[end] === "}") depth--;
    end++;
  }
  const loaded = Function(...Object.keys(bindings), `return (${source.slice(start, end)})`)(...Object.values(bindings));
  return name === "renderAgentsSettingsPanel" ? (panel, options) => loaded(panel, { availableMembers: candidateMembers, ...options }) : loaded;
}

test("Settings entry points reuse only the requested surface owner", () => {
  const { document, CustomEvent } = parseHTML('<html><body><div id="normal"></div><div id="pm"></div></body></html>');
  const normal = document.getElementById("normal"), pm = document.getElementById("pm");
  for (const body of [normal, pm]) body.dataset.settingsProjectRoot = "/A";
  pm.dataset.surfaceOwner = "pm";
  const selected = [], focused = [], pending = [];
  load("installSettingsOpenHandler", { switchSettingsTab: (body, target) => selected.push([body.id, target]) })({
    document, settingsWindowBodies: new Set([normal, pm]), activeProjectTab: () => ({ project_root: "/A" }),
    focusOrSpawnPreset: (...args) => focused.push(args), setPendingTarget: (...args) => pending.push(args),
  });
  document.dispatchEvent(new CustomEvent("settings:open", { detail: { target: "project-manager", surface_owner: "pm" } }));
  document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "issues" } }));
  assert.deepEqual(selected, [["pm", "project-manager"], ["normal", "issues"]]);
  assert.deepEqual(focused, [["settings", { surfaceOwner: "pm" }], ["settings", { surfaceOwner: null }]]);
  pm.remove();
  document.dispatchEvent(new CustomEvent("settings:open", { detail: { target: "project-manager", surface_owner: "pm" } }));
  assert.deepEqual(pending, [["project-manager", "pm"]]);
});

test("Reopening an existing System Settings window reloads CLI update preferences", () => {
  const { document, CustomEvent } = parseHTML('<html><body><div id="settings"></div></body></html>');
  const body = document.getElementById("settings");
  body.dataset.settingsProjectRoot = "/A";
  const sent = [];
  load("installSettingsOpenHandler", { switchSettingsTab: () => {} })({
    document, settingsWindowBodies: new Set([body]), activeProjectTab: () => ({ project_root: "/A" }),
    focusOrSpawnPreset: () => {}, setPendingTarget: () => assert.fail("reuse the existing window"),
    send: message => sent.push(message),
  });
  document.dispatchEvent(new CustomEvent("settings:open", { detail: { section: "system" } }));
  assert.deepEqual(sent, [{ kind: "get_update_settings" }]);
});

test("PM auxiliary Settings scrolls its own content without scrolling app chrome", () => {
  const { document } = parseHTML('<div id="settings" data-surface-owner="pm"><div class="settings-body"><section class="settings-panel" data-settings-panel="project-manager"></section></div></div>');
  const body = document.getElementById("settings"), scroller = body.querySelector(".settings-body"), panel = body.querySelector("section");
  Object.defineProperty(scroller, "offsetTop", { value: 20 });
  Object.defineProperty(panel, "offsetTop", { value: 120 });
  panel.scrollIntoView = () => assert.fail("must not scroll the document or outer app chrome");
  let focused = false;
  panel.focus = options => { focused = options.preventScroll; };
  load("switchSettingsTab")(body, "project-manager", true);
  assert.equal(scroller.scrollTop, 100);
  assert.equal(focused, true);
});

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
  assert.deepEqual([...panel.querySelectorAll("legend")].map(el => el.textContent), ["Launch control", "Refill", "Target"]);
  assert.equal(panel.firstElementChild.tagName, "H3", "project identity precedes settings groups");
  assert.equal(panel.querySelector('[aria-label="Max active"]').closest("fieldset").querySelector("legend").textContent, "Launch control");
  assert.equal(panel.querySelector('[aria-label="Auto-refill"]').closest("fieldset").querySelector("legend").textContent, "Refill");
  assert.equal(panel.querySelector('[aria-label="Allowed labels"]').closest("fieldset").querySelector("legend").textContent, "Target");
  assert.deepEqual([...panel.querySelector("fieldset").querySelectorAll(".settings-field > :first-child")].map(el => el.textContent), ["Autonomous", "Max active", "Monitoring"]);
  const change = (name, value) => {
    const control = panel.querySelector(`[aria-label="${name}"]`);
    assert.ok(control, name);
    if (control.type === "checkbox") control.checked = value;
    else control.value = value;
    control.dispatchEvent(new Event("change"));
  };
  change("Max active", "6");
  panel.querySelector('[aria-label="Apply Max active"]').click();
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

test("Issues disclosure reuses authoritative launch controls and returns them to the compact summary", () => {
  const { document } = parseHTML('<div class="settings-root"><div class="knowledge-monitor-controls"><button data-action="monitor-auto-refill">Refill</button><button data-action="monitor-autonomous">Autonomous</button><label class="issue-monitor-limit">Limit</label><button data-action="monitor-settings">Settings</button><button data-action="monitor-toggle">Start</button></div><details class="settings-issues-options"><section></section></details></div>');
  const body = document.querySelector(".settings-root");
  const panel = body.querySelector("section");
  const disclosure = body.querySelector("details");
  const launchControls = [...body.querySelectorAll('[data-action="monitor-autonomous"], [data-action="monitor-toggle"]')];
  let clicked = 0;
  launchControls[1].addEventListener("click", () => clicked++);
  const render = load("renderIssuesSettingsPanel");
  const position = load("positionIssuesLaunchControls");
  disclosure.setAttribute("open", "");
  for (let i = 0; i < 2; i++) {
    render(panel, { project: { name: "A" }, status: {}, send: () => {}, launchControls });
    position(body);
    assert.equal(panel.querySelector('[data-settings-launch-action="monitor-toggle"] > :last-child').firstElementChild === launchControls[1], true);
    assert.equal(panel.querySelector('[data-action="monitor-autonomous"]').closest("fieldset").querySelector("legend").textContent, "Launch control");
    launchControls[1].click();
  }
  disclosure.removeAttribute("open");
  position(body);
  assert.ok(body.querySelector('[data-action="monitor-auto-refill"]').nextElementSibling === launchControls[0], "toggles remain adjacent after closing Settings");
  assert.ok(launchControls[0].nextElementSibling.matches('.issue-monitor-limit'));
  assert.equal(body.querySelectorAll('[data-action="monitor-toggle"]').length, 1);
  assert.equal(clicked, 2, "original listeners survive status rerenders");
  const css = readFileSync(new URL("../styles/components.css", import.meta.url), "utf8");
  assert.match(css, /\[data-settings-issues-controls\] \.settings-field\s*\{[^}]*grid-template-columns:\s*140px minmax\(0, 1fr\)/);
});

test("Auto capacity is read-only and Manual starts with the machine worker limit", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderIssuesSettingsPanel")(panel, { project: { project_root: "/A" },
    status: { max_active_agents: 3, max_active_agents_override: null,
      agent_capacity: { machine_budget: 7, own_pm_agents: 1, other_live_agents: 3, other_pm_agents: 1 } },
    send: m => sent.push(m) });
  const max = panel.querySelector('[aria-label="Max active"]');
  const mode = panel.querySelector('[aria-label="Max active mode"]');
  assert.equal(max.disabled, true);
  assert.equal(mode.value, "auto");
  assert.equal(max.value, "3");
  max.value = "9";
  max.dispatchEvent(new Event("input"));
  max.dispatchEvent(new Event("change"));
  panel.querySelector('[aria-label="Apply Max active"]').click();
  assert.deepEqual(sent, [], "Auto never sends a numeric override");
  max.value = "3";
  mode.querySelector('[value="manual"]').selected = true;
  mode.dispatchEvent(new Event("change"));
  assert.equal(max.value, "5", "the project allocation of three must not become the shared Manual request");
  assert.deepEqual(sent, [{ kind: "set_issue_monitor_max_active_agents", max_active_agents: 5 }]);
  panel.updateMaxActiveStatus({ max_active_agents: 3, max_active_agents_override: null,
    agent_capacity: { machine_budget: null, other_live_agents: 3, other_pm_agents: 1, own_untracked_workers: 1 } });
  mode.querySelector('[value="manual"]').selected = true;
  mode.dispatchEvent(new Event("change"));
  assert.equal(max.value, "6", "unknown machine capacity reconstructs the full current worker allowance");
  assert.equal(sent.at(-1).max_active_agents, 6);
});

test("Max active stepper edits commit once on Apply and survive status redraw", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  const render = load("renderIssuesSettingsPanel");
  const options = { project: { project_root: "/A" },
    status: { max_active_agents: 6, max_active_agents_override: 6, active_count: 4 },
    send: message => sent.push(message) };
  render(panel, options);
  const max = panel.querySelector('[aria-label="Max active"]');
  for (const value of ["5", "4", "3"]) {
    max.value = value;
    max.dispatchEvent(new Event("input"));
    max.dispatchEvent(new Event("change"));
  }
  assert.deepEqual(sent, [], "intermediate values are draft edits only");
  render(panel, options);
  assert.equal(panel.querySelector('[aria-label="Max active"]').value, "3");
  const apply = panel.querySelector('[aria-label="Apply Max active"]');
  apply.click();
  apply.click();
  assert.deepEqual(sent, [{ kind: "set_issue_monitor_max_active_agents", max_active_agents: 3 }]);
  render(panel, { ...options, status: { ...options.status, max_active_agents: 3, max_active_agents_override: 3 } });
  assert.match(panel.textContent, /Existing agents continue.*new launches wait/i);
});

test("authoritative Auto status disables a focused Manual draft before deferred redraw", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderIssuesSettingsPanel")(panel, { project: { project_root: "/A" },
    status: { max_active_agents: 6, max_active_agents_override: 6 }, send: message => sent.push(message) });
  const max = panel.querySelector('[aria-label="Max active"]');
  const mode = panel.querySelector('[aria-label="Max active mode"]');
  const apply = panel.querySelector('[aria-label="Apply Max active"]');
  max.value = "3";
  max.dispatchEvent(new Event("input"));
  assert.equal(apply.disabled, false);
  // The edit guard keeps these nodes; only the status callback runs while focused.
  panel.updateMaxActiveStatus({ max_active_agents: 7, max_active_agents_override: null });
  assert.equal(max.disabled, true);
  assert.equal(mode.value, "auto");
  assert.equal(apply.disabled, true);
  assert.equal(max.value, "7");
  assert.equal(panel.dataset.maxActiveDraft, undefined);
  max.dispatchEvent(new Event("change"));
  apply.click();
  assert.deepEqual(sent, [], "an old Manual draft cannot overwrite remote Auto");
  panel.updateMaxActiveStatus({ max_active_agents: 5, max_active_agents_override: 5 });
  assert.equal(max.disabled, false);
  assert.equal(mode.value, "manual");
  assert.equal(max.value, "5", "returning to Manual does not resurrect the old draft");
  assert.equal(apply.disabled, true);
});

test("machine Max active updates shared controls and project allocation in the existing panel", () => {
  const { document } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderIssuesSettingsPanel")(panel, { project: { project_root: "/A" }, status: {
    max_active_agents_scope: "machine", max_active_agents: 3, max_active_agents_override: 8,
    agent_capacity: { measurement_complete: true, other_live_agents: 5, other_pm_agents: 2 },
  }, send: message => sent.push(message) });
  const capacity = panel.querySelector(".settings-issues-capacity");
  const max = capacity.querySelector('[aria-label="Max active"]');
  const mode = capacity.querySelector('[aria-label="Max active mode"]');
  assert.match(capacity.textContent, /All projects on this machine/);
  assert.match(capacity.textContent, /This project allocation: 3.*Other project workers: 3/);
  assert.equal(max.value, "8", "Manual edits the shared request, not the project's allocation");
  assert.equal(mode.value, "manual");
  panel.updateMaxActiveStatus({ max_active_agents_scope: "machine", max_active_agents: 2,
    max_active_agents_override: null,
    agent_capacity: { measurement_complete: true, other_live_agents: 7, other_pm_agents: 3 } });
  assert.equal(panel.querySelector('[aria-label="Max active"]'), max, "status updates reuse the mounted controls");
  assert.equal(mode.value, "auto");
  assert.equal(max.disabled, true);
  assert.equal(max.value, "2");
  assert.match(capacity.textContent, /This project allocation: 2.*Other project workers: 4/);
  panel.updateMaxActiveStatus({ max_active_agents_scope: "machine", max_active_agents: 1,
    max_active_agents_override: 6,
    agent_capacity: { measurement_complete: true, other_live_agents: 8, other_pm_agents: 2 } });
  assert.equal(mode.value, "manual");
  assert.equal(max.disabled, false);
  assert.equal(max.value, "6");
  assert.match(capacity.textContent, /This project allocation: 1.*Other project workers: 6/);
  assert.deepEqual(sent, [], "machine status updates do not save settings");
});

test("Auto shows measuring state beside its effective limit until capacity is ready", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section");
  const render = load("renderIssuesSettingsPanel");
  const options = { project: { project_root: "/A" }, status: {
    max_active_agents: 4, max_active_agents_override: null,
    agent_capacity: { measurement_complete: false },
  }, send: () => {} };
  render(panel, options);
  assert.equal(panel.querySelector('[aria-label="Max active"]').value, "4");
  assert.match(panel.querySelector(".settings-issues-capacity").textContent, /Measuring capacity/);
  const mode = panel.querySelector('[aria-label="Max active mode"]');
  mode.querySelector('[value="manual"]').selected = true;
  mode.dispatchEvent(new Event("change"));
  assert.doesNotMatch(panel.querySelector(".settings-issues-capacity").textContent, /Measuring capacity/);
  mode.querySelector('[value="auto"]').selected = true;
  mode.dispatchEvent(new Event("change"));
  panel.updateMaxActiveStatus({ ...options.status, max_active_agents: 7,
    agent_capacity: { measurement_complete: true } });
  assert.equal(panel.querySelector('[aria-label="Max active"]').value, "7", "focused Auto status stays live");
  render(panel, { ...options, status: { ...options.status, agent_capacity: { measurement_complete: true } } });
  assert.doesNotMatch(panel.querySelector(".settings-issues-capacity").textContent, /Measuring capacity/);
});

test("AC-6: Issues Settings exposes explanations for capacity, refill and targeting", t => {
  const { document } = parseHTML("<html><body><section></section></body></html>");
  const panel = document.querySelector("section"), sent = [];
  const explanations = bindControlExplanations({ document, root: panel, id: "settings-explanation" });
  t.after(() => explanations.dispose());
  load("renderIssuesSettingsPanel")(panel, { project: { project_root: "/A" },
    status: { max_active_agents: 4, max_active_agents_override: null }, send: message => sent.push(message) });
  for (const [name, meaning] of [
    ["Max active", /maximum.*agents/i], ["Max active mode", /automatic.*manual/i],
    ["Auto-refill", /automatically.*queue/i], ["Refill limit", /0.*disables/i],
    ["Allowed labels", /matching issues/i],
  ]) {
    const control = panel.querySelector(`[aria-label="${name}"]`);
    assert.match(control.getAttribute("aria-description") || "", meaning, name);
    assert.equal(control.dataset.issueExplanation, control.getAttribute("aria-description"));
    assert.ok(control.dataset.issueExplanationKey, "status redraw can find the replacement control");
    if (!control.disabled) {
      control.dispatchEvent(new document.defaultView.Event("focusin", { bubbles: true }));
      assert.equal(document.getElementById("settings-explanation").textContent, control.getAttribute("aria-description"));
      assert.equal(explanations.visible, true);
      control.dispatchEvent(new document.defaultView.Event("focusout", { bubbles: true }));
    }
    control.dispatchEvent(new document.defaultView.Event("mouseover", { bubbles: true }));
    assert.equal(document.getElementById("settings-explanation").textContent, control.getAttribute("aria-description"));
    assert.equal(explanations.visible, true);
    control.dispatchEvent(new document.defaultView.Event("mouseout", { bubbles: true }));
  }
  const capacity = panel.querySelector(".settings-issues-capacity");
  assert.equal(capacity.getAttribute("tabindex"), "0", "Auto's disabled value has a keyboard reachable explanation");
  assert.match(capacity.getAttribute("aria-description"), /maximum.*agents/i);
  capacity.dispatchEvent(new document.defaultView.Event("focusin", { bubbles: true }));
  load("renderIssuesSettingsPanel")(panel, { project: { project_root: "/A" },
    status: { max_active_agents: 5, max_active_agents_override: null }, send: message => sent.push(message) });
  panel.dispatchEvent(new document.defaultView.Event("control-explanations:refresh"));
  assert.equal(explanations.visible, true, "a Settings redraw keeps a focused explanation readable");
  const replacement = panel.querySelector(".settings-issues-capacity");
  assert.equal(replacement.getAttribute("aria-describedby"), "settings-explanation");
  assert.equal(document.getElementById("settings-explanation").textContent, replacement.getAttribute("aria-description"));
  assert.deepEqual(sent, [], "registering explanations does not change settings");
  const viewport = document.defaultView;
  const previousViewport = { width: viewport.innerWidth, height: viewport.innerHeight };
  t.after(() => { viewport.innerWidth = previousViewport.width; viewport.innerHeight = previousViewport.height; });
  viewport.innerWidth = 400; viewport.innerHeight = 800;
  replacement.getBoundingClientRect = () => ({ left: 100, right: 200, top: 50, bottom: 74 });
  const settingsPopup = document.getElementById("settings-explanation");
  settingsPopup.getBoundingClientRect = () => ({ width: 120, height: 80 });
  explanations.refresh();
  assert.equal(settingsPopup.style.left, "100px", "the default placement remains aligned below the control");
  assert.equal(settingsPopup.style.top, "73px");

  const rail = document.createElement("nav"), control = document.createElement("button");
  rail.appendChild(control); document.body.appendChild(rail);
  setControlExplanation(control, "Works: live output and terminals");
  let railRect = { left: 0, right: 72, top: 44, bottom: 772, width: 72, height: 728 };
  rail.getBoundingClientRect = () => railRect;
  control.getBoundingClientRect = () => ({ left: 4, right: 68, top: 140, bottom: 184 });
  const railExplanations = bindControlExplanations({ document, root: rail, id: "rail-explanation", placement: "right", passThrough: true });
  t.after(() => railExplanations.dispose());
  const railPopup = document.getElementById("rail-explanation");
  railPopup.getBoundingClientRect = () => {
    const left = parseFloat(railPopup.style.left) || 0, top = parseFloat(railPopup.style.top) || 0;
    const width = Math.min(360, parseFloat(railPopup.style.maxWidth) || 360);
    return { left, right: left + width, top, bottom: top + 80, width, height: 80 };
  };
  assert.equal(railPopup.style.pointerEvents, "none", "rail explanations let native main controls receive pointer input");
  control.dispatchEvent(new viewport.Event("mouseover", { bubbles: true }));
  assert.equal(railPopup.style.left, "72px", "a side explanation clears every rail control");
  assert.equal(railPopup.style.maxWidth, "320px", "the popup shrinks to the available right space at 400px");
  const toPadding = new viewport.Event("mouseout", { bubbles: true });
  Object.assign(toPadding, { relatedTarget: rail }); control.dispatchEvent(toPadding);
  assert.equal(railExplanations.visible, true, "rail padding bridges the pointer into the hoverable popup");
  const toPopup = new viewport.Event("mouseout", { bubbles: true });
  Object.assign(toPopup, { relatedTarget: panel, clientX: 80, clientY: 150 }); rail.dispatchEvent(toPopup);
  const readPopup = new viewport.Event("pointermove", { bubbles: true });
  Object.assign(readPopup, { clientX: 80, clientY: 150 }); document.dispatchEvent(readPopup);
  assert.equal(railExplanations.visible, true, "a transparent popup stays readable from its real pointer coordinates");
  const leavePopup = new viewport.Event("pointermove", { bubbles: true });
  Object.assign(leavePopup, { clientX: 399, clientY: 799 }); document.dispatchEvent(leavePopup);
  assert.equal(railExplanations.visible, false, "leaving both rail and popup dismisses the explanation");
  control.dispatchEvent(new viewport.Event("focusin", { bubbles: true }));
  replacement.dispatchEvent(new viewport.Event("focusin", { bubbles: true }));
  assert.equal(railExplanations.visible, false, "outside keyboard focus dismisses the rail explanation");
  assert.equal(settingsPopup.hidden, false, "the focused main control retains its own explanation");
  assert.equal(document.querySelectorAll('[role="tooltip"]:not([hidden])').length, 1);
  control.dispatchEvent(new viewport.Event("mouseover", { bubbles: true }));
  let nativePointerDown = 0;
  replacement.addEventListener("pointerdown", () => { nativePointerDown += 1; });
  const press = new viewport.Event("pointerdown", { bubbles: true, cancelable: true }); replacement.dispatchEvent(press);
  assert.equal(nativePointerDown, 1, "outside pointer input proceeds through its original handler");
  assert.equal(press.defaultPrevented, false);
  assert.equal(railExplanations.visible, false);
  control.dispatchEvent(new viewport.Event("mouseover", { bubbles: true }));
  const escape = new viewport.Event("keydown", { bubbles: true, cancelable: true });
  Object.assign(escape, { key: "Escape" }); control.dispatchEvent(escape);
  assert.equal(railExplanations.visible, false);
  document.dispatchEvent(readPopup); railExplanations.refresh();
  assert.equal(railExplanations.visible, false, "pointer geometry and refresh never reopen an Escape-dismissed explanation");
  control.dispatchEvent(new viewport.Event("mouseover", { bubbles: true }));
  railRect = { left: 0, right: 400, top: 44, bottom: 180, width: 400, height: 136 };
  railExplanations.refresh();
  assert.ok(parseFloat(railPopup.style.top) >= railRect.bottom, "the wrapped top strip stays clear of explanations");
  assert.ok(parseFloat(railPopup.style.left) + Math.min(360, parseFloat(railPopup.style.maxWidth)) <= 392);
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
  assert.match(panel.textContent, /This project allocation: 0/, "a Manual request does not replace the authoritative allocation");
  assert.deepEqual(sent, [{ kind: "set_issue_monitor_max_active_agents", max_active_agents: 1 }]);
});

test("Agents Settings edits full profiles while preserving runtime fields and project isolation", () => {
  const { document, Event } = parseHTML("<html><body><section id=a></section><section id=b></section></body></html>");
  const a = document.getElementById("a"), b = document.getElementById("b"), sent = [];
  const profiles = [
    { agent_member_id: "m-codex", agent_id: "codex", model: "gpt-6", reasoning: "high", runtime_target: "Docker", docker_service: "dev", prefer_for: ["type:fix"] },
    { agent_member_id: "m-claude", agent_id: "claude", model: "opus", reasoning: "medium", prefer_for: [] },
  ];
  const renderer = load("renderAgentsSettingsPanel");
  const render = (panel, options) => renderer(panel, { choices: candidateChoices, ...options });
  render(a, { project: { project_root: "/A", name: "A" }, profiles, status: { usage_threshold_percent: 73 }, send: m => sent.push(m) });
  render(b, { project: { project_root: "/B", name: "B" }, profiles: [{ agent_member_id: "m-codex", agent_id: "codex", model: "B model" }], send: () => assert.fail("wrong project") });
  const model = a.querySelector('[aria-label="Model codex"]');
  assert.equal(model.value, "gpt-6");
  assert.deepEqual([...a.querySelector('[aria-label="Runtime codex"]').options].map(o => o.value), ["Host", "Docker"]);
  selectValue(model, "new model");
  model.dispatchEvent(new Event("input", { bubbles: true }));
  assert.equal(a.dataset.dirty, "true", "incoming status must not destroy unsaved profile edits");
  model.dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(Boolean(a.querySelector('[data-action="save-profiles"]')), false);
  assert.equal(sent[0].profiles[0].model, "new model");
  assert.equal(sent[0].profiles[0].docker_service, "dev");
  assert.deepEqual(sent[0].profiles[0].prefer_for, ["type:fix"]);
  assert.equal(Object.hasOwn(sent[0], "usage_threshold_percent"), false);
  assert.equal(a.querySelector('[data-profile-field="threshold"]'), null);
  assert.match(a.textContent, /The first member in the list fixes the launch tool\. Only selected members using that tool are tried in order\. If none is available, new launches wait; tools never switch automatically\./);
  assert.equal(b.querySelector('[aria-label="Model codex"]').value, "B model");
  a.querySelector('[aria-label="Move down codex"]').click();
  assert.deepEqual(sent.at(-1).profiles.map(p => p.agent_id), ["claude", "codex"]);
  assert.match(a.textContent, /Candidates/);
  assert.ok(a.querySelector('[aria-label="Prefer for codex"]'));
  assert.ok(a.querySelector('[data-action="add-candidate"]'));
});

test("Candidates restore shared choices, preserve saved values, and gate dependent controls", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderAgentsSettingsPanel", { crypto: {} })(panel, {
    profiles: [{ agent_member_id: "m-codex", agent_id: "codex", model: "retired-model", reasoning: "retired-effort", docker_service: "saved-service" },
      { agent_member_id: "m-custom", agent_id: "custom", fast_mode: true }, { agent_member_id: "m-saved-unknown-agent", agent_id: "saved-unknown-agent", model: "historic" }],
    choices: candidateChoices,
    send: message => sent.push(message),
  });
  const control = key => panel.querySelector(`fieldset [data-profile-field="${key}"]`);
  for (const key of ["agent_member_id", "model", "reasoning", "docker_service"]) {
    assert.equal(control(key).localName, "select", key);
    assert.equal(control(key).className, "settings-select", "reuse Operator controls");
  }
  assert.equal(control("model").value, "retired-model");
  assert.equal(control("reasoning").value, "retired-effort");
  assert.ok([...control("agent_member_id").options].some(option => option.value === "m-hermes"),
    "shared choices populate Agent before the separate catalog response arrives");
  assert.equal(panel.querySelector('[data-profile-field="new"]').disabled, false,
    "registered members do not wait for provider detection catalogs");
  const unknown = panel.querySelector('[data-agent-id="saved-unknown-agent"]');
  assert.equal(unknown.querySelector('[data-profile-field="agent_member_id"]').value, "m-saved-unknown-agent");
  assert.equal(unknown.querySelector('[data-profile-field="model"]').value, "historic");
  assert.equal(control("docker_service").disabled, true);
  selectValue(control("runtime_target"), "Docker");
  control("runtime_target").dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(control("docker_service").disabled, false);
  assert.deepEqual([...control("docker_service").options].map(option => option.value), ["", "dev", "worker", "saved-service"]);
  selectValue(control("model"), "gpt-6-luna");
  control("model").dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(control("reasoning").value, "medium", "a new model uses its shared default");
  assert.equal(sent.at(-1).profiles[0].reasoning, "medium");
  const custom = panel.querySelector('[data-agent-id="custom"]');
  for (const key of ["reasoning", "fast_mode", "skip_permissions"]) assert.equal(custom.querySelector(`[data-profile-field="${key}"]`).disabled, true, key);
  assert.equal(custom.querySelector('[data-profile-field="fast_mode"]').checked, true, "disabled saved values survive unrelated edits");
  selectValue(control("agent_member_id"), "m-claude");
  control("agent_member_id").dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(control("model").value, "", "Claude default is the empty CLI alias");
  selectValue(control("model"), "haiku");
  control("model").dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(control("reasoning").disabled, true);
  assert.equal(sent.at(-1).profiles[0].reasoning, null, "unsupported models do not inherit the previous model's effort");
  selectValue(control("agent_member_id"), "m-hermes");
  control("agent_member_id").dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(control("model").localName, "input", "Hermes retains arbitrary model input");
  const suggestions = document.getElementById(control("model").getAttribute("list"));
  assert.ok(suggestions, "Hermes offers the configured default provider's model suggestions");
  assert.deepEqual([...suggestions.querySelectorAll("option")].map(option => option.value), ["fixture-model", "other-model"]);
  control("model").value = "custom-hermes-model";
  control("model").dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(sent.at(-1).profiles[0].model, "custom-hermes-model");
  load("renderAgentsSettingsPanel")(panel, {
    profiles: [{ agent_member_id: "m-codex", agent_id: "codex", model: null, reasoning: null }], choices: candidateChoices,
    send: message => sent.push(message),
  });
  assert.equal(control("model").value, "gpt-6-astra");
  assert.equal(control("reasoning").value, "medium");
  selectValue(control("runtime_target"), "Docker");
  control("runtime_target").dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(sent.at(-1).profiles[0].model, null, "displaying a default does not pin it on unrelated saves");
  assert.equal(sent.at(-1).profiles[0].reasoning, null);
});

test("candidate fields save exactly once on change, never on incomplete input", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderAgentsSettingsPanel")(panel, {
    profiles: [{ agent_member_id: "m-codex", agent_id: "codex", model: "gpt-6-astra" }], choices: candidateChoices,
    send: message => sent.push(message),
  });
  for (const input of panel.querySelectorAll('fieldset [data-profile-field]')) {
    if (input.disabled) continue;
    if (input.type === "checkbox") input.checked = !input.checked;
    else if (input.localName === "select") input.options[0].selected = true;
    else input.value = "new-value";
    const before = sent.length;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    assert.equal(sent.length, before, "typing does not persist partial values");
    input.dispatchEvent(new Event("change", { bubbles: true }));
    assert.equal(sent.length, before + 1, input.dataset.profileField);
    assert.equal(sent.at(-1).kind, "issue_monitor_profiles_set");
    input.dispatchEvent(new Event("blur"));
    assert.equal(sent.length, before + 1, "blur does not duplicate change");
  }
  assert.ok(sent.length >= 6, "enabled fields autosave");
  assert.equal(Boolean(panel.querySelector('[data-action="save-profiles"]')), false);
});

test("a correlated save reconciles untouched fields and preserves newer input", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section");
  load("renderAgentsSettingsPanel")(panel, {
    profiles: [{ agent_member_id: "m-codex", agent_id: "codex", model: "old", reasoning: "high" }], choices: candidateChoices,
    send: () => { panel.dataset.pendingRequest = "7"; panel.dataset.saving = "true"; },
  });
  const model = panel.querySelector('[data-profile-field="model"]');
  const reasoning = panel.querySelector('[data-profile-field="reasoning"]');
  selectValue(model, "saved");
  model.dispatchEvent(new Event("change", { bubbles: true }));
  selectValue(reasoning, "unfinished");
  reasoning.dispatchEvent(new Event("input", { bubbles: true }));
  const reconcile = load("reconcileCandidateSave");
  reconcile(panel, { request_id: 6, profiles: [{ agent_member_id: "m-codex", agent_id: "codex", model: "stale" }] });
  assert.equal(model.value, "saved", "stale ACK is ignored");
  reconcile(panel, { request_id: 7, profiles: [{ agent_member_id: "m-codex", agent_id: "codex", model: "normalized", reasoning: "high" }] });
  assert.equal(model.value, "normalized");
  assert.equal(reasoning.value, "unfinished");
  assert.equal(panel.dataset.dirty, "true");
  assert.equal(panel.dataset.saving, "false");
  selectValue(reasoning, "high");
  model.dispatchEvent(new Event("change", { bubbles: true }));
  reconcile(panel, { request_id: 7, profiles: [{ agent_member_id: "m-codex", agent_id: "codex", model: "normalized", reasoning: "high" }] });
  assert.equal(panel.dataset.dirty, "false");
  assert.match(panel.querySelector('[data-candidate-status]').textContent, /Saved candidates/);
});

test("a rejected autosave identifies the edited field and retains its value", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section");
  load("renderAgentsSettingsPanel")(panel, {
    profiles: [{ agent_member_id: "m-codex", agent_id: "codex", prefer_for: [] }, { agent_member_id: "m-claude", agent_id: "claude", prefer_for: [] }], choices: candidateChoices,
    send: () => { panel.dataset.pendingRequest = "1"; },
  });
  const input = panel.querySelector('[data-profile-field="prefer_for"]');
  input.value = "invalid";
  input.dispatchEvent(new Event("change", { bubbles: true }));
  load("renderCandidateSaveError")(panel, { request_id: 1, message: 'prefer_for entry "invalid" must match a supported tag' });
  assert.equal(input.value, "invalid");
  assert.equal(input.getAttribute("aria-invalid"), "true");
  assert.match(input.closest("label").nextElementSibling.textContent, /prefer_for entry "invalid"/);
  const model = panel.querySelector('[aria-label="Model claude"]');
  selectValue(model, "valid-model");
  model.dispatchEvent(new Event("change", { bubbles: true }));
  load("renderCandidateSaveError")(panel, { request_id: 1, message: 'prefer_for entry "invalid" must match a supported tag' });
  assert.equal(input.getAttribute("aria-invalid"), "true");
  assert.equal(model.hasAttribute("aria-invalid"), false);
});

test("structural saves wait for their correlated response before allowing row edits", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  load("renderAgentsSettingsPanel")(panel, {
    profiles: [{ agent_member_id: "m-claude", agent_id: "claude", model: "sonnet" }, { agent_member_id: "m-codex", agent_id: "codex", model: "gpt" }],
    send: message => { sent.push(message); panel.dataset.pendingRequest = "1"; },
  });
  panel.querySelector('[aria-label="Move up codex"]').click();
  const model = panel.querySelector('[aria-label="Model claude"]');
  assert.equal(model.disabled, true);
  model.dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(sent.length, 1);
  assert.equal(load("reconcileCandidateSave")(panel, { request_id: 1, profiles: sent[0].profiles }), true,
    "structural ACK requests an authoritative redraw");
});

test("Candidates follow the shared roster before version metadata and retain dirty controls", () => {
  const { document, Event } = parseHTML("<html><body><main></main></body></html>");
  const window = document.defaultView, sent = [];
  const frames = new Set(), flush = () => { for (const fn of [...frames]) { frames.delete(fn); fn(); } };
  const rosterModel = createUiStateStore({ revision: null, members: [] }, { scheduleFrame: fn => { frames.add(fn); return () => frames.delete(fn); } });
  const factory = Function("document", "window", "createInteractionGuard", "createUiStateStore", "setControlExplanation", "bindControlExplanations",
    "renderIndexSettingsPanel", "renderCustomAgentEnvEditor", `${source.replace(/^import .*;\r?\n/gm, "").replace(/^export /gm, "")}\nreturn createSettingsSurface;`)(
    document, window, createInteractionGuard, createUiStateStore, setControlExplanation, () => {}, () => {}, renderCustomAgentEnvEditor);
  const surface = factory({ send: message => sent.push(message), rosterModel,
    createNode(tag, className, text) { const node = document.createElement(tag); if (className) node.className = className; if (text) node.textContent = text; return node; },
    focusWindowLocally() {}, activeProjectTab: () => ({ project_root: "/A" }), focusOrSpawnPreset() {}, renderUsagePanel() {}, pmSettingsPanel: { mount() {} },
    mountIssues(_windowData, host) { host.innerHTML = '<div class="issue-bridge-root"><div class="knowledge-monitor-bar"></div><div class="workspace-toolbar"></div><button data-action="monitor-settings"></button></div>'; },
  });
  surface.renderSettingsWindow(document.querySelector("main"), { id: "settings" });
  surface.applyIssueMonitorProfiles({ project_root: "/A", profiles: [{ agent_member_id: "m-claude", agent_id: "claude" }], choices: candidateChoices });
  const panel = document.querySelector("[data-settings-candidates]");
  assert.equal(panel.querySelector('[data-profile-field="new"]').disabled, true);
  assert.equal(typeof surface.refreshProfilesAfterRoster, "function");
  const readCount = () => sent.filter(message => message.kind === "get_issue_monitor_profiles").length;
  surface.applyIssueMonitorProfiles({ project_root: "/A", profiles: [{ agent_id: "claude" }], choices: candidateChoices });
  rosterModel.update(() => ({ project_key: "p", revision: 1, members: candidateMembers }));
  const beforeAck = readCount();
  surface.refreshProfilesAfterRoster({ project_key: "other", revision: 1 });
  assert.equal(readCount(), beforeAck, "another project's ACK cannot authorize reads");
  surface.refreshProfilesAfterRoster({ project_key: "p", revision: 1 });
  assert.equal(readCount(), beforeAck + 1);
  surface.applyIssueMonitorProfiles({ project_root: "/A", profiles: [{ agent_id: "claude" }], choices: candidateChoices });
  assert.equal(readCount(), beforeAck + 1, "a stale unbound response cannot create a retry loop");
  surface.applyIssueMonitorProfiles({ project_root: "/A", profiles: [{ agent_member_id: "m-claude", agent_id: "claude" }], choices: candidateChoices });
  surface.applyIssueMonitorProfiles({ project_root: "/A", profiles: [{ agent_id: "claude" }], choices: candidateChoices });
  assert.equal(readCount(), beforeAck + 2, "an unbound reply arriving after the ACK uses the same readback path");
  surface.refreshProfilesAfterRoster({ project_key: "p", revision: 1 });
  assert.equal(readCount(), beforeAck + 3, "a completed profile-only migration authorizes readback without a roster revision change");
  surface.applyIssueMonitorProfiles({ project_root: "/A", profiles: [], choices: candidateChoices });
  surface.refreshProfilesAfterRoster({ project_key: "p", revision: 1 });
  assert.equal(readCount(), beforeAck + 3, "an empty saved pool is complete and does not request another read");
  surface.applyIssueMonitorProfiles({ project_root: "/A", profiles: [{ agent_member_id: "m-claude", agent_id: "claude" }], choices: candidateChoices });
  flush();
  assert.equal(panel.querySelector('[data-profile-field="new"]').disabled, false);
  assert.deepEqual([...panel.querySelector('[data-profile-field="new"]').options].map(option => option.value), ["", "m-codex", "m-hermes", "m-custom"]);
  const model = panel.querySelector('[data-profile-field="model"]'); selectValue(model, "draft"); model.dispatchEvent(new Event("input", { bubbles: true }));
  rosterModel.update(state => ({ revision: 2, members: state.members.map(member => ({ ...member, name: `New ${member.name}` })) })); flush();
  assert.ok(panel.querySelector('[data-profile-field="model"]') === model);
  assert.equal(model.value, "draft");
});

test("Candidates disable addition with an accessible reason when the catalog is exhausted", () => {
  const { document } = parseHTML("<section></section>");
  const panel = document.querySelector("section");
  load("renderAgentsSettingsPanel")(panel, {
    profiles: [{ agent_member_id: "m-codex", agent_id: "Codex" }], availableMembers: [candidateMembers[0]],
    send: () => assert.fail("no available candidate"),
  });
  assert.equal(panel.querySelector('[data-action="add-candidate"]').disabled, true);
  assert.match(panel.querySelector('[data-candidate-status]').textContent, /All registered members.*candidate pool/);
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


test("persistent roster edits project member capacity while retaining the separate effective allocation", () => {
  const { document } = parseHTML("<section></section>");
  const panel = document.querySelector("section");
  load("renderIssuesSettingsPanel")(panel, { project: { project_root: "/A" }, status: {
    agent_roster_capacity: 5, agent_roster_revision: 3, max_active_agents: 2, max_active_agents_override: null,
  }, send: () => {} });
  const max = panel.querySelector('[aria-label="Max active"]');
  const mode = panel.querySelector('[aria-label="Max active mode"]');
  assert.equal(max.value, "5");
  assert.equal(max.disabled, false);
  assert.equal(mode.disabled, true);
  assert.match(max.dataset.issueExplanation, /members.*project/i);
  panel.updateMaxActiveStatus({ agent_roster_capacity: 6, agent_roster_revision: 4, max_active_agents: 1 });
  assert.equal(max.value, "6");
});


test("Agents Settings binds registered members and keeps tool choices fixed", () => {
  const { document, Event } = parseHTML("<section></section>");
  const panel = document.querySelector("section"), sent = [];
  const availableMembers = [
    { member_id: "a", name: "Aster", agent_id: "codex", enabled: true },
    { member_id: "b", name: "Birch", agent_id: "codex", enabled: true },
    { member_id: "c", name: "Cedar", agent_id: "claude", enabled: true },
  ];
  load("renderAgentsSettingsPanel")(panel, { availableMembers, choices: candidateChoices,
    profiles: [{ agent_member_id: "a", agent_id: "codex", model: "gpt-6-astra", reasoning: "high" }],
    send: message => sent.push(message) });
  const member = panel.querySelector('[data-profile-field="agent_member_id"]');
  assert.ok(member, "Agent selects registered member identity");
  assert.deepEqual([...member.options].map(option => option.value), ["a", "b", "c"]);
  assert.equal(panel.querySelector('[data-profile-field="agent_id"]'), null, "tool is not editable");
  assert.equal(panel.querySelector('[data-profile-tool]').textContent, "Codex");
  selectValue(member, "b"); member.dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(panel.querySelector("legend").textContent, "1. Birch");
  assert.equal(member.getAttribute("aria-label"), "Agent Birch");
  assert.ok(panel.querySelector('[aria-label="Model Birch"]'));
  assert.equal(panel.querySelector('[aria-label="Move up Birch"]').textContent, "Move up Birch");
  assert.ok(panel.querySelector('[aria-label="Remove Birch"]'));

  assert.deepEqual(sent.at(-1).profiles[0], { agent_member_id: "b", agent_id: "codex", model: "gpt-6-astra", reasoning: "high", docker_service: null, prefer_for: [], session_mode: "Normal", runtime_target: "Host", windows_shell: null, skip_permissions: false, fast_mode: false });
  selectValue(member, "c"); member.dispatchEvent(new Event("change", { bubbles: true }));
  assert.equal(sent.at(-1).profiles[0].agent_id, "claude");
  assert.equal(sent.at(-1).profiles[0].model, null, "different tools use their own default");
  assert.equal(sent.at(-1).profiles[0].reasoning, "auto");
  assert.equal(panel.querySelector('[data-profile-tool]').textContent, "Claude Code");
  const add = panel.querySelector('[data-profile-field="new"]');
  selectValue(add, "a"); panel.querySelector('[data-action="add-candidate"]').click();
  assert.deepEqual(sent.at(-1).profiles.map(profile => profile.agent_member_id), ["c", "a"]);
  assert.equal(sent.at(-1).profiles[1].agent_id, "codex");
});

test("Agents Settings preserves unavailable and resting members without provider substitution", () => {
  const { document } = parseHTML("<section></section>");
  const panel = document.querySelector("section");
  load("renderAgentsSettingsPanel")(panel, { choices: candidateChoices,
    availableMembers: [{ member_id: "rest", name: "Resting member", agent_id: "codex", enabled: false }],
    profiles: [{ agent_member_id: "rest", agent_id: "codex" }, { agent_member_id: "missing", agent_id: "claude", model: "saved-model" }],
    send: () => assert.fail("rendering must not substitute a member") });
  assert.match(panel.querySelector('[data-agent-member-id="rest"]').textContent, /Resting/);
  assert.match(panel.querySelector('[data-agent-member-id="missing"]').textContent, /unavailable/i);
  assert.equal(panel.querySelector('[data-agent-member-id="missing"] [data-profile-field="agent_member_id"]').value, "missing");
  assert.equal(panel.querySelector('[data-agent-member-id="missing"] [data-profile-field="model"]').value, "saved-model");
});
