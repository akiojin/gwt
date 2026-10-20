import { createUiStateStore } from "./ui-state-store.js";

// Persistent project-state load diagnostics, cleared only after a successful retry.
export function createWorkspaceStateNotice({ document, send }) {
  const model = createUiStateStore(null);
  let banner = null;
  function render(notice) {
    banner?.remove();
    banner = null;
    if (!notice) return;
    banner = document.createElement("div");
    banner.className = "workspace-state-notice";
    banner.setAttribute("role", "alert");
    const detail = document.createElement("span");
    detail.textContent = `${notice.path}: ${notice.message}`;
    banner.appendChild(detail);
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "Retry";
    retry.addEventListener("click", () => send({ kind: "retry_workspace_state_load" }));
    banner.appendChild(retry);
    document.getElementById("app").appendChild(banner);
  }
  model.subscribe(notice => notice, render);
  function receive(notice) {
    model.update(() => notice);
  }
  return { receive };
}
