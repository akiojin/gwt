// SPEC-3431 FR-026 — Project Manager settings panel.
//
// The PM is the one agent that never goes through the Launch Wizard: it is
// started for the user when a project opens, so the only place its
// configuration can live is beside its launcher.
//
// The panel keeps two facts visibly separate — what the PM is running as right
// now, and what the next start will use. They can differ, and no amount of UI
// can migrate a live conversation across agents (a Claude history is not a
// Codex history), so a profile change is stored and then *applied* by an
// explicit, confirmed restart. The pending chip exists to make that gap
// impossible to miss.
//
// Pure rendering + a `send` transport, so the whole contract is unit-testable
// without a socket.

const AUTO_START_HELP =
  "Start the Project Manager automatically when this project opens.";
const RESTART_CONFIRM =
  "Restart the Project Manager?\n\n"
  + "The current PM session ends and a new conversation starts on the "
  + "configured agent. History is not carried over.";

function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

/// The snapshot the panel renders before any `pm_status` arrives. Chosen so a
/// panel opened during hydration shows "not running" rather than inventing a
/// configuration the project may not have.
function emptyStatus() {
  return {
    autoStart: true,
    configuredAgentId: "",
    configuredModel: "",
    runningAgentId: "",
    isRunning: false,
    agentOptions: [],
  };
}

function normalizeStatus(status) {
  const options = Array.isArray(status?.agent_options) ? status.agent_options : [];
  return {
    autoStart: status?.auto_start !== false,
    configuredAgentId: String(status?.configured_agent_id ?? ""),
    configuredModel: String(status?.configured_model ?? ""),
    runningAgentId: String(status?.running_agent_id ?? ""),
    isRunning: Boolean(status?.is_running),
    agentOptions: options
      .filter((option) => option && option.id)
      .map((option) => ({ id: String(option.id), name: String(option.name ?? option.id) })),
  };
}

function agentLabel(state, agentId) {
  if (!agentId) return "";
  const match = state.agentOptions.find((option) => option.id === agentId);
  return match ? match.name : agentId;
}

export function createPmSettingsPanel({ document, send, confirm } = {}) {
  if (!document) throw new TypeError("createPmSettingsPanel requires a document");
  const dispatch = typeof send === "function" ? send : () => {};
  const ask = typeof confirm === "function"
    ? confirm
    : (message) => Boolean(document.defaultView?.confirm?.(message));

  let state = emptyStatus();
  let panelEl = null;
  let toggleEl = null;
  let agentSelect = null;
  let modelInput = null;
  let autoStartInput = null;
  let runningLine = null;
  let pendingChip = null;

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  // The two selectors write the SAME event: the backend stores one profile, so
  // sending a partial update from either control would silently drop the other
  // field.
  function sendProfile() {
    const agentId = agentSelect?.value || state.configuredAgentId;
    if (!agentId) return;
    const model = (modelInput?.value ?? "").trim();
    dispatch({
      kind: "set_pm_launch_profile",
      agent_id: agentId,
      model: model === "" ? null : model,
      reasoning: null,
    });
  }

  function build() {
    clear(panelEl);

    panelEl.appendChild(el("h2", "pm-settings-panel__title", "Project Manager"));

    runningLine = el("p", "pm-settings-panel__running");
    runningLine.dataset.role = "pm-running-as";
    panelEl.appendChild(runningLine);

    pendingChip = el("p", "pm-settings-panel__pending");
    pendingChip.dataset.role = "pm-pending-chip";
    pendingChip.textContent = "Pending — restart to apply";
    pendingChip.hidden = true;
    panelEl.appendChild(pendingChip);

    const agentField = el("label", "pm-settings-panel__field");
    agentField.appendChild(el("span", "pm-settings-panel__label", "Agent"));
    agentSelect = el("select", "pm-settings-panel__select");
    agentSelect.dataset.role = "pm-agent-select";
    agentSelect.addEventListener("change", sendProfile);
    agentField.appendChild(agentSelect);
    panelEl.appendChild(agentField);

    const modelField = el("label", "pm-settings-panel__field");
    modelField.appendChild(el("span", "pm-settings-panel__label", "Model"));
    modelInput = el("input", "pm-settings-panel__input");
    modelInput.type = "text";
    modelInput.placeholder = "Agent default";
    modelInput.dataset.role = "pm-model-input";
    modelInput.addEventListener("change", sendProfile);
    modelField.appendChild(modelInput);
    panelEl.appendChild(modelField);

    const autoField = el("label", "pm-settings-panel__field pm-settings-panel__field--inline");
    autoStartInput = el("input");
    autoStartInput.type = "checkbox";
    autoStartInput.className = "pm-settings-panel__checkbox";
    autoStartInput.dataset.role = "pm-auto-start";
    autoStartInput.addEventListener("change", () => {
      dispatch({ kind: "set_pm_auto_start", enabled: Boolean(autoStartInput.checked) });
    });
    autoField.appendChild(autoStartInput);
    autoField.appendChild(el("span", "pm-settings-panel__label", "Auto start"));
    panelEl.appendChild(autoField);
    panelEl.appendChild(el("p", "pm-settings-panel__help", AUTO_START_HELP));

    const restart = el("button", "pm-settings-panel__restart", "Restart PM");
    restart.type = "button";
    restart.dataset.role = "pm-restart";
    restart.addEventListener("click", () => {
      // Losing the running conversation is not undoable, so it is never a
      // single click.
      if (!ask(RESTART_CONFIRM)) return;
      dispatch({ kind: "restart_pm_agent" });
    });
    panelEl.appendChild(restart);
  }

  function render() {
    if (!panelEl) return;

    const runningName = agentLabel(state, state.runningAgentId);
    runningLine.textContent = state.isRunning && runningName
      ? `Running as: ${runningName}`
      : "Not running";

    // "Pending" only means something while a PM is actually running a
    // different agent. A stopped PM is not waiting on a restart — it is simply
    // stopped, and the next start already uses the configured profile.
    pendingChip.hidden = !(
      state.isRunning
      && state.runningAgentId !== ""
      && state.configuredAgentId !== ""
      && state.configuredAgentId !== state.runningAgentId
    );

    clear(agentSelect);
    for (const option of state.agentOptions) {
      const node = document.createElement("option");
      node.value = option.id;
      node.textContent = option.name;
      // Selection is expressed on the option, not `select.value`: that is the
      // form the DOM actually stores it in, and it survives the select being
      // rebuilt on the next snapshot.
      if (option.id === state.configuredAgentId) node.selected = true;
      agentSelect.appendChild(node);
    }

    modelInput.value = state.configuredModel;
    autoStartInput.checked = state.autoStart;
  }

  function setOpen(open) {
    if (!panelEl) return;
    panelEl.hidden = !open;
    toggleEl?.setAttribute("aria-expanded", open ? "true" : "false");
  }

  return {
    mount() {
      panelEl = document.getElementById("pm-settings-panel");
      toggleEl = document.getElementById("op-pm-settings-button");
      if (!panelEl) return this;
      build();
      render();
      setOpen(false);
      toggleEl?.addEventListener("click", (event) => {
        event.preventDefault();
        setOpen(panelEl.hidden);
      });
      return this;
    },
    applyStatus(status) {
      state = normalizeStatus(status);
      render();
    },
    open() {
      setOpen(true);
    },
    close() {
      setOpen(false);
    },
    isOpen() {
      return Boolean(panelEl) && !panelEl.hidden;
    },
  };
}
