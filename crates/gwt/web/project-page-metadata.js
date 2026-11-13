import { createUiStateStore } from "./ui-state-store.js";

const count = (value) => (Number.isInteger(value) && value >= 0 ? value : 0);

// SPEC 5016 AC-HS1: the single projection of the server aggregate. The tab
// title, the status strip and the Works heading read only this value; no
// surface recounts agents from its own input.
export function projectAgentSummary(aggregate = {}) {
  return Object.freeze({
    running: count(aggregate?.running_count),
    block: count(aggregate?.block_count),
    idle: count(aggregate?.idle_count),
    error: count(aggregate?.error_count),
  });
}

// Metadata is a projection of the server aggregate, including off-screen windows.
export function createProjectPageMetadata({
  document: documentRef = globalThis.document,
  window: windowRef = globalThis.window,
  project = true,
  send = () => {},
  scheduleFrame,
} = {}) {
  let name = "Project";
  const model = createUiStateStore({ running_count: 0, block_count: 0, idle_count: 0, error_count: 0, unread: false }, { scheduleFrame });
  const summary = createUiStateStore(projectAgentSummary(model.read()), { scheduleFrame });
  let latestRevision = -1;
  let attentionSyncPending = false;
  function render() {
    if (!project) {
      documentRef.title = "gwt — Hub";
      try { documentRef.querySelector('link[rel="icon"]')?.setAttribute("href", "data:,"); } catch { /* optional favicon */ }
      return;
    }
    const { unread = false } = model.read();
    const { running, block: blocked, error: errors } = summary.read();
    documentRef.title = `${unread ? "● " : ""}${name} — RUN ${running} · BLOCK ${blocked} — gwt`;
    try {
      let icon = documentRef.querySelector('link[rel="icon"]');
      if (!icon) {
        icon = documentRef.createElement("link");
        icon.rel = "icon";
        documentRef.head.append(icon);
      }
      const state = errors > 0 ? "error" : blocked > 0 ? "blocked" : running > 0 ? "running" : "idle";
      const token = { error: "--color-state-blocked", blocked: "--color-state-needs-input", running: "--color-state-active", idle: "--color-state-idle" }[state];
      const style = windowRef.getComputedStyle(documentRef.documentElement);
      const color = style.getPropertyValue(token).trim() || "currentColor";
      const foreground = style.getPropertyValue("--color-text-strong").trim() || "currentColor";
      const escape = (value) => value.replaceAll("&", "&amp;").replaceAll('"', "&quot;").replaceAll("<", "&lt;");
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect x="2" y="2" width="28" height="28" rx="7" fill="${escape(color)}"/>${unread ? `<circle cx="25" cy="7" r="6" fill="${escape(foreground)}"/>` : ""}</svg>`;
      icon.type = "image/svg+xml";
      icon.dataset.state = state;
      icon.dataset.unread = String(unread);
      icon.setAttribute("href", `data:image/svg+xml,${encodeURIComponent(svg)}`);
    } catch { /* Favicon support is optional; title and acknowledgement continue. */ }
  }
  function setProjectName(value) { name = String(value || "Project"); render(); }
  function reportAttention() {
    if (!project) return;
    send({
      kind: "project_client_attention",
      visible: documentRef.visibilityState === "visible",
      focused: documentRef.hasFocus?.() === true,
      notification_permission: windowRef.Notification?.permission || "unsupported",
    });
  }
  function acknowledge() {
    const aggregate = model.read();
    const visible = documentRef.visibilityState === "visible";
    const focused = documentRef.hasFocus?.() === true;
    if (project && latestRevision >= 0 && aggregate.unread && visible && focused) {
      send({ kind: "project_aggregate_ack", revision: aggregate.revision, visible, focused });
    }
  }
  function update(value) {
    if (!value || value.revision < latestRevision) return;
    latestRevision = value.revision;
    model.update(() => value);
    const next = projectAgentSummary(value);
    summary.update(previous => (["running", "block", "idle", "error"].every(key => previous[key] === next[key]) ? previous : next));
  }
  function resetConnection() {
    latestRevision = -1;
    // Retain the server-owned display snapshot; latestRevision fences old ACKs.
    // A Recent Project may materialize after the initial socket-open report.
    attentionSyncPending = true;
  }
  function attentionChanged() {
    reportAttention();
    acknowledge();
  }
  documentRef.addEventListener("visibilitychange", attentionChanged);
  windowRef.addEventListener("focus", attentionChanged);
  windowRef.addEventListener("blur", reportAttention);
  try {
    if (project && windowRef.navigator?.permissions?.query) {
      windowRef.navigator.permissions.query({ name: "notifications" })
        .then((status) => status.addEventListener("change", reportAttention))
        .catch(() => {});
    }
  } catch { /* Permission observation is optional and must never prompt. */ }
  if (windowRef.MutationObserver) {
    new windowRef.MutationObserver(render).observe(documentRef.documentElement, {
      attributes: true, attributeFilter: ["data-theme"],
    });
  }
  model.subscribeFrame(aggregate => {
    const { running, block, error } = projectAgentSummary(aggregate);
    return JSON.stringify([Boolean(aggregate.unread), running, block, error > 0]);
  }, render);
  model.subscribe(aggregate => aggregate, () => {
    if (attentionSyncPending) {
      attentionSyncPending = false;
      reportAttention();
    }
    acknowledge();
  });
  return { setProjectName, update, resetConnection, reportAttention, summary };
}
