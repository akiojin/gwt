// Own the imperative mount boundary between the legacy shell and a React root.
export function createIslandContainer(parent) {
  const container = parent.ownerDocument.createElement("div");
  container.className = "react-island";
  parent.appendChild(container);
  return container;
}
