// SPEC-2809 — Console window for external process stdout/stderr live tail.
//
// 5 fixed kind tabs (gh / git / docker / agent / runner) backed by the
// `ProcessConsoleHub` broadcast channel that the backend exposes through
// `BackendEvent::ProcessLine`. Per-tab buffer is kept on the JS side (each
// `ProcessLine` from the wire is appended to its kind's buffer; older lines
// are dropped past CAP_PER_KIND so the DOM does not grow without bound).
//
// Mounting contract: `app.js` calls `createConsoleWindow({ document })` once
// per Console window instance and stores the controller on the window record.
// When a `process_line` event arrives the dispatcher calls `controller.push(line)`.
// Closing the window discards the controller; the next open call recreates it.

const KINDS = ["gh", "git", "docker", "agent", "runner"];
const CAP_PER_KIND = 5000;
const FOLLOW_THRESHOLD_PX = 24;

export function createConsoleWindow({ document, send = null, windowId = null } = {}) {
  if (!document) {
    throw new Error("createConsoleWindow requires a document");
  }

  const state = {
    root: null,
    tabRow: null,
    panes: new Map(), // kind -> { container, tab, buffer, lastSpawnId, scrollFollow, empty }
    activeKind: "gh",
    windowId,
    send,
  };

  for (const kind of KINDS) {
    state.panes.set(kind, {
      container: null,
      tab: null,
      emptyHint: null,
      buffer: [],
      lastSpawnId: null,
      scrollFollow: true,
    });
  }

  function clearChildren(node) {
    while (node && node.firstChild) {
      node.removeChild(node.firstChild);
    }
  }

  function mount(parent) {
    const root = document.createElement("div");
    root.className = "console-window";
    root.dataset.activeKind = state.activeKind;

    const tabRow = document.createElement("div");
    tabRow.className = "console-window__tabs";
    tabRow.setAttribute("role", "tablist");

    for (const kind of KINDS) {
      const tab = document.createElement("button");
      tab.type = "button";
      tab.className = "console-window__tab";
      tab.dataset.kind = kind;
      tab.setAttribute("role", "tab");
      tab.setAttribute("aria-selected", kind === state.activeKind ? "true" : "false");
      tab.textContent = kind;
      tab.addEventListener("click", () => activate(kind));
      tabRow.appendChild(tab);
      state.panes.get(kind).tab = tab;
    }

    const body = document.createElement("div");
    body.className = "console-window__body";

    for (const kind of KINDS) {
      const container = document.createElement("pre");
      container.className = "console-window__pane";
      container.dataset.kind = kind;
      container.setAttribute("role", "tabpanel");
      container.hidden = kind !== state.activeKind;
      container.addEventListener("scroll", () => updateScrollFollow(kind));
      // Empty-state hint shown when no lines have arrived for this kind
      // yet. Removed when the first line is pushed.
      const emptyHint = document.createElement("span");
      emptyHint.className = "console-window__empty";
      emptyHint.textContent = emptyHintText(kind);
      container.appendChild(emptyHint);
      body.appendChild(container);
      const pane = state.panes.get(kind);
      pane.container = container;
      pane.emptyHint = emptyHint;
    }

    root.appendChild(tabRow);
    root.appendChild(body);
    if (parent) {
      parent.appendChild(root);
    }
    state.root = root;
    state.tabRow = tabRow;

    // SPEC-2809 — request the current ring buffer so historical lines
    // surfaces immediately. Reply arrives as `process_console_snapshot`.
    if (typeof state.send === "function" && state.windowId) {
      state.send({ kind: "load_process_console", id: state.windowId });
    }
    return root;
  }

  function emptyHintText(kind) {
    return `Waiting for ${kind} process output...`;
  }

  function ingestSnapshot(lines) {
    if (!Array.isArray(lines)) return;
    for (const line of lines) {
      push(line);
    }
  }

  function activate(kind) {
    if (!state.panes.has(kind)) return;
    state.activeKind = kind;
    if (state.root) {
      state.root.dataset.activeKind = kind;
    }
    for (const [paneKind, pane] of state.panes.entries()) {
      if (pane.tab) {
        pane.tab.setAttribute(
          "aria-selected",
          paneKind === kind ? "true" : "false",
        );
      }
      if (pane.container) {
        pane.container.hidden = paneKind !== kind;
      }
    }
    const active = state.panes.get(kind);
    if (active && active.container && active.scrollFollow) {
      active.container.scrollTop = active.container.scrollHeight;
    }
  }

  function updateScrollFollow(kind) {
    const pane = state.panes.get(kind);
    if (!pane || !pane.container) return;
    const container = pane.container;
    const distanceFromBottom =
      container.scrollHeight - (container.scrollTop + container.clientHeight);
    pane.scrollFollow = distanceFromBottom <= FOLLOW_THRESHOLD_PX;
  }

  function push(line) {
    if (!line || typeof line !== "object") return;
    const kind = typeof line.kind === "string" ? line.kind : null;
    if (!kind || !state.panes.has(kind)) return;
    const pane = state.panes.get(kind);

    pane.lastSpawnId = line.spawn_id;
    pane.buffer.push(line);
    while (pane.buffer.length > CAP_PER_KIND) {
      pane.buffer.shift();
    }

    if (pane.container) {
      if (pane.emptyHint && pane.emptyHint.parentNode === pane.container) {
        pane.container.removeChild(pane.emptyHint);
        pane.emptyHint = null;
      }
      // The backend pushes a synthetic header line as the very first
      // entry of each spawn (the actual command string prefixed with
      // "$ "). Detect that prefix here and render it as a banner so the
      // user sees `$ git rev-parse --git-dir` instead of the previous
      // useless `$ spawn_id=N` marker.
      const message = String(line.message ?? "");
      const isHeader =
        message.startsWith("$ ") || message.startsWith("[") ||
        message.startsWith("→ ");
      if (isHeader) {
        appendHeaderNode(pane.container, message);
      } else {
        appendLineNode(pane.container, line);
      }
      if (pane.scrollFollow) {
        pane.container.scrollTop = pane.container.scrollHeight;
      }
      while (pane.container.childNodes.length > CAP_PER_KIND + 64) {
        pane.container.removeChild(pane.container.firstChild);
      }
    }
  }

  function appendHeaderNode(container, text) {
    const node = document.createElement("span");
    node.className = "console-window__invocation-header";
    node.textContent = text;
    container.appendChild(node);
    container.appendChild(document.createTextNode("\n"));
  }

  function appendLineNode(container, line) {
    const node = document.createElement("span");
    node.className =
      line.stream === "stderr"
        ? "console-window__line console-window__line--stderr"
        : "console-window__line console-window__line--stdout";
    node.textContent = line.message ?? "";
    container.appendChild(node);
    container.appendChild(document.createTextNode("\n"));
  }

  function close() {
    if (state.root && state.root.parentElement) {
      state.root.parentElement.removeChild(state.root);
    }
    state.root = null;
    state.tabRow = null;
    for (const pane of state.panes.values()) {
      pane.container = null;
      pane.tab = null;
    }
  }

  function isOpen() {
    return Boolean(state.root && state.root.parentElement);
  }

  return {
    mount,
    push,
    ingestSnapshot,
    activate,
    close,
    isOpen,
    // Exposed for tests; not part of the public WS contract.
    _state: state,
  };
}
