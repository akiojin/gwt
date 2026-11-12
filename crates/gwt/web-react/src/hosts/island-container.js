// Own the imperative mount boundary between the legacy shell and a React root.
export function createIslandContainer(parent) {
  const container = parent.ownerDocument.createElement("div");
  container.className = "react-island";
  parent.appendChild(container);
  return container;
}

// Create an island container the caller attaches itself: some shells order
// their surfaces only after every surface has been created.
export function createDetachedIslandContainer(document) {
  const container = document.createElement("div");
  container.className = "react-island";
  return container;
}
