// Persistent project-state load diagnostics, cleared only after a successful retry.
export function createWorkspaceStateNotice({ document, send }) {
  let banner = null;
  function receive(notice) {
    banner?.remove();
    banner = null;
    if (!notice) return;
    banner = document.createElement("div");
    banner.className = "workspace-state-notice";
    const failed = notice.kind === "load_error";
    banner.setAttribute("role", failed ? "alert" : "status");
    const detail = document.createElement("span");
    detail.textContent = `${notice.path}: ${notice.message}`;
    banner.appendChild(detail);
    if (failed) {
      const retry = document.createElement("button");
      retry.type = "button";
      retry.textContent = "Retry";
      retry.addEventListener("click", () => send({ kind: "retry_workspace_state_load" }));
      banner.appendChild(retry);
    }
    document.getElementById("app").appendChild(banner);
  }
  return { receive };
}
