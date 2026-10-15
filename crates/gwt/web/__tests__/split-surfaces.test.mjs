import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");

test("the title bar offers a reversible split and the split uses Operator tokens", () => {
  const { document } = parseHTML(html);
  const button = document.querySelector(".project-bar #split-view-button");
  assert.ok(button, "title bar must offer Split view");
  assert.equal(button.getAttribute("aria-pressed"), "false");
  assert.ok(document.querySelector(".canvas-area #split-surfaces[hidden]"));
  const css = readFileSync(new URL("../styles/components.css", import.meta.url), "utf8");
  assert.match(css, /\.split-pane\[data-active="true"\][\s\S]*?var\(--color-focus-ring\)/);
  assert.match(css, /\.split-pane \.resize-handle\s*\{\s*display: none !important;/);
  assert.match(css, /\.split-pane\[hidden\][\s\S]*?display: none !important;/);
  assert.match(css, /\.split-surfaces\[data-split="false"\] \.split-pane__header/);
});

async function fixture() {
  const { createUiStateStore } = await import("../ui-state-store.js");
  const model = createUiStateStore({ catalog: { version: 0 } });
  const { createSplitSurfaces } = await import("../split-surfaces.js");
  const { document, window } = parseHTML(html);
  const stage = document.getElementById("canvas-stage");
  const windows = [
    { id: "issues", preset: "settings" },
    { id: "board", preset: "board" },
    { id: "settings", preset: "settings" },
    { id: "a", preset: "agent" },
    { id: "b", preset: "codex" },
    { id: "pm", preset: "claude", is_pm: true },
  ];
  const elements = new Map(windows.map((data) => {
    const element = document.createElement("div");
    element.className = "workspace-window";
    element.dataset.id = data.id;
    element.style.left = "120px";
    element.style.width = "600px";
    stage.appendChild(element);
    return [data.id, element];
  }));
  const opened = [];
  const agentHosts = [];
  const agents = document.createElement("section");
  const controller = createSplitSurfaces({
    document,
    stage,
    windowCatalogModel: model,
    onAgentsHost: (host, previewHost) => {
      agentHosts.push([host, previewHost]);
      if (host) host.appendChild(agents); else agents.remove();
    },
    getWindows: () => windows,
    getElement: (id) => elements.get(id),
    openSurface: (surface) => opened.push(surface),
    onChange: () => {},
    onLayout: () => {},
  });
  return { controller, model, document, window, stage, windows, elements, opened, agents, agentHosts };
}

test("two panes select independently and release restores canvas geometry", async () => {
  const { controller, document, stage, elements } = await fixture();
  controller.open("issues");
  const panes = document.querySelectorAll(".split-pane");
  assert.equal(panes.length, 2);
  assert.equal(panes[0].dataset.surface, "issues");
  assert.equal(panes[1].dataset.surface, "board");
  assert.equal(elements.get("issues").closest(".split-pane"), panes[0]);
  controller.select("settings", 1);
  assert.equal(panes[0].dataset.surface, "issues");
  assert.equal(panes[1].dataset.surface, "settings");
  assert.equal(elements.get("board").parentElement, stage);
  assert.equal(controller.activeSurface(), "settings");
  assert.equal(controller.select("issues", 1), true, "both panes may select the same surface");
  assert.equal(elements.get("issues").closest(".split-pane"), panes[0], "the existing view stays in its pane");
  assert.equal(panes[1].querySelector("option[value='issues']").disabled, false);
  controller.focusWindow("issues");
  assert.equal(panes[0].dataset.active, "true");
  controller.close();
  assert.equal(controller.isOpen(), false);
  for (const element of elements.values()) {
    assert.equal(element.parentElement, stage);
    assert.equal(element.style.left, "120px");
    assert.equal(element.style.width, "600px");
  }
});

test("same non-Agent surfaces use distinct live windows and request only the missing view", async () => {
  for (const surface of ["issues", "board", "settings"]) {
    const { controller, document, windows, elements, stage, opened } = await fixture();
    if (surface === "issues" || surface === "settings") {
      windows.splice(windows.findIndex(w => w.id === (surface === "issues" ? "settings" : "issues")), 1);
    }
    controller.open(surface);
    assert.equal(controller.select(surface, 1), true);
    controller.sync();
    assert.deepEqual(opened, [surface]);
    const second = { id: `${surface}-second`, preset: surface === "issues" ? "settings" : surface };
    const element = document.createElement("div");
    stage.appendChild(element);
    elements.set(second.id, element);
    windows.push(second);
    controller.sync();
    const panes = document.querySelectorAll(".split-pane");
    assert.equal(elements.get(surface).closest(".split-pane"), panes[0]);
    assert.equal(element.closest(".split-pane"), panes[1]);
    controller.close();
    assert.equal(element.parentElement, stage);
    assert.equal(elements.get(surface).parentElement, stage);
  }
});

test("same Agents surface keeps the active live host and a preview in the other pane", async () => {
  const { controller, document, window, agents, agentHosts } = await fixture();
  controller.open("agents");
  assert.equal(controller.select("agents", 1), true);
  const panes = document.querySelectorAll(".split-pane");
  assert.equal(agents.closest(".split-pane"), panes[1]);
  assert.equal(agentHosts.at(-1)[1].closest(".split-pane"), panes[0]);
  panes[0].querySelector("select").dispatchEvent(new window.Event("focusin", { bubbles: true }));
  assert.equal(agents.closest(".split-pane"), panes[0]);
  controller.sync();
  assert.equal(agents.closest(".split-pane"), panes[0], "metadata sync preserves active ownership");
  controller.focusSurface("agents");
  assert.equal(agents.closest(".split-pane"), panes[0]);
  controller.select("board", 0);
  assert.equal(agents.closest(".split-pane"), panes[1]);
  assert.equal(agentHosts.at(-1)[1], null);
  controller.close();
  assert.equal(agentHosts.at(-1)[0], null);
  assert.equal(agentHosts.at(-1)[1], null);
});

test("Split reconciles arriving views from catalog commits and unsubscribes", async () => {
  const { controller, model, windows, elements, stage, document } = await fixture();
  const settings = document.createElement("select");
  settings.innerHTML = '<option value="Normal" disabled>Normal</option><option value="Continue" selected>Continue</option>';
  elements.get("board").appendChild(settings);
  controller.open("board");
  controller.select("board", 1);
  const second = { id: "second-board", preset: "board" };
  const element = document.createElement("div");
  stage.appendChild(element);
  elements.set(second.id, element);
  windows.push(second);
  model.update(() => ({ catalog: { version: 1 } }));
  assert.equal(settings.value, "Continue", "host sync must preserve embedded settings choices");
  assert.equal(settings.options[0].hasAttribute("disabled"), true);
  assert.equal(settings.options[1].textContent, "Continue");
  assert.ok(element.closest(".split-pane") === document.querySelectorAll(".split-pane")[1], "catalog commit mounts the arriving view without manual sync");
  assert.equal(document.getElementById("split-surfaces").dataset.catalogVersion, "1");
  controller.dispose();
  windows.splice(windows.indexOf(second), 1);
  model.update(() => ({ catalog: { version: 2 } }));
  assert.equal(document.getElementById("split-surfaces").dataset.catalogVersion, "1");
});

test("Agents hosts its dedicated grid and releases it when switching surfaces", async () => {
  const { controller, elements, agents, stage } = await fixture();
  controller.open("agents");
  assert.equal(agents.closest(".split-pane").dataset.surface, "agents");
  assert.equal(elements.get("a").parentElement, stage);
  assert.equal(elements.get("pm").parentElement, stage);
  controller.select("settings");
  assert.equal(agents.parentElement, null);
});

test("a missing surface requests the existing launcher once and mounts the arriving window", async () => {
  const { controller, windows, opened } = await fixture();
  windows.splice(windows.findIndex((w) => w.id === "settings"), 1);
  windows.splice(windows.findIndex((w) => w.id === "issues"), 1);
  controller.open("agents");
  controller.select("settings", 1);
  controller.sync();
  controller.sync();
  controller.select("board", 1);
  controller.select("settings", 0);
  assert.deepEqual(opened, ["settings"], "moving the pending surface to another pane does not relaunch it");
  controller.close();
  controller.open("settings");
  assert.deepEqual(opened, ["settings"], "reopening split preserves the in-flight launch");
  controller.close();
  windows.push({ id: "settings", preset: "settings" });
  controller.sync();
  windows.splice(windows.findIndex(w => w.id === "settings"), 1);
  controller.open("settings");
  assert.deepEqual(opened, ["settings", "settings"], "a view that arrived and closed outside split can be launched again");
  windows.push({ id: "settings", preset: "settings" });
  controller.sync();
  assert.equal(controller.containsWindow("settings"), true);
});

test("rail surfaces share a single fixed host before and after splitting", async () => {
  const { controller, document, elements, stage } = await fixture();
  controller.show("board");
  assert.equal(controller.hasLayout, true);
  assert.equal(controller.isOpen(), false);
  assert.equal(document.getElementById("split-surfaces").hidden, false);
  let panes = [...document.querySelectorAll(".split-pane")].filter(p => !p.hidden);
  assert.equal(panes.length, 1);
  assert.equal(elements.get("board").closest(".split-pane"), panes[0]);
  controller.open();
  assert.equal(controller.isOpen(), true);
  controller.select("settings", 1);
  const selected = [...document.querySelectorAll(".split-pane")][1].querySelector(".workspace-window");
  controller.collapse();
  assert.equal(controller.hasLayout, true);
  assert.equal(controller.isOpen(), false);
  assert.equal(controller.activeSurface(), "settings");
  panes = [...document.querySelectorAll(".split-pane")].filter(p => !p.hidden);
  assert.equal(panes.length, 1);
  assert.equal(selected.closest(".split-pane"), panes[0]);
  assert.equal(elements.get("board").parentElement, stage);
  controller.show("board");
  assert.equal(elements.get("board").closest(".split-pane"), panes[0]);
  controller.close();
  assert.equal(controller.hasLayout, false);
  assert.equal(document.getElementById("split-surfaces").hidden, true);
  assert.equal(elements.get("board").parentElement, stage);
});

test("collapse retains the active same-surface live view and single Agents host", async () => {
  const { controller, document, elements, agentHosts, agents } = await fixture();
  controller.open("settings");
  controller.select("settings", 1);
  const panes = [...document.querySelectorAll(".split-pane")];
  const inactiveView = panes[0].querySelector(".workspace-window");
  const activeView = panes[1].querySelector(".workspace-window");
  controller.collapse();
  assert.equal(controller.containsWindow(activeView.dataset.id), true);
  assert.equal(controller.containsWindow(inactiveView.dataset.id), false);
  assert.equal(elements.get(activeView.dataset.id), activeView);
  controller.show("agents");
  assert.equal(agents.closest(".split-pane").hidden, false);
  assert.equal(agentHosts.at(-1)[1], null);
  controller.open();
  controller.select("agents", 0);
  controller.collapse();
  assert.equal(agents.closest(".split-pane").hidden, false);
  assert.equal(agentHosts.at(-1)[1], null);
});

test("Issues and Settings select their canonical views, not legacy windows sharing a rail category", async () => {
  const { controller, windows, elements, stage, document } = await fixture();
  for (const preset of ["work", "index", "agent_kanban", "profile", "branches"]) {
    const element = document.createElement("div");
    stage.appendChild(element);
    elements.set(preset, element);
    windows.unshift({ id: preset, preset });
  }
  controller.open("issues");
  assert.equal(controller.containsWindow("issues"), true);
  controller.select("settings", 1);
  assert.equal(controller.containsWindow("settings"), true);
  assert.equal(controller.containsWindow("branches"), false);
});
