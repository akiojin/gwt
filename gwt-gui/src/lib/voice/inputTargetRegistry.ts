const terminalTargets = new Map<string, HTMLElement>();
const inputFieldTargets = new Map<string, HTMLTextAreaElement>();

export function registerTerminalInputTarget(paneId: string, rootEl: HTMLElement): () => void {
  terminalTargets.set(paneId, rootEl);

  return () => {
    const current = terminalTargets.get(paneId);
    if (current === rootEl) {
      terminalTargets.delete(paneId);
    }
  };
}

/**
 * Register an input field textarea as a voice input target for a pane.
 * When registered, voice transcripts are inserted into the textarea instead of PTY.
 */
export function registerInputFieldTarget(paneId: string, textarea: HTMLTextAreaElement): () => void {
  inputFieldTargets.set(paneId, textarea);

  return () => {
    const current = inputFieldTargets.get(paneId);
    if (current === textarea) {
      inputFieldTargets.delete(paneId);
    }
  };
}

/**
 * Get the input field textarea for a pane, if registered.
 */
export function getInputFieldTarget(paneId: string): HTMLTextAreaElement | null {
  return inputFieldTargets.get(paneId) ?? null;
}

export function getFocusedTerminalPaneId(doc: Document = document): string | null {
  const active = doc.activeElement;
  if (!active) return null;

  // Check input field targets first (they take priority for voice input)
  for (const [paneId, textarea] of inputFieldTargets.entries()) {
    if (textarea === active || textarea.contains(active as Node)) {
      return paneId;
    }
  }

  for (const [paneId, rootEl] of terminalTargets.entries()) {
    if (rootEl.contains(active)) {
      return paneId;
    }
  }

  return null;
}

export function clearTerminalInputTargetsForTests() {
  terminalTargets.clear();
  inputFieldTargets.clear();
}
