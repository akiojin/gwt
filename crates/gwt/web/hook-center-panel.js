// Project-scoped presentation of hook.doctor observations. Viewing never repairs.
const healthLabels = {
  ready: "Ready", needs_attention: "Needs attention", self_healed: "Self healed",
  degraded: "Degraded", inactive: "Inactive", waiting_for_first_hook_event: "Waiting for first hook event",
};
const repairLabels = {
  materialize_managed_git_hooks: "Materialize managed Git hooks",
  regenerate_managed_provider_configs: "Regenerate managed provider configs",
  verify_codex_hook_trust: "Verify Codex hook trust",
  start_new_agent_session: "Start a new agent session",
};

export function hookRepairUnavailableReason(state = {}) {
  if (state.pending || state.error || !state.report) return "Read a successful hook audit before regenerating managed hooks.";
  if (state.report.configuration_audit?.configs?.some(config => config.state === "invalid_json" || config.state === "unreadable")) {
    return "Fix the listed invalid or unreadable configuration, then refresh before regenerating.";
  }
  return null;
}

export function renderHookCenterPanel(panel, { projectRoot, state = {}, available = Boolean(projectRoot), onRefresh, onRepair } = {}) {
  const document = panel.ownerDocument;
  panel.replaceChildren();
  panel.dataset.hookAuditStatus = !available ? "unavailable" : state.pending
    ? state.repairRequested ? "repairing" : "loading" : state.error ? "error" : state.report ? "loaded" : "idle";
  const node = (parent, tag, className, text) => {
    const element = document.createElement(tag);
    element.className = className;
    element.textContent = String(text);
    parent.appendChild(element);
    return element;
  };
  const section = title => {
    const element = document.createElement("section");
    element.className = "settings-section";
    panel.appendChild(element);
    node(element, "h3", "settings-section-heading", title);
    return element;
  };
  const field = (parent, label, value, fallback = "Unknown") => node(parent, "p", "settings-help", `${label}: ${value ?? fallback}`);
  const list = (parent, title, values, empty = "None reported", label = value => value) => {
    node(parent, "h4", "settings-label", title);
    const items = Array.isArray(values) ? values : [];
    if (!items.length) { node(parent, "p", "settings-help", empty); return; }
    const ul = document.createElement("ul");
    ul.className = "settings-help";
    parent.appendChild(ul);
    for (const item of items) node(ul, "li", "", label(item));
  };
  const overview = section("Hook Center");
  field(overview, "Project", projectRoot, "No project");
  node(overview, "p", "settings-help", "Audit managed hook configuration without changing it. Regeneration changes managed entries only; user hooks and unrelated settings are preserved.");
  const actions = node(overview, "div", "settings-section", "");
  const repairUnavailableReason = hookRepairUnavailableReason(state);
  const button = (label, action, handler, unavailable = false) => {
    const element = node(actions, "button", "wizard-button", label);
    element.type = "button";
    element.dataset.action = action;
    element.disabled = !available || state.pending === true || unavailable;
    element.addEventListener("click", () => { if (!element.disabled) handler?.(); });
  };
  button(state.error ? "Retry" : "Refresh audit", "refresh-hook-audit", onRefresh);
  button("Regenerate managed hooks", "repair-managed-hooks", onRepair, Boolean(repairUnavailableReason));
  if (!available) {
    node(overview, "p", "settings-help", projectRoot ? "Switch to this project to audit managed hooks." : "Open a project to audit managed hooks.");
    return;
  }
  if (state.pending) {
    node(overview, "p", "settings-status", state.repairRequested ? "Regenerating managed hook configuration…" : "Loading hook audit…").setAttribute("role", "status");
    return;
  }
  if (repairUnavailableReason) node(overview, "p", "settings-help", repairUnavailableReason);
  if (state.error) {
    node(overview, "p", "settings-status", state.error).setAttribute("role", "alert");
    return;
  }
  const report = state.report;
  if (!report) {
    node(overview, "p", "settings-help", "Select Hook Center or refresh to read the current audit.");
    return;
  }
  const health = report.health || {};
  const status = healthLabels[health.status] || "Unknown";
  const healthStatus = field(overview, "Hook health", status);
  healthStatus.dataset.role = "hook-health";
  healthStatus.dataset.status = healthLabels[health.status] ? health.status : "unknown";
  field(overview, "Last event", health.last_event);
  field(overview, "Last event at", health.last_event_at);
  if (health.pending_discussion) field(overview, "Pending discussion", health.pending_discussion.proposal_title || health.pending_discussion.proposal_label);
  if (health.pending_goal) field(overview, "Pending goal", health.pending_goal.condition || health.pending_goal.proposal_title);
  list(overview, "Issues", health.issues);
  list(overview, "Slow handlers", health.slow_handlers, "None reported (profiling is opt-in)", handler => `${handler.event} · ${handler.handler} · ${handler.duration_ms} ms`);

  const configuration = report.configuration_audit;
  const binaries = section("Configuration and binaries");
  field(binaries, "Asset root", configuration?.asset_root);
  field(binaries, "Managed generator binary", report.managed_hook_bin);
  field(binaries, "GWT_BIN_PATH", configuration?.gwt_bin_path, "Not set");
  field(binaries, "GWT_HOOK_BIN", configuration?.gwt_hook_bin, "Not set");
  node(binaries, "p", "settings-help", "Environment values describe the current GUI caller, not each running agent session. Generated fallback observations are separate; existing sessions are not upgraded by this audit.");
  field(binaries, "Opt-in profile path", configuration?.profile_path, "Not configured");
  node(binaries, "p", "settings-help", "Displaying a profile path does not enable profiling.");
  if (!configuration) node(binaries, "p", "settings-help", "Configuration audit unavailable.");
  for (const config of configuration?.configs || []) {
    const entry = section(`${config.provider === "claude" ? "Claude" : config.provider === "codex" ? "Codex" : config.provider} configuration`);
    entry.dataset.role = "hook-config";
    field(entry, "Path", config.path);
    field(entry, "Read state", ({ present: "Readable JSON", missing: "Missing", unreadable: "Unreadable", invalid_json: "Invalid JSON" })[config.state]);
    if (config.error) node(entry, "p", "settings-help", config.error);
    list(entry, "Observed generated fallbacks", config.event_binaries, "None recognized", event => `${event.event} · ${event.fallback_binary ?? "Unknown"}`);
  }
  node(binaries, "p", "settings-help", "Readable JSON does not imply managed or trusted content. Unknown fallback means the command was not recognized, including unsupported PowerShell brace paths or legacy quoted invocations.");

  const codex = section("Codex discovery and trust");
  const audit = report.codex_hook_audit;
  if (!audit) node(codex, "p", "settings-help", "Codex audit unavailable or no Codex assets materialized.");
  else {
    field(codex, "Discovery mode", audit.discovery_mode);
    list(codex, "Expected hook paths", audit.expected_hook_paths);
    if (audit.error) node(codex, "p", "settings-help", audit.error);
    const trust = audit.trust;
    if (!trust) node(codex, "p", "settings-help", "Trust inspection unavailable.");
    else {
      field(codex, "Trust config", trust.config_path);
      list(codex, "Trusted entries (exact stored hashes)", trust.trusted_entries, "None reported", entry => `${entry.key} · ${entry.trusted_hash}`);
      list(codex, "Missing managed events", trust.missing_managed_events);
      list(codex, "Modified managed hooks", trust.modified_gwt_hooks);
      list(codex, "Untrusted managed identities", trust.untrusted_gwt_hooks);
    }
  }
  const repair = section("Managed configuration repair");
  node(repair, "p", "settings-help", "Trust registration and a new agent session require separate verification. Regeneration does not confirm trust, launch success, or adoption by running sessions.");
  field(repair, "Last request", report.repair == null ? "No repair requested" : report.repair.repaired === true
    ? "Managed configuration regenerated" : "No managed configuration changes reported");
  const plan = report.repair_plan;
  if (!plan) node(repair, "p", "settings-help", "Repair plan unavailable.");
  else {
    node(repair, "p", "settings-help", "Planned steps below are not evidence that they ran.");
    const actionLabel = action => repairLabels[action] || action;
    list(repair, "Configuration regeneration steps", plan.automatic, "None reported", actionLabel);
    list(repair, "Separate user actions", plan.user_actions, "None reported", actionLabel);
    list(repair, "New session actions", plan.new_session_actions, "None reported", actionLabel);
  }
}
