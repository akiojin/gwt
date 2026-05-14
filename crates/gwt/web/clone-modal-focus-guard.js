// Issue #2704 — terminal-focus guard for modal-friendly workspace renders.
//
// `renderWorkspace()` schedules a `terminal.focus()` cycle on every backend
// `workspace_state` event. When a modal (Clone Project, Launch Wizard,
// preset, branch cleanup, migration, kanban drawer) is open, or when a
// text input owns focus, that focus steal pulls the active element back to
// xterm.js's hidden textarea and the user's keystrokes never reach the
// input field. The Clone Project URL/Search input ended up unable to
// accept any typed character while the background agent terminal kept
// streaming output.
//
// This guard is a pure decision helper. The caller still runs the refresh
// + fit + sendGeometry work — only the trailing `terminal.focus()` is
// suppressed for one activation cycle when this returns true. The next
// `renderWorkspace()` re-evaluates the guard, so terminal focus is
// naturally restored as soon as the modal closes or the input is
// blurred.

const TEXT_INPUT_TAGS = new Set(["INPUT", "TEXTAREA"]);

function modalOwnsFocus(modalElements) {
  if (!Array.isArray(modalElements)) {
    return false;
  }
  for (const el of modalElements) {
    if (!el) continue;
    const classList = el.classList;
    if (
      classList &&
      typeof classList.contains === "function" &&
      classList.contains("open")
    ) {
      return true;
    }
  }
  return false;
}

function textInputOwnsFocus(doc) {
  const active = doc && doc.activeElement;
  if (!active) {
    return false;
  }
  if (TEXT_INPUT_TAGS.has(active.tagName)) {
    return true;
  }
  if (active.isContentEditable === true) {
    return true;
  }
  return false;
}

export function shouldSkipTerminalFocusActivation(options = {}) {
  const { doc, modalElements } = options;
  if (modalOwnsFocus(modalElements)) {
    return true;
  }
  if (textInputOwnsFocus(doc)) {
    return true;
  }
  return false;
}
