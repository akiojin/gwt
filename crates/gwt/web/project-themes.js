import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";

// Themes host (Issue 7643, SPEC 7007 T-025). Rendered by the React surface
// `ProjectThemes` (crates/gwt/web-react). A theme is a GitHub Issue that has
// sub-issues: the backend reads them (`load_themes` → `themes`) together with
// the number of Issues completed today, and this host only derives rows and
// keeps what the screen is doing: which theme is open, the delete
// confirmation, and the request it is waiting on.

const NOT_CONNECTED = "Not connected to gwt. Try again once the connection is back.";

// A theme whose every issue is completed is still open on GitHub: it only
// waits to be closed, so it reads as almost done.
function themeRows(view) {
  return (view?.themes || []).map(theme => {
    const total = Math.max(0, Number(theme.issues_total) || 0);
    const completed = Math.min(total, Math.max(0, Number(theme.issues_completed) || 0));
    return {
      number: theme.number,
      title: theme.title,
      summary: theme.summary || "",
      url: theme.url,
      total,
      completed,
      percent: total ? Math.round((completed / total) * 100) : 0,
      status: total > 0 && completed === total ? "almost_done" : "in_progress",
      issues: (theme.issues || []).map(issue => ({
        number: issue.number,
        title: issue.title,
        completed: issue.state === "closed",
        url: issue.url,
        inRepository: Boolean(issue.in_repository),
      })),
    };
  });
}

const INITIAL = Object.freeze({
  loaded: false, loading: false, error: null, themes: [], completedToday: 0, truncated: false,
  selected: null, dialog: null, deleting: false, deleteError: null, notice: null,
});

export function createProjectThemes({ document, send, repliesModel, onOpenIssue, scheduleFrame }) {
  const model = createUiStateStore(INITIAL, { scheduleFrame });
  let nextRequestId = 1;
  // The request this screen is waiting on; any other reply is stale.
  let pending = null;

  function request(message, waiting) {
    const requestId = nextRequestId++;
    if (send({ ...message, request_id: requestId }) === "unavailable") {
      pending = null;
      model.update(state => ({
        ...state, loading: false, deleting: false,
        ...(message.kind === "delete_theme" ? { deleteError: NOT_CONNECTED } : { error: NOT_CONNECTED }),
      }));
      return;
    }
    pending = { requestId, kind: message.kind, number: message.number ?? null };
    model.update(state => ({ ...state, ...waiting }));
  }
  function reload() {
    request({ kind: "load_themes" }, { loading: true, error: null, notice: null });
  }
  function applyReply(reply) {
    if (!reply || !pending || reply.request_id !== pending.requestId) return;
    const answered = pending;
    pending = null;
    if (reply.kind === "theme_error") {
      model.update(state => ({
        ...state, loading: false, deleting: false,
        ...(answered.kind === "delete_theme" ? { deleteError: reply.message } : { error: reply.message }),
      }));
      return;
    }
    const themes = themeRows(reply.view);
    const deleted = answered.kind === "delete_theme";
    model.update(state => ({
      ...state, loaded: true, loading: false, deleting: false, error: null, deleteError: null,
      themes,
      completedToday: Math.max(0, Number(reply.view?.completed_today) || 0),
      truncated: Boolean(reply.view?.truncated),
      dialog: deleted ? null : state.dialog,
      notice: deleted ? { deleted: answered.number } : null,
      selected: themes.some(theme => theme.number === state.selected) ? state.selected : null,
    }));
  }
  const release = repliesModel.subscribe(state => state.reply, applyReply);

  const actions = {
    reload,
    openTheme: number => model.update(state => ({ ...state, selected: number, notice: null })),
    closeTheme: () => model.update(state => ({ ...state, selected: null, dialog: null, deleteError: null })),
    openIssue: number => onOpenIssue(number),
    askDelete: number => model.update(state => ({ ...state, dialog: { number }, deleteError: null })),
    cancelDelete: () => model.update(state => (state.deleting ? state : { ...state, dialog: null, deleteError: null })),
    confirmDelete: () => {
      const number = model.read().dialog?.number;
      if (number == null || model.read().deleting) return;
      request({ kind: "delete_theme", number }, { deleting: true, deleteError: null });
    },
  };
  const container = createDetachedIslandContainer(document);
  const island = mountIsland("project-themes", container, { store: model, actions });
  function dispose() {
    release();
    island.unmount();
  }
  return { element: container, reload, dispose };
}
