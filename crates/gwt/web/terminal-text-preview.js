// Issue #4777 T-4: mirror the parsed screen without moving its live terminal.
export function createTerminalTextPreview({ document, terminal, container, tailLines = null }) {
  const preview = document.createElement("pre");
  preview.className = "terminal-text-preview";
  preview.setAttribute("aria-label", "Read-only terminal preview");
  container.appendChild(preview);

  function render() {
    const buffer = terminal.buffer.active;
    const lines = [];
    for (let row = 0; row < terminal.rows; row += 1) {
      lines.push(buffer.getLine(buffer.baseY + row)?.translateToString(true) ?? "");
    }
    if (tailLines !== null) {
      while (lines.length && !lines.at(-1).trim()) lines.pop();
      preview.textContent = lines.slice(-tailLines).join("\n");
    } else preview.textContent = lines.join("\n");
  }

  render();
  const subscription = terminal.onWriteParsed(render);
  const resize = terminal.onResize(render);
  return () => {
    subscription.dispose();
    resize.dispose();
    preview.remove();
  };
}
