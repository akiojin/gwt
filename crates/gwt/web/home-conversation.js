import { createUiStateStore } from "./ui-state-store.js";
import { createDetachedIslandContainer, mountIsland } from "./react/islands.js";
import { decisionAnswerMessage, decisionItemFromReport, decisionMessageLines } from "./pm-chat.js";

// Home is the conversation with the Project Manager (Issue #7639, SPEC #7007
// T-021). Rendered by the React surface `HomeConversation`
// (crates/gwt/web-react); this host owns the conversation state.
//
// A PM runs per project, so Home gathers what the PM of every open project is
// waiting on into one timeline and sends each answer to the PM of the project
// it belongs to. Home shows no project work status and no project counts
// (design item 5): only what the PM says and what you answer.
//
// The decisions and the answers each go through exactly one function,
// `requestDecisions` and `sendToPm`, so their source can be replaced in one
// place.
//
// A topic is settled here once its answer is confirmed delivered. Nothing
// about a settled topic is stored: after a reload the decision reports are the
// only source again, the same rule the Decide queue follows (T-023).

// Every string Home shows that depends on its state.
const COPY = {
  pmName: "Project Manager",
  pmRole: "Your PM",
  waiting: count => `${count} decision${count === 1 ? "" : "s"} waiting`,
  noneWaiting: "No decisions waiting",
  loading: "Checking what needs you…",
  greeting: {
    noProjects: "Hello. There are no projects yet. Open your first project and I will tell you here when something needs you.",
    noneOpen: "No project is open right now. Open one from Projects and I will tell you here when something needs you.",
    waiting: count => `${count === 1 ? "One decision is" : `${count} decisions are`} waiting for you. `
      + "Answer with the buttons in each message, or reply from the box below. "
      + "Project details stay on each project's page.",
    clear: "Nothing needs a decision right now. Send me a message from the box below whenever you want something done.",
  },
  sending: "Sending…",
  deferred: "Kept waiting. Nothing was decided.",
  delivered: project => `Delivered to the Project Manager of ${project}.`,
  notConfirmed: "Delivery was not confirmed. Check the project before sending again.",
  disconnected: "Home is not connected. Reconnect, then send again.",
  refused: "The Project Manager did not accept the message.",
  loadFailed: (project, error) => `Decisions of ${project} could not be read: ${error}`,
};

const SEND_TIMEOUT_MS = 15_000;
const REFRESH_MS = 5_000;

const topicId = (projectKey, decisionId) => `${projectKey}:${decisionId}`;
const isoTime = ms => new Date(ms).toISOString();

const EMPTY_STATE = Object.freeze({
  catalogLoaded: false, hasProjects: false, loaded: false, appliedRequest: 0, projects: [],
  settled: [], hidden: [], deferred: [], sent: [], sentCount: 0,
  pending: null, failure: null, replyTo: null,
});

// What a decisions snapshot changes. It keeps the reports as delivered; the
// view below reads them.
function withDecisions(state, event) {
  if (!Number.isSafeInteger(event.request_id) || event.request_id <= state.appliedRequest) return state;
  const projects = (event.projects || []).map(project => ({
    key: project.project_key, title: project.title || project.project_key,
    pmAvailable: Boolean(project.pm_available), error: project.error || null,
    reports: project.decisions || [],
  }));
  const live = new Set(projects.flatMap(project => project.reports.map(report => topicId(project.key, report.id))));
  return {
    ...state, loaded: true, appliedRequest: event.request_id, projects,
    deferred: state.deferred.filter(id => live.has(id)),
    replyTo: live.has(state.replyTo) ? state.replyTo : null,
  };
}

// What the outcome of the message in flight changes. Only a confirmed delivery
// settles a topic; a refusal keeps it open and says why.
function withSendResult(state, requestId, ok, error) {
  const pending = state.pending;
  if (!pending || pending.requestId !== requestId) return state;
  if (!ok) {
    return { ...state, pending: null, failure: { topicId: pending.topicId, message: error || COPY.refused } };
  }
  const at = Date.now();
  const sentCount = state.sentCount + (pending.fromComposer ? 1 : 0);
  if (!pending.topicId) {
    return {
      ...state, pending: null, failure: null, sentCount,
      sent: [...state.sent, { id: `sent-${requestId}`, projectKey: pending.projectKey, project: pending.project, text: pending.answer, at }],
    };
  }
  return {
    ...state, pending: null, failure: null, sentCount,
    replyTo: state.replyTo === pending.topicId ? null : state.replyTo,
    deferred: state.deferred.filter(id => id !== pending.topicId),
    settled: [...state.settled, {
      id: pending.topicId, project: pending.project, subject: pending.subject,
      lines: pending.lines, answer: pending.answer, answeredAt: at,
    }],
  };
}

function decisionsOf(state) {
  return state.projects.flatMap(project => project.reports.map(report => {
    const item = decisionItemFromReport(report);
    return { id: topicId(project.key, item.id), project, item };
  }));
}

// The model the React surface renders, derived from the conversation state.
function homeConversationView(state) {
  const settled = new Set(state.settled.map(entry => entry.id));
  const projects = state.projects.map(project => ({
    key: project.key, title: project.title, pmAvailable: project.pmAvailable,
  }));
  const pending = state.pending;
  const waiting = decisionsOf(state)
    .filter(({ id }) => !settled.has(id) && !state.hidden.includes(id))
    .map(({ id, project, item }) => {
      const deferred = state.deferred.includes(id);
      return {
        id, kind: "decision", projectKey: project.key, project: project.title,
        subject: item.question, createdAt: item.createdAt, lines: decisionMessageLines(item),
        options: item.options || [], recommended: item.options ? item.options.indexOf(item.recommendation) : -1,
        deferred,
        status: pending?.topicId === id ? COPY.sending : deferred ? COPY.deferred : "",
        error: state.failure?.topicId === id ? state.failure.message : "",
      };
    });
  const messages = state.sent.map(message => ({
    id: message.id, kind: "message", projectKey: message.projectKey, project: message.project,
    createdAt: isoTime(message.at), lines: [message.text], status: COPY.delivered(message.project),
  }));
  // Oldest at the top, newest at the bottom (design item 230).
  const topics = [...waiting, ...messages]
    .sort((a, b) => String(a.createdAt ?? "").localeCompare(String(b.createdAt ?? "")));
  const count = state.loaded ? waiting.length : null;
  const greeting = !state.catalogLoaded || !state.loaded ? COPY.loading
    : !state.hasProjects ? COPY.greeting.noProjects
      : !projects.length ? COPY.greeting.noneOpen
        : count ? COPY.greeting.waiting(count) : COPY.greeting.clear;
  const replyTo = waiting.find(topic => topic.id === state.replyTo) ?? null;
  return {
    pm: { name: COPY.pmName, role: COPY.pmRole },
    // The entries that add a project are offered while there is none.
    guide: state.catalogLoaded && !state.hasProjects,
    noneOpen: state.catalogLoaded && state.loaded && state.hasProjects && !projects.length,
    waiting: count,
    waitingLabel: count ? COPY.waiting(count) : COPY.noneWaiting,
    // The first decision down the timeline that has not been put off.
    firstWaitingId: (topics.find(topic => topic.kind === "decision" && !topic.deferred)
      ?? topics.find(topic => topic.kind === "decision"))?.id ?? null,
    greeting,
    notices: state.projects.filter(project => project.error)
      .map(project => COPY.loadFailed(project.title, project.error)),
    resolved: state.settled.filter(entry => !state.hidden.includes(entry.id))
      .map(entry => ({ ...entry, answeredAt: isoTime(entry.answeredAt) })),
    topics,
    busy: Boolean(pending),
    composer: {
      replyTo: replyTo && { id: replyTo.id, subject: replyTo.subject, projectKey: replyTo.projectKey, project: replyTo.project },
      projects,
      status: pending?.fromComposer ? COPY.sending : "",
      error: state.failure && !state.failure.topicId ? state.failure.message : "",
      // Counts the composer messages confirmed delivered, so the surface
      // clears exactly the draft that was sent.
      sentCount: state.sentCount,
    },
  };
}

export function createHomeConversation({
  document, window: win, send, onWaitingChange = () => {}, onOpenFolder = () => {}, onClone = () => {},
  onOpenProjects = () => {}, scheduleFrame,
}) {
  const state = createUiStateStore(EMPTY_STATE);
  const model = createUiStateStore(homeConversationView(EMPTY_STATE), { scheduleFrame });
  let requestSequence = 0;
  let sendTimer = null;
  let refreshTimer = null;

  // The one read of the decisions Home shows.
  function requestDecisions() {
    send({ kind: "load_home_conversation", request_id: ++requestSequence });
  }

  // The one send of a message to the PM of a project. The pane input path
  // pastes the body and submits it on its own carriage return.
  function sendToPm(pending) {
    if (state.read().pending) return false;
    const requestId = ++requestSequence;
    const delivery = send({ kind: "home_pm_send", project_key: pending.projectKey, text: `${pending.message}\r`, request_id: requestId });
    if (delivery !== "sent") {
      state.update(current => ({ ...current, failure: { topicId: pending.topicId, message: COPY.disconnected } }));
      return false;
    }
    state.update(current => ({ ...current, pending: { ...pending, requestId }, failure: null }));
    return true;
  }

  function decision(id) {
    return decisionsOf(state.read()).find(candidate => candidate.id === id) ?? null;
  }

  function answer(id, text, optionIndex, fromComposer) {
    const found = decision(id);
    if (!found || state.read().settled.some(entry => entry.id === id)) return false;
    return sendToPm({
      topicId: id, projectKey: found.project.key, project: found.project.title, subject: found.item.question,
      lines: decisionMessageLines(found.item), answer: text, fromComposer,
      message: decisionAnswerMessage(found.item, text, optionIndex),
    });
  }

  const actions = {
    answer(id, optionIndex) {
      const option = decision(id)?.item.options?.[optionIndex];
      if (option != null) answer(id, option, optionIndex, false);
    },
    // Deferring is not an answer: the decision keeps waiting and keeps counting.
    defer(id) {
      if (state.read().pending || !decision(id)) return;
      state.update(current => ({ ...current, deferred: [...current.deferred.filter(other => other !== id), id], failure: null }));
    },
    reply(id) {
      if (decision(id)) state.update(current => ({ ...current, replyTo: id }));
    },
    clearReply() {
      state.update(current => ({ ...current, replyTo: null }));
    },
    // From the composer: an answer to the reply target, or a new message to
    // the PM of the chosen project.
    submit(text, projectKey) {
      const body = String(text ?? "").trim();
      const current = state.read();
      if (!body || current.pending) return false;
      if (current.replyTo && decision(current.replyTo)) return answer(current.replyTo, body, null, true);
      const project = current.projects.find(candidate => candidate.key === projectKey);
      if (!project) return false;
      return sendToPm({ topicId: null, projectKey: project.key, project: project.title, answer: body, message: body, fromComposer: true });
    },
    deleteTopic(id) {
      state.update(current => ({ ...current, hidden: [...current.hidden.filter(other => other !== id), id] }));
    },
    openFolder: () => onOpenFolder(),
    importFromGitHub: () => onClone("search"),
    clone: () => onClone("url"),
    openProjects: () => onOpenProjects(),
  };

  // A receive only commits state; everything that follows from it is below.
  function receive(event) {
    if (event?.kind === "home_conversation") {
      state.update(current => withDecisions(current, event));
      return true;
    }
    if (event?.kind === "pane_send_result") {
      state.update(current => withSendResult(current, event.request_id, Boolean(event.ok), event.error));
      return true;
    }
    return false;
  }

  state.subscribe(current => current, current => model.update(() => homeConversationView(current)));
  model.subscribe(view => view.waiting, waiting => onWaitingChange(waiting));
  // A message in flight is given a bounded time to be confirmed; once it is
  // settled either way, the decisions are read again.
  state.subscribe(current => current.pending?.requestId ?? null, requestId => {
    if (sendTimer) win.clearTimeout(sendTimer);
    sendTimer = null;
    if (requestId === null) {
      if (state.read().loaded) requestDecisions();
      return;
    }
    sendTimer = win.setTimeout(() => {
      sendTimer = null;
      state.update(current => withSendResult(current, requestId, false, COPY.notConfirmed));
    }, SEND_TIMEOUT_MS);
  });

  // The catalog says whether there is any project at all; a change to the
  // open projects is a reason to read their decisions again.
  function receiveCatalog({ open, recent }) {
    state.update(current => ({ ...current, catalogLoaded: true, hasProjects: open + recent > 0 }));
    requestDecisions();
  }

  // The catalog that follows a connection starts the first read.
  function connected() {
    if (!refreshTimer) refreshTimer = win.setInterval(requestDecisions, REFRESH_MS);
  }

  function connectionLost() {
    if (refreshTimer) win.clearInterval(refreshTimer);
    refreshTimer = null;
    const pending = state.read().pending;
    if (pending) state.update(current => withSendResult(current, pending.requestId, false, COPY.notConfirmed));
  }

  const container = createDetachedIslandContainer(document);
  const island = mountIsland("home-conversation", container, { store: model, actions });

  function dispose() {
    connectionLost();
    island.unmount();
  }

  return { element: container, receive, receiveCatalog, connected, connectionLost, dispose, actions };
}
