import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";
import { agentAvatar } from "./agent-avatar.js";

// Issue #7740 (SPEC #7007 slice F-1): the header and footer every page shares.
// Rendered by the React surfaces in `AppChrome.jsx` (crates/gwt/web-react);
// this host owns the one definition of the header's destinations.
//
// A later slice puts a screen in the header by adding one row to
// HEADER_DESTINATIONS (or HEADER_MENU) and passing a handler with the same id
// from the page that owns the screen. Nothing else changes: the order is the
// definition's order, and the header stays one sideways-scrolling row.
//
// A destination with no handler is never hidden. It stays in place, disabled,
// and says why (design item 80).

// Every string the shared chrome shows.
const COPY = {
  brand: "gwt",
  navLabel: "App",
  pm: {
    title: "Project Manager",
    waiting: count => `${count} decision${count === 1 ? "" : "s"} waiting`,
    none: "no decisions waiting",
  },
  notifications: {
    unread: (count, hasError) => `Notifications (${count} unread${hasError ? ", includes errors" : ""})`,
  },
  destinations: {
    pm: { label: "PM", title: "Project Manager" },
    notifications: { label: "Notifications", title: "Notifications" },
    projects: { label: "Projects", title: "Projects" },
    agents: { label: "Agents", title: "Agent ledger: members, their order and Project Manager" },
    environments: { label: "Environments", title: "Execution environments: accounts, usage and limits" },
    menu: { label: "Menu ▾", title: "Menu" },
    settings: { label: "Settings", title: "Settings: agents, concurrency limit, and theme" },
  },
  unavailable: {
    needsProject: "Open a project to use this.",
    prefix: "Unavailable",
  },
  footer: { pm: "PM", pmUnknown: "—" },
};

// The tab row, in display order. `surface` marks the entry while that
// surface is showing.
const HEADER_DESTINATIONS = Object.freeze([
  // Home is the conversation with the PM (Issue #7639).
  { id: "pm", kind: "pm", surface: "home" },
  { id: "notifications", kind: "notifications" },
  { id: "projects", surface: "projects" },
  { id: "agents", surface: "agents" },
  { id: "environments", surface: "environments" },
  { id: "menu", kind: "menu" },
]);

// Entries of the header menu. Settings lives here, not in the tab row
// (design item 155).
const HEADER_MENU = Object.freeze([
  { id: "settings", surface: "settings" },
]);

const BADGE_MAX = 99;

function countOf(value) {
  return Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

// The model the React surfaces render: every label, title and reason is
// decided here, from the definition, the page's handlers and the live state.
function chromeView(state, handlers) {
  const entry = destination => {
    const copy = COPY.destinations[destination.id];
    const reason = destination.kind === "menu" || typeof handlers[destination.id] === "function"
      ? "" : COPY.unavailable.needsProject;
    return {
      id: destination.id,
      kind: destination.kind || "screen",
      label: copy.label,
      title: reason || copy.title,
      ariaLabel: reason ? `${copy.label}. ${COPY.unavailable.prefix}: ${reason}` : undefined,
      reason,
      current: Boolean(destination.surface) && destination.surface === state.surface,
    };
  };
  const pmName = String(state.pmName || "").trim();
  const entries = HEADER_DESTINATIONS.map(destination => {
    const view = entry(destination);
    if (destination.kind === "pm") {
      const waiting = countOf(state.decisions);
      view.label = pmName || view.label;
      const avatar = agentAvatar({ name: view.label, avatar: state.pmAvatar });
      view.avatar = avatar.glyph;
      if (avatar.imageUrl) view.avatarImage = avatar.imageUrl;
      view.count = waiting;
      if (!view.reason) {
        view.title = `${COPY.pm.title}${pmName ? ` ${pmName}` : ""}: ${waiting ? COPY.pm.waiting(waiting) : COPY.pm.none}`;
        view.ariaLabel = view.title;
      }
    } else if (destination.kind === "notifications") {
      const unread = countOf(state.unread);
      // The notification drawer and its tests address the entry by this id.
      view.elementId = "op-notifications-button";
      view.controls = "notification-center";
      view.expanded = Boolean(state.notificationsOpen);
      view.unread = String(unread);
      view.hasError = Boolean(state.unreadHasError);
      view.count = unread > BADGE_MAX ? `${BADGE_MAX}+` : unread;
      if (!view.reason && unread) {
        view.title = COPY.notifications.unread(unread, view.hasError);
        view.ariaLabel = view.title;
      }
    } else if (destination.kind === "menu") {
      view.items = HEADER_MENU.map(entry);
    }
    return view;
  });
  return {
    brand: COPY.brand,
    navLabel: COPY.navLabel,
    entries,
    footer: { version: state.version, pm: `${COPY.footer.pm}: ${pmName || COPY.footer.pmUnknown}` },
  };
}

// `handlers` maps a destination id to the function that opens its screen.
// A page without a project passes none, which disables every such entry.
export function createAppChrome({ document, handlers = {} }) {
  let state = {
    pmName: "", pmAvatar: null, decisions: 0, unread: 0, unreadHasError: false, notificationsOpen: false, surface: null, version: "",
  };
  const model = createUiStateStore(chromeView(state, handlers));
  const set = patch => {
    state = { ...state, ...patch };
    model.update(() => chromeView(state, handlers));
  };
  const mount = (name, container) => {
    mountIsland(name, container, { store: model, onChoose: id => handlers[id]?.() });
    return container;
  };
  return {
    // A page that ships the header and footer skeleton in its HTML mounts
    // only the parts that change.
    mountNav: container => mount("app-header-nav", container),
    mountFooterPm: container => mount("app-footer-pm", container),
    // Any other page gets the whole header and footer to place around its content.
    mountPage: () => ({
      header: mount("app-header", createDetachedIslandContainer(document)),
      footer: mount("app-footer", createDetachedIslandContainer(document)),
    }),
    setPm: ({ name = "", decisions = 0, avatar = null } = {}) => set({ pmName: name, decisions, pmAvatar: avatar }),
    setNotifications: ({ count = 0, hasError = false, open = false } = {}) =>
      set({ unread: count, unreadHasError: hasError, notificationsOpen: open }),
    setActiveSurface: surface => set({ surface }),
    setVersion: version => set({ version: version || "" }),
  };
}
