import { createCloseProjectController } from "./close-project-confirm-modal.js";
// Issue #4538 AC-3: the Hub served at `/`. It is a picker only — Open Folder,
// Clone, Recent, and the open Projects — and never a workspace, dashboard, or
// tab strip. Every Project entry is a real, path-free link that opens the
// Project in its own browser tab.
import { renderProjectCloneModal } from "./project-clone-modal.js";
import { HUB_URL_PATH, projectUrlPath, routeWebSocketUrl } from "./frontend-route.js";

const HUB_RESULT_KINDS = new Set([
  "hub_state",
  "project_open_error",
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
  };
}

export function createHubApp({ document: doc, window: win }) {
  const node = (tag, className, text) => {
    const element = doc.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = text;
    return element;
  };

  let catalog = { app_version: "", projects: [], recent_projects: [] };
  let cloneState = initialCloneState();
  // Keys already open when this Hub asked for Open Folder / Clone. The first
  // new key in a later catalog is the Project that request produced.
  let pendingOpenKeys = null;
  let socket = null;
  let reconnectTimer = null;
  const queued = [];

  const root = node("main", "gwt-hub");
  root.dataset.hub = "";
  const header = node("header", "gwt-hub__header");
  const title = node("h1", "gwt-hub__title", "gwt");
  const version = node("span", "gwt-hub__version");
  version.hidden = true;
  header.append(title, version);

  const actions = node("div", "gwt-hub__actions");
  const openFolder = node("button", "wizard-button primary", "Open Folder…");
  openFolder.type = "button";
  openFolder.dataset.hubAction = "open-folder";
  const cloneButton = node("button", "wizard-button", "Clone from GitHub…");
  cloneButton.type = "button";
  cloneButton.dataset.hubAction = "clone";
  actions.append(openFolder, cloneButton);

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

  root.append(header, actions, notice, error, openSection.element, recentSection.element);

  function projectLink(project, className = "gwt-hub__project") {
    const link = node("a", className, project.title || project.project_key);
    link.href = projectUrlPath(project.project_key);
    link.target = "_blank";
    link.rel = "noopener";
    link.dataset.projectKey = project.project_key;
    return link;
  }

  function renderList(list, entries, emptyText, renderEntry) {
    list.replaceChildren();
    if (entries.length === 0) {
      list.append(node("li", "gwt-hub__empty", emptyText));
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
      catalog.projects = catalog.projects.filter((project) => project.project_key !== projectKey);
      render();
    },
  });

  function render() {
    version.hidden = !catalog.app_version;
    version.textContent = catalog.app_version ? `v${catalog.app_version}` : "";
    renderList(openSection.list, catalog.projects, "No open projects", (project) => {
      const row = node("div", "gwt-hub__project-actions");
      const close = node("button", "text-button", "Close Project");
      close.type = "button";
      close.dataset.closeProject = project.project_key;
      close.addEventListener("click", () => closeController.request(project.project_key));
      row.append(projectLink(project), close);
      return row;
    });
    renderList(recentSection.list, catalog.recent_projects, "No recent projects", (project) => {
      if (project.project_key) return projectLink(project);
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
    catalog = {
      app_version: hub?.app_version || "",
      projects: Array.isArray(hub?.projects) ? hub.projects : [],
      recent_projects: Array.isArray(hub?.recent_projects) ? hub.recent_projects : [],
    };
    if (pendingOpenKeys) {
      const opened = catalog.projects.find((project) => !pendingOpenKeys.has(project.project_key));
      if (opened) {
        pendingOpenKeys = null;
        showNotice(opened);
      }
    }
    render();
  }

  function beginOpenRequest() {
    pendingOpenKeys = new Set(catalog.projects.map((project) => project.project_key));
    notice.hidden = true;
    showError("");
  }

  function renderClone() {
    renderProjectCloneModal({
      modalEl: cloneModal,
      dialogEl: cloneDialog,
      state: cloneState,
      createNode: node,
      onClose: () => {
        if (cloneState.cloning) return;
        cloneState = { ...cloneState, open: false, searching: false, error: "", progress: "" };
        renderClone();
      },
      onModeChange: (mode) => {
        cloneState = { ...cloneState, mode, error: "", progress: "" };
        renderClone();
      },
      onUrlChange: (url) => {
        cloneState = { ...cloneState, url };
      },
      onParentSelect: () => send({ kind: "select_clone_project_parent" }),
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
        cloneState = { ...cloneState, selectedRepositoryUrl: url, url, error: "" };
        renderClone();
      },
      onClone: () => {
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
        cloneState = { ...cloneState, cloning: true, progress: "Cloning repository...", error: "" };
        renderClone();
        send({ kind: "clone_project_start", url, parent_path: parentPath });
      },
    });
  }

  function receiveClone(event) {
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
        cloneState = { ...cloneState, cloning: true, progress: event.message || "Cloning repository...", error: "" };
        break;
      case "clone_project_done":
        cloneState = { ...cloneState, open: false, cloning: false, searching: false, progress: "", error: "" };
        break;
      case "clone_project_error":
        pendingOpenKeys = null;
        cloneState = { ...cloneState, cloning: false, progress: "", error: event.message || "Clone failed." };
        break;
      default:
        return;
    }
    renderClone();
  }

  function receive(event) {
    if (event && closeController.receive(event)) return;
    if (!event || !HUB_RESULT_KINDS.has(event.kind)) return;
    if (event.kind === "hub_state") {
      receiveCatalog(event.hub);
    } else if (event.kind === "project_open_error") {
      pendingOpenKeys = null;
      showError(event.message || "Could not open the project.");
    } else {
      receiveClone(event);
    }
  }

  function send(message) {
    if (socket && socket.readyState === win.WebSocket.OPEN) {
      socket.send(JSON.stringify(message));
      return "sent";
    }
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
      connection.send(JSON.stringify({ kind: "frontend_ready" }));
      for (const message of queued.splice(0)) connection.send(JSON.stringify(message));
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
      reconnectTimer = win.setTimeout(connect, 1000);
    });
  }

  openFolder.addEventListener("click", () => {
    beginOpenRequest();
    send({ kind: "open_project_dialog" });
  });
  cloneButton.addEventListener("click", () => {
    cloneState = { ...cloneState, open: true, error: "", progress: "" };
    renderClone();
  });

  function installTestBridge() {
    if (win.__gwtPlaywrightTestBridge !== true || win.__gwtPlaywrightTestBridgeInstalled === true) {
      return;
    }
    win.__gwtPlaywrightTestBridgeInstalled = true;
    win.addEventListener("__gwt_test_send", (event) => {
      if (typeof event?.detail?.kind === "string") send(event.detail);
    });
  }

  render();
  return { root, cloneModal, closeModal: closeController.modal, connect, receive, send, installTestBridge, homeUrl: HUB_URL_PATH };
}

export function mountHubApp({ window: win, document: doc }) {
  doc.title = "gwt — Hub";
  const app = createHubApp({ window: win, document: doc });
  doc.body.className = "route-body";
  doc.body.replaceChildren(app.root, app.cloneModal, app.closeModal);
  app.installTestBridge();
  app.connect();
  return app;
}
