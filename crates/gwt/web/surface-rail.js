// Issue #4777 T-1 — the rail picks one of four surfaces.
//
// Issues / Agents / Board / Settings are the only places the user can be.
// Every known window preset folds into one of them, including restored legacy
// views, so the rail can report which surface the focused window
// belongs to without the host keeping a second selection state.

const SURFACE_BY_PRESET = Object.freeze({
  issue: "issues",
  issue_monitor: "issues",
  agent_kanban: "issues",
  spec: "issues",
  index: "issues",
  work: "issues",
  workspace: "issues",
  branches: "issues",
  pr: "issues",
  agent: "agents",
  claude: "agents",
  codex: "agents",
  shell: "agents",
  board: "board",
  memo: "board",
  settings: "settings",
  profile: "settings",
  logs: "settings",
  console: "settings",
  file_tree: "settings",
});

export function surfaceForPreset(preset) {
  if (typeof preset !== "string") {
    return null;
  }
  return SURFACE_BY_PRESET[preset] || null;
}

// The PM is the user's own window, not one of the agents the app runs. The
// backend marks it from the pane role (`is_pm`), so the rail never guesses
// from a title or a worktree path.
export function surfaceForWindow(windowData) {
  if (windowData?.is_pm) {
    return "pm";
  }
  if (["agent", "claude", "codex"].includes(windowData?.preset)) return "agents";
  if (windowData?.preset === "settings" && windowData.surface_owner === "pm") return "pm";
  if (windowData?.preset === "shell" && windowData.surface_owner === "agents") return "agents";
  if (windowData?.preset === "settings" && windowData.settingsSection === "issues") return "issues";
  return surfaceForPreset(windowData?.preset);
}

export function applySurfaceSelection(document, surface) {
  for (const item of document.querySelectorAll(".op-rail [data-surface]")) {
    item.setAttribute("aria-pressed", item.dataset.surface === surface ? "true" : "false");
  }
}

// The PM entry keeps its own launcher wiring (it starts a missing PM), so
// only the four surfaces are routed through `openSurface`.
export function installSurfaceRail(document, { openSurface }) {
  for (const item of document.querySelectorAll(".op-rail__surface[data-surface]")) {
    item.addEventListener("click", () => openSurface(item.dataset.surface));
  }
}
