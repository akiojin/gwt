function ensureChild(parent, selector, create) {
  const existing = parent.querySelector(selector);
  if (existing) {
    return existing;
  }
  const child = create(parent.ownerDocument);
  parent.appendChild(child);
  return child;
}

function createTabButton(document, send) {
  const button = document.createElement("div");
  button.className = "project-tab";
  button.setAttribute("role", "button");
  button.tabIndex = 0;

  const dot = document.createElement("span");
  dot.className = "project-tab-dot";
  dot.dataset.role = "project-tab-dot";
  dot.dataset.state = "";
  dot.setAttribute("aria-hidden", "true");
  button.appendChild(dot);

  const label = document.createElement("span");
  label.className = "project-tab-label";
  button.appendChild(label);

  const close = document.createElement("button");
  close.className = "project-tab-close";
  close.type = "button";
  close.textContent = "×";
  button.appendChild(close);

  button.addEventListener("click", () => {
    send({ kind: "select_project_tab", tab_id: button.dataset.projectTabId });
  });
  button.addEventListener("keydown", (event) => {
    if (event.key !== "Enter" && event.key !== " ") {
      return;
    }
    event.preventDefault();
    send({ kind: "select_project_tab", tab_id: button.dataset.projectTabId });
  });
  close.addEventListener("click", (event) => {
    event.stopPropagation();
    send({ kind: "close_project_tab", tab_id: button.dataset.projectTabId });
  });

  return button;
}

export function updateProjectTabDot(
  buttonEl,
  projectRoot,
  { indexStatusByProjectRoot, aggregateProjectTabDotState },
) {
  const dot = buttonEl.querySelector("[data-role='project-tab-dot']");
  if (!dot) {
    return;
  }
  const status =
    (projectRoot && indexStatusByProjectRoot.get(projectRoot)) || null;
  dot.dataset.state = aggregateProjectTabDotState(status);
}

export function renderProjectTabs({
  projectTabs,
  tabs,
  activeTabId,
  indexStatusByProjectRoot,
  aggregateProjectTabDotState,
  send,
}) {
  if (!projectTabs) {
    return;
  }
  const document = projectTabs.ownerDocument;
  const nextTabs = Array.isArray(tabs) ? tabs : [];
  const nextIds = new Set(nextTabs.map((tab) => tab.id));

  for (const button of projectTabs.querySelectorAll(
    ".project-tab[data-project-tab-id]",
  )) {
    if (!nextIds.has(button.dataset.projectTabId)) {
      button.remove();
    }
  }

  const existingButtons = new Map(
    Array.from(
      projectTabs.querySelectorAll(".project-tab[data-project-tab-id]"),
    ).map((button) => [button.dataset.projectTabId, button]),
  );

  nextTabs.forEach((tab, index) => {
    let button = existingButtons.get(tab.id);
    if (!button) {
      button = createTabButton(document, send);
    }

    const dot = ensureChild(button, "[data-role='project-tab-dot']", (doc) => {
      const element = doc.createElement("span");
      element.className = "project-tab-dot";
      element.dataset.role = "project-tab-dot";
      element.setAttribute("aria-hidden", "true");
      return element;
    });
    const label = ensureChild(button, ".project-tab-label", (doc) => {
      const element = doc.createElement("span");
      element.className = "project-tab-label";
      return element;
    });
    const close = ensureChild(button, ".project-tab-close", (doc) => {
      const element = doc.createElement("button");
      element.className = "project-tab-close";
      element.type = "button";
      element.textContent = "×";
      element.addEventListener("click", (event) => {
        event.stopPropagation();
        send({ kind: "close_project_tab", tab_id: button.dataset.projectTabId });
      });
      return element;
    });

    button.dataset.projectTabId = tab.id;
    button.dataset.projectRoot = tab.project_root || "";
    button.title = tab.project_root || "";
    button.setAttribute("role", "button");
    button.tabIndex = 0;
    button.classList.toggle("active", tab.id === activeTabId);
    if (tab.id === activeTabId) {
      button.setAttribute("aria-current", "page");
    } else {
      button.removeAttribute("aria-current");
    }

    label.textContent = tab.title || "";
    close.setAttribute("aria-label", `Close ${tab.title || "project"}`);
    close.title = `Close ${tab.title || "project"}`;
    dot.dataset.state = dot.dataset.state || "";
    updateProjectTabDot(button, tab.project_root, {
      indexStatusByProjectRoot,
      aggregateProjectTabDotState,
    });

    const current = projectTabs.children[index] || null;
    if (current !== button) {
      projectTabs.insertBefore(button, current);
    }
  });
}
