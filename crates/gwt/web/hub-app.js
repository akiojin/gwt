import { createCloseProjectController } from "./close-project-confirm-modal.js";
import { createProjectAutonomyControl } from "./project-autonomy.js";
// The page served at `/` (Issue #4538 AC-3, Issue #7639). It has three views
// and is never a workspace, dashboard, or tab strip:
// - Home: the conversation with the Project Manager. It shows no project
//   contents (design items 5 and 46).
// - Projects: the picker — Open Folder, Clone, Recent, and the open Projects —
//   opened from the header. Every Project entry is a real, path-free link that
//   opens the Project in its own browser tab.
// - Setup: the first-run setup (Issue #7731). It opens by itself while this
//   machine still needs it, and from Settings at any time.
// - Environments: the execution environments and accounts of this machine
//   (Issue #7648 AC-7), the same screen a project page opens from its header.
import { renderProjectCloneModal, createFolderPickerController, createOpenProjectPathForm } from "./project-clone-modal.js";
import { HUB_URL_PATH, projectUrlPath, routeWebSocketUrl } from "./frontend-route.js";
import { createProjectExecutionPolicyModel, mountProjectExecutionStatus } from "./project-execution-policy-model.js";
import { createUiStateStore } from "./ui-state-store.js";
import { createAppChrome } from "./app-chrome.js";
import { createHomeConversation } from "./home-conversation.js";
import { createFirstRunSetup, firstRunSetupUrlPath } from "./first-run-setup.js";
import { createExecutionEnvironments } from "./execution-environments.js";
import { createProviderUsageSurface } from "./provider-usage-surface.js";

// The view a `/?view=…` link opens; anything else is Home.
const HUB_VIEWS = Object.freeze(["home", "projects", "setup", "environments"]);

// The account sign-in terminal. xterm loads on first use: the Hub does not
// draw terminals otherwise.
const XTERM_MODULE = "/assets/xterm/xterm.mjs";
function decodeBase64(base64) {
  return Uint8Array.from(atob(base64), value => value.charCodeAt(0));
}
function openLoginTerminal(doc, element, onData) {
  let terminal = null;
  let input = null;
  let disposed = false;
  const pending = [];
  import(XTERM_MODULE).then(({ Terminal }) => {
    if (disposed) return;
    const fontFamily = doc.defaultView.getComputedStyle(doc.documentElement).getPropertyValue("--font-mono").trim();
    terminal = new Terminal({ cols: 80, rows: 24, fontSize: 13, scrollback: 0, ...(fontFamily ? { fontFamily } : {}) });
    terminal.open(element);
    terminal.focus();
    input = terminal.onData(onData);
    for (const data of pending.splice(0)) terminal.write(decodeBase64(data));
  }).catch(error => doc.defaultView.console?.warn?.("[gwt hub] login terminal failed to load", error));
  return {
    appendOutput: data => (terminal ? terminal.write(decodeBase64(data)) : pending.push(data)),
    dispose: () => {
      disposed = true;
      input?.dispose();
      terminal?.dispose();
    },
  };
}

const HUB_RESULT_KINDS = new Set([
  "hub_state",
  "orphan_daemon_result",
  "project_open_error",
  "project_opened",
  "clone_project_parent_selected",
  "github_repository_search_results",
  "github_repository_search_error",
  "clone_project_progress",
  "clone_project_done",
  "clone_project_error",
]);

function initialCloneState() {
  return {
    open: false,
    mode: "url",
    url: "",
    parentPath: "",
    query: "",
    repositories: [],
    selectedRepositoryUrl: "",
    searching: false,
    cloning: false,
    progress: "",
    error: "",
    existingProjectUrl: "",
  };
}

function createHubApp({ document: doc, window: win, setVersion, setPm = () => {}, setActiveSurface = () => {} }) {
  const node = (tag, className, text) => {
    const element = doc.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  };

  const executionPolicies = new Map();
  let releaseExecutionViews = [];
  let catalogConnectionFresh = true;
  function executionPolicy(key) {
    if (!executionPolicies.has(key)) executionPolicies.set(key, createProjectExecutionPolicyModel(key));
    return executionPolicies.get(key);
  }
  function executionView(project) {
    const view = node("div", "gwt-hub__orphan-detail"); view.dataset.projectExecution = project.project_key;
    releaseExecutionViews.push(mountProjectExecutionStatus(view, executionPolicy(project.project_key).model, { projectTitle: project.title }));
    return view;
  }
  let catalog = { app_version: "", projects: [], recent_projects: [] };
  let cloneState = initialCloneState();
  let socket = null;
  let reconnectTimer = null;
  let queued = [];
  let cloneTab = null;
  const cloneResults = createUiStateStore(null);
  const catalogResults = createUiStateStore(null);
  const executionResults = createUiStateStore(null);
  executionResults.subscribe(status => status, status => {
    if (status && executionPolicies.has(status.project_key)) executionPolicies.get(status.project_key).applyStatus(status);
  });
  const openResults = createUiStateStore(null);

  // `data-hub` marks the page; the picker is the Projects view inside it.
  const root = node("main", "gwt-hub-page");
  root.dataset.hub = "";
  const projectsView = node("section", "gwt-hub");
  projectsView.dataset.hubProjects = "";
  projectsView.setAttribute("aria-labelledby", "gwt-hub-title");
  const header = node("header", "gwt-hub__header");
  const title = node("h1", "gwt-hub__title", "Projects");
  title.id = "gwt-hub-title";
  header.append(title);
  // Projects are found by name; a path never reaches this page's markup.
  const search = node("input", "gwt-hub__search");
  search.type = "search";
  search.placeholder = "Search projects…";
  search.setAttribute("aria-label", "Search projects");
  search.dataset.hubSearch = "";
  search.addEventListener("input", () => render());

  const actions = node("div", "gwt-hub__actions");
  const openFolder = node("button", "wizard-button primary", "Open Folder…");
  openFolder.type = "button";
  openFolder.dataset.hubAction = "open-folder";
  const cloneButton = node("button", "wizard-button", "Clone from GitHub…");
  cloneButton.type = "button";
  cloneButton.dataset.hubAction = "clone";
  actions.append(openFolder, cloneButton);
  actions.append(createOpenProjectPathForm(doc, (path) => {
    beginOpenRequest();
    send({ kind: "reopen_recent_project", path });
  }));

  const notice = node("p", "gwt-hub__notice");
  notice.setAttribute("role", "status");
  notice.hidden = true;
  const error = node("p", "gwt-hub__error");
  error.setAttribute("role", "alert");
  error.hidden = true;

  const section = (id, label) => {
    const element = node("section", "gwt-hub__section");
    element.setAttribute("aria-labelledby", id);
    const heading = node("h2", "gwt-hub__section-title", label);
    heading.id = id;
    const list = node("ul", "gwt-hub__list");
    element.append(heading, list);
    return { element, list };
  };
  const openSection = section("gwt-hub-open-title", "Open");
  openSection.list.dataset.hubList = "open";
  const orphanSection = section("gwt-hub-orphans-title", "Projects awaiting shutdown");
  orphanSection.list.dataset.hubList = "orphans";
  const refreshOrphans = node("button", "text-button", "Refresh");
  refreshOrphans.type = "button";
  refreshOrphans.dataset.hubAction = "refresh-orphans";
  refreshOrphans.addEventListener("click", () => send({ kind: "refresh_orphan_daemons" }));
  orphanSection.element.append(refreshOrphans);
  const recentSection = section("gwt-hub-recent-title", "Recent");
  recentSection.list.dataset.hubList = "recent";

  const cloneModal = node("div", "modal-backdrop");
  cloneModal.id = "clone-project-modal";
  cloneModal.setAttribute("aria-hidden", "true");
  const cloneDialog = node("div", "modal-shell");
  cloneDialog.setAttribute("role", "dialog");
  cloneDialog.setAttribute("aria-modal", "true");
  cloneDialog.setAttribute("aria-labelledby", "clone-project-modal-title");
  cloneDialog.tabIndex = -1;
  cloneModal.append(cloneDialog);

  const picker = createFolderPickerController({
    send,
    onChange: (pending, message) => {
      openFolder.textContent = pending ? "Choosing folder…" : "Open Folder…";
      openFolder.setAttribute("aria-busy", String(Boolean(pending)));
      cloneState = { ...cloneState, pickerPending: Boolean(pending), pickerMessage: message };
      showError(message);
      renderClone();
    },
    onSelected: (event) => {
      if (event.purpose === "clone_parent") {
        cloneState = { ...cloneState, parentPath: event.path, error: "" };
        renderClone();
      }
    },
  });

  const home = createHomeConversation({
    document: doc, window: win, send,
    onPmChange: setPm,
    onOpenFolder: () => openFolderDialog(),
    onClone: mode => openCloneDialog(mode),
    onOpenProjects: () => showView("projects"),
  });
  const homeView = home.element;
  const setup = createFirstRunSetup({
    document: doc, send,
    onOpenFolder: () => openFolderDialog(),
    onClone: () => openCloneDialog(),
    onHome: () => showView("home"),
    // A machine that cannot start work yet lands on its setup.
    onRequired: needed => { if (needed) showView("setup"); },
  });
  const setupView = setup.element;
  // Issue #7648 AC-7: the execution environments screen. Its definitions,
  // the usage snapshot and the tool catalog reach this page as they reach a
  // project page; nothing here is project-scoped, so it has no priority list.
  const { applyProviderUsageUi, renderAccountUsage, usageModel } = createProviderUsageSurface({ renderWorkspaceWindows: () => {} });
  const supportedAgentsModel = createUiStateStore({ agents: null });
  const environmentDefinitions = createUiStateStore({ view: null, refusal: null });
  const poolLoginEvents = createUiStateStore(null);
  const environments = createExecutionEnvironments({
    document: doc, usageModel, supportedAgentsModel, renderAccountUsage, send,
    definitionsModel: environmentDefinitions,
    loginEventModel: poolLoginEvents,
    openLoginTerminal: (element, onData) => openLoginTerminal(doc, element, onData),
    // Tool management lives in a project's Settings; here the setup's Tools step registers them.
    onOpenToolManagement: () => win.location.assign(firstRunSetupUrlPath("tools")),
    onSetUsageCollection: (provider, enabled) => {
      if (provider === "claude_code") send({ kind: "set_claude_account_usage_enabled", enabled });
    },
  });
  const environmentsView = environments.element;
  environmentsView.classList.add("gwt-hub__environments");
  environmentsView.hidden = true;
  // Answers to the screen's requests and the broadcasts it is drawn from.
  function receiveEnvironments(event) {
    switch (event?.kind) {
      case "execution_environments":
        environmentDefinitions.update(state => ({ ...state, view: event.view }));
        return true;
      case "execution_environment_error":
        environmentDefinitions.update(state => ({ ...state, refusal: { message: event.message } }));
        return true;
      case "pool_account_login_started":
      case "pool_account_login_output":
      case "pool_account_login_finished":
        poolLoginEvents.update(() => event);
        return true;
      case "provider_usage":
        applyProviderUsageUi({
          accounts: event.accounts || [],
          sessions: (event.sessions || []).filter(session => session.active !== false),
          consumption: event.consumption || [],
        });
        return true;
      case "supported_agent_list":
        supportedAgentsModel.update(state => ({ ...state, agents: event.agents || [] }));
        return true;
      default:
        return false;
    }
  }
  projectsView.append(header, actions, search, openSection.element, recentSection.element, orphanSection.element);
  root.append(notice, error, homeView, projectsView, setupView, environmentsView);

  let view = "home";
  function showView(next) {
    view = HUB_VIEWS.includes(next) ? next : "home";
    root.dataset.hubView = view;
    homeView.hidden = view !== "home";
    projectsView.hidden = view !== "projects";
    setupView.hidden = view !== "setup";
    const showEnvironments = view === "environments";
    if (environmentsView.hidden === showEnvironments) {
      environmentsView.hidden = !showEnvironments;
      if (showEnvironments) {
        environments.open("environments");
        send({ kind: "list_supported_agents" });
        environments.reload();
      } else {
        environments.closeLogin(false);
      }
    }
    // Home fills the screen and scrolls its own timeline.
    doc.body.classList.toggle("route-body--chat", view === "home");
    setActiveSurface(view);
  }

  function projectLink(project, className = "gwt-hub__project") {
    const link = node("a", className, project.title || project.project_key);
    link.href = projectUrlPath(project.project_key);
    link.target = "_blank";
    link.rel = "noopener";
    link.dataset.projectKey = project.project_key;
    return link;
  }

  function renderList(list, all, emptyText, renderEntry) {
    list.replaceChildren();
    const needle = search.value.trim().toLowerCase();
    const entries = needle
      ? all.filter(entry => String(entry.title || entry.project_key || "").toLowerCase().includes(needle))
      : all;
    if (entries.length === 0) {
      list.append(node("li", "gwt-hub__empty", all.length ? "No project matches." : emptyText));
      return;
    }
    for (const entry of entries) {
      const item = node("li", "gwt-hub__item");
      item.append(renderEntry(entry));
      list.append(item);
    }
  }

  const closeController = createCloseProjectController({
    document: doc, send, onError: showError,
    onClosed: (projectKey) => {
      send({ kind: "frontend_ready" });
    },
  });

  // Issue #7728: the Hub list and Settings mount the same stop/start surface.
  function autonomyControl(project) {
    const control = createProjectAutonomyControl({
      document: doc, projectTitle: project.title || project.project_key,
      send: running => send({ kind: "set_project_autonomy", project_key: project.project_key, running }),
    });
    control.element.dataset.projectAutonomy = project.project_key;
    control.update(project.autonomy);
    releaseExecutionViews.push(() => control.unmount());
    return control.element;
  }

  function render() {
    for (const release of releaseExecutionViews) release();
    releaseExecutionViews = [];
    setVersion(catalog.app_version ? `v${catalog.app_version}` : "");
    renderList(openSection.list, catalog.projects, "No open projects", (project) => {
      const row = node("div", "gwt-hub__project-actions");
      const close = node("button", "text-button", "Close Project");
      close.type = "button";
      close.dataset.closeProject = project.project_key;
      close.addEventListener("click", () => closeController.request(project.project_key, { projectWide: true }));
      row.append(projectLink(project));
      if (project.owner_health) {
        const health = project.owner_health;
        const badge = node("span", "project-owner-health", health.state === "healthy" ? "Ready" : health.state === "wedged" ? "Wedged" : health.state === "degraded" ? "Unavailable" : "Connecting");
        badge.dataset.projectHealth = project.project_key;
        badge.dataset.state = health.state;
        badge.title = health.detail || "";
        badge.setAttribute("role", "status");
        row.append(badge);
      }
      row.append(autonomyControl(project), close, executionView(project));
      return row;
    });
    renderList(orphanSection.list, catalog.orphan_daemons || [], "No projects awaiting shutdown", (project) => {
      const row = node("div", "gwt-hub__orphan");
      row.append(node("span", "gwt-hub__project", project.title || project.project_key));
      row.append(node("span", "gwt-hub__orphan-detail", project.detail));
      const retry = node("button", "text-button", "Retry shutdown");
      retry.type = "button";
      retry.dataset.orphanRetry = "";
      retry.disabled = !project.observation_token;
      retry.addEventListener("click", () => {
        if (retry.disabled) return;
        retry.disabled = true;
        send({ kind: "reap_orphan_daemon", observation_token: project.observation_token });
      });
      row.append(retry);
      return row;
    });
    renderList(recentSection.list, catalog.recent_projects, "No recent projects", (project) => {
      if (project.project_key) {
        const row = node("div", "gwt-hub__project-actions"); row.append(projectLink(project), executionView(project)); return row;
      }
      // The runtime resolves the key off-thread; until then the entry is
      // shown but not linkable (a link must never carry a path).
      const pending = node("span", "gwt-hub__project is-pending", project.title);
      pending.setAttribute("aria-disabled", "true");
      return pending;
    });
  }

  function showNotice(project) {
    const link = projectLink(project, "gwt-hub__notice-link");
    link.textContent = "Open in a new tab";
    notice.replaceChildren(node("span", "", `Opened ${project.title || project.project_key}. `), link);
    notice.hidden = false;
  }

  function showError(message) {
    error.textContent = message || "";
    error.hidden = !message;
  }

  function receiveCatalog(hub) {
    // Recent links may duplicate an observed project without its runtime
    // projection; the current project observation owns that shared row state.
    const projects = new Map([...(hub?.recent_projects || []), ...(hub?.projects || [])].map(project => [project.project_key, project]));
    for (const project of projects.values()) {
      if (!project.project_key) continue;
      if (project.execution) {
        if (catalogConnectionFresh) executionPolicy(project.project_key).bindGeneration(project.execution.generation, { strict: false });
        executionPolicy(project.project_key).applyStatus(project.execution);
      }
      else executionPolicy(project.project_key).connectionLost();
    }
    catalogConnectionFresh = false;
    catalog = {
      app_version: hub?.app_version || "",
      projects: Array.isArray(hub?.projects) ? hub.projects : [],
      recent_projects: Array.isArray(hub?.recent_projects) ? hub.recent_projects : [],
      orphan_daemons: Array.isArray(hub?.orphan_daemons) ? hub.orphan_daemons : [],
    };
    render();
    home.receiveCatalog({ open: catalog.projects.length, recent: catalog.recent_projects.length });
  }

  catalogResults.subscribe(state => state, hub => {
    if (hub) receiveCatalog(hub);
  });

  function beginOpenRequest() {
    notice.hidden = true;
    showError("");
  }

  function closeCloneTab() {
    if (cloneTab && !cloneTab.closed) cloneTab.close();
    cloneTab = null;
  }

  function renderClone() {
    renderProjectCloneModal({
      modalEl: cloneModal,
      dialogEl: cloneDialog,
      state: cloneState,
      createNode: node,
      onClose: () => {
        if (cloneState.cloning) return;
        cloneState = { ...cloneState, open: false, searching: false, error: "", progress: "", existingProjectUrl: "" };
        renderClone();
      },
      onModeChange: (mode) => {
        cloneState = { ...cloneState, mode, error: "", progress: "", existingProjectUrl: "" };
        renderClone();
      },
      onUrlChange: (url) => {
        cloneState = { ...cloneState, url };
      },
      onParentSelect: () => picker.begin("clone_parent"),
      onParentChange: (parentPath) => { cloneState = { ...cloneState, parentPath }; },
      onSearchQueryChange: (query) => {
        cloneState = { ...cloneState, query };
      },
      onSearch: () => {
        const query = cloneState.query.trim();
        if (!query) return;
        cloneState = { ...cloneState, searching: true, error: "" };
        renderClone();
        send({ kind: "github_repository_search", query });
      },
      onRepositorySelect: (url) => {
        cloneState = { ...cloneState, selectedRepositoryUrl: url, url, error: "", existingProjectUrl: "" };
        renderClone();
      },
      onClone: () => {
        if (cloneState.cloning) return;
        const url = (cloneState.mode === "search" ? cloneState.selectedRepositoryUrl : cloneState.url).trim();
        const parentPath = cloneState.parentPath.trim();
        if (!url || !parentPath) {
          cloneState = {
            ...cloneState,
            error: !url ? "Select or enter a repository URL." : "Choose a destination parent folder.",
          };
          renderClone();
          return;
        }
        beginOpenRequest();
        // Reserve during the click's user activation; asynchronous completion
        // can then navigate this tab without a second popup permission.
        try {
          cloneTab = win.open("about:blank", "_blank");
          if (cloneTab) cloneTab.opener = null;
        } catch {
          closeCloneTab();
        }
        cloneState = { ...cloneState, cloning: true, progress: "Cloning repository...", error: "", existingProjectUrl: "" };
        renderClone();
        send({ kind: "clone_project_start", url, parent_path: parentPath });
      },
    });
  }

  function receiveClone(event) {
    cloneResults.update(() => event);
  }

  function renderCloneResult(event) {
    switch (event.kind) {
      case "clone_project_parent_selected":
        cloneState = { ...cloneState, parentPath: event.path || "", error: "" };
        break;
      case "github_repository_search_results":
        if (event.query !== cloneState.query.trim()) return;
        cloneState = {
          ...cloneState,
          repositories: event.repositories || [],
          selectedRepositoryUrl: "",
          searching: false,
          error: "",
        };
        break;
      case "github_repository_search_error":
        if (event.query !== cloneState.query.trim()) return;
        cloneState = { ...cloneState, searching: false, error: event.message || "Repository search failed." };
        break;
      case "clone_project_progress":
        if (!cloneState.cloning) return;
        cloneState = { ...cloneState, cloning: true, progress: event.message || "Cloning repository...", error: "" };
        break;
      case "clone_project_done":
        if (!cloneState.cloning) return;
        if (event.project_key) {
          const url = projectUrlPath(event.project_key);
          if (cloneTab && !cloneTab.closed) cloneTab.location.href = new URL(url, win.location.href).href;
          cloneTab = null;
          showNotice(event);
        } else {
          closeCloneTab();
        }
        cloneState = { ...cloneState, open: false, cloning: false, searching: false, progress: "", error: "" };
        break;
      case "clone_project_error":
        if (!cloneState.cloning) return;
        closeCloneTab();
        cloneState = {
          ...cloneState, cloning: false, progress: "", error: event.message || "Clone failed.",
          existingProjectUrl: event.existing_project_key ? projectUrlPath(event.existing_project_key) : "",
        };
        break;
      default:
        return;
    }
    renderClone();
  }

  cloneResults.subscribe((state) => state, (event) => {
    if (event) renderCloneResult(event);
  });

  openResults.subscribe(state => state, result => {
    if (!result) return;
    if (result.kind === "project_opened") showNotice(result);
    else if (result.kind === "orphan_daemon_result") showError(result.message);
    else showError(result.message || "Could not open the project.");
  });

  function receive(event) {
    if (event && picker.receive(event)) return;
    if (event && home.receive(event)) return;
    if (event && setup.receive(event)) return;
    if (event && closeController.receive(event)) return;
    if (receiveEnvironments(event)) return;
    if (event?.kind === "project_execution_status") {
      executionResults.update(() => event.status);
      return;
    }
    if (!event || !HUB_RESULT_KINDS.has(event.kind)) return;
    if (event.kind === "orphan_daemon_result") {
      openResults.update(() => ({ ...event }));
    } else if (event.kind === "hub_state") {
      catalogResults.update(() => ({ ...event.hub }));
    } else if (event.kind === "project_opened") {
      openResults.update(() => ({ ...event }));
    } else if (event.kind === "project_open_error") {
      openResults.update(() => ({ ...event }));
    } else {
      receiveClone(event);
    }
  }

  function send(message) {
    if (socket && socket.readyState === win.WebSocket.OPEN) {
      socket.send(JSON.stringify(message));
      return "sent";
    }
    if (["confirm_close_project", "cancel_close_project", "preview_close_project", "load_home_conversation", "home_pm_send", "home_answer_decision", "home_defer_decision",
      "get_first_run_setup", "save_first_run_setup"].includes(message.kind)) return "unavailable";
    queued.push(message);
    return "queued";
  }

  function connect() {
    if (reconnectTimer) win.clearTimeout(reconnectTimer);
    reconnectTimer = null;
    const connection = new win.WebSocket(routeWebSocketUrl(win.location.href, null));
    socket = connection;
    connection.addEventListener("open", () => {
      if (socket !== connection) return;
      catalogConnectionFresh = true;
      connection.send(JSON.stringify({ kind: "frontend_ready" }));
      closeController.reconnect();
      home.connected();
      setup.connected();
      const replayedCloneSearchQueries = [];
      for (const message of queued.splice(0)) {
        connection.send(JSON.stringify(message));
        if (message.kind === "github_repository_search") replayedCloneSearchQueries.push(message.query);
      }
      const query = cloneState.query.trim();
      if (cloneState.open && cloneState.searching && !cloneState.cloning) {
        if (!query) {
          cloneState = { ...cloneState, searching: false };
          renderClone();
        } else if (!replayedCloneSearchQueries.includes(query)) {
          send({ kind: "github_repository_search", query });
        }
      }
    });
    connection.addEventListener("message", (message) => {
      if (socket !== connection) return;
      try {
        receive(JSON.parse(String(message.data)));
      } catch (receiveError) {
        win.console?.warn?.("[gwt hub] receive failed", receiveError);
      }
    });
    connection.addEventListener("close", () => {
      if (socket !== connection) return;
      closeController.connectionLost();
      home.connectionLost();
      setup.connectionLost();
      environments.connectionLost();
      for (const policy of executionPolicies.values()) policy.connectionLost();
      closeCloneTab();
      if (cloneState.cloning) {
        cloneState = { ...cloneState, cloning: false, progress: "", error: "Connection lost while cloning. Check the project list before retrying." };
      }
      picker.connectionLost();
      queued = queued.filter(({ kind }) =>
        !["open_project_dialog", "select_clone_project_parent", "clone_project_start", "load_home_conversation", "home_pm_send", "home_answer_decision", "home_defer_decision"].includes(kind));
      reconnectTimer = win.setTimeout(connect, 1000);
    });
  }

  function openFolderDialog() {
    beginOpenRequest();
    picker.begin("open");
  }
  function openCloneDialog(mode = cloneState.mode) {
    cloneState = { ...cloneState, open: true, mode, error: "", progress: "", existingProjectUrl: "" };
    renderClone();
  }
  openFolder.addEventListener("click", () => openFolderDialog());
  cloneButton.addEventListener("click", () => openCloneDialog());

  function installTestBridge() {
    if (win.__gwtPlaywrightTestBridge !== true || win.__gwtPlaywrightTestBridgeInstalled === true) {
      return;
    }
    win.__gwtPlaywrightTestBridgeInstalled = true;
    win.addEventListener("__gwt_test_send", (event) => {
      if (typeof event?.detail?.kind === "string") send(event.detail);
    });
  }

  function openInitialAction() {
    const params = new URL(win.location.href).searchParams;
    const action = params.get("action");
    // An action link opens the picker's own flow, so it lands on Projects.
    showView(action === "open-folder" || action === "clone" ? "projects" : params.get("view"));
    // A Settings link opens the setup on the step it names.
    if (params.get("step")) setup.showStep(params.get("step"));
    if (action === "open-folder") openFolderDialog();
    else if (action === "clone") openCloneDialog();
  }

  render();
  showView("home");
  // Issue #7646 AC-7: the agent ledger is served by a project page, so the
  // header's Agents opens it on the first open project. With none open, the
  // Projects view is where one is opened; nothing is opened on the user's behalf.
  function openLedger() {
    const project = catalog.projects.find(entry => entry?.project_key);
    if (!project) { showView("projects"); return; }
    win.location.assign(`${projectUrlPath(project.project_key)}?view=agents`);
  }
  return { root, cloneModal, openLedger, closeModal: closeController.modal, connect, receive, send, installTestBridge, openInitialAction, showView, home, setup, homeUrl: HUB_URL_PATH };
}

export function mountHubApp({ window: win, document: doc }) {
  doc.title = "gwt — Home";
  // Issue #7740: this page shares the header and footer; the version lives in
  // the footer. Issue #7639: the header's PM entry is Home and its Projects
  // entry is the picker. Issue #7646: its Agents entry is the agent ledger.
  let app = null;
  const chrome = createAppChrome({ document: doc, handlers: {
    pm: () => app?.showView("home"),
    projects: () => app?.showView("projects"),
    agents: () => app?.openLedger(),
    environments: () => app?.showView("environments"),
  } });
  const page = chrome.mountPage();
  app = createHubApp({ window: win, document: doc, setVersion: chrome.setVersion,
    setPm: chrome.setPm, setActiveSurface: chrome.setActiveSurface });
  doc.body.className = "route-body";
  doc.body.replaceChildren(page.header, app.root, app.cloneModal, app.closeModal, page.footer);
  // Run once after mounting, before connecting: picker requests queue behind
  // frontend_ready, while reconnect only re-establishes the Hub connection.
  app.openInitialAction();
  app.installTestBridge();
  app.connect();
  return app;
}
