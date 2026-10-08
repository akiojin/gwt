// Issue #4777 T-1 — the rail picks one of four surfaces.
//
// Issues / Agents / Board / Settings are the only places the user can be.
// Every window preset folds into one of them (or into none, for presets the
// SPEC retires), so the rail can report which surface the focused window
// belongs to without the host keeping a second selection state.

const SURFACE_BY_PRESET = Object.freeze({
  issue: "issues",
  issue_monitor: "issues",
  agent_kanban: "issues",
  spec: "issues",
  index: "issues",
  work: "issues",
  workspace: "issues",
  agent: "agents",
  claude: "agents",
  codex: "agents",
  board: "board",
  settings: "settings",
  branches: "settings",
  profile: "settings",
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
