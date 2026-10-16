import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { readFileSync } from "node:fs";

const appSource = readFileSync(new URL("../app.js", import.meta.url), "utf8");
const titleBody = appSource.match(/function windowDisplayTitle\(windowData\)\s*\{([\s\S]*?)\n      \}/)[1];
const titleFromApp = new Function("windowData", "isAgentWindowPreset", "PM_WINDOW_TITLE", titleBody);
const windowDisplayTitle = data => titleFromApp(data, preset => ["agent", "claude", "codex"].includes(preset), "Project Manager");

async function fixture({ isMounted = () => true, storage } = {}) {
  const { createUiStateStore } = await import("../ui-state-store.js");
  const model = createUiStateStore({ catalog: { windows: [], version: 0 } });
  const { createAgentsSurface } = await import("../agents-surface.js");
  const { document, window } = parseHTML("<main></main>");
  Object.defineProperty(window, "localStorage", { configurable: true, get: () => storage });
  const mounted = [], focused = [], layouts = [];
  const previews = new Set();
  const surface = createAgentsSurface({ document, windowCatalogModel: model, windowDisplayTitle,
    getWindows: catalog => catalog.windows, isMounted,
    mountTerminal: (id, root) => { mounted.push([id, root]); if (!root.firstChild) root.appendChild(document.createElement("textarea")); },
    mountPreview: (id, root) => {
      const pre = document.createElement("pre");
      pre.textContent = id;
      root.appendChild(pre);
      previews.add(pre);
      return () => { previews.delete(pre); pre.remove(); };
    },
    onFocus: id => focused.push(id),
    onLayout: () => layouts.push(true),
  });
  document.querySelector("main").appendChild(surface.element);
  return { surface, model, setWindows: windows => model.update(state => ({ catalog: { windows, version: state.catalog.version + 1 } })), document, window, focused, mounted, layouts, previews };
}

const agents = [
  { id: "one", preset: "codex", title: "One", agent_name: "One", session_id: "s1", status: "running" },
  { id: "two", preset: "claude", title: "Two", agent_name: "Two", session_id: "s2", placement: { kind: "issue_preview" }, status: "running" },
  { id: "three", preset: "agent", title: "Three", agent_name: "Three", status: "stopped" },
  { id: "pm", preset: "claude", is_pm: true },
];

test("Agents titles preserve a fixed name and scrollable purpose in cards, tabs and previews", async () => {
  const { surface, setWindows, document } = await fixture();
  const purpose = "Investigate the terminal output and deliver the smallest verified change. ".repeat(3);
  const data = { ...agents[0], agent_name: "Lumen", dynamic_title: purpose.trim() };
  setWindows([data]);
  const host = document.createElement("div");
  document.querySelector("main").appendChild(host);
  surface.setPreviewHost(host);
  const terminal = surface.element.querySelector("textarea");
  for (const title of [surface.element.querySelector("h3"), document.getElementById("agents-tab-one"), host.querySelector("h3")]) {
    assert.equal(title.querySelector(".agent-title__name")?.textContent, "Lumen");
    assert.equal(title.querySelector(".agent-title__purpose")?.textContent, ` — ${purpose.trim()}`);
    assert.equal(title.dataset.issueExplanation, title.textContent);
    assert.equal(title.getAttribute("aria-label"), title.textContent);
  }
  assert.equal(surface.element.querySelector("h3").getAttribute("tabindex"), "0");
  setWindows([{ ...data, agent_name: "Vireo", dynamic_title: "Updated purpose" }]);
  for (const title of [surface.element.querySelector("h3"), document.getElementById("agents-tab-one"), host.querySelector("h3")]) {
    assert.equal(title.querySelector(".agent-title__name").textContent, "Vireo");
    assert.equal(title.dataset.issueExplanation, "Vireo — Updated purpose");
  }
  assert.equal(surface.element.querySelector("textarea"), terminal);
  assert.match(surface.element.querySelector(".agent-tile__header").textContent, /Interactive/);
  surface.dispose();
});

test("Agents title styles keep one line and constrain only the purpose scroll area", () => {
  const css = readFileSync(new URL("../styles/components.css", import.meta.url), "utf8");
  assert.match(css, /\.agent-title\s*\{[^}]*white-space:\s*nowrap/);
  assert.match(css, /\.agent-title__name\s*\{[^}]*flex-shrink:\s*0/);
  assert.match(css, /\.agent-title__purpose\s*\{[^}]*overflow-x:\s*auto/);
  assert.match(css, /\.agent-tile__header\s*\{[^}]*flex-shrink:\s*0/);
});

test("Agents tiles, tabs and readonly preview acquire a late assigned name", async () => {
  const { surface, setWindows, document } = await fixture();
  const data = { ...agents[0], agent_name: null, dynamic_title: "Fix cache" };
  setWindows([data]);
  const host = document.createElement("div");
  document.querySelector("main").appendChild(host);
  surface.setPreviewHost(host);
  assert.equal(surface.element.querySelector("h3").textContent, "Naming… — Fix cache");
  const terminal = surface.element.querySelector("textarea");
  setWindows([{ ...data, agent_name: "Lumen" }]);
  assert.equal(surface.element.querySelector("h3").textContent, "Lumen — Fix cache");
  assert.equal(document.getElementById("agents-tab-one").textContent, "Lumen — Fix cache");
  assert.equal(host.querySelector("h3").textContent, "Lumen — Fix cache");
  assert.equal(surface.element.querySelector("textarea"), terminal);
  surface.dispose();
});

test("Agents and its preview pull one catalog revision and release their subscription", async () => {
  let active = false;
  const { surface, model, setWindows, document, mounted } = await fixture({ isMounted: () => active });
  setWindows(agents);
  assert.equal(mounted.length, 0, "inactive Agents must not steal terminal hosts");
  active = true;
  surface.sync();
  const host = document.createElement("div");
  document.querySelector("main").appendChild(host);
  surface.setPreviewHost(host);
  assert.equal(surface.element.querySelectorAll(".agent-tile").length, 3);
  const terminal = surface.element.querySelector("textarea");
  setWindows([{ ...agents[0], dynamic_title: "Latest", status: "stopped" }]);
  assert.equal(surface.element.querySelector("h3").textContent, "One — Latest");
  assert.equal(host.querySelector("h3").textContent, "One — Latest");
  assert.match(surface.element.textContent, /Input unavailable/);
  assert.equal(surface.element.querySelector("textarea"), terminal);
  for (const view of [surface.element, host.firstElementChild]) {
    assert.equal(view.dataset.catalogVersion, String(model.read().catalog.version));
  }
  surface.sync([{ ...agents[0], dynamic_title: "Bypass" }]);
  assert.equal(surface.element.querySelector("h3").textContent, "One — Latest", "a view cannot accept uncommitted payloads");
  surface.dispose();
  assert.equal(host.childNodes.length, 0);
  const mounts = mounted.length;
  setWindows(agents);
  assert.equal(mounted.length, mounts, "disposed views no longer react");
});

test("Agents includes off-canvas agents, excludes PM and clamps the column spin control", async () => {
  const { surface, setWindows, document, window, mounted } = await fixture();
  setWindows(agents);
  assert.equal(document.querySelectorAll(".agent-tile").length, 3);
  assert.equal(mounted.length, 3);
  const columns = document.querySelector("input[type=number]");
  assert.equal(columns.getAttribute("min"), "1");
  assert.equal(columns.getAttribute("max"), "4");
  columns.value = "9";
  columns.dispatchEvent(new window.Event("input"));
  assert.equal(columns.value, "4");
  assert.equal(document.querySelector(".agents-grid").style.gridTemplateColumns, "repeat(4, minmax(0, 1fr))");
  setWindows(agents.slice(0, 1));
  assert.equal(document.querySelectorAll(".agent-tile").length, 1);
});

test("Agents restores Columns when recreated and normalizes saved values", async () => {
  const saved = new Map();
  const storage = { getItem: key => saved.get(key) ?? null, setItem: (key, value) => saved.set(key, value) };
  const first = await fixture({ storage });
  const columns = first.surface.element.querySelector("input");
  assert.equal(columns.value, "2", "missing preference keeps the existing default");
  columns.value = "3";
  columns.dispatchEvent(new first.window.Event("input"));
  first.surface.dispose();
  const restored = await fixture({ storage });
  assert.equal(restored.surface.element.querySelector("input").value, "3");
  assert.equal(restored.surface.element.querySelector(".agents-grid").style.gridTemplateColumns, "repeat(3, minmax(0, 1fr))");
  restored.surface.dispose();
  for (const [value, expected] of [["0", "1"], ["9", "4"], ["invalid", "1"]]) {
    saved.set("gwt:ui:agent-columns", value);
    const current = await fixture({ storage });
    assert.equal(current.surface.element.querySelector("input").value, expected);
    current.surface.dispose();
  }
});

test("Agents keeps Columns usable when browser storage is unavailable", async () => {
  const storage = { getItem() { throw new Error("unavailable"); }, setItem() { throw new Error("unavailable"); } };
  const { surface, window } = await fixture({ storage });
  const columns = surface.element.querySelector("input");
  assert.equal(columns.value, "2");
  columns.value = "3";
  columns.dispatchEvent(new window.Event("input"));
  assert.equal(surface.element.querySelector(".agents-grid").style.gridTemplateColumns, "repeat(3, minmax(0, 1fr))");
  surface.dispose();
});

test("a second pane shares the agent selection as a readonly preview and cleans up subscriptions", async () => {
  const { surface, setWindows, document, window, previews } = await fixture();
  setWindows(agents);
  const host = document.createElement("div");
  document.querySelector("main").appendChild(host);
  const textarea = surface.element.querySelector("textarea");
  surface.setPreviewHost(host);
  assert.equal(previews.size, 3);
  assert.match(host.textContent, /Read-only preview/);
  assert.equal(host.querySelectorAll("textarea, [role=tab]").length, 0);
  document.getElementById("agents-tab-one").click();
  assert.equal(host.querySelector('[data-agent-id="one"]').hidden, false);
  assert.equal(host.querySelector('[data-agent-id="two"]').hidden, true);
  setWindows(agents.map(data => data.id === "one" ? { ...data, dynamic_title: "Updated" } : data));
  assert.match(host.textContent, /Updated/);
  assert.equal(previews.size, 3);
  document.querySelector('[aria-label="Close One — Updated tab"]').click();
  assert.equal(host.querySelector('[data-agent-id="two"]').hidden, false, "closing the shared selection returns both panes to All agents");
  const columns = surface.element.querySelector("input");
  columns.value = "3";
  columns.dispatchEvent(new window.Event("input"));
  assert.equal(host.querySelector(".agents-grid").style.gridTemplateColumns, "repeat(3, minmax(0, 1fr))");
  setWindows(agents.slice(0, 1));
  assert.equal(previews.size, 1);
  assert.equal(host.querySelectorAll(".agent-tile").length, 1);
  surface.setPreviewHost(null);
  assert.equal(previews.size, 0);
  assert.equal(host.childNodes.length, 0);
  assert.equal(surface.element.querySelector("textarea"), textarea);
});

test("tiles keep terminal input as the only input path and report unavailable sessions", async () => {
  const { surface, setWindows, document } = await fixture();
  setWindows(agents);
  const first = document.querySelector("[data-agent-id=one]");
  assert.equal(document.querySelectorAll(".agent-tile form, .agent-tile__input").length, 0);
  assert.ok(first.querySelector(".terminal-root textarea"));
  assert.match(document.querySelector("[data-agent-id=three]").textContent, /Input unavailable/);
  assert.match(first.textContent, /Interactive/);
});

test("launched and restored Shells are interactive without agent sessions and retain stopped status gating", async () => {
  const { setWindows, document } = await fixture();
  const shell = { id: "restored-shell", preset: "shell", title: "Shell", status: "running" };
  const dedicated = { ...shell, id: "start-work-shell", surface_owner: "agents" };
  setWindows([shell, dedicated, { id: "naming", preset: "codex", status: "running" },
    { ...shell, id: "pm", is_pm: true }, { id: "console", preset: "console" },
    { id: "unknown", preset: "unknown" }]);
  const tile = document.querySelector('[data-agent-id="restored-shell"]');
  assert.ok(tile, "restored Shell belongs to Agents");
  assert.equal(document.querySelectorAll(".agent-tile").length, 3, "PM and non-terminal views stay outside Agents");
  assert.match(document.querySelector('[data-agent-id="start-work-shell"]').textContent, /Interactive/);
  const input = tile.querySelector("textarea");
  assert.ok(input);
  assert.match(tile.textContent, /Interactive/);
  assert.match(document.querySelector('[data-agent-id="naming"]').textContent, /Input unavailable/, "ordinary agents still require their session");
  setWindows([{ ...shell, status: "stopped" }]);
  assert.match(tile.textContent, /Input unavailable/);
  assert.equal(tile.querySelector("textarea"), input);
});

test("agent tabs select existing terminals, navigate with keys and return to the equal grid", async () => {
  const { surface, setWindows, document, window, focused } = await fixture();
  setWindows(agents);
  const tabs = [...document.querySelectorAll('[role="tab"]')];
  assert.deepEqual(tabs.map(tab => tab.textContent), ["All agents", "One", "Two", "Three"]);
  const one = document.querySelector('[data-agent-id="one"]');
  const terminal = one.querySelector(".terminal-root textarea");
  tabs[2].click();
  assert.equal(tabs[2].getAttribute("aria-selected"), "true");
  assert.equal(tabs[2].getAttribute("tabindex"), "0");
  assert.equal(one.hidden, true);
  assert.equal(surface.contains("one"), true);
  assert.equal(surface.isVisible("one"), false);
  assert.equal(surface.isVisible("two"), true);
  assert.deepEqual(focused, ["two"]);
  const key = (target, value) => {
    const event = new window.Event("keydown", { cancelable: true });
    Object.defineProperty(event, "key", { value });
    target.dispatchEvent(event);
  };
  key(tabs[2], "ArrowLeft");
  assert.equal(tabs[1].getAttribute("aria-selected"), "true");
  assert.equal(one.hidden, false);
  assert.equal(one.querySelector(".terminal-root textarea"), terminal);
  key(tabs[1], "Home");
  assert.equal(tabs[0].getAttribute("aria-selected"), "true");
  assert.ok(agents.slice(0, 3).every(agent => surface.isVisible(agent.id)));
  assert.equal(document.querySelector(".agents-grid").style.gridTemplateColumns, "repeat(2, minmax(0, 1fr))");
});

test("tab selection survives updates and falls back when the selected agent disappears", async () => {
  const { surface, setWindows, document, layouts } = await fixture();
  setWindows(agents);
  const tab = [...document.querySelectorAll('[role="tab"]')].find(tab => tab.textContent === "Two");
  assert.ok(tab, "each agent has a tab");
  tab.click();
  const layoutCount = layouts.length;
  setWindows(agents.map(data => data.id === "two" ? { ...data, dynamic_title: "Updated" } : data));
  assert.equal(layouts.length, layoutCount, "metadata updates do not trigger resize feedback");
  assert.equal(tab.textContent, "Two — Updated");
  assert.equal(tab.getAttribute("aria-selected"), "true");
  setWindows(agents.filter(data => data.id !== "two"));
  assert.equal(document.querySelector('[role="tab"]').getAttribute("aria-selected"), "true");
  assert.equal(surface.isVisible("one"), true);
  setWindows([]);
  assert.equal(document.querySelectorAll('[role="tab"]').length, 1);
  assert.equal(document.querySelector(".agents-empty").hidden, false);
});

test("closing agent tabs preserves the live grid and reopens the same terminal", async () => {
  const { surface, setWindows, document, window } = await fixture();
  setWindows(agents);
  const tab = id => document.getElementById(`agents-tab-${id}`);
  const one = document.querySelector('[data-agent-id="one"]');
  const terminal = one.querySelector("textarea");
  const reopen = one.querySelector('[aria-label="Open One tab"]');
  assert.equal(reopen.textContent, "Open tab", "reopening has a visible action label");
  assert.equal(reopen.hidden, true, "open tabs already have a selection button");
  tab("one").click();
  const close = document.querySelector('[aria-label="Close One tab"]');
  assert.ok(close, "each individual tab has a named close button");
  assert.equal(close.closest('[role="tab"]'), null, "buttons are siblings, not nested");
  close.click();
  assert.equal(tab("one"), null);
  assert.equal(tab("all").getAttribute("aria-selected"), "true");
  assert.equal(surface.contains("one"), true);
  assert.equal(surface.isVisible("one"), true, "All agents still shows the running agent");
  assert.equal(reopen.hidden, false);
  setWindows(agents);
  assert.equal(tab("one"), null, "metadata updates do not reopen closed tabs");
  document.querySelector('[aria-label="Open One tab"]').click();
  assert.equal(tab("one").getAttribute("aria-selected"), "true");
  assert.equal(one.querySelector("textarea"), terminal);
  assert.equal(reopen.hidden, true);
  document.querySelector('[aria-label="Close Two tab"]').click();
  assert.equal(tab("one").getAttribute("aria-selected"), "true", "background close preserves selection");
  const event = new window.Event("keydown", { cancelable: true });
  Object.defineProperty(event, "key", { value: "Delete" });
  tab("one").dispatchEvent(event);
  assert.equal(event.defaultPrevented, true);
  assert.equal(tab("one"), null);
  assert.equal(tab("all").getAttribute("aria-selected"), "true");
  setWindows(agents.filter(data => data.id !== "two"));
  setWindows(agents);
  assert.ok(tab("two"), "a newly arriving agent has an open tab");
  assert.equal(document.querySelectorAll('[aria-label="Close All agents tab"]').length, 0);
});

test("external tab selection reopens the same terminal and reports whether the agent exists", async () => {
  const { surface, setWindows, document, focused } = await fixture();
  setWindows(agents);
  const terminal = document.querySelector('[data-agent-id="two"] textarea');
  document.querySelector('[aria-label="Close Two tab"]').click();
  assert.equal(surface.openTab("two"), true);
  assert.equal(document.getElementById("agents-tab-two").getAttribute("aria-selected"), "true");
  assert.equal(document.querySelector('[data-agent-id="two"] textarea'), terminal);
  assert.equal(focused.at(-1), "two");
  assert.equal(surface.openTab("pm"), false);
  assert.equal(surface.openTab("missing"), false);
  assert.equal(document.getElementById("agents-tab-two").getAttribute("aria-selected"), "true");
});

test("the grid uses Operator tokens and contains the terminal inside each tile", () => {
  const css = readFileSync(new URL("../styles/components.css", import.meta.url), "utf8");
  const surface = css.slice(css.indexOf("/* Issue 4777 T-4:"));
  assert.match(surface, /grid-template-columns: repeat\(2, minmax\(0, 1fr\)\)/);
  assert.match(surface, /\.agent-tile \.agent-tile__terminal \{ position: relative; inset: auto;/);
  assert.match(surface, /var\(--color-focus-ring\)/);
  assert.match(surface, /\.agents-tabs/);
  assert.match(surface, /\.agent-tile\[hidden\]/);
  assert.match(surface, /\.agents-grid\.is-single > \.agent-tile \{ grid-area: 1 \/ 1;/);
  assert.match(surface, /\.agent-tile\[hidden\] \{ display: flex; visibility: hidden;/);
  assert.match(surface, /\.agents-tab-close/);
  assert.match(surface, /\.agent-tile__open:focus-visible/);
  assert.doesNotMatch(surface.replace(/\/\*[\s\S]*?\*\//g, ""), /#[a-f0-9]{3,8}\b|rgba?\(/i);
});
