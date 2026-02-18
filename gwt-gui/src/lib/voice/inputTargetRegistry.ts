const terminalTargets = new Map<string, HTMLElement>();

export function registerTerminalInputTarget(paneId: string, rootEl: HTMLElement): () => void {
  terminalTargets.set(paneId, rootEl);

  return () => {
    const current = terminalTargets.get(paneId);
    if (current === rootEl) {
      terminalTargets.delete(paneId);
    }
  };
}

export function getFocusedTerminalPaneId(doc: Document = document): string | null {
  const active = doc.activeElement;
  if (!active) return null;

  for (const [paneId, rootEl] of terminalTargets.entries()) {
    if (rootEl.contains(active)) {
      return paneId;
    }
  }

  return null;
}

export function clearTerminalInputTargetsForTests() {
  terminalTargets.clear();
}
