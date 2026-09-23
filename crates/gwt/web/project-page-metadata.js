// Metadata is a projection of the server aggregate, including off-screen windows.
export function createProjectPageMetadata({
  document: documentRef = globalThis.document,
  window: windowRef = globalThis.window,
  project = true,
  send = () => {},
} = {}) {
  let name = "Project";
  let aggregate = { running_count: 0, block_count: 0, error_count: 0, unread: false };
  let latestRevision = -1;
  function render() {
    if (!project) {
      documentRef.title = "gwt — Hub";
      try { documentRef.querySelector('link[rel="icon"]')?.setAttribute("href", "data:,"); } catch { /* optional favicon */ }
      return;
    }
    const { running_count: running = 0, block_count: blocked = 0, error_count: errors = 0, unread = false } = aggregate;
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
  function acknowledge() {
    const visible = documentRef.visibilityState === "visible";
    const focused = documentRef.hasFocus?.() === true;
    if (project && aggregate.unread && visible && focused) {
      send({ kind: "project_aggregate_ack", revision: aggregate.revision, visible, focused });
    }
  }
  function update(value) {
    if (!value || value.revision < latestRevision) return;
    latestRevision = value.revision;
    aggregate = value;
    render();
    acknowledge();
  }
  function resetConnection() {
    latestRevision = -1;
    aggregate = { ...aggregate, unread: false };
  }
  documentRef.addEventListener("visibilitychange", acknowledge);
  windowRef.addEventListener("focus", acknowledge);
  if (windowRef.MutationObserver) {
    new windowRef.MutationObserver(render).observe(documentRef.documentElement, {
      attributes: true, attributeFilter: ["data-theme"],
    });
  }
  render();
  return { setProjectName, update, resetConnection };
}
