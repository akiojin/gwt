import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");

function assertParked(element) {
  assert.equal(element.parentElement?.className, "surface-cache");
  assert.equal(element.parentElement.hidden, true);
  assert.equal(element.parentElement.hasAttribute("inert"), true);
  assert.equal(element.isConnected, true, "parking preserves existing view lifecycle checks");
}

test("the title bar offers a reversible split and the split uses Operator tokens", () => {
  const { document } = parseHTML(html);
  const button = document.querySelector(".project-bar #split-view-button");
  assert.ok(button, "title bar must offer Split view");
  assert.equal(button.getAttribute("aria-pressed"), "false");
  assert.ok(document.querySelector(".surface-area #split-surfaces"));
  assert.equal(document.getElementById("canvas-stage"), null);
  const css = readFileSync(new URL("../styles/components.css", import.meta.url), "utf8");
  assert.match(css, /\.split-pane\[data-active="true"\][\s\S]*?var\(--color-focus-ring\)/);
  assert.match(css, /\.split-pane\[hidden\][\s\S]*?display: none !important;/);
  assert.match(css, /\.split-surfaces\[data-split="false"\] \.split-pane__header/);
});

async function fixture() {
  const { createUiStateStore } = await import("../ui-state-store.js");
  const model = createUiStateStore({ catalog: { version: 0 } });
  const { createSplitSurfaces } = await import("../split-surfaces.js");
  const { document, window } = parseHTML(html);
  const parking = document.createDocumentFragment();
  const windows = [
    { id: "issues", preset: "settings", settingsSection: "issues" },
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
    if (data.preset === "settings") {
      for (const section of ["issues", "system"]) {
        const button = document.createElement("button");
        button.dataset.settingsTab = section;
        button.addEventListener("click", () => {
          data.settingsSection = section;
          model.update(state => ({ catalog: { version: state.catalog.version + 1 } }));
        });
        element.appendChild(button);
      }
    }
    parking.appendChild(element);
    return [data.id, element];
  }));
  const opened = [];
  const agentHosts = [];
  const agents = document.createElement("section");
  const controller = createSplitSurfaces({
    document,
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
  for (const element of elements.values()) controller.park(element);
  return { controller, model, document, window, windows, elements, opened, agents, agentHosts };
}

test("two panes select independently and reset parks live DOM without hiding the rail host", async () => {
  const { controller, document, elements } = await fixture();
  controller.open("issues");
  const panes = document.querySelectorAll(".split-pane");
  assert.equal(panes.length, 2);
  assert.equal(panes[0].dataset.surface, "issues");
  assert.equal(panes[1].dataset.surface, "board");
  assert.equal(elements.get("issues").closest(".split-pane"), panes[0]);
  controller.select("settings", 1);
  assert.equal(panes[0].dataset.surface, "issues");
  assert.equal(panes[1].dataset.surface, "settings");
  assertParked(elements.get("board"));
  assert.equal(controller.activeSurface(), "settings");
  assert.equal(controller.select("issues", 1), true, "both panes may select the same surface");
  assert.equal(elements.get("issues").closest(".split-pane"), panes[0], "the existing view stays in its pane");
  assert.equal(panes[1].querySelector("option[value='issues']").disabled, false);
  controller.focusWindow("issues");
  assert.equal(panes[0].dataset.active, "true");
  controller.close();
  assert.equal(controller.isOpen(), false);
  for (const element of elements.values()) {
    assertParked(element);
    assert.equal(element.style.left, "120px");
    assert.equal(element.style.width, "600px");
  }
  assert.equal(controller.hasLayout, true);
  assert.equal(document.getElementById("split-surfaces").hidden, false);
  assert.equal([...document.querySelectorAll(".split-pane")].filter(p => !p.hidden).length, 1);
});

test("same non-Agent surfaces use distinct live windows and request only the missing view", async () => {
  for (const surface of ["issues", "board", "settings"]) {
    const { controller, document, windows, elements, opened } = await fixture();
    if (surface === "issues" || surface === "settings") {
      windows.splice(windows.findIndex(w => w.id === (surface === "issues" ? "settings" : "issues")), 1);
    }
    controller.open(surface);
    assert.equal(controller.select(surface, 1), true);
    controller.sync();
    assert.deepEqual(opened, [surface]);
    const second = { id: `${surface}-second`, preset: surface === "issues" ? "settings" : surface,
      ...(surface === "issues" ? { settingsSection: "issues" } : {}) };
    const element = document.createElement("div");
    controller.park(element);
    elements.set(second.id, element);
    windows.push(second);
    controller.sync();
    const panes = document.querySelectorAll(".split-pane");
    assert.equal(elements.get(surface).closest(".split-pane"), panes[0]);
    assert.equal(element.closest(".split-pane"), panes[1]);
    controller.close();
    assertParked(element);
    assertParked(elements.get(surface));
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
  const { controller, model, windows, elements, document } = await fixture();
  const settings = document.createElement("select");
  settings.innerHTML = '<option value="Normal" disabled>Normal</option><option value="Continue" selected>Continue</option>';
  elements.get("board").appendChild(settings);
  controller.open("board");
  controller.select("board", 1);
  const second = { id: "second-board", preset: "board" };
  const element = document.createElement("div");
  controller.park(element);
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
  const { controller, elements, agents } = await fixture();
  controller.open("agents");
  assert.equal(agents.closest(".split-pane").dataset.surface, "agents");
  assertParked(elements.get("a"));
  assertParked(elements.get("pm"));
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
  controller.collapse();
  controller.show("settings");
  assert.deepEqual(opened, ["settings"], "switching layout preserves the in-flight launch");
  controller.show("agents");
  windows.push({ id: "settings", preset: "settings" });
  controller.sync();
  windows.splice(windows.findIndex(w => w.id === "settings"), 1);
  controller.show("settings");
  assert.deepEqual(opened, ["settings", "settings"], "a view that arrived and closed outside split can be launched again");
  windows.push({ id: "settings", preset: "settings" });
  controller.sync();
  assert.equal(controller.containsWindow("settings"), true);
});

test("rail surfaces share a single fixed host before and after splitting", async () => {
  const { controller, document, elements } = await fixture();
  controller.show("issues");
  controller.show("board");
  assert.equal(controller.hasLayout, true);
  assert.equal(controller.isOpen(), false);
  assert.equal(document.getElementById("split-surfaces").hidden, false);
  let panes = [...document.querySelectorAll(".split-pane")].filter(p => !p.hidden);
  assert.equal(panes.length, 1);
  assert.equal(elements.get("board").closest(".split-pane"), panes[0]);
  controller.open();
  assert.equal(controller.isOpen(), true);
  assert.equal(document.querySelectorAll(".split-pane")[1].dataset.surface, "issues",
    "the first split complements Board with Issues");
  controller.select("settings", 1);
  const selected = [...document.querySelectorAll(".split-pane")][1].querySelector(".workspace-window");
  controller.collapse();
  assert.equal(controller.hasLayout, true);
  assert.equal(controller.isOpen(), false);
  assert.equal(controller.activeSurface(), "settings");
  panes = [...document.querySelectorAll(".split-pane")].filter(p => !p.hidden);
  assert.equal(panes.length, 1);
  assert.equal(selected.closest(".split-pane"), panes[0]);
  assertParked(elements.get("board"));
  controller.show("board");
  assert.equal(elements.get("board").closest(".split-pane"), panes[0]);
  controller.close();
  assert.equal(controller.hasLayout, true);
  assert.equal(document.getElementById("split-surfaces").hidden, false);
  assertParked(elements.get("board"));
});

test("collapse retains the active same-surface live view and single Agents host", async () => {
  const { controller, document, windows, elements, agentHosts, agents } = await fixture();
  const second = document.createElement("div");
  second.className = "workspace-window";
  second.dataset.id = "settings-second";
  windows.push({ id: "settings-second", preset: "settings" });
  elements.set("settings-second", second);
  controller.park(second);
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
  const { controller, windows, elements, document } = await fixture();
  for (const preset of ["work", "index", "agent_kanban", "profile", "branches"]) {
    const element = document.createElement("div");
    controller.park(element);
    elements.set(preset, element);
    windows.unshift({ id: preset, preset });
  }
  controller.open("issues");
  assert.equal(controller.containsWindow("issues"), true);
  controller.select("settings", 1);
  assert.equal(controller.containsWindow("settings"), true);
  assert.equal(controller.containsWindow("branches"), false);
});

test("PM uses one fixed window without entering the core split selector and retains its live DOM", async () => {
  const { controller, document, elements, opened } = await fixture();
  const pm = elements.get("pm");
  const input = document.createElement("textarea");
  pm.appendChild(input);
  controller.open("agents");
  assert.equal(controller.showWindow("pm"), true);
  assert.equal(controller.activeSurface(), "pm");
  assert.equal(controller.isOpen(), false);
  assert.equal(controller.hasLayout, true);
  assert.equal([...document.querySelectorAll(".split-pane")].filter(pane => !pane.hidden).length, 1);
  assert.equal(pm.closest(".split-pane").dataset.surface, "pm");
  assert.equal(document.querySelectorAll('option[value="pm"]').length, 0);
  const button = document.getElementById("split-view-button");
  assert.equal(button.disabled, true);
  assert.match(button.title, /PM.*single pane/);
  controller.open("issues");
  assert.equal(controller.activeSurface(), "pm", "PM cannot be split through the title bar action");
  controller.sync();
  assert.deepEqual(opened, [], "PM sync never starts a window");
  controller.show("agents");
  assert.equal(button.disabled, false);
  assertParked(pm);
  assert.equal(controller.showWindow("pm"), true);
  assert.equal(pm.querySelector("textarea"), input);
  controller.close();
  assert.equal(button.disabled, false);
});

test("PM auxiliary selection survives sync and returns to the existing PM only after catalog removal", async () => {
  const { controller, model, document, windows, elements, opened } = await fixture();
  const auxiliary = { id: "pm-settings", preset: "settings", surface_owner: "pm" };
  const element = document.createElement("div");
  controller.park(element);
  elements.set(auxiliary.id, element);
  windows.unshift(auxiliary);
  controller.show("settings");
  assert.equal(controller.containsWindow(auxiliary.id), false, "core Settings does not steal PM-owned Settings");
  controller.showWindow(auxiliary.id);
  controller.sync();
  assert.equal(controller.containsWindow(auxiliary.id), true, "a cancelled close or metadata update preserves the auxiliary selection");
  windows.splice(windows.indexOf(auxiliary), 1);
  model.update(() => ({ catalog: { version: 1 } }));
  assert.equal(controller.activeSurface(), "pm");
  assert.equal(controller.containsWindow("pm"), true);
  assert.equal(controller.containsWindow(auxiliary.id), false);
  windows.splice(windows.findIndex(data => data.id === "pm"), 1);
  model.update(() => ({ catalog: { version: 2 } }));
  controller.sync();
  assert.equal(controller.containsWindow("pm"), false);
  assert.equal(controller.activeSurface(), "pm");
  assert.deepEqual(opened, [], "removal does not implicitly respawn the PM");
  assert.equal(controller.showWindow("missing-pm"), false);
});

test("showWindow selects any classified view by id without geometry or tab-group state", async () => {
  const { controller, document, windows, elements, agents, model } = await fixture();
  const secondary = { id: "specific-settings", preset: "settings", tab_group_active: false,
    bounds: { x: 4000, y: 4000, width: 10, height: 10 } };
  const logs = { id: "logs", preset: "logs" };
  const consoleView = { id: "console", preset: "console" };
  for (const data of [secondary, logs, consoleView]) {
    const element = document.createElement("div");
    elements.set(data.id, element);
    windows.push(data);
    controller.park(element);
  }
  const input = document.createElement("textarea");
  elements.get(secondary.id).appendChild(input);
  controller.open("board");
  assert.equal(controller.showWindow(secondary.id), true);
  assert.equal(controller.isOpen(), true, "non-PM windows keep split layout");
  assert.equal(controller.activeSurface(), "settings");
  assert.equal(controller.containsWindow(secondary.id), true);
  assert.equal(elements.get(secondary.id).hidden, false);
  model.update(() => ({ catalog: { version: 1 } }));
  assert.equal(controller.containsWindow(secondary.id), true, "metadata sync preserves explicit selection");
  controller.show("board");
  assertParked(elements.get(secondary.id));
  assert.equal(controller.showWindow(secondary.id), true);
  assert.equal(elements.get(secondary.id).querySelector("textarea"), input);
  assert.equal(controller.showWindow(logs.id), true);
  assert.equal(controller.containsWindow(logs.id), true, "classified auxiliary views use the same host");
  assert.equal(controller.showWindow(consoleView.id), true);
  assert.equal(controller.containsWindow(consoleView.id), true, "console keeps its own live view");
  assert.equal(controller.showWindow("a"), true);
  assert.equal(controller.activeSurface(), "agents");
  assert.ok(agents.closest(".split-pane"), "agents retain the dedicated live grid");
  assert.equal(controller.containsWindow("a"), false);
  assert.equal(controller.showWindow("missing"), false);
});

test("inactive live views stay in a hidden inert cache outside both pane bodies", async () => {
  const { controller, document, elements } = await fixture();
  controller.show("board");
  const cache = document.querySelector("#split-surfaces > .surface-cache");
  assert.equal(cache.getAttribute("aria-hidden"), "true");
  assert.equal(cache.closest(".split-pane"), null);
  assertParked(elements.get("settings"));
  controller.show("settings");
  assert.equal(elements.get("issues").closest(".split-pane").hidden, false);
  assertParked(elements.get("board"));
});

test("Settings and Issues reuse one canonical view and follow its internal section changes", async () => {
  const { controller, windows, elements, model } = await fixture();
  windows.splice(windows.findIndex(data => data.id === "settings"), 1);
  controller.show("issues");
  const view = elements.get("issues");
  assert.equal(controller.containsWindow("issues"), true);
  controller.show("settings");
  assert.equal(controller.containsWindow("issues"), true);
  assert.equal(windows.find(data => data.id === "issues").settingsSection, "system");
  view.querySelector('[data-settings-tab="issues"]').click();
  model.update(() => ({ catalog: { version: 1 } }));
  assert.equal(controller.activeSurface(), "issues");
  view.querySelector('[data-settings-tab="system"]').click();
  model.update(() => ({ catalog: { version: 2 } }));
  assert.equal(controller.activeSurface(), "settings");
  assert.equal(controller.containsWindow("issues"), true);
});
