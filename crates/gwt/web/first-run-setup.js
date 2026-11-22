import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";

// First-run setup (Issue #7731, SPEC #7007 slice N-4). Rendered by the React
// surface `FirstRunSetup` (crates/gwt/web-react); this host owns the state
// and every string the screen shows.
//
// The screen is the same whether the machine still needs it or it was opened
// again from Settings: it always shows the machine's current state. Whether
// it is required is the backend's `needs_setup`, never a "seen" mark.
//
// Two gates, both enforced again by the backend:
// - Tools: without a registered tool the setup does not go further, and the
//   screen says why.
// - Project Manager: one row chooses who it is — an agent already in the
//   ledger or a new agent — and nothing goes further until one is chosen.

const SETUP_STEPS = Object.freeze(["welcome", "tools", "pm", "project", "done"]);
// Steps Settings links to (design item 203).
export const SETUP_REVIEW_STEPS = Object.freeze([
  { step: "welcome", label: "Welcome" },
  { step: "tools", label: "Tools" },
  { step: "pm", label: "Project Manager" },
  { step: "project", label: "Open a project" },
]);
export const firstRunSetupUrlPath = step => `/?view=setup&step=${step}`;

const NEW_AGENT = "new";

const COPY = {
  heading: "First-run setup",
  lead: "Get this machine ready for agents. Every step stays available from Settings.",
  loading: "Reading this machine's setup…",
  disconnected: "Setup is not connected. It continues when the connection is back.",
  steps: { welcome: "Welcome", tools: "Tools", pm: "Project Manager", project: "Project", done: "Done" },
  back: "Back",
  welcome: {
    title: "Welcome to gwt",
    group: "What you will set up",
    rows: [
      { title: "A tool (required)", detail: "At least one agent tool is registered, so agents can run on this machine." },
      { title: "The Project Manager (required)", detail: "You choose the agent who serves as Project Manager." },
      { title: "A project (optional)", detail: "Open your first project now, or any time later." },
    ],
    next: "Start — Tools",
  },
  tools: {
    title: "Register a tool",
    lead: "Agents run with the agent tools installed on this machine. gwt registers each supported tool it finds.",
    registered: "Registered",
    notInstalled: "Not installed",
    noneTitle: "No tool is registered yet",
    noneDetail: "Install a supported agent tool, such as Claude Code or Codex, and sign in to it. Then choose Check again.",
    retry: "Check again",
    next: "Next — Project Manager",
    blocked: "Next is unavailable because no tool is registered on this machine.",
  },
  pm: {
    title: "Choose the Project Manager",
    lead: "One agent serves as Project Manager. You can reassign it later in the agent ledger.",
    row: "Project Manager",
    rowDetail: "Choose an agent already in the ledger, or a new agent.",
    newAgent: "A new agent",
    name: "Name of the new agent",
    namePlaceholder: "Name",
    next: "Next — Project",
    saving: "Saving…",
    needChoice: "Next becomes available once a Project Manager is chosen.",
    needName: "Next becomes available once the new agent has a name.",
    joins: "A new agent joins the ledger when the Project Manager first starts in a project.",
  },
  project: {
    title: "Open a project",
    lead: "Open your first project (optional).",
    openFolder: { label: "Open Folder…", detail: "Use a folder on this machine as a project." },
    clone: { label: "Clone from GitHub…", detail: "Pick a repository or paste a Git URL. It is cloned, then opened." },
    next: "Next — Done",
    note: "You can continue without opening a project.",
  },
  done: {
    title: "You are ready",
    lead: "Setup is complete. The Project Manager starts when a project opens, and you talk to it on Home.",
    tools: count => `${count} tool${count === 1 ? "" : "s"} registered.`,
    pm: name => `${name} serves as Project Manager.`,
    project: "Open one any time from Projects in the header.",
    doneBadge: "Done",
    laterBadge: "Any time",
    home: "Go to Home",
  },
};

const EMPTY_STATE = Object.freeze({
  step: "welcome", loaded: false, connected: false, view: null, error: "",
  // The choice on the Project Manager row: a member id, NEW_AGENT, or "".
  choice: "", name: "", touched: false, saving: false,
});

function installedTools(state) {
  return (state.view?.tools || []).filter(tool => tool.installed);
}

// A later step never opens ahead of an unmet gate: the step asked for becomes
// the gate's own step. Before the first snapshot the gates are unknown, so the
// step asked for is kept and settled by that snapshot.
function reachable(state, step) {
  if (!state.loaded) return step;
  const after = gate => SETUP_STEPS.indexOf(step) > SETUP_STEPS.indexOf(gate);
  if (!installedTools(state).length && after("tools")) return "tools";
  if (!state.view.pm && after("pm")) return "pm";
  return step;
}

// What the backend's snapshot changes. The row keeps what the person chose on
// this screen; before that it shows the saved designation.
function withSnapshot(state, view) {
  const saved = state.saving && Boolean(view.pm);
  const next = { ...state, loaded: true, view, error: "", saving: false, touched: state.touched && !saved };
  // A saved Project Manager opens the step after it.
  next.step = reachable(next, saved && state.step === "pm" ? "project" : state.step);
  if (next.touched) return next;
  const members = view.members || [];
  if (view.pm?.member_id) return { ...next, choice: view.pm.member_id, name: "" };
  if (view.pm) return { ...next, choice: NEW_AGENT, name: view.pm.name };
  // With nobody in the ledger the only possible Project Manager is a new agent.
  return { ...next, choice: members.length ? "" : NEW_AGENT, name: "" };
}

// The model the React surface renders, derived from the setup state.
function firstRunSetupView(state) {
  const tools = state.view?.tools || [];
  const members = state.view?.members || [];
  const toolRegistered = installedTools(state).length > 0;
  const reached = state.step;
  const index = SETUP_STEPS.indexOf(reached);
  const name = state.name.trim();
  const chosen = state.choice === NEW_AGENT ? Boolean(name) : Boolean(state.choice);
  return {
    heading: COPY.heading,
    lead: COPY.lead,
    status: !state.loaded ? (state.connected ? COPY.loading : COPY.disconnected) : !state.connected ? COPY.disconnected : "",
    error: state.error,
    loaded: state.loaded,
    step: reached,
    steps: SETUP_STEPS.map((step, at) => ({
      id: step, label: `${at + 1}. ${COPY.steps[step]}`,
      state: at < index ? "done" : at === index ? "current" : "todo",
    })),
    back: COPY.back,
    welcome: COPY.welcome,
    tools: {
      ...COPY.tools,
      items: tools.map(tool => ({
        name: tool.name, installed: Boolean(tool.installed),
        badge: tool.installed ? COPY.tools.registered : COPY.tools.notInstalled,
      })),
      registered: toolRegistered,
      reason: toolRegistered ? "" : COPY.tools.blocked,
    },
    pm: {
      ...COPY.pm,
      options: [
        ...members.map(member => ({ value: member.member_id, label: member.name })),
        { value: NEW_AGENT, label: COPY.pm.newAgent },
      ],
      choice: state.choice,
      naming: state.choice === NEW_AGENT,
      nameValue: state.name,
      nameMax: state.view?.name_max_chars || 40,
      note: state.choice === NEW_AGENT ? COPY.pm.joins : "",
      canSave: chosen && !state.saving && state.connected,
      next: state.saving ? COPY.pm.saving : COPY.pm.next,
      reason: chosen ? "" : state.choice === NEW_AGENT ? COPY.pm.needName : COPY.pm.needChoice,
    },
    project: COPY.project,
    done: {
      title: COPY.done.title, lead: COPY.done.lead, home: COPY.done.home,
      rows: [
        { title: COPY.steps.tools, detail: COPY.done.tools(installedTools(state).length), badge: COPY.done.doneBadge, state: "done" },
        { title: COPY.steps.pm, detail: COPY.done.pm(state.view?.pm?.name || ""), badge: COPY.done.doneBadge, state: "done" },
        { title: COPY.steps.project, detail: COPY.done.project, badge: COPY.done.laterBadge, state: "later" },
      ],
    },
  };
}

export function createFirstRunSetup({ document, send, onOpenFolder, onClone, onHome, onRequired = () => {} }) {
  const state = createUiStateStore(EMPTY_STATE);
  const model = createUiStateStore(firstRunSetupView(state.read()));
  state.subscribe(current => current, current => model.update(() => firstRunSetupView(current)));
  // Whether this machine still needs the setup, once the backend has said so.
  state.subscribe(current => (current.loaded ? Boolean(current.view.needs_setup) : null), needed => {
    if (needed !== null) onRequired(needed);
  });
  const set = patch => state.update(current => ({ ...current, ...patch }));
  // `refresh` has the backend detect the installed tools again.
  const request = (refresh = false) => send(refresh ? { kind: "get_first_run_setup", refresh: true } : { kind: "get_first_run_setup" });
  const goTo = step => {
    if (SETUP_STEPS.includes(step)) state.update(current => ({ ...current, step: reachable(current, step), error: "" }));
  };

  const element = createDetachedIslandContainer(document);
  mountIsland("first-run-setup", element, {
    store: model,
    actions: {
      goTo,
      retry: () => request(true),
      choose: choice => set({ choice, touched: true, error: "" }),
      rename: name => set({ name, touched: true, error: "" }),
      save: () => {
        const current = state.read();
        if (!model.read().pm.canSave) return;
        const message = current.choice === NEW_AGENT
          ? { kind: "save_first_run_setup", pm_name: current.name.trim() }
          : { kind: "save_first_run_setup", pm_member_id: current.choice };
        if (send(message) === "unavailable") return;
        set({ saving: true, error: "" });
      },
      openFolder: () => onOpenFolder(),
      clone: () => onClone(),
      home: () => onHome(),
    },
  });

  function connected() {
    set({ connected: true });
    request();
  }

  // A save in flight has no answer coming; the next snapshot tells.
  function connectionLost() {
    set({ connected: false, saving: false });
  }

  // Returns true when the event belonged to the setup.
  function receive(event) {
    if (event?.kind === "first_run_setup") {
      state.update(current => withSnapshot(current, event.view || {}));
      return true;
    }
    if (event?.kind === "first_run_setup_error") {
      state.update(current => ({ ...current, saving: false, error: event.message || "Setup could not be saved." }));
      return true;
    }
    return false;
  }

  return { element, showStep: goTo, connected, connectionLost, receive };
}
