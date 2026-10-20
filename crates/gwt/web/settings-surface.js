// SPEC-3064 Phase 3 (E4) — Settings windows surface extracted from app.js.
// Owns the Settings window body (tabbed System / Custom Agents / Agent
// Backends / Usage & Limits surface), the customAgentsModel /
// agentBackendsModel / systemSettingsModel stores, the Teams channel link
// converters, the add-from-preset flow, the autostart appliers, and the
// systemSettingsInteractionGuard that defers destructive System panel
// re-renders while a native <select> dropdown is open. Pure movement from
// app.js: behavior, DOM output, and WS protocol are unchanged; the moved
// code keeps its original app.js indentation.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - createNode(tag, className, textContent): shared DOM helper owned by
//   app.js (also used by Board / wizard surfaces).
// - focusWindowLocally(windowId): local z-order focus bookkeeping.
// - activeProjectTab(): active project tab accessor.
// - focusOrSpawnPreset(preset): focus-or-spawn a preset window (used by the
//   document-level "settings:open" dispatch).
// - renderUsagePanel(panel): Usage & Limits panel renderer. Instance
//   function created by createProviderUsageSurface in app.js (its usage
//   snapshot state lives there), so it must be injected rather than
//   imported as a sibling module.
// - getIndexStatus: per-project shared index status snapshot owned by the
//   Project Index surface instance created in app.js (E3).
import { createInteractionGuard } from "/interaction-guard.js";
import { renderIndexSettingsPanel } from "/index-settings-panel.js";
import { renderCustomAgentEnvEditor } from "/custom-agent-env-editor.js";
import { createUiStateStore } from "./ui-state-store.js";
import { setControlExplanation } from "./control-explanations.js";

function renderUpdateSettingsStatus(panel, status) {
  const document = panel.ownerDocument;
  panel.replaceChildren();
  const values = document.createElement("p");
  values.className = "settings-help";
  const configured = status.auto_apply_updates;
  values.textContent = `Saved: ${configured == null ? "Default (Off)" : configured ? "On" : "Off"} · Effective: ${status.auto_apply_updates_effective ? "On" : "Off"}`;
  panel.appendChild(values);
  const drain = status.update_drain;
  const pending = document.createElement("p");
  pending.className = "settings-help";
  pending.textContent = drain ? `Update ${drain.version} pending · ${drain.reason || "waiting"}` : "No update pending";
  panel.appendChild(pending);
  if (!drain) return;
  const list = document.createElement("ul");
  list.className = "settings-help";
  list.setAttribute("aria-label", "Update blockers");
  for (const blocker of drain.blocking || []) {
    const item = document.createElement("li");
    switch (blocker.kind) {
      case "active_pane": item.textContent = `Agent: ${blocker.label} (${blocker.state})`; break;
      case "pending_acquire_claim": item.textContent = `Pending claim: #${blocker.issue_number}`; break;
      case "active_execution": item.textContent = `Active execution: ${blocker.worktree}`; break;
      case "held_verification_lease": item.textContent = `Verification: ${blocker.lease_id}`; break;
      default: item.textContent = blocker.kind || "Waiting for work to finish";
    }
    list.appendChild(item);
  }
  if (!list.children.length) pending.textContent += " · waiting for quiet observations";
  panel.appendChild(list);
}

function settingsTabId(windowId, tabId) {
  return `settings-${windowId}-tab-${tabId}`;
}

function settingsPanelId(windowId, tabId) {
  return `settings-${windowId}-panel-${tabId}`;
}

function linkSettingsPanel(panel, windowId, tabId) {
  panel.id = settingsPanelId(windowId, tabId);
  panel.setAttribute("aria-labelledby", settingsTabId(windowId, tabId));
}

// Keep vendor banners and build hashes in diagnostics, not comparison columns.
function supportedAgentVersion(value) {
  return value?.match(/(?:^|[\s(])v?(\d+\.\d+\.\d+(?:-[\da-z.-]+)?)(?:\+[\da-z.-]+)?(?=$|[\s)])/i)?.[1] || null;
}

// SPEC #5017: maintain the built-in catalog without changing launch profiles.
function renderSupportedAgentsPanel(panel, agents, options) {
  const document = panel.ownerDocument;
  panel.replaceChildren();
  if (agents === null) {
    const loading = document.createElement("p");
    loading.className = "settings-help";
    loading.setAttribute("role", "status");
    loading.textContent = "Loading agent detection…";
    panel.appendChild(loading);
    return;
  }
  const send = options?.send || (() => {});
  const maintenance = options?.maintenanceByAgent || new Map();
  const busy = Array.from(maintenance.values()).some((entry) => entry.pending);
  const automaticLabel = document.createElement("label");
  automaticLabel.className = "settings-checkbox-label";
  const automatic = document.createElement("input");
  automatic.type = "checkbox";
  automatic.className = "settings-checkbox";
  automatic.setAttribute("aria-label", "Automatically update agents");
  automatic.checked = options?.autoUpdate === true;
  automatic.disabled = busy;
  automatic.addEventListener("change", () => {
    send({ kind: "set_agent_auto_update", enabled: automatic.checked });
  });
  automaticLabel.appendChild(automatic);
  automaticLabel.appendChild(document.createTextNode("Automatically update agents"));
  panel.appendChild(automaticLabel);
  const help = document.createElement("p");
  help.className = "settings-help";
  help.textContent = "At the next app startup, update installed agents with a verified update route and a known newer version. "
    + "Updates require no open agent windows or pending launches, including the Project Manager. "
    + "Nothing is installed automatically. Manual installs and updates are refused while agent windows are running; "
    + "close them and retry. Update failures leave launches available.";
  panel.appendChild(help);
  const globalResult = maintenance.get("");
  if (globalResult?.message) {
    const status = document.createElement("p");
    status.className = "settings-status";
    status.setAttribute("role", globalResult.success === false ? "alert" : "status");
    status.textContent = globalResult.message;
    panel.appendChild(status);
  }
  const table = document.createElement("table");
  table.className = "settings-supported-agents";
  table.setAttribute("aria-label", "Supported agents and installed versions");
  const head = document.createElement("thead");
  const header = document.createElement("tr");
  head.appendChild(header);
  table.appendChild(head);
  for (const label of ["Agent", "Status", "Installed version", "Available version", "Actions"]) {
    const cell = document.createElement("th");
    cell.setAttribute("scope", "col");
    cell.textContent = label;
    cell.title = label;
    header.appendChild(cell);
  }
  const body = document.createElement("tbody");
  table.appendChild(body);
  for (const agent of agents) {
    const row = document.createElement("tr");
    body.appendChild(row);
    row.dataset.agentId = agent.id;
    const rawVersion = agent.installed_version?.trim();
    const version = supportedAgentVersion(rawVersion);
    const values = [
      `${agent.name} (${agent.id})`,
      agent.installed ? "Installed" : "Not installed",
      agent.installed
        ? version || "Unknown"
        : "Not installed",
      agent.available_version
        ? agent.up_to_date
          ? `${agent.available_version} · Latest`
          : agent.available_version
        : "Unknown",
    ];
    for (const value of values) {
      const cell = document.createElement("td");
      cell.textContent = value;
      cell.title = value;
      row.appendChild(cell);
    }
    if (agent.installed) row.children[2].title = rawVersion || "Installed version unavailable";
    row.children[3].title = agent.update_check_error || agent.available_version || "Available version unavailable";
    const actions = document.createElement("td");
    const action = !agent.installed && agent.install_supported
      ? "install"
      : agent.installed && agent.update_available ? "update" : null;
    if (action) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "wizard-button";
      const label = `${action === "install" ? "Install" : "Update"} ${agent.name}`;
      button.textContent = action === "install" ? "Install" : "Update";
      button.setAttribute("aria-label", label);
      button.disabled = busy;
      button.addEventListener("click", () => {
        send({ kind: "maintain_supported_agent", agent_id: agent.id, action });
      });
      actions.appendChild(button);
    }
    const result = maintenance.get(agent.id);
    if (result?.message) {
      const status = document.createElement("p");
      status.className = "settings-status";
      status.setAttribute("role", result.success === false ? "alert" : "status");
      status.textContent = result.message;
      if (result.before_version || result.after_version) {
        status.textContent += ` Before: ${result.before_version || "Unknown (version unavailable)"}. `
          + `After: ${result.after_version || "Unknown (version unavailable)"}.`;
      }
      status.title = status.textContent;
      actions.appendChild(status);
    }
    row.appendChild(actions);
  }
  panel.appendChild(table);
}

function scopeSettingsControlIds(panel) {
  for (const control of panel.querySelectorAll("input[id], select[id]")) {
    const id = control.id;
    control.id = `${panel.id}-${id}`;
    for (const label of panel.querySelectorAll("label[for]")) {
      if (label.getAttribute("for") === id) label.setAttribute("for", control.id);
    }
  }
}

export function mountProjectManagerSettingsPanel(
  document,
  parent,
  pmSettingsPanel,
  windowId = "shared",
) {
  const panel = document.createElement("section");
  panel.className = "settings-panel hidden";
  panel.setAttribute("role", "tabpanel");
  panel.dataset.settingsPanel = "project-manager";
  panel.id = `settings-${windowId}-panel-project-manager`;
  panel.setAttribute(
    "aria-labelledby",
    `settings-${windowId}-tab-project-manager`,
  );
  parent.appendChild(panel);
  pmSettingsPanel?.mount?.(panel);
  return panel;
}

function renderIssuesSettingsPanel(panel, options) {
  const { project, status, send, launchControls = [] } = options;
  const document = panel.ownerDocument;
  panel.replaceChildren();
  const heading = document.createElement("h3");
  heading.className = "settings-section-heading";
  heading.textContent = `Project settings · ${project?.name || project?.project_root || "No project"}`;
  panel.appendChild(heading);
  const description = document.createElement("p");
  description.className = "settings-help settings-issues-description";
  description.textContent = project?.project_root || "Open a project to configure Issues.";
  description.title = description.textContent;
  panel.appendChild(description);
  const group = name => {
    const section = document.createElement("fieldset");
    section.className = "settings-section";
    section.disabled = !status;
    const legend = document.createElement("legend");
    legend.textContent = name;
    section.appendChild(legend);
    panel.appendChild(section);
    return section;
  };
  const row = (section, name, control) => {
    const label = document.createElement("label");
    label.className = "settings-field";
    const caption = document.createElement("span");
    caption.textContent = name;
    label.appendChild(caption);
    label.appendChild(control);
    section.appendChild(label);
    return label;
  };
  const field = (section, name, type, value) => {
    const input = document.createElement("input");
    input.type = type;
    input.className = type === "checkbox" ? "settings-checkbox" : "settings-input";
    input.setAttribute("aria-label", name);
    if (type === "checkbox") input.checked = value === true;
    else input.value = String(value ?? "");
    row(section, name, input);
    return input;
  };
  const launch = group("Launch control");
  for (const [action, name] of [["monitor-autonomous", "Autonomous"], ["monitor-toggle", "Monitoring"]]) {
    const slot = document.createElement("span");
    const control = launchControls.find(control => control.dataset.action === action);
    if (control) slot.appendChild(control);
    row(launch, name, slot).dataset.settingsLaunchAction = action;
  }
  if (panel.dataset.maxActiveProject !== project?.project_root) delete panel.dataset.maxActiveDraft;
  panel.dataset.maxActiveProject = project?.project_root || "";
  let committedMax = status?.max_active_agents_override;
  const max = field(launch, "Max active", "number", committedMax == null
    ? status?.max_active_agents ?? 0
    : panel.dataset.maxActiveDraft ?? committedMax);
  const capacityExplanation = "Maximum number of issue agents the monitor may run at once. Auto uses the current recommended capacity; Manual sets a fixed maximum.";
  setControlExplanation(max, capacityExplanation, "settings-max-active");
  launch.insertBefore(max.closest("label"), launch.querySelector('[data-settings-launch-action="monitor-toggle"]'));
  max.min = "1";
  max.step = "1";
  const mode = document.createElement("select");
  mode.className = "settings-select";
  mode.setAttribute("aria-label", "Max active mode");
  setControlExplanation(mode, "Choose Automatic capacity from current system limits, or Manual to set a fixed maximum.", "settings-max-active-mode");
  for (const [value, name] of [["auto", "Auto"], ["manual", "Manual"]]) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = name;
    option.selected = value === (status?.max_active_agents_override == null ? "auto" : "manual");
    mode.appendChild(option);
  }
  max.disabled = status?.max_active_agents_override == null;
  const capacity = document.createElement("span");
  capacity.className = "settings-issues-capacity";
  setControlExplanation(capacity, capacityExplanation, "settings-capacity");
  max.replaceWith(capacity);
  const apply = document.createElement("button");
  apply.type = "button";
  apply.className = "wizard-button";
  apply.textContent = "Apply";
  apply.setAttribute("aria-label", "Apply Max active");
  setControlExplanation(apply, "Save the final Manual limit. Editing the number does not save intermediate values.", "settings-max-active-apply");
  const updateApply = () => {
    const value = Number(max.value);
    apply.disabled = max.disabled || !Number.isInteger(value) || value < 1 || value === committedMax;
  };
  updateApply();
  capacity.append(mode, max, apply);
  const message = document.createElement("span");
  message.className = "settings-help";
  message.setAttribute("role", "status");
  capacity.appendChild(message);
  let latestStatus = status;
  panel.updateMaxActiveStatus = nextStatus => {
    latestStatus = nextStatus;
    const wasAuto = max.disabled;
    committedMax = nextStatus?.max_active_agents_override ?? null;
    max.disabled = committedMax == null;
    mode.querySelector(`[value="${max.disabled ? "auto" : "manual"}"]`).selected = true;
    if (max.disabled || wasAuto || panel.dataset.maxActiveDraft == null) {
      delete panel.dataset.maxActiveDraft;
      max.value = String(committedMax ?? nextStatus?.max_active_agents ?? 0);
    }
    updateApply();
    message.textContent = max.disabled && !nextStatus?.agent_capacity?.measurement_complete
      ? "Measuring capacity…"
      : Number(nextStatus?.active_count) > Number(nextStatus?.max_active_agents)
        ? "Existing agents continue; new launches wait until active agents fall below this limit."
        : "";
    message.hidden = !message.textContent;
  };
  panel.updateMaxActiveStatus(status);
  mode.addEventListener("change", () => {
    delete panel.dataset.maxActiveDraft;
    max.disabled = mode.value === "auto";
    if (!max.disabled && !(Number.isInteger(Number(max.value)) && Number(max.value) > 0)) max.value = "1";
    committedMax = max.disabled ? null : Number(max.value);
    updateApply();
    panel.updateMaxActiveStatus({ ...latestStatus, max_active_agents_override: committedMax,
      max_active_agents: committedMax ?? latestStatus?.max_active_agents });
    send({ kind: "set_issue_monitor_max_active_agents",
      max_active_agents: committedMax });
  });
  const editMax = () => {
    if (max.disabled) return;
    panel.dataset.maxActiveDraft = max.value;
    updateApply();
  };
  max.addEventListener("input", editMax);
  max.addEventListener("change", editMax);
  apply.addEventListener("click", () => {
    if (apply.disabled || latestStatus?.max_active_agents_override == null) return;
    const value = Number(max.value);
    committedMax = value;
    delete panel.dataset.maxActiveDraft;
    updateApply();
    send({ kind: "set_issue_monitor_max_active_agents", max_active_agents: value });
  });
  const replenishment = group("Refill");
  const refill = field(replenishment, "Auto-refill", "checkbox", status?.terminal_queue_auto_refill);
  const limit = field(replenishment, "Refill limit", "number", status?.terminal_queue_auto_refill_limit ?? 3);
  setControlExplanation(refill, "Automatically add eligible issues to the launch queue when it has fewer issues than the refill limit.", "settings-auto-refill");
  setControlExplanation(limit, "Target number of eligible issues kept in the launch queue by Auto-refill. 0 disables automatic refill.", "settings-refill-limit");
  limit.min = "0";
  limit.step = "1";
  const saveRefill = () => {
    const value = Number(limit.value);
    if (Number.isInteger(value) && value >= 0) send({ kind: "set_issue_monitor_auto_refill", enabled: refill.checked, limit: value });
  };
  refill.addEventListener("change", saveRefill);
  limit.addEventListener("change", saveRefill);
  const labels = field(group("Target"), "Allowed labels", "text", (status?.allowed_labels || []).join(", "));
  setControlExplanation(labels, "Comma-separated labels. Only matching issues enter the launch queue.", "settings-allowed-labels");
  labels.addEventListener("change", () => send({ kind: "set_issue_monitor_allowed_labels", allowed_labels: [...new Set(labels.value.split(",").map(v => v.trim()).filter(Boolean))] }));
}

function renderCandidateSaveError(panel, event) {
  if (!event || Number(panel.dataset.pendingRequest) !== event.request_id) return;
  panel.dataset.saving = "false";
  delete panel.dataset.pendingRequest;
  for (const [control, disabled] of panel.candidateDisabledControls || []) control.disabled = disabled;
  panel.candidateStructuralSave = false;
  const feedback = panel.querySelector("[data-candidate-status]");
  if (feedback) {
    feedback.textContent = event.message;
    feedback.dataset.kind = "error";
    feedback.setAttribute("role", "alert");
    const field = event.message.match(/\b(prefer_for|agent_id|model|reasoning|session_mode|runtime_target|docker_service|windows_shell|skip_permissions|fast_mode)\b/)?.[1];
    const row = panel.candidateSaveTarget?.closest("fieldset") || panel;
    const rejectedTag = event.message.match(/prefer_for entry ("(?:\\.|[^"\\])*")/)?.[1];
    const rejectedInput = rejectedTag && [...panel.querySelectorAll('[data-profile-field="prefer_for"]')]
      .find(control => control.value.split(",").some(tag => tag.trim() === JSON.parse(rejectedTag)));
    const input = rejectedInput || (field && row.querySelector(`[data-profile-field="${field}"]`)) || panel.candidateSaveTarget;
    if (input?.isConnected) {
      input.setAttribute("aria-invalid", "true");
      input.closest(".settings-field")?.after(feedback);
    }
  }
}

function reconcileCandidateSave(panel, event) {
  if (!event || Number(panel.dataset.pendingRequest) !== event.request_id) return;
  panel.dataset.saving = "false";
  delete panel.dataset.pendingRequest;
  if (panel.candidateStructuralSave) {
    panel.dataset.dirty = "false";
    panel.candidateStructuralSave = false;
    return true;
  }
  let dirty = [...panel.candidateSubmittedValues || []].some(([input, value]) =>
    (input.type === "checkbox" ? input.checked : input.value) !== value);
  const focused = panel.ownerDocument.activeElement;
  for (const [index, row] of [...panel.querySelectorAll("fieldset[data-agent-id]")].entries()) {
    const profile = event.profiles?.[index];
    if (!profile) continue;
    for (const input of row.querySelectorAll("[data-profile-field]")) {
      const value = input.type === "checkbox" ? input.checked : input.value;
      if (panel.candidateSubmittedValues?.get(input) !== value) {
        dirty = true;
        continue;
      }
      // Even a successful normalization must not move a live text cursor.
      if (input === focused) continue;
      const key = input.dataset.profileField;
      if (input.dataset.inheritedDefault !== undefined && !input.dataset.profileEdited && profile[key] == null) continue;
      if (input.type === "checkbox") input.checked = profile[key] === true;
      else {
        const saved = key === "prefer_for" ? (profile[key] || []).join(", ")
          : String(profile[key] ?? (key === "session_mode" ? "Normal" : key === "runtime_target" ? "Host" : ""));
        if (input.localName === "select") {
          if (![...input.options].some(option => option.value === saved)) {
            const option = panel.ownerDocument.createElement("option");
            option.value = saved;
            option.textContent = saved || "Auto";
            input.appendChild(option);
          }
          [...input.options].find(option => option.value === saved).selected = true;
        } else input.value = saved;
      }
    }
  }
  if (!dirty) for (const invalid of panel.querySelectorAll('[aria-invalid="true"]')) invalid.removeAttribute("aria-invalid");
  panel.dataset.dirty = String(dirty);
  const feedback = panel.querySelector("[data-candidate-status]");
  if (feedback && !(dirty && feedback.dataset.kind === "error")) {
    feedback.textContent = "Saved candidates.";
    feedback.dataset.kind = "success";
    feedback.setAttribute("role", "status");
  }
}

function positionIssuesLaunchControls(body) {
  const open = body.querySelector(".settings-issues-options")?.hasAttribute("open");
  const summary = body.querySelector(".knowledge-monitor-controls");
  for (const action of ["monitor-autonomous", "monitor-toggle"]) {
    const control = body.querySelector(`[data-action="${action}"]`);
    const slot = body.querySelector(`[data-settings-launch-action="${action}"] > :last-child`);
    if (!control) continue;
    if (open) slot?.appendChild(control);
    else summary?.insertBefore(control, summary.querySelector('[data-action="monitor-settings"]'));
  }
}

export function renderAgentsSettingsPanel(panel, options) {
  const { project, profiles, send, availableAgents = null, choices = null } = options;
  const document = panel.ownerDocument;
  panel.replaceChildren();
  panel.dataset.dirty = "false";
  panel.oninput = () => {
    panel.dataset.dirty = "true";
  };
  const heading = document.createElement("h3");
  heading.className = "settings-section-heading";
  heading.textContent = `Project settings · ${project?.name || project?.project_root || "No project"}`;
  panel.appendChild(heading);
  const help = document.createElement("p");
  help.className = "settings-help";
  help.textContent = `${project?.project_root || "Open a project"} · Candidates are tried in order. Switch to the next candidate when a provider rejects a launch due to a rate limit. Changes apply to future launches.`;
  panel.appendChild(help);
  if (!profiles) {
    help.textContent += " Loading launch profiles…";
    return;
  }
  const feedback = document.createElement("p");
  feedback.className = "settings-status";
  feedback.dataset.candidateStatus = "";
  const showFeedback = (message, error = false) => {
    feedback.textContent = message;
    feedback.dataset.kind = error ? "error" : "info";
    feedback.setAttribute("role", error ? "alert" : "status");
    if (error) panel.dataset.dirty = "true";
  };
  // Match resolve_agent_id aliases and the backend's provider comparison.
  const providerAliases = new Map([
    ["claudecode", "claude"], ["claude-code", "claude"], ["claude code", "claude"],
    ["grok build", "grok"], ["grok-build", "grok"],
    ["antigravity", "agy"], ["antigravity cli", "agy"], ["antigravity-cli", "agy"],
    ["open-code", "opencode"], ["open-claw", "openclaw"],
    ["hermes agent", "hermes"], ["hermes-agent", "hermes"],
    ["copilot", "gh"], ["github copilot", "gh"], ["github-copilot", "gh"],
  ]);
  const provider = agentId => {
    const id = String(agentId ?? "").trim().replace(/[A-Z]/g, character => character.toLowerCase());
    return providerAliases.get(id) || id;
  };
  const rows = [];
  const setChoices = (input, options, value) => {
    input.replaceChildren();
    const entries = options.map(option => typeof option === "string" ? { value: option, label: option || "Auto" } : option);
    const saved = String(value ?? "");
    if (!entries.some(option => option.value === saved)) entries.push({ value: saved, label: saved || "Auto" });
    for (const entry of entries) {
      const option = document.createElement("option");
      option.value = entry.value;
      option.textContent = entry.label;
      option.selected = entry.value === saved;
      input.appendChild(option);
    }
  };
  const field = (parent, name, key, value, type = "text", choices = null) => {
    const label = document.createElement("label");
    label.className = "settings-field";
    const span = document.createElement("span");
    span.textContent = name;
    label.appendChild(span);
    const input = document.createElement(choices ? "select" : "input");
    if (choices) setChoices(input, choices, value);
    else input.type = type;
    input.className = choices ? "settings-select" : type === "checkbox" ? "settings-checkbox" : "settings-input";
    input.dataset.profileField = key;
    input.setAttribute("aria-label", `${name} ${parent.dataset.agentId || "candidate"}`);
    if (type === "checkbox") input.checked = value === true;
    else if (!choices) input.value = String(value ?? "");
    label.appendChild(input);
    parent.appendChild(label);
    if (key !== "new") input.addEventListener("change", () => {
      if (!input.disabled) {
        input.dataset.profileEdited = "true";
        parent.updateCandidateChoices?.(key);
        submit(draft(), input);
      }
    });
    return input;
  };
  const draft = () => rows.map(({ row, profile }) => {
    const next = { ...profile };
    for (const input of row.querySelectorAll("[data-profile-field]")) {
      const key = input.dataset.profileField;
      if (input.dataset.inheritedDefault !== undefined && !input.dataset.profileEdited) continue;
      next[key] = input.type === "checkbox" ? input.checked
        : key === "prefer_for" ? input.value.split(",").map(v => v.trim()).filter(Boolean)
          : input.value.trim() || null;
    }
    return next;
  });
  const submit = (next, input = null) => {
    panel.candidateSaveTarget = input;
    panel.candidateSubmittedValues = new Map([...panel.querySelectorAll("[data-profile-field]")]
      .map(control => [control, control.type === "checkbox" ? control.checked : control.value]));
    // Structural responses change row identities. Freeze those controls until
    // the correlated result so input cannot attach to a different candidate.
    panel.candidateStructuralSave = input === null;
    panel.candidateDisabledControls = new Map();
    if (panel.candidateStructuralSave) {
      for (const control of panel.querySelectorAll("input, select, button")) {
        panel.candidateDisabledControls.set(control, control.disabled);
        control.disabled = true;
      }
    }
    showFeedback("Saving candidates…");
    send({ kind: "issue_monitor_profiles_set", profiles: next });
  };
  const button = (parent, title, action, callback) => {
    const node = document.createElement("button");
    node.type = "button";
    node.className = "wizard-button";
    node.textContent = title;
    node.dataset.action = action;
    node.setAttribute("aria-label", title);
    node.addEventListener("click", callback);
    parent.appendChild(node);
    return node;
  };
  const poolHeading = document.createElement("h3");
  poolHeading.className = "settings-section-heading";
  poolHeading.textContent = "Candidates";
  panel.appendChild(poolHeading);
  for (const [index, profile] of profiles.entries()) {
    const row = document.createElement("fieldset");
    row.className = "settings-section";
    row.dataset.agentId = profile.agent_id;
    const legend = document.createElement("legend");
    legend.textContent = `${index + 1}. ${profile.agent_id}`;
    row.appendChild(legend);
    panel.appendChild(row);
    rows.push({ row, profile });
    const agent = field(row, "Agent", "agent_id", profile.agent_id, "text",
      (availableAgents || []).map(agent => ({ value: agent.id, label: agent.name || agent.id })));
    const capabilities = () => choices?.agents.find(entry => provider(entry.id) === provider(agent.value));
    const initial = capabilities();
    let model = field(row, "Model", "model", profile.model ?? initial?.default_model ?? "", "text", []);
    const initialModel = initial?.models.find(option => option.value === model.value);
    const initialReasoning = initialModel ? initialModel.default_reasoning : initial?.default_reasoning;
    const reasoning = field(row, "Reasoning", "reasoning", profile.reasoning ?? initialReasoning ?? "", "text", []);
    const docker = field(row, "Docker service", "docker_service", profile.docker_service ?? "", "text", [
      { value: "", label: choices?.default_docker_service ? `Auto (${choices.default_docker_service})` : "Auto" },
      ...(choices?.docker_services || []),
    ]);
    field(row, "Prefer for", "prefer_for", (profile.prefer_for || []).join(", "));
    const mode = field(row, "Session mode", "session_mode", profile.session_mode || "Normal", "text", []);
    const runtime = field(row, "Runtime", "runtime_target", profile.runtime_target || "Host", "text", ["Host", "Docker"]);
    const shell = field(row, "Windows shell", "windows_shell", profile.windows_shell || "", "text", ["", "command_prompt", "windows_power_shell", "power_shell_7"]);
    const skip = field(row, "Skip permissions", "skip_permissions", profile.skip_permissions, "checkbox");
    const fast = field(row, "Fast mode", "fast_mode", profile.fast_mode, "checkbox");
    const modelSuggestions = document.createElement("datalist");
    modelSuggestions.id = `candidate-models-${crypto.randomUUID()}`;
    row.appendChild(modelSuggestions);
    row.updateCandidateChoices = changed => {
      const entry = capabilities();
      const agentChanged = changed === "agent_id";
      if (!changed || agentChanged) {
        const value = agentChanged ? entry?.default_model ?? "" : model.value;
        const freeText = Boolean(entry?.supports_model_input && !entry.models.length);
        if (freeText !== (model.localName === "input")) {
          const previous = model.closest("label");
          model = field(row, "Model", "model", value, "text", freeText ? null : []);
          previous.replaceWith(model.closest("label"));
        }
        if (freeText) model.value = value;
        else setChoices(model, entry?.models || [], value);
        modelSuggestions.replaceChildren();
        for (const value of freeText ? entry?.model_suggestions || [] : []) {
          const option = document.createElement("option");
          option.value = value;
          modelSuggestions.appendChild(option);
        }
        if (modelSuggestions.children.length) model.setAttribute("list", modelSuggestions.id);
        else model.removeAttribute("list");
        if (agentChanged) model.dataset.profileEdited = "true";
        model.disabled = !entry || (!freeText && !entry.models.length);
        setChoices(mode, entry?.session_modes || ["Normal"], agentChanged ? "Normal" : mode.value);
        mode.disabled = !entry || entry.session_modes.length <= 1;
        if (agentChanged) fast.checked = false;
        if (agentChanged && !entry?.supports_skip_permissions) skip.checked = false;
      }
      const selectedModel = entry?.models.find(option => option.value === model.value);
      const levels = selectedModel?.reasoning_options || entry?.reasoning_options || [];
      const defaultReasoning = selectedModel ? selectedModel.default_reasoning : entry?.default_reasoning;
      const level = agentChanged || changed === "model"
        ? defaultReasoning ?? "" : reasoning.value;
      if (agentChanged || changed === "model") reasoning.dataset.profileEdited = "true";
      setChoices(reasoning, levels, level);
      reasoning.disabled = levels.length === 0;
      skip.disabled = !entry?.supports_skip_permissions;
      fast.disabled = !entry?.supports_fast_mode;
      docker.disabled = runtime.value !== "Docker";
      shell.disabled = runtime.value === "Docker" || !choices?.windows_shell_supported;
    };
    row.updateCandidateChoices();
    if (profile.model == null) model.dataset.inheritedDefault = model.value;
    if (profile.reasoning == null) reasoning.dataset.inheritedDefault = reasoning.value;
    for (const [direction, offset] of [["up", -1], ["down", 1]]) {
      const move = button(row, `Move ${direction} ${profile.agent_id}`, direction, () => {
        const next = draft();
        [next[index], next[index + offset]] = [next[index + offset], next[index]];
        submit(next);
      });
      move.disabled = index + offset < 0 || index + offset >= profiles.length;
    }
    const remove = button(row, `Remove ${profile.agent_id}`, "remove", () => submit(draft().filter((_, i) => i !== index)));
    remove.disabled = profiles.length <= 1;
  }
  const add = document.createElement("section");
  add.className = "settings-section";
  panel.appendChild(add);
  const remaining = (availableAgents || []).filter(agent => !profiles.some(profile => provider(profile.agent_id) === provider(agent.id)));
  const candidate = field(add, "Add candidate", "new", "", "text", ["", ...remaining.map(agent => agent.id)]);
  candidate.options[0].textContent = "Select an agent…";
  for (const option of candidate.options) {
    const agent = remaining.find(agent => agent.id === option.value);
    if (agent) option.textContent = `${agent.name || agent.id} (${agent.id})`;
  }
  const addButton = button(add, "Add candidate", "add-candidate", () => {
    const agent = candidate.value.trim();
    const next = draft();
    if (!agent) showFeedback("Select an agent to add.", true);
    else if (next.some(profile => provider(profile.agent_id) === provider(agent))) showFeedback(`Agent ${agent} is already in the candidate pool.`, true);
    else if (!availableAgents?.some(known => known.id === agent)) showFeedback(`Unknown agent: ${agent}. Select a known agent.`, true);
    else submit([...next, { agent_id: agent }]);
  });
  candidate.disabled = addButton.disabled = availableAgents === null || remaining.length === 0;
  showFeedback(availableAgents === null ? "Loading available agents…"
    : remaining.length === 0 ? "All known agents are already in the candidate pool." : "Select an agent to add.");
  add.appendChild(feedback);
}

export function switchSettingsTab(body, target, focus = false) {
  const showCandidates = target === "agents";
  if (showCandidates) target = "issues";
  body.dataset.settingsSection = target;
  if (showCandidates) {
    const disclosure = body.querySelector(".settings-issues-options");
    if (disclosure) disclosure.open = true;
  }
  const tabs = body.querySelectorAll(".settings-tab");
  tabs.forEach((tab) => {
    const isSelected = tab.dataset.settingsTab === target;
    tab.setAttribute("aria-selected", String(isSelected));
    tab.classList.toggle("active", isSelected);
  });
  const panels = body.querySelectorAll(".settings-panel");
  panels.forEach((panel) => {
    panel.classList.toggle("hidden", panel.dataset.settingsPanel !== target);
    if (focus && panel.dataset.settingsPanel === target) {
      panel.tabIndex = -1;
      const destination = showCandidates ? panel.querySelector("[data-settings-candidates]") || panel : panel;
      destination.tabIndex = -1;
      const scroller = body.dataset.surfaceOwner === "pm" ? body.querySelector(".settings-body") : null;
      if (scroller) scroller.scrollTop = Math.max(0, destination.offsetTop - scroller.offsetTop);
      else destination.scrollIntoView?.({ block: "start" });
      destination.focus({ preventScroll: true });
    }
  });
  body.dispatchEvent(new body.ownerDocument.defaultView.CustomEvent("settings:section-changed", { bubbles: true,
    detail: { windowId: body.dataset.settingsWindowId, section: target } }));
}

export function installSettingsOpenHandler(options) {
  const {
    document,
    send,
    settingsWindowBodies,
    focusOrSpawnPreset,
    setPendingTarget,
    activeProjectTab,
  } = options;
  document.addEventListener("settings:open", (event) => {
    const target = event?.detail?.section || event?.detail?.target || "system";
    const projectRoot = event?.detail?.project_root || activeProjectTab?.()?.project_root || "";
    const surfaceOwner = event?.detail?.surface_owner === "pm" ? "pm" : null;
    if (target === "index") {
      focusOrSpawnPreset("index");
      return;
    }
    const existingBody = Array.from(settingsWindowBodies).find(
      (settingsBody) => settingsBody.isConnected && (settingsBody.dataset.settingsProjectRoot || "") === projectRoot
        && (settingsBody.dataset.surfaceOwner || null) === surfaceOwner,
    );
    if (existingBody) {
      switchSettingsTab(existingBody, target, true);
      focusOrSpawnPreset("settings", { surfaceOwner });
      if (target === "system") send({ kind: "get_update_settings" });
      return;
    }
    setPendingTarget(target, surfaceOwner);
    focusOrSpawnPreset("settings", { surfaceOwner });
  });
}

export function createSettingsSurface({
  send,
  createNode,
  focusWindowLocally,
  activeProjectTab,
  focusOrSpawnPreset,
  renderUsagePanel,
  getIndexStatus,
  pmSettingsPanel,
  mountIssues,
}) {
      // Issue #2698 PR 4 — same guard applied to the System Settings
      // Output Language `<select>`. Backend echoes `system_settings`
      // and `system_settings_updated` events; if either arrives while
      // the user has the dropdown open, `renderSystemPanel()` does a
      // `while (panel.firstChild) panel.removeChild(panel.firstChild)`
      // pass that destroys the live `<select>` and breaks the user's
      // commit. Delegated listeners scope to `select.settings-select`
      // so the guard covers every Settings window without per-window
      // wiring.
      function applyBoardAuthStatus(event) {
        systemSettingsModel.update(state => ({
          ...state,
          boardAuth: { slack: event.slack === true, teams: event.teams === true },
          boardAuthMessage: event.message || "",
          boardConfig: {
            slackClientId: event.slack_client_id || "",
            slackDefaultChannel: event.slack_default_channel || "",
            slackHasSecret: event.slack_has_secret === true,
            teamsClientId: event.teams_client_id || "",
            teamsTenantId: event.teams_tenant_id || "",
            teamsDefaultChannel: event.teams_default_channel || "",
            oauthRedirectPort: event.oauth_redirect_port || 8765,
          },
        }));
      }

      function applyAutostartStatus(event, statusMessage = systemSettingsModel.read().autostartPending
        ? "Saved login launch setting." : "") {
        systemSettingsModel.update(state => ({
          ...state,
          autostartEnabled: event.enabled === true,
          autostartPreviousEnabled: event.enabled === true,
          autostartMechanism: event.mechanism || "",
          autostartInstallPath: event.install_path || "",
          autostartLoaded: true,
          autostartPending: false,
          autostartChecking: false,
          autostartStatusMessage: statusMessage,
          autostartStatusKind: statusMessage ? "success" : "",
        }));
      }

      function applyAutostartError(event) {
        systemSettingsModel.update(state => ({
          ...state,
          autostartEnabled: state.autostartPending
            ? state.autostartPreviousEnabled === true : state.autostartEnabled,
          autostartPending: false,
          autostartChecking: false,
          autostartStatusMessage: event.message || (state.autostartLoaded
            ? "Failed to update login launch setting."
            : "Failed to check login launch status."),
          autostartStatusKind: "error",
        }));
      }

      // SPEC #1921 Phase 86 (#3813): the backend echoes the complete agent
      // resource policy on every system_settings / system_settings_updated
      // event; it is authoritative over any optimistic local edit.
      function normalizeAgentResource(snapshot, previous) {
        if (!snapshot || typeof snapshot !== "object") return previous;
        return {
          enabled: snapshot.enabled !== false,
          preset: typeof snapshot.preset === "string" && snapshot.preset ? snapshot.preset : "automatic",
          priority: typeof snapshot.priority === "string" && snapshot.priority ? snapshot.priority : "below-normal",
          cpuLimitPercent: Number.isInteger(snapshot.cpu_limit_percent) ? snapshot.cpu_limit_percent : null,
          buildJobs: Number.isInteger(snapshot.build_jobs) ? snapshot.build_jobs : null,
        };
      }

      function applySystemSettingsEvent(event) {
        systemSettingsModel.update(state => {
          if (event.kind === "system_settings_error") {
            return { ...state, statusMessage: event.message || "Failed to update system settings.", statusKind: "error" };
          }
          const updated = event.kind === "system_settings_updated";
          const clearStatus = !state.statusMessage || state.statusKind === "info";
          return {
            ...state,
            language: event.language || (updated ? state.language : "auto"),
            codexTrustManagedHooks: event.codex_trust_managed_hooks !== false,
            boardProvider: event.board_provider || state.boardProvider || "local",
            agentResource: normalizeAgentResource(event.agent_resource, state.agentResource),
            loaded: state.loaded || !updated,
            statusMessage: updated ? "Saved system settings." : clearStatus ? "" : state.statusMessage,
            statusKind: updated ? "success" : clearStatus ? "" : state.statusKind,
          };
        });
      }

      // Empty input = automatic (null). A non-integer or value below 1 is
      // invalid (undefined) and is rejected before anything is sent.
      function parseOptionalPositiveInteger(raw) {
        const text = String(raw ?? "").trim();
        if (text === "") {
          return null;
        }
        const value = Number(text);
        if (!Number.isInteger(value) || value < 1) {
          return undefined;
        }
        return value;
      }

      function agentResourceWirePayload() {
        const resource = systemSettingsModel.read().agentResource;
        return {
          enabled: resource.enabled !== false,
          preset: resource.preset || "automatic",
          priority: resource.priority || "below-normal",
          cpu_limit_percent: Number.isInteger(resource.cpuLimitPercent)
            ? resource.cpuLimitPercent
            : null,
          build_jobs: Number.isInteger(resource.buildJobs)
            ? resource.buildJobs
            : null,
        };
      }

      const systemSettingsInteractionGuard = createInteractionGuard({
        onFlush: () => renderSystemPanelInAllSettingsWindows(),
      });


      // All DOM nodes are built via createElement + textContent to avoid
      // innerHTML with mixed trust. Secrets in env tables are redacted by
      // the backend before reaching this layer (see redact_secrets_in_agent).
      const customAgentsModel = createUiStateStore({
        catalog: { agents: [], loading: false },
        statusMessage: "",
        statusKind: "",
      });
      // SPEC-1921 2026-05-18 amendment / FR-099: Settings > Agent Backends
      // per-built-in backend profile state. `backends` is keyed by
      // BuiltinAgentId string ("claudeCode" / "codex"). Status messages keep
      // using the shared Custom Agents status channel.
      const agentBackendsModel = createUiStateStore({
        backends: { claudeCode: [], codex: [] },
        loadingAgent: null,
      });
      // SPEC-1933 US-4: System tab state. `language` is the raw stored value
      // (auto/en/ja); the backend `system_settings` reply seeds it.
      const systemSettingsModel = createUiStateStore({
        language: "auto",
        codexTrustManagedHooks: true,
        // SPEC #1921 Phase 86 (#3813): agent process-tree resource policy.
        // Numeric null = automatic (derived from max active agents / cores).
        agentResource: {
          enabled: true,
          preset: "automatic",
          priority: "below-normal",
          cpuLimitPercent: null,
          buildJobs: null,
        },
        // SPEC-2959/2963: selected Board backend (local/slack/teams).
        boardProvider: "local",
        loaded: false,
        statusMessage: "",
        statusKind: "",
        // SPEC-2963: remote provider sign-in state + last sign-in message.
        boardAuth: { slack: false, teams: false },
        boardAuthMessage: "",
        // SPEC-2963: editable (non-secret) provider config for the settings UI.
        // Secrets are never echoed back; `*HasSecret` flags reflect store state.
        boardConfig: {
          slackClientId: "",
          slackDefaultChannel: "",
          slackHasSecret: false,
          teamsClientId: "",
          teamsTenantId: "",
          teamsDefaultChannel: "",
          oauthRedirectPort: 8765,
        },
        autostartEnabled: false,
        autostartPreviousEnabled: false,
        autostartMechanism: "",
        autostartInstallPath: "",
        autostartLoaded: false,
        autostartPending: false,
        autostartChecking: false,
        autostartStatusMessage: "",
        autostartStatusKind: "",
      });
      // Renderer cache only: the model remains the sole source of settings values.
      const renderedSystemPanels = new WeakMap();
      const settingsWindowBodies = new Set();
      agentBackendsModel.subscribe(state => state, state => {
        renderAgentBackendsPanelInAllSettingsWindows(state);
      });
      systemSettingsModel.subscribe(state => state, state => {
        renderSystemPanelInAllSettingsWindows(state);
      });
      let nextProfilesRequest = 0;
      const issueSettingsModel = createUiStateStore({});
      const globalUpdateModel = createUiStateStore({ auto_apply_updates: null, auto_apply_updates_effective: false, loaded: false, drains: {} });
      globalUpdateModel.subscribe(state => state, state => {
        purgeDetachedSettingsBodies();
        for (const body of settingsWindowBodies) {
          const toggle = body.querySelector('[aria-label="Auto-apply updates"]');
          if (toggle) {
            toggle.checked = state.auto_apply_updates_effective;
            toggle.disabled = !state.loaded;
          }
          const panel = body.querySelector('[data-update-settings-status]');
          if (panel && state.loaded) renderUpdateSettingsStatus(panel, {
            ...state, update_drain: state.drains[body.dataset.settingsProjectRoot] || null,
          });
        }
      });
      function applyGlobalUpdateSettings(event) {
        if (!event.status) {
          globalUpdateModel.update(state => ({ ...state }));
          return;
        }
        globalUpdateModel.update(state => ({ ...state, ...event.status, loaded: true,
          drains: { ...state.drains, [event.project_root || ""]: event.status.update_drain },
        }));
      }
      function projectSettingsSnapshot(body) {
        const root = body.dataset.settingsProjectRoot || "";
        return issueSettingsModel.read()[root] || {};
      }
      function renderProjectSettings(body) {
        const root = body.dataset.settingsProjectRoot || "";
        const current = projectSettingsSnapshot(body);
        const project = { project_root: root, name: body.dataset.settingsProjectName };
        const launchControls = [...body.querySelectorAll('[data-action="monitor-autonomous"], [data-action="monitor-toggle"]')];
        const focusedLaunchControl = launchControls.find(control => control === document.activeElement);
        const agentsPanel = body.querySelector("[data-settings-candidates]");
        if (agentsPanel) renderCandidateSaveError(agentsPanel, current.profilesFailure);
        if (agentsPanel && reconcileCandidateSave(agentsPanel, {
          request_id: current.profilesRequestId, profiles: current.profiles,
        })) body.querySelector('[data-settings-tab="issues"]')?.focus();
        const dispatch = message => {
          // Project WebSockets own mutation authority. Never replay this
          // window's edit through a different active project's connection.
          if (!root || activeProjectTab()?.project_root !== root) return;
          if (message.kind === "issue_monitor_profiles_set") {
            message.request_id = ++nextProfilesRequest;
            const panel = body.querySelector("[data-settings-candidates]");
            if (panel) {
              panel.dataset.dirty = "true";
              panel.dataset.saving = "true";
              panel.dataset.pendingRequest = String(message.request_id);
            }
          }
          send(message);
        };
        for (const [section, render] of [["issues", renderIssuesSettingsPanel], ["agents", renderAgentsSettingsPanel]]) {
          const panel = body.querySelector(section === "issues" ? "[data-settings-issues-controls]" : "[data-settings-candidates]");
          const editing = panel?.contains(document.activeElement)
            && document.activeElement?.matches(section === "agents" ? "input, select, textarea" : "input:not([type=checkbox]), select, textarea, [aria-label='Apply Max active']");
          if (section === "issues") panel?.updateMaxActiveStatus?.(current.status);
          if (editing) body.dataset.issuesRenderDeferred = "true";
          if (panel && !editing && !(section === "agents" && panel.dataset.dirty === "true")) render(panel, {
            project, status: current.status, profiles: current.profiles, choices: current.choices, send: dispatch, launchControls,
            availableAgents: supportedAgentsModel.read().agents === null || customAgentsModel.read().catalog.loading ? null : [
              ...supportedAgentsModel.read().agents,
              ...customAgentsModel.read().catalog.agents.map(agent => ({ id: agent.id, name: agent.display_name || agent.id })),
            ],
          });
        }
        positionIssuesLaunchControls(body);
        focusedLaunchControl?.focus({ preventScroll: true });
        body.querySelector(".issue-bridge-root")?.dispatchEvent(new document.defaultView.Event("control-explanations:refresh"));
      }
      function applyIssueSettingsStatus(status, projectRoot) {
        if (!projectRoot) return;
        globalUpdateModel.update(state => ({ ...state, drains: { ...state.drains, [projectRoot]: status.update_drain } }));
        issueSettingsModel.update(state => ({ ...state, [projectRoot]: { ...state[projectRoot], status } }));
      }
      function applyIssueMonitorProfiles(event) {
        const root = event.project_root;
        if (!root) return;
        issueSettingsModel.update(state => ({ ...state, [root]: {
          ...state[root], profiles: event.profiles, choices: event.choices, profilesRequestId: event.request_id,
        } }));
      }
      function applyIssueMonitorProfilesError(event) {
        if (!event.project_root) return;
        issueSettingsModel.update(state => ({ ...state, [event.project_root]: {
          ...state[event.project_root], profilesFailure: event,
        } }));
      }
      issueSettingsModel.subscribe(state => state, () => {
        purgeDetachedSettingsBodies();
        for (const body of settingsWindowBodies) {
          renderProjectSettings(body);
        }
      });
      const supportedAgentsModel = createUiStateStore({
        agents: null,
        autoUpdate: false,
        maintenanceByAgent: {},
      });

      function supportedAgentPanelOptions() {
        const state = supportedAgentsModel.read();
        return {
          autoUpdate: state.autoUpdate,
          maintenanceByAgent: new Map(Object.entries(state.maintenanceByAgent)),
          send(message) {
            if (message.kind === "maintain_supported_agent") {
              supportedAgentsModel.update(current => ({
                ...current,
                maintenanceByAgent: {
                  ...current.maintenanceByAgent,
                  [message.agent_id]: {
                    pending: true,
                    message: message.action === "install" ? "Installing…" : "Updating…",
                  },
                },
              }));
            }
            send(message);
          },
        };
      }

      function renderSupportedAgentPanels() {
        purgeDetachedSettingsBodies();
        for (const body of settingsWindowBodies) {
          const panel = body.querySelector("[data-settings-panel='supported-agents']");
          if (panel) renderSupportedAgentsPanel(panel, supportedAgentsModel.read().agents, supportedAgentPanelOptions());
        }
      }

      function applySupportedAgentList(event) {
        supportedAgentsModel.update(state => {
          const maintenanceByAgent = { ...state.maintenanceByAgent };
          delete maintenanceByAgent[""];
          if (event.maintenance_pending) {
            maintenanceByAgent[""] = { pending: true, message: "Agent maintenance is in progress." };
          } else {
            for (const [id, result] of Object.entries(maintenanceByAgent)) {
              if (result.pending) delete maintenanceByAgent[id];
            }
          }
          return {
            agents: event.agents,
            autoUpdate: event.auto_update === true,
            maintenanceByAgent,
          };
        });
      }

      function applySupportedAgentMaintenance(event) {
        supportedAgentsModel.update(state => ({
          ...state,
          maintenanceByAgent: { ...state.maintenanceByAgent, [event.agent_id]: event },
        }));
      }

      supportedAgentsModel.subscribe(state => state, () => {
        renderSupportedAgentPanels();
        for (const body of settingsWindowBodies) renderProjectSettings(body);
      });

      customAgentsModel.subscribe(state => state, state => {
        renderSettingsAgentList(state);
      });
      customAgentsModel.subscribe(state => state.catalog, catalog => {
        // Read recovery must not replace candidate inputs while awaiting a reply.
        if (catalog.loading) return;
        purgeDetachedSettingsBodies();
        for (const body of settingsWindowBodies) renderProjectSettings(body);
      });

      function requestSupportedAgentRefresh() {
        purgeDetachedSettingsBodies();
        if (settingsWindowBodies.size) send({ kind: "list_supported_agents" });
      }

      function requestSettingsSnapshots(recovering = false) {
        if (activeProjectTab()?.project_root) {
          send({ kind: "list_issue_monitor" });
          send({ kind: "get_issue_monitor_profiles" });
        }
        send({ kind: "list_supported_agents" });
        send({ kind: "get_system_settings" });
        send({ kind: "get_update_settings" });
        send({ kind: "get_board_auth_status" });
        const autostart = systemSettingsModel.read();
        const shouldRequestAutostartStatus = recovering || (!autostart.autostartPending
          && !autostart.autostartChecking);
        if (shouldRequestAutostartStatus) {
          systemSettingsModel.update(state => ({ ...state, autostartChecking: true,
            autostartStatusMessage: "", autostartStatusKind: "",
          }));
          send({ kind: "get_autostart_status" });
        }
        const customAgents = customAgentsModel.read().catalog;
        if (recovering || (!customAgents.loading && customAgents.agents.length === 0)) {
          // Keep a known catalog and any unsaved environment editor while reads recover.
          customAgentsModel.update(state => state.catalog.loading || state.catalog.agents.length > 0 ? state : ({ ...state,
            catalog: { ...state.catalog, loading: true },
          }));
          send({ kind: "list_custom_agents" });
        }
        for (const agent of ["claudeCode", "codex"]) {
          send({ kind: "list_agent_backends", agent });
        }
      }

      function resyncSettings() {
        purgeDetachedSettingsBodies();
        if (settingsWindowBodies.size) {
          requestSettingsSnapshots(true);
          renderSystemPanelInAllSettingsWindows();
        }
      }
      let editingCustomAgentId = null;

      function createDiv(className) {
        const el = document.createElement("div");
        if (className) el.className = className;
        return el;
      }

      function purgeDetachedSettingsBodies() {
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) settingsWindowBodies.delete(body);
        }
      }

      // SPEC-1933 Phase: System Settings (Output Language).
      // Build a tabbed Settings surface (System | Custom Agents) using
      // Operator Design tokens. Existing renderSettingsAgentList continues
      // to populate the Custom Agents panel via [data-role='settings-scroll'].
      function renderSettingsWindow(body, windowData) {
        // Sweep detached bodies up-front so repeated open/close cycles do
        // not accumulate references.
        purgeDetachedSettingsBodies();
        while (body.firstChild) body.removeChild(body.firstChild);
        const project = activeProjectTab();
        body.dataset.settingsSection = "issues";
        body.dataset.settingsWindowId = windowData.id;
        body.dataset.surfaceOwner = windowData.surface_owner || "";
        body.dataset.settingsProjectRoot = project?.project_root || "";
        body.dataset.settingsProjectName = project?.name || project?.title || project?.project_root?.split(/[\\/]/).pop() || "No project";

        const root = createDiv("settings-root");

        const toolbar = document.createElement("header");
        toolbar.className = "settings-toolbar";
        const heading = document.createElement("h2");
        heading.className = "settings-heading";
        heading.textContent = windowData.title || "Settings";

        const tabs = document.createElement("nav");
        tabs.className = "settings-tabs";
        tabs.setAttribute("role", "tablist");
        tabs.setAttribute("aria-orientation", "vertical");
        tabs.setAttribute("aria-label", "Settings sections");
        const appScope = createNode("span", "settings-label", "Application settings");
        tabs.appendChild(appScope);
        tabs.appendChild(buildSettingsTab("system", "System", false, windowData.id));
        tabs.appendChild(
          buildSettingsTab("supported-agents", "Supported Agents", false, windowData.id),
        );
        tabs.appendChild(
          buildSettingsTab(
            "project-manager",
            "Project Manager",
            false,
            windowData.id,
          ),
        );
        tabs.appendChild(
          buildSettingsTab("custom-agents", "Custom Agents", false, windowData.id),
        );
        // SPEC-1921 2026-05-18 amendment / FR-099: Agent Backends tab is the
        // dedicated surface for Claude Code / Codex Backend Override profiles.
        // Kept distinct from `custom-agents` so External CLI rows and
        // built-in LLM redirection have separate physical UI.
        tabs.appendChild(
          buildSettingsTab("agent-backends", "Agent Backends", false, windowData.id),
        );
        // SPEC-2970: provider usage display preferences (Claude opt-in).
        tabs.appendChild(
          buildSettingsTab("usage", "Usage & Limits", false, windowData.id),
        );
        tabs.appendChild(createNode("span", "settings-label", `Project settings · ${body.dataset.settingsProjectName}`));
        tabs.appendChild(buildSettingsTab("issues", "Issues", true, windowData.id));
        // PM settings are also project-owned; keep all project tabs together.
        tabs.appendChild(tabs.querySelector('[data-settings-tab="project-manager"]'));

        toolbar.appendChild(heading);

        const bodyEl = createDiv("settings-body");

        const panelSystem = document.createElement("section");
        panelSystem.className = "settings-panel hidden";
        panelSystem.setAttribute("role", "tabpanel");
        panelSystem.dataset.settingsPanel = "system";
        linkSettingsPanel(panelSystem, windowData.id, "system");
        panelSystem.appendChild(createNode("h3", "settings-section-heading", "Application settings"));
        const updateSection = createDiv("settings-section");
        const updateLabel = createNode("label", "settings-checkbox-label");
        const updateToggle = document.createElement("input");
        updateToggle.type = "checkbox";
        updateToggle.className = "settings-checkbox";
        updateToggle.setAttribute("aria-label", "Auto-apply updates");
        updateToggle.checked = globalUpdateModel.read().auto_apply_updates_effective;
        updateToggle.disabled = !globalUpdateModel.read().loaded;
        updateToggle.addEventListener("change", () => send({ kind: "set_auto_apply_updates", enabled: updateToggle.checked }));
        updateLabel.appendChild(updateToggle);
        updateLabel.appendChild(document.createTextNode("Auto-apply updates"));
        updateSection.appendChild(updateLabel);
        updateSection.appendChild(createNode("p", "settings-help", "Applies across all projects when at least one open project has Autonomous enabled. Updates wait until agents finish."));
        const updateStatus = createDiv();
        updateStatus.dataset.updateSettingsStatus = "";
        updateStatus.setAttribute("role", "status");
        updateSection.appendChild(updateStatus);
        panelSystem.appendChild(updateSection);
        const systemContent = createDiv();
        systemContent.id = `${panelSystem.id}-content`;
        systemContent.dataset.systemSettingsContent = "";
        panelSystem.appendChild(systemContent);

        const panelSupportedAgents = document.createElement("section");
        panelSupportedAgents.className = "settings-panel hidden";
        panelSupportedAgents.setAttribute("role", "tabpanel");
        panelSupportedAgents.dataset.settingsPanel = "supported-agents";
        linkSettingsPanel(panelSupportedAgents, windowData.id, "supported-agents");

        const panelAgents = document.createElement("section");
        panelAgents.className = "settings-panel hidden";
        panelAgents.setAttribute("role", "tabpanel");
        panelAgents.dataset.settingsPanel = "custom-agents";
        linkSettingsPanel(panelAgents, windowData.id, "custom-agents");
        // Existing renderSettingsAgentList queries this attribute to inject
        // the Add button and agent rows.
        panelAgents.dataset.role = "settings-scroll";

        const panelBackends = document.createElement("section");
        panelBackends.className = "settings-panel hidden";
        panelBackends.setAttribute("role", "tabpanel");
        panelBackends.dataset.settingsPanel = "agent-backends";
        linkSettingsPanel(panelBackends, windowData.id, "agent-backends");
        panelBackends.dataset.role = "settings-scroll";

        const panelUsage = document.createElement("section");
        panelUsage.className = "settings-panel hidden";
        panelUsage.setAttribute("role", "tabpanel");
        panelUsage.dataset.settingsPanel = "usage";
        linkSettingsPanel(panelUsage, windowData.id, "usage");
        panelUsage.dataset.role = "settings-scroll";

        bodyEl.appendChild(panelSystem);
        bodyEl.appendChild(panelSupportedAgents);
        mountProjectManagerSettingsPanel(
          document,
          bodyEl,
          pmSettingsPanel,
          windowData.id,
        );
        bodyEl.appendChild(panelAgents);
        bodyEl.appendChild(panelBackends);
        bodyEl.appendChild(panelUsage);
        const panelIssues = document.createElement("section");
        panelIssues.className = "settings-panel settings-issues-panel";
        panelIssues.setAttribute("role", "tabpanel");
        panelIssues.dataset.settingsPanel = "issues";
        linkSettingsPanel(panelIssues, windowData.id, "issues");
        bodyEl.appendChild(panelIssues);
        const issuesHost = createDiv("settings-issues-host surface-knowledge");
        panelIssues.appendChild(issuesHost);

        const layout = createDiv("settings-layout");
        layout.appendChild(tabs);
        layout.appendChild(bodyEl);
        root.appendChild(toolbar);
        root.appendChild(layout);
        body.appendChild(root);
        mountIssues(windowData, issuesHost);
        const issuesRoot = issuesHost.querySelector(".issue-bridge-root");
        const issuesOptions = document.createElement("details");
        issuesOptions.className = "settings-issues-options";
        const summary = document.createElement("summary");
        summary.textContent = "Settings · Capacity, refill, labels and candidates";
        issuesOptions.appendChild(summary);
        const controls = createDiv();
        controls.dataset.settingsIssuesControls = "";
        issuesOptions.appendChild(controls);
        const candidates = createDiv();
        candidates.dataset.settingsCandidates = "";
        issuesOptions.appendChild(candidates);
        issuesRoot.querySelector(".knowledge-monitor-bar").after(issuesOptions);
        const gear = issuesRoot.querySelector('[data-action="monitor-settings"]');
        gear.setAttribute("aria-expanded", "false");
        issuesOptions.addEventListener("toggle", () => {
          gear.setAttribute("aria-expanded", String(issuesOptions.open));
          positionIssuesLaunchControls(body);
        });
        gear.addEventListener("click", () => { issuesOptions.open = !issuesOptions.open; });

        tabs.addEventListener("click", (e) => {
          const btn = e.target.closest("[data-settings-tab]");
          if (!btn) return;
          switchSettingsTab(body, btn.dataset.settingsTab);
        });
        tabs.addEventListener("keydown", (e) => {
          if (e.key !== "Enter" && e.key !== " ") return;
          const btn = e.target.closest("[data-settings-tab]");
          if (!btn) return;
          e.preventDefault();
          switchSettingsTab(body, btn.dataset.settingsTab);
        });

        body.addEventListener("mousedown", () => {
          focusWindowLocally(windowData.id);
          send({ kind: "focus_window", id: windowData.id });
        });
        settingsWindowBodies.add(body);
        body.addEventListener("focusout", () => queueMicrotask(() => {
          if (body.dataset.issuesRenderDeferred === "true") {
            delete body.dataset.issuesRenderDeferred;
            renderProjectSettings(body);
          }
        }));
        renderProjectSettings(body);
        requestSettingsSnapshots();
        renderSystemPanel(systemContent);
        renderSupportedAgentsPanel(panelSupportedAgents, supportedAgentsModel.read().agents, supportedAgentPanelOptions());
        renderUsagePanel(panelUsage);
        renderSettingsAgentList();
        // SPEC-1921 2026-05-18 amendment / FR-099: hydrate the Agent
        // Backends panel for both built-in agents that support Backend
        // Override (Claude Code, Codex). The backend returns the redacted
        // list (api_key replaced with `***REDACTED***`).
        renderAgentBackendsPanel(panelBackends);

        // Honour any pending settings:open dispatch (e.g. from the badge
        // click) by switching to the requested tab once the panel is mounted.
        const target = pendingSettingsTabTargets.get(windowData.surface_owner || null);
        switchSettingsTab(body, target || "issues", Boolean(target));
        pendingSettingsTabTargets.delete(windowData.surface_owner || null);
      }

      const pendingSettingsTabTargets = new Map();

      function renderIndexPanel(panel) {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        const status =
          getIndexStatus(activeProjectRoot);
        renderIndexSettingsPanel({
          panel,
          status,
          projectRoot: activeProjectRoot,
          send,
        });
      }

      // SPEC-1921 2026-05-18 amendment / FR-099: render the `Agent Backends`
      // Settings tab body. Two `[data-agent]` sections (`claudeCode` /
      // `codex`) host the per-built-in backend lists; each saved backend
      // renders as a row with the redacted profile shape. The Add /
      // Edit / Delete affordances will land alongside the protocol
      // dispatch when the inline forms move out of the legacy
      // `Custom Agents` tab (T308 follow-up). Today the panel exposes a
      // read-only mirror that confirms FR-101 silent migration produced
      // the expected `[builtinAgents.claudeCode.backends.*]` rows.
      function renderAgentBackendsPanel(panel, state = agentBackendsModel.read()) {
        while (panel.firstChild) panel.removeChild(panel.firstChild);

        for (const agent of ["claudeCode", "codex"]) {
          const section = createDiv("settings-section");
          section.dataset.agent = agent;

          const heading = document.createElement("h3");
          heading.className = "settings-section-heading";
          heading.textContent =
            agent === "claudeCode" ? "Claude Code" : "Codex";
          section.appendChild(heading);

          const list = createDiv("agent-backends-list");
          list.dataset.role = "agent-backends-list";
          list.dataset.agent = agent;
          renderAgentBackendsList(list, agent, state);
          section.appendChild(list);

          panel.appendChild(section);
        }
      }

      function renderAgentBackendsList(container, agent, state = agentBackendsModel.read()) {
        while (container.firstChild) container.removeChild(container.firstChild);
        const profiles = state.backends[agent] || [];
        if (profiles.length === 0) {
          const empty = document.createElement("p");
          empty.className = "settings-help";
          empty.textContent =
            agent === "claudeCode"
              ? "No Claude Code backend profiles saved. Default Anthropic upstream is used."
              : "No Codex backend profiles saved. Default OpenAI upstream is used.";
          container.appendChild(empty);
          return;
        }
        for (const profile of profiles) {
          const row = createDiv("agent-backend-row");
          row.dataset.backendId = profile.id;
          const title = document.createElement("strong");
          title.textContent =
            profile.display_name || profile.displayName || profile.id;
          row.appendChild(title);
          const detail = document.createElement("span");
          detail.className = "settings-help";
          const baseUrl = profile.base_url || profile.baseUrl || "";
          const model = profile.model || "";
          detail.textContent = ` · ${baseUrl} · ${model}`;
          row.appendChild(detail);
          container.appendChild(row);
        }
      }

      function renderAgentBackendsPanelInAllSettingsWindows(state = agentBackendsModel.read()) {
        for (const settingsBody of Array.from(settingsWindowBodies)) {
          if (!settingsBody.isConnected) {
            settingsWindowBodies.delete(settingsBody);
            continue;
          }
          const panel = settingsBody.querySelector(
            "[data-settings-panel='agent-backends']",
          );
          if (panel) renderAgentBackendsPanel(panel, state);
        }
      }

      function renderIndexPanelInAllSettingsWindows() {
        for (const settingsBody of Array.from(settingsWindowBodies)) {
          if (!settingsBody.isConnected) {
            settingsWindowBodies.delete(settingsBody);
            continue;
          }
          const panel = settingsBody.querySelector(
            "[data-settings-panel='index']",
          );
          if (panel) renderIndexPanel(panel);
        }
      }

      function requestFullIndexStatusRefresh() {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        if (!activeProjectRoot) return;
        send({ kind: "refresh_index_status", project_root: activeProjectRoot });
      }

      installSettingsOpenHandler({
        document,
        send,
        settingsWindowBodies,
        focusOrSpawnPreset,
        activeProjectTab,
        setPendingTarget: (target, surfaceOwner) => {
          pendingSettingsTabTargets.set(surfaceOwner, target);
        },
      });

      function buildSettingsTab(id, label, selected, windowId) {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = selected ? "settings-tab active" : "settings-tab";
        btn.setAttribute("role", "tab");
        btn.setAttribute("aria-selected", String(selected));
        btn.id = settingsTabId(windowId, id);
        btn.setAttribute("aria-controls", settingsPanelId(windowId, id));
        btn.dataset.settingsTab = id;
        btn.textContent = label;
        return btn;
      }

      function composeTeamsDefaultChannel(teamId, channelId) {
        const team = String(teamId || "").trim();
        const channel = String(channelId || "").trim();
        if (!team && !channel) return "";
        if (!team) return channel;
        if (!channel) return team;
        return `${team}/${channel}`;
      }

      function parseTeamsDefaultChannel(value) {
        const raw = String(value || "").trim();
        if (!raw) return { teamId: "", channelId: "" };
        const slash = raw.indexOf("/");
        if (slash === -1) return { teamId: "", channelId: raw };
        return {
          teamId: raw.slice(0, slash).trim(),
          channelId: raw.slice(slash + 1).trim(),
        };
      }

      function formatTeamsChannelLink(defaultChannel, tenantId) {
        const { teamId, channelId } = parseTeamsDefaultChannel(defaultChannel);
        if (!teamId || !channelId) return "";
        const tenant = String(tenantId || "").trim();
        const params = new URLSearchParams({ groupId: teamId });
        if (tenant) params.set("tenantId", tenant);
        return `https://teams.microsoft.com/l/channel/${encodeURIComponent(channelId)}/configured-channel?${params.toString()}`;
      }

      function parseTeamsChannelLink(value) {
        const raw = String(value || "").trim();
        if (!raw) return null;
        let url;
        try {
          url = new URL(raw);
        } catch (_) {
          return null;
        }
        const teamId = (url.searchParams.get("groupId") || "").trim();
        const segments = url.pathname.split("/").filter(Boolean);
        const channelIndex = segments.findIndex(
          (segment) => segment.toLowerCase() === "channel",
        );
        const encodedChannel =
          channelIndex >= 0 ? segments[channelIndex + 1] || "" : "";
        let channelId = encodedChannel.trim();
        if (channelId) {
          try {
            channelId = decodeURIComponent(channelId);
          } catch (_) {
            // Keep the raw segment; it is still a better hint than clearing it.
          }
        }
        if (!teamId && !channelId) return null;
        return { teamId, channelId };
      }

      function syncAutostartCheckbox(autostartCheckbox, state = systemSettingsModel.read()) {
        autostartCheckbox.checked = state.autostartEnabled === true;
        autostartCheckbox.disabled = !state.autostartLoaded ||
          state.autostartChecking === true || state.autostartPending === true;
      }

      function renderAutostartStatus(autostartStatus, state = systemSettingsModel.read()) {
        if (state.autostartPending) {
          autostartStatus.textContent = "Saving…";
          autostartStatus.dataset.kind = "info";
        } else if (state.autostartChecking) {
          autostartStatus.textContent = "Checking login launch status…";
          autostartStatus.dataset.kind = "info";
        } else {
          const message = state.autostartStatusMessage || "";
          autostartStatus.textContent = !state.autostartLoaded && message
            ? `Login launch status unavailable. ${message}`
            : message;
          if (state.autostartStatusKind) {
            autostartStatus.dataset.kind = state.autostartStatusKind;
          } else {
            delete autostartStatus.dataset.kind;
          }
        }
      }

      function renderSystemPanel(panel, state = systemSettingsModel.read()) {
        // Reads used for recovery must not destroy explicit-save drafts,
        // including secrets that only exist in these input elements.
        if (panel.querySelector(".board-config-form[data-dirty='true'], .board-oauth-port-form[data-dirty='true']")) {
          const autostart = panel.querySelector("input[id$='-settings-system-autostart']");
          if (autostart) {
            syncAutostartCheckbox(autostart, state);
          }
          const autostartStatus = panel.querySelector("[data-role='autostart-settings-status']");
          if (autostartStatus) renderAutostartStatus(autostartStatus, state);
          renderSystemPanelStatus(panel, state);
          return;
        }
        const previous = renderedSystemPanels.get(panel);
        if (previous && ["boardProvider", "boardAuth", "boardAuthMessage", "boardConfig",
          "autostartLoaded", "autostartMechanism", "autostartInstallPath"]
          .every(key => Object.is(previous[key], state[key]))) {
          // Scalar changes must not detach the next pointer target when a
          // numeric input's focusout flushes the interaction guard.
          const resource = state.agentResource;
          const control = suffix => panel.querySelector(`[id$='-settings-${suffix}']`);
          for (const [suffix, value] of [
            ["system-language", state.language || "auto"],
            ["system-agent-preset", resource.preset || "automatic"],
            ["system-agent-priority", resource.priority || "below-normal"],
            ["system-agent-cpu-limit", resource.cpuLimitPercent ?? ""],
            ["system-agent-build-jobs", resource.buildJobs ?? ""],
          ]) control(suffix).value = String(value);
          control("system-codex-hooks").checked = state.codexTrustManagedHooks !== false;
          control("system-agent-resource-enabled").checked = resource.enabled !== false;
          for (const suffix of ["system-agent-preset", "system-agent-priority",
            "system-agent-cpu-limit", "system-agent-build-jobs"]) {
            control(suffix).disabled = resource.enabled === false;
          }
          panel.querySelector("[data-role='agent-resource-custom']").hidden = resource.preset !== "custom";
          syncAutostartCheckbox(control("system-autostart"), state);
          renderAutostartStatus(panel.querySelector("[data-role='autostart-settings-status']"), state);
          renderSystemPanelStatus(panel, state);
          renderedSystemPanels.set(panel, state);
          return;
        }
        while (panel.firstChild) panel.removeChild(panel.firstChild);

        const section = createDiv("settings-section");

        const label = document.createElement("label");
        label.className = "settings-label";
        label.setAttribute("for", "settings-system-language");
        label.textContent = "Output Language";
        section.appendChild(label);

        const select = document.createElement("select");
        select.className = "settings-select";
        select.id = "settings-system-language";
        for (const opt of [
          { value: "auto", text: "Auto (OS locale)" },
          { value: "en", text: "English" },
          { value: "ja", text: "日本語" },
        ]) {
          const option = document.createElement("option");
          option.value = opt.value;
          option.textContent = opt.text;
          select.appendChild(option);
        }
        select.value = state.language || "auto";
        select.addEventListener("change", (e) => {
          const next = e.target.value;
          systemSettingsModel.update(state => ({ ...state, language: next,
            statusMessage: "Saving…", statusKind: "info" }));
          send({ kind: "update_system_settings", language: next });
        });
        section.appendChild(select);

        const help = document.createElement("p");
        help.className = "settings-help";
        help.textContent =
          "Used for narrative outputs (Work summaries and Board post bodies). " +
          "Settings UI text and gwtd subcommands stay English.";
        section.appendChild(help);

        const trustSection = createDiv("settings-section");
        const trustLabel = document.createElement("label");
        trustLabel.className = "settings-checkbox-label";
        trustLabel.setAttribute("for", "settings-system-codex-hooks");

        const trustCheckbox = document.createElement("input");
        trustCheckbox.type = "checkbox";
        trustCheckbox.className = "settings-checkbox";
        trustCheckbox.id = "settings-system-codex-hooks";
        trustCheckbox.checked = state.codexTrustManagedHooks !== false;
        trustCheckbox.addEventListener("change", (e) => {
          const next = e.target.checked === true;
          systemSettingsModel.update(state => ({ ...state, codexTrustManagedHooks: next,
            statusMessage: "Saving…", statusKind: "info" }));
          send({
            kind: "update_system_settings",
            language: systemSettingsModel.read().language || "auto",
            codex_trust_managed_hooks: next,
          });
        });

        const trustText = document.createElement("span");
        trustText.textContent = "Trust gwt-managed Codex hooks";
        trustLabel.appendChild(trustCheckbox);
        trustLabel.appendChild(trustText);
        trustSection.appendChild(trustLabel);

        const trustHelp = document.createElement("p");
        trustHelp.className = "settings-help";
        trustHelp.textContent =
          "Enabled by default. Registers only generated gwt hook commands in Codex hook trust state.";
        trustSection.appendChild(trustHelp);

        // SPEC #1921 Phase 86 (#3813): agent process-tree resource isolation.
        // Every control sends the complete policy object so the backend can
        // validate it as a unit and echo the authoritative result.
        const resourceSection = createDiv("settings-section");
        const agentResource = state.agentResource;
        const resourceEnabled = agentResource.enabled !== false;
        const sendAgentResourceUpdate = patch => {
          systemSettingsModel.update(state => ({ ...state,
            agentResource: { ...state.agentResource, ...patch },
            statusMessage: "Saving…", statusKind: "info" }));
          send({
            kind: "update_system_settings",
            language: systemSettingsModel.read().language || "auto",
            agent_resource: agentResourceWirePayload(),
          });
        };
        const rejectAgentResourceInput = (message, input, previous) => {
          input.value = Number.isInteger(previous) ? String(previous) : "";
          systemSettingsModel.update(state => ({ ...state, statusMessage: message, statusKind: "error" }));
        };

        const resourceLabel = document.createElement("label");
        resourceLabel.className = "settings-checkbox-label";
        resourceLabel.setAttribute("for", "settings-system-agent-resource-enabled");
        const resourceCheckbox = document.createElement("input");
        resourceCheckbox.type = "checkbox";
        resourceCheckbox.className = "settings-checkbox";
        resourceCheckbox.id = "settings-system-agent-resource-enabled";
        resourceCheckbox.checked = resourceEnabled;
        resourceCheckbox.addEventListener("change", (e) => {
          sendAgentResourceUpdate({ enabled: e.target.checked === true });
        });
        const resourceText = document.createElement("span");
        resourceText.textContent = "Isolate agent process trees";
        resourceLabel.appendChild(resourceCheckbox);
        resourceLabel.appendChild(resourceText);
        resourceSection.appendChild(resourceLabel);

        const resourceHelp = document.createElement("p");
        resourceHelp.className = "settings-help";
        resourceHelp.textContent =
          "Enabled by default. Agent launches (and everything they spawn: cargo, bun, tsc, " +
          "docker build, ...) run below gwt's scheduling priority; on Windows the whole tree " +
          "also gets a CPU cap.";
        resourceSection.appendChild(resourceHelp);

        // User feedback 2026-09-02: thresholds are hard to pick by hand, so the
        // preset carries the intent and Custom is the only place with numbers.
        const presetLabel = document.createElement("label");
        presetLabel.className = "settings-label";
        presetLabel.setAttribute("for", "settings-system-agent-preset");
        presetLabel.textContent = "Resource preset";
        resourceSection.appendChild(presetLabel);
        const presetSelect = document.createElement("select");
        presetSelect.className = "settings-select";
        presetSelect.id = "settings-system-agent-preset";
        for (const opt of [
          { value: "automatic", text: "Automatic (recommended)" },
          { value: "gui-responsiveness", text: "Prioritize GUI responsiveness" },
          { value: "build-speed", text: "Prioritize build speed" },
          { value: "custom", text: "Custom" },
        ]) {
          const option = document.createElement("option");
          option.value = opt.value;
          option.textContent = opt.text;
          presetSelect.appendChild(option);
        }
        presetSelect.value = agentResource.preset || "automatic";
        presetSelect.disabled = !resourceEnabled;
        presetSelect.addEventListener("change", (e) => {
          sendAgentResourceUpdate({ preset: e.target.value });
        });
        resourceSection.appendChild(presetSelect);

        const presetHelp = document.createElement("p");
        presetHelp.className = "settings-help";
        presetHelp.textContent =
          "Automatic: below-normal priority, CPU and build parallelism shared by the max " +
          "active agents. GUI responsiveness: idle priority and half of that budget. " +
          "Build speed: below-normal priority with no CPU cap and full parallelism.";
        resourceSection.appendChild(presetHelp);

        const customSection = createDiv("settings-section");
        customSection.dataset.role = "agent-resource-custom";
        customSection.hidden = agentResource.preset !== "custom";

        const priorityLabel = document.createElement("label");
        priorityLabel.className = "settings-label";
        priorityLabel.setAttribute("for", "settings-system-agent-priority");
        priorityLabel.textContent = "Agent process priority";
        customSection.appendChild(priorityLabel);
        const prioritySelect = document.createElement("select");
        prioritySelect.className = "settings-select";
        prioritySelect.id = "settings-system-agent-priority";
        for (const opt of [
          { value: "normal", text: "Normal (same as gwt)" },
          { value: "below-normal", text: "Below normal" },
          { value: "idle", text: "Idle" },
        ]) {
          const option = document.createElement("option");
          option.value = opt.value;
          option.textContent = opt.text;
          prioritySelect.appendChild(option);
        }
        prioritySelect.value = agentResource.priority || "below-normal";
        prioritySelect.disabled = !resourceEnabled;
        prioritySelect.addEventListener("change", (e) => {
          sendAgentResourceUpdate({ priority: e.target.value });
        });
        customSection.appendChild(prioritySelect);

        const cpuLabel = document.createElement("label");
        cpuLabel.className = "settings-label";
        cpuLabel.setAttribute("for", "settings-system-agent-cpu-limit");
        cpuLabel.textContent = "CPU limit per agent (%, Windows)";
        customSection.appendChild(cpuLabel);
        const cpuInput = document.createElement("input");
        cpuInput.type = "number";
        cpuInput.className = "settings-input";
        cpuInput.id = "settings-system-agent-cpu-limit";
        cpuInput.min = "1";
        cpuInput.max = "100";
        cpuInput.step = "1";
        cpuInput.placeholder = "Automatic";
        cpuInput.value = Number.isInteger(agentResource.cpuLimitPercent)
          ? String(agentResource.cpuLimitPercent)
          : "";
        cpuInput.disabled = !resourceEnabled;
        cpuInput.addEventListener("change", (e) => {
          const next = parseOptionalPositiveInteger(e.target.value);
          if (next === undefined || (next !== null && next > 100)) {
            rejectAgentResourceInput(
              "CPU limit must be a whole number between 1 and 100, or empty for automatic.",
              e.target,
              systemSettingsModel.read().agentResource.cpuLimitPercent,
            );
            return;
          }
          sendAgentResourceUpdate({ cpuLimitPercent: next });
        });
        customSection.appendChild(cpuInput);

        const jobsLabel = document.createElement("label");
        jobsLabel.className = "settings-label";
        jobsLabel.setAttribute("for", "settings-system-agent-build-jobs");
        jobsLabel.textContent = "Build parallelism per agent (jobs)";
        customSection.appendChild(jobsLabel);
        const jobsInput = document.createElement("input");
        jobsInput.type = "number";
        jobsInput.className = "settings-input";
        jobsInput.id = "settings-system-agent-build-jobs";
        jobsInput.min = "1";
        jobsInput.step = "1";
        jobsInput.placeholder = "Automatic";
        jobsInput.value = Number.isInteger(agentResource.buildJobs)
          ? String(agentResource.buildJobs)
          : "";
        jobsInput.disabled = !resourceEnabled;
        jobsInput.addEventListener("change", (e) => {
          const next = parseOptionalPositiveInteger(e.target.value);
          if (next === undefined) {
            rejectAgentResourceInput(
              "Build parallelism must be a whole number of at least 1, or empty for automatic.",
              e.target,
              systemSettingsModel.read().agentResource.buildJobs,
            );
            return;
          }
          sendAgentResourceUpdate({ buildJobs: next });
        });
        customSection.appendChild(jobsInput);

        const customHelp = document.createElement("p");
        customHelp.className = "settings-help";
        customHelp.textContent =
          "Leave a field empty for automatic values: CPU limit = 100 / max active agents, " +
          "parallelism = logical cores / max active agents. The parallelism is handed to " +
          "build tools through their environment (CARGO_BUILD_JOBS, MAKEFLAGS); an explicit " +
          "value already in the launch environment always wins.";
        customSection.appendChild(customHelp);
        resourceSection.appendChild(customSection);

        // SPEC-2959/2963: Board provider selector. `local` keeps the Board
        // offline; `slack` / `teams` are network-backed and selectable. Picking
        // a remote provider reveals its config form (client id / channel /
        // secret) and a sign-in affordance below.
        const boardSection = createDiv("settings-section");
        const boardLabel = document.createElement("label");
        boardLabel.className = "settings-label";
        boardLabel.setAttribute("for", "settings-system-board-provider");
        boardLabel.textContent = "Board provider";
        boardSection.appendChild(boardLabel);

        const boardSelect = document.createElement("select");
        boardSelect.className = "settings-select";
        boardSelect.id = "settings-system-board-provider";
        for (const opt of [
          { value: "local", text: "Local (offline)" },
          { value: "slack", text: "Slack" },
          { value: "teams", text: "Teams" },
        ]) {
          const option = document.createElement("option");
          option.value = opt.value;
          option.textContent = opt.text;
          boardSelect.appendChild(option);
        }
        boardSelect.value = state.boardProvider || "local";
        boardSelect.addEventListener("change", (e) => {
          const next = e.target.value;
          for (const form of panel.querySelectorAll("[data-dirty]")) delete form.dataset.dirty;
          systemSettingsModel.update(state => ({ ...state, boardProvider: next,
            statusMessage: "Saving…", statusKind: "info" }));
          send({
            kind: "update_system_settings",
            language: systemSettingsModel.read().language || "auto",
            board_provider: next,
          });
        });
        boardSection.appendChild(boardSelect);

        const boardHelp = document.createElement("p");
        boardHelp.className = "settings-help";
        boardHelp.textContent =
          "Where the coordination Board is stored. Local keeps the Board offline and " +
          "on this machine. Slack / Teams require sign-in and are network-backed.";
        boardSection.appendChild(boardHelp);

        // SPEC-2963 FR-011/FR-012: sign-in affordance + auth status for the
        // selected remote provider. Local needs no sign-in.
        const selectedProvider = state.boardProvider || "local";
        if (selectedProvider === "slack" || selectedProvider === "teams") {
          // SPEC-2963 FR-006: provider config form. Non-secret fields persist to
          // config.toml; the client secret is routed to the secure credential
          // store and never echoed back (placeholder shows "configured").
          const cfg = state.boardConfig || {};
          const configForm = createDiv("settings-section board-config-form");
          configForm.dataset.provider = selectedProvider;
          configForm.addEventListener("input", () => { configForm.dataset.dirty = "true"; });

          const makeField = (id, labelText, value, opts = {}) => {
            const wrap = createDiv("settings-field");
            const fieldLabel = document.createElement("label");
            fieldLabel.className = "settings-label";
            fieldLabel.setAttribute("for", id);
            fieldLabel.textContent = labelText;
            wrap.appendChild(fieldLabel);
            const input = document.createElement("input");
            input.className = "settings-input";
            input.id = id;
            input.type = opts.password ? "password" : "text";
            input.value = value || "";
            if (opts.placeholder) input.placeholder = opts.placeholder;
            if (opts.autocomplete) input.autocomplete = opts.autocomplete;
            wrap.appendChild(input);
            configForm.appendChild(wrap);
            return input;
          };

          let clientIdInput;
          let defaultChannelInput;
          let tenantIdInput;
          let teamsChannelLinkInput;
          let secretInput;
          if (selectedProvider === "slack") {
            clientIdInput = makeField(
              "settings-board-slack-client-id",
              "Client ID",
              cfg.slackClientId,
              { placeholder: "e.g. 1234567890.1234567890" },
            );
            defaultChannelInput = makeField(
              "settings-board-slack-channel",
              "Default channel ID",
              cfg.slackDefaultChannel,
              { placeholder: "e.g. C0123456789" },
            );
            secretInput = makeField(
              "settings-board-slack-secret",
              "Client secret",
              "",
              {
                password: true,
                autocomplete: "new-password",
                placeholder: cfg.slackHasSecret
                  ? "configured — leave blank to keep"
                  : "required for Slack sign-in",
              },
            );
            // The secret is stored securely and never echoed back, so the
            // field intentionally clears after Save. Show an explicit saved
            // state so it is obvious the secret persisted (id used by tests).
            const secretState = createNode(
              "p",
              "settings-help board-secret-state",
              cfg.slackHasSecret
                ? "✓ A client secret is saved (the field stays blank for security)."
                : "No client secret saved yet.",
            );
            secretState.dataset.hasSecret = cfg.slackHasSecret ? "true" : "false";
            configForm.appendChild(secretState);
          } else {
            clientIdInput = makeField(
              "settings-board-teams-client-id",
              "Application (client) ID",
              cfg.teamsClientId,
              { placeholder: "Entra app id" },
            );
            tenantIdInput = makeField(
              "settings-board-teams-tenant-id",
              "Tenant ID",
              cfg.teamsTenantId,
              { placeholder: "tenant id / common / organizations" },
            );
            teamsChannelLinkInput = makeField(
              "settings-board-teams-channel-link",
              "Teams channel link",
              formatTeamsChannelLink(
                cfg.teamsDefaultChannel,
                cfg.teamsTenantId,
              ),
              { placeholder: "https://teams.microsoft.com/l/channel/..." },
            );
            const teamsChannelHelp = createNode(
              "p",
              "settings-help",
              cfg.teamsDefaultChannel
                ? "Saved channel link is shown here. Paste a new channel link to change it."
                : "Paste the link from Teams > Get link to channel. gwt extracts the team and channel IDs when saving.",
            );
            configForm.appendChild(teamsChannelHelp);
          }

          const saveBtn = createNode(
            "button",
            "wizard-button",
            "Save configuration",
          );
          saveBtn.type = "button";
          saveBtn.addEventListener("click", () => {
            if (selectedProvider === "teams") {
              const teamsChannelLinkValue = teamsChannelLinkInput
                ? teamsChannelLinkInput.value.trim()
                : "";
              let nextTeamsDefaultChannel = cfg.teamsDefaultChannel || "";
              if (teamsChannelLinkValue) {
                const parsedTeamsChannel = parseTeamsChannelLink(
                  teamsChannelLinkValue,
                );
                if (
                  !parsedTeamsChannel ||
                  !parsedTeamsChannel.teamId ||
                  !parsedTeamsChannel.channelId
                ) {
                  systemSettingsModel.update(state => ({ ...state,
                    statusMessage: "Paste a valid Teams channel link with groupId and /channel/...",
                    statusKind: "error" }));
                  return;
                }
                nextTeamsDefaultChannel = composeTeamsDefaultChannel(
                  parsedTeamsChannel.teamId,
                  parsedTeamsChannel.channelId,
                );
              }
              delete configForm.dataset.dirty;
              send({
                kind: "update_board_provider_config",
                provider: selectedProvider,
                client_id: clientIdInput ? clientIdInput.value.trim() : "",
                default_channel: nextTeamsDefaultChannel,
                tenant_id: tenantIdInput ? tenantIdInput.value.trim() : "",
              });
              return;
            }
            const payload = {
              kind: "update_board_provider_config",
              provider: selectedProvider,
              client_id: clientIdInput ? clientIdInput.value.trim() : "",
              default_channel: defaultChannelInput
                ? defaultChannelInput.value.trim()
                : "",
            };
            if (selectedProvider === "slack" && secretInput) {
              // Only send the secret when the user typed one, so an empty box
              // does not clear an already-configured secret.
              if (secretInput.value.length > 0) {
                payload.client_secret = secretInput.value;
              }
            }
            delete configForm.dataset.dirty;
            send(payload);
          });
          configForm.appendChild(saveBtn);
          boardSection.appendChild(configForm);

          // SPEC-2963 FR-005: fixed OAuth callback port. The redirect_uri must
          // exactly match a URL registered in the provider app; gwt binds this
          // loopback port so sign-in works regardless of the (ephemeral) GUI
          // server port. Editable so a busy 8765 can be changed.
          const oauthPort = Number(cfg.oauthRedirectPort) || 8765;
          const oauthForm = createDiv("settings-section board-oauth-port-form");
          const portField = createDiv("settings-field");
          const portLabel = document.createElement("label");
          portLabel.className = "settings-label";
          portLabel.setAttribute("for", "settings-board-oauth-port");
          portLabel.textContent = "OAuth callback port";
          portField.appendChild(portLabel);
          const portInput = document.createElement("input");
          portInput.className = "settings-input";
          portInput.id = "settings-board-oauth-port";
          portInput.type = "number";
          portInput.min = "1";
          portInput.max = "65535";
          portInput.value = String(oauthPort);
          portField.appendChild(portInput);
          oauthForm.appendChild(portField);

          const redirectHint = createNode(
            "p",
            "settings-help board-oauth-redirect-hint",
            "",
          );
          const renderRedirectHint = () => {
            const p = Number(portInput.value) || oauthPort;
            redirectHint.textContent =
              "Register this exact Redirect URL in the Slack/Teams app: " +
              `http://127.0.0.1:${p}/oauth/callback`;
          };
          renderRedirectHint();
          portInput.addEventListener("input", renderRedirectHint);
          portInput.addEventListener("input", () => { oauthForm.dataset.dirty = "true"; });
          oauthForm.appendChild(redirectHint);

          const savePortBtn = createNode("button", "wizard-button", "Save port");
          savePortBtn.type = "button";
          savePortBtn.addEventListener("click", () => {
            const next = Number(portInput.value);
            delete oauthForm.dataset.dirty;
            send({
              kind: "update_board_oauth_port",
              port: Number.isFinite(next) && next > 0 ? Math.floor(next) : 0,
            });
          });
          oauthForm.appendChild(savePortBtn);
          boardSection.appendChild(oauthForm);

          const auth = state.boardAuth || { slack: false, teams: false };
          const signedIn = auth[selectedProvider] === true;
          const authRow = createDiv("settings-section board-auth-row");
          const statusText = createNode(
            "span",
            "board-auth-status",
            signedIn
              ? `Signed in to ${selectedProvider}`
              : `Not signed in to ${selectedProvider}`,
          );
          statusText.dataset.signedIn = signedIn ? "true" : "false";
          authRow.appendChild(statusText);

          const signInBtn = createNode(
            "button",
            "wizard-button",
            signedIn ? "Re-sign in" : "Sign in",
          );
          signInBtn.type = "button";
          signInBtn.addEventListener("click", () => {
            send({ kind: "board_provider_sign_in", provider: selectedProvider });
          });
          authRow.appendChild(signInBtn);

          if (signedIn) {
            const signOutBtn = createNode("button", "text-button", "Sign out");
            signOutBtn.type = "button";
            signOutBtn.addEventListener("click", () => {
              send({ kind: "board_provider_sign_out", provider: selectedProvider });
            });
            authRow.appendChild(signOutBtn);
          }

          const refreshBtn = createNode("button", "text-button", "Refresh");
          refreshBtn.type = "button";
          refreshBtn.addEventListener("click", () => {
            send({ kind: "get_board_auth_status" });
          });
          authRow.appendChild(refreshBtn);
          boardSection.appendChild(authRow);

          if (state.boardAuthMessage) {
            boardSection.appendChild(
              createNode("p", "settings-help", state.boardAuthMessage),
            );
          }
        }

        const autostartSection = createDiv("settings-section");
        const autostartLabel = document.createElement("label");
        autostartLabel.className = "settings-checkbox-label";
        autostartLabel.setAttribute("for", "settings-system-autostart");

        const autostartCheckbox = document.createElement("input");
        autostartCheckbox.type = "checkbox";
        autostartCheckbox.className = "settings-checkbox";
        autostartCheckbox.id = "settings-system-autostart";
        syncAutostartCheckbox(autostartCheckbox, state);
        autostartCheckbox.addEventListener("change", (e) => {
          const next = e.target.checked === true;
          systemSettingsModel.update(state => ({ ...state,
            autostartPreviousEnabled: state.autostartEnabled === true,
            autostartEnabled: next, autostartPending: true,
            autostartStatusMessage: "Saving…", autostartStatusKind: "info",
          }));
          send({ kind: "update_autostart", enabled: next });
        });

        const autostartText = document.createElement("span");
        autostartText.textContent = "Launch GWT at login";
        autostartLabel.appendChild(autostartCheckbox);
        autostartLabel.appendChild(autostartText);
        autostartSection.appendChild(autostartLabel);

        const autostartHelp = document.createElement("p");
        autostartHelp.className = "settings-help";
        autostartHelp.textContent =
          "Starts GWT in the menu bar when you log in. The browser does not open automatically.";
        autostartSection.appendChild(autostartHelp);

        if (state.autostartLoaded) {
          const autostartDetail = document.createElement("p");
          autostartDetail.className = "settings-help";
          const mechanism = state.autostartMechanism || "Unknown";
          const installPath = state.autostartInstallPath || "";
          autostartDetail.textContent = installPath
            ? `Autostart: ${mechanism} · ${installPath}`
            : `Autostart: ${mechanism}`;
          autostartSection.appendChild(autostartDetail);
        }

        const autostartStatus = document.createElement("p");
        autostartStatus.className = "settings-status";
        autostartStatus.dataset.role = "autostart-settings-status";
        autostartStatus.setAttribute("role", "status");
        autostartStatus.setAttribute("aria-live", "polite");
        renderAutostartStatus(autostartStatus, state);
        autostartSection.appendChild(autostartStatus);

        const status = document.createElement("p");
        status.className = "settings-status";
        status.dataset.role = "system-settings-status";

        panel.appendChild(section);
        panel.appendChild(trustSection);
        panel.appendChild(resourceSection);
        panel.appendChild(boardSection);
        panel.appendChild(autostartSection);
        panel.appendChild(status);
        scopeSettingsControlIds(panel);
        renderSystemPanelStatus(panel, state);
        renderedSystemPanels.set(panel, state);
      }

      function renderSystemPanelStatus(panel, state = systemSettingsModel.read()) {
        const status = panel.querySelector(
          "[data-role='system-settings-status']",
        );
        if (!status) return;
        status.textContent = state.statusMessage || "";
        if (state.statusKind) {
          status.dataset.kind = state.statusKind;
        } else {
          delete status.dataset.kind;
        }
      }

      function renderSystemPanelInAllSettingsWindows(state = systemSettingsModel.read()) {
        // State commits immediately; only destructive DOM swaps wait for a
        // native select/input interaction to finish. Flush reads the latest version.
        const deferred = systemSettingsInteractionGuard.defer(true);
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) {
            settingsWindowBodies.delete(body);
            continue;
          }
          const panel = body.querySelector("[data-system-settings-content]");
          if (panel) {
            if (deferred) renderSystemPanelStatus(panel, state);
            else renderSystemPanel(panel, state);
          }
        }
      }

      function renderSettingsAgentList(state = customAgentsModel.read()) {
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) {
            settingsWindowBodies.delete(body);
            continue;
          }
          const scroll = body.querySelector("[data-role='settings-scroll']");
          if (!scroll) continue;
          while (scroll.firstChild) scroll.removeChild(scroll.firstChild);

          const addBtn = document.createElement("button");
          addBtn.className = "wizard-button";
          addBtn.style.margin = "8px 0";
          // SPEC-1921 Phase 63H / T326: the legacy
          // `+ Add Claude Code (OpenAI-compat backend)` button now points
          // users at the new `Agent Backends` tab. The underlying
          // `add_custom_agent_from_preset` dispatch is preserved for
          // existing callers (Phase 52 contract), but the entry point
          // visible in Custom Agents redirects to the proper surface so
          // External CLI rows and Backend Override profiles never get
          // conflated again.
          addBtn.textContent = "＋ Add Claude Code backend (moved to Agent Backends)";
          addBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            // Switch the Settings window to the Agent Backends tab.
            const body = scroll.closest(".settings-body")?.parentElement;
            if (body) switchSettingsTab(body, "agent-backends");
            setSettingsStatus(
              "Backend Override moved to Agent Backends. Add your Claude Code / Codex backend there.",
              "success",
            );
          });
          scroll.appendChild(addBtn);

          if (state.statusMessage) {
            const section = createDiv("mock-section");
            const label = createDiv("mock-label");
            label.textContent = "Status";
            label.style.color =
              state.statusKind === "error"
                ? "#ff6b6b"
                : state.statusKind === "success"
                  ? "#7abf7a"
                  : "#999";
            section.appendChild(label);
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = state.statusMessage;
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
          }

          if (state.catalog.loading && state.catalog.agents.length === 0) {
            const section = createDiv("mock-section");
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = "Loading custom agents…";
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
            continue;
          }

          if (state.catalog.agents.length === 0) {
            const section = createDiv("mock-section");
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = "No custom agents configured yet.";
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
            continue;
          }

          for (const agent of state.catalog.agents) {
            const section = createDiv("mock-section");
            const label = createDiv("mock-label");
            label.textContent = agent.display_name || agent.id;
            section.appendChild(label);
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            const envCount = Object.keys(agent.env || {}).length;
            const baseUrl =
              agent.env && agent.env.ANTHROPIC_BASE_URL
                ? ` · ${agent.env.ANTHROPIC_BASE_URL}`
                : "";
            text.textContent = `${agent.id} · ${agent.command} · ${envCount} env entries${baseUrl}`;
            row.appendChild(text);
            const delBtn = document.createElement("button");
            delBtn.className = "icon-button";
            delBtn.setAttribute("aria-label", "Delete agent");
            delBtn.textContent = "×";
            delBtn.addEventListener("click", (e) => {
              e.stopPropagation();
              if (window.confirm(`Delete custom agent "${agent.id}"?`)) {
                send({ kind: "delete_custom_agent", agent_id: agent.id });
              }
            });
            const editBtn = document.createElement("button");
            editBtn.className = "icon-button";
            editBtn.setAttribute("aria-label", "Edit agent environment");
            editBtn.title = "Edit environment";
            editBtn.textContent = "✎";
            editBtn.addEventListener("click", (e) => {
              e.stopPropagation();
              editingCustomAgentId =
                editingCustomAgentId === agent.id ? null : agent.id;
              renderSettingsAgentList();
            });
            row.appendChild(editBtn);
            row.appendChild(delBtn);
            section.appendChild(row);
            if (editingCustomAgentId === agent.id) {
              section.appendChild(
                renderCustomAgentEnvEditor({
                  document,
                  agent,
                  onSave: (updatedAgent) => {
                    editingCustomAgentId = null;
                    setSettingsStatus("Saving custom agent…", "info");
                    send({ kind: "update_custom_agent", agent: updatedAgent });
                  },
                  onCancel: () => {
                    editingCustomAgentId = null;
                    renderSettingsAgentList();
                  },
                }),
              );
            }
            scroll.appendChild(section);
          }
        }
      }


      function setSettingsStatus(message, kind) {
        customAgentsModel.update(state => ({
          ...state, statusMessage: message, statusKind: kind,
        }));
      }

      function applyAgentBackendsReceiveEvent(event) {
        const key = event.agent;
        if (event.kind === "agent_backend_list" && key) {
          agentBackendsModel.update(state => ({ ...state,
            backends: { ...state.backends, [key]: event.backends || [] }, loadingAgent: null,
          }));
        } else if (event.kind === "agent_backend_saved" && key && event.profile) {
          agentBackendsModel.update(state => {
            const profiles = [...(state.backends[key] || [])];
            const index = profiles.findIndex(profile => profile.id === event.profile.id);
            if (index >= 0) profiles[index] = event.profile;
            else profiles.push(event.profile);
            return { ...state, backends: { ...state.backends, [key]: profiles } };
          });
          setSettingsStatus(`Saved backend "${event.profile.id}" for ${key}.`, "success");
        } else if (event.kind === "agent_backend_deleted" && key && event.id) {
          agentBackendsModel.update(state => ({ ...state, backends: { ...state.backends,
            [key]: (state.backends[key] || []).filter(profile => profile.id !== event.id),
          } }));
          setSettingsStatus(`Deleted backend "${event.id}" for ${key}.`, "success");
        } else if (event.kind === "agent_backend_error") {
          setSettingsStatus(event.message || "Agent backend error.", "error");
        }
      }

      function applyCustomAgentsReceiveEvent(event) {
        if (event.kind === "custom_agent_deleted" && editingCustomAgentId === event.agent_id) {
          editingCustomAgentId = null;
        }
        customAgentsModel.update(state => {
          switch (event.kind) {
            case "custom_agent_list":
              return { ...state, catalog: { agents: event.agents || [], loading: false } };
            case "custom_agent_saved": {
              const agents = [...state.catalog.agents];
              if (event.agent) {
                const index = agents.findIndex(agent => agent.id === event.agent.id);
                if (index >= 0) agents[index] = event.agent;
                else agents.push(event.agent);
              }
              return { ...state, catalog: { agents, loading: false },
                statusMessage: `Saved custom agent "${event.agent ? event.agent.id : "?"}".`, statusKind: "success" };
            }
            case "custom_agent_deleted":
              return { ...state, catalog: { ...state.catalog,
                  agents: state.catalog.agents.filter(agent => agent.id !== event.agent_id) },
                statusMessage: `Deleted custom agent "${event.agent_id}".`, statusKind: "success" };
            case "custom_agent_error":
              return { ...state, catalog: { ...state.catalog, loading: false },
                statusMessage: `Error [${event.code}]: ${event.message}`, statusKind: "error" };
            case "backend_connection_result":
              return { ...state, statusMessage: `/v1/models returned ${event.models.length} model(s).`, statusKind: "success" };
            default: return state;
          }
        });
      }

      return {
        applyGlobalUpdateSettings,
        applyIssueSettingsStatus,
        applyIssueMonitorProfiles,
        applyIssueMonitorProfilesError,
        customAgentsModel,
        agentBackendsModel,
        applyAgentBackendsReceiveEvent,
        systemSettingsModel,
        systemSettingsInteractionGuard,
        applySystemSettingsEvent,
        applyBoardAuthStatus,
        applyAutostartStatus,
        applyAutostartError,
        applyCustomAgentsReceiveEvent,
        applySupportedAgentList,
        applySupportedAgentMaintenance,
        requestSupportedAgentRefresh,
        resyncSettings,
        renderSettingsWindow,
        renderSettingsAgentList,
        renderAgentBackendsPanel,
        renderAgentBackendsPanelInAllSettingsWindows,
        renderSystemPanelInAllSettingsWindows,
        renderIndexPanelInAllSettingsWindows,
        requestFullIndexStatusRefresh,
        setSettingsStatus,
      };
}
