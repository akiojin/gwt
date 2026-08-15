// SPEC-3431 FR-026 / FR-132 — shared Project Manager settings editor.
//
// Settings windows are independent DOM mounts over one controller snapshot.
// The controller owns every PM settings event so entry points and Settings
// rendering cannot grow separate mutation paths.

const DEFAULT_LOOP_INTERVAL_SECS = 60;
const MIN_LOOP_INTERVAL_SECS = 10;
const MAX_U64 = 18446744073709551615n;
const AUTO_START_HELP =
  "Start the Project Manager automatically when this project opens.";
const LOOP_INTERVAL_HELP =
  "Default: 60 seconds. Minimum: 10 seconds. Changes apply to the next cycle without restarting the Project Manager.";
const PROFILE_HELP =
  "Agent and model changes apply after restart. Restarting starts a new conversation; history is not carried over.";
const RESTART_CONFIRM =
  "Restart the Project Manager?\n\n"
  + "The current PM session ends and a new conversation starts on the "
  + "configured agent. History is not carried over.";

function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

function normalizedLoopInterval(value) {
  const decimal = String(value ?? "").trim();
  if (!/^\d+$/.test(decimal)) return String(DEFAULT_LOOP_INTERVAL_SECS);
  try {
    const interval = BigInt(decimal);
    return interval >= BigInt(MIN_LOOP_INTERVAL_SECS) && interval <= MAX_U64
      ? interval.toString()
      : String(DEFAULT_LOOP_INTERVAL_SECS);
  } catch {
    return String(DEFAULT_LOOP_INTERVAL_SECS);
  }
}

// Before the first pm_status, keep controls unavailable while exposing the
// effective interval default. This prevents edits from targeting an unknown
// project during hydration.
function emptyStatus() {
  return {
    available: false,
    autoStart: true,
    configuredAgentId: "",
    configuredModel: "",
    loopIntervalSecs: String(DEFAULT_LOOP_INTERVAL_SECS),
    runningAgentId: "",
    isRunning: false,
    agentOptions: [],
  };
}

function normalizeStatus(status) {
  const options = Array.isArray(status?.agent_options) ? status.agent_options : [];
  return {
    available: status?.available !== false,
    autoStart: status?.auto_start !== false,
    configuredAgentId: String(status?.configured_agent_id ?? ""),
    configuredModel: String(status?.configured_model ?? ""),
    loopIntervalSecs: normalizedLoopInterval(
      status?.loop_interval_secs_decimal ?? status?.loop_interval_secs,
    ),
    runningAgentId: String(status?.running_agent_id ?? ""),
    isRunning: Boolean(status?.is_running),
    agentOptions: options
      .filter((option) => option && option.id)
      .map((option) => ({
        id: String(option.id),
        name: String(option.name ?? option.id),
      })),
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
  const mounts = new Map();
  const boundDocuments = new Set();
  let nextErrorId = 0;

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  function pruneDetachedMounts() {
    for (const [container] of mounts) {
      if (!container.isConnected) mounts.delete(container);
    }
  }

  function renderView(view) {
    const runningName = agentLabel(state, state.runningAgentId);
    view.runningLine.textContent = !state.available
      ? "Project Manager unavailable for this project."
      : state.isRunning && runningName
        ? `Running as: ${runningName}`
        : "Not running";

    view.pendingChip.hidden = !(
      state.isRunning
      && state.runningAgentId !== ""
      && state.configuredAgentId !== ""
      && state.configuredAgentId !== state.runningAgentId
    );

    clear(view.agentSelect);
    for (const option of state.agentOptions) {
      const node = document.createElement("option");
      node.value = option.id;
      node.textContent = option.name;
      if (option.id === state.configuredAgentId) node.selected = true;
      view.agentSelect.appendChild(node);
    }

    view.modelInput.value = state.configuredModel;
    view.intervalInput.value = String(state.loopIntervalSecs);
    view.autoStartInput.checked = state.autoStart;
    for (const control of [
      view.agentSelect,
      view.modelInput,
      view.intervalInput,
      view.autoStartInput,
      view.restart,
    ]) {
      control.disabled = !state.available;
    }
    view.intervalError.hidden = true;
    view.intervalError.textContent = "";
    view.intervalInput.setAttribute("aria-invalid", "false");
  }

  function renderAll() {
    pruneDetachedMounts();
    for (const view of mounts.values()) renderView(view);
  }

  function sendProfile(view) {
    if (!state.available) return;
    const agentId = view.agentSelect.value || state.configuredAgentId;
    if (!agentId) return;
    const model = view.modelInput.value.trim();
    dispatch({
      kind: "set_pm_launch_profile",
      agent_id: agentId,
      model: model === "" ? null : model,
      reasoning: null,
    });
  }

  function setIntervalError(view, message) {
    view.intervalError.textContent = message;
    view.intervalError.dataset.kind = "error";
    view.intervalError.hidden = false;
    view.intervalInput.setAttribute("aria-invalid", "true");
  }

  function sendLoopInterval(view) {
    if (!state.available) return;
    const decimal = view.intervalInput.value.trim();
    if (!/^\d+$/.test(decimal)) {
      setIntervalError(view, "Enter a whole number of seconds.");
      return;
    }
    const interval = BigInt(decimal);
    if (interval < BigInt(MIN_LOOP_INTERVAL_SECS)) {
      setIntervalError(view, "Loop interval must be at least 10 seconds.");
      return;
    }
    if (interval > MAX_U64) {
      setIntervalError(view, "Loop interval exceeds the supported range.");
      return;
    }
    view.intervalError.hidden = true;
    view.intervalError.textContent = "";
    view.intervalInput.setAttribute("aria-invalid", "false");
    dispatch({
      kind: "set_pm_loop_interval",
      loop_interval_secs: interval <= BigInt(Number.MAX_SAFE_INTEGER)
        ? Number(interval)
        : interval.toString(),
    });
  }

  function build(container) {
    clear(container);

    const section = el("div", "settings-section");
    const heading = el("h3", "settings-section-heading", "Project Manager");
    section.appendChild(heading);

    const runningLine = el("p", "settings-status");
    runningLine.dataset.role = "pm-running-as";
    section.appendChild(runningLine);

    const pendingChip = el(
      "p",
      "settings-status",
      "Pending profile change — restart to apply.",
    );
    pendingChip.dataset.role = "pm-pending-chip";
    pendingChip.hidden = true;
    section.appendChild(pendingChip);

    const agentField = el("label", "settings-field");
    agentField.appendChild(el("span", "settings-label", "Agent"));
    const agentSelect = el("select", "settings-select");
    agentSelect.dataset.role = "pm-agent-select";
    agentField.appendChild(agentSelect);
    section.appendChild(agentField);

    const modelField = el("label", "settings-field");
    modelField.appendChild(el("span", "settings-label", "Model"));
    const modelInput = el("input", "settings-input");
    modelInput.type = "text";
    modelInput.placeholder = "Agent default";
    modelInput.dataset.role = "pm-model-input";
    modelField.appendChild(modelInput);
    section.appendChild(modelField);
    section.appendChild(el("p", "settings-help", PROFILE_HELP));

    const intervalField = el("label", "settings-field");
    intervalField.appendChild(el("span", "settings-label", "Loop interval (seconds)"));
    const intervalInput = el("input", "settings-input");
    intervalInput.type = "number";
    intervalInput.min = String(MIN_LOOP_INTERVAL_SECS);
    intervalInput.step = "1";
    intervalInput.dataset.role = "pm-loop-interval";
    intervalInput.setAttribute("aria-invalid", "false");
    intervalField.appendChild(intervalInput);
    section.appendChild(intervalField);
    section.appendChild(el("p", "settings-help", LOOP_INTERVAL_HELP));

    const intervalError = el("p", "settings-status");
    intervalError.dataset.role = "pm-loop-interval-error";
    intervalError.dataset.kind = "error";
    intervalError.id = `pm-loop-interval-error-${++nextErrorId}`;
    intervalError.setAttribute("role", "alert");
    intervalError.setAttribute("aria-live", "polite");
    intervalError.hidden = true;
    intervalInput.setAttribute("aria-describedby", intervalError.id);
    section.appendChild(intervalError);

    const autoField = el("label", "settings-checkbox-label");
    const autoStartInput = el("input", "settings-checkbox");
    autoStartInput.type = "checkbox";
    autoStartInput.dataset.role = "pm-auto-start";
    autoField.appendChild(autoStartInput);
    autoField.appendChild(el("span", "", "Auto start"));
    section.appendChild(autoField);
    section.appendChild(el("p", "settings-help", AUTO_START_HELP));

    const restart = el("button", "wizard-button", "Restart Project Manager");
    restart.type = "button";
    restart.dataset.role = "pm-restart";
    section.appendChild(restart);
    container.appendChild(section);

    const view = {
      agentSelect,
      autoStartInput,
      intervalError,
      intervalInput,
      modelInput,
      pendingChip,
      restart,
      runningLine,
    };

    agentSelect.addEventListener("change", () => sendProfile(view));
    modelInput.addEventListener("change", () => sendProfile(view));
    intervalInput.addEventListener("change", () => sendLoopInterval(view));
    autoStartInput.addEventListener("change", () => {
      if (!state.available) return;
      dispatch({
        kind: "set_pm_auto_start",
        enabled: Boolean(autoStartInput.checked),
      });
    });
    restart.addEventListener("click", () => {
      if (!state.available) return;
      if (!ask(RESTART_CONFIRM)) return;
      dispatch({ kind: "restart_pm_agent" });
    });

    return view;
  }

  function openSettings(entryDocument) {
    const CustomEventCtor = entryDocument.defaultView?.CustomEvent || CustomEvent;
    entryDocument.dispatchEvent(
      new CustomEventCtor("settings:open", {
        detail: { target: "project-manager" },
      }),
    );
  }

  return {
    mount(container) {
      if (!container) {
        throw new TypeError("pmSettingsPanel.mount requires a container");
      }
      pruneDetachedMounts();
      const view = build(container);
      mounts.set(container, view);
      renderView(view);
      return this;
    },
    applyStatus(status) {
      state = normalizeStatus(status);
      renderAll();
    },
    bindEntryPoints({ document: entryDocument = document } = {}) {
      if (!entryDocument || boundDocuments.has(entryDocument)) return this;
      boundDocuments.add(entryDocument);

      entryDocument
        .getElementById("op-pm-settings-button")
        ?.addEventListener("click", (event) => {
          event.preventDefault();
          event.stopPropagation();
          openSettings(entryDocument);
        });
      entryDocument.addEventListener("op:command", (event) => {
        if (event.detail?.id !== "pm-settings") return;
        openSettings(entryDocument);
      });
      return this;
    },
  };
}
