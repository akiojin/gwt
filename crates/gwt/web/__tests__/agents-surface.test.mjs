import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { readFileSync } from "node:fs";

async function fixture() {
  const { createAgentsSurface } = await import("../agents-surface.js");
  const { document, window } = parseHTML("<main></main>");
  const mounted = [], focused = [], layouts = [];
  const previews = new Set();
  const surface = createAgentsSurface({ document,
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
  return { surface, document, window, focused, mounted, layouts, previews };
}

const agents = [
  { id: "one", preset: "codex", title: "One", session_id: "s1", status: "running" },
  { id: "two", preset: "claude", title: "Two", session_id: "s2", placement: { kind: "issue_preview" }, status: "running" },
  { id: "three", preset: "agent", title: "Three", status: "stopped" },
  { id: "pm", preset: "claude", is_pm: true },
];

test("Agents includes off-canvas agents, excludes PM and clamps the column spin control", async () => {
  const { surface, document, window, mounted } = await fixture();
  surface.sync(agents);
  assert.equal(document.querySelectorAll(".agent-tile").length, 3);
  assert.equal(mounted.length, 3);
  const columns = document.querySelector("input[type=number]");
  assert.equal(columns.getAttribute("min"), "1");
  assert.equal(columns.getAttribute("max"), "4");
  columns.value = "9";
  columns.dispatchEvent(new window.Event("input"));
  assert.equal(columns.value, "4");
  assert.equal(document.querySelector(".agents-grid").style.gridTemplateColumns, "repeat(4, minmax(0, 1fr))");
  surface.sync(agents.slice(0, 1));
  assert.equal(document.querySelectorAll(".agent-tile").length, 1);
});

test("a second pane shares the agent selection as a readonly preview and cleans up subscriptions", async () => {
  const { surface, document, window, previews } = await fixture();
  surface.sync(agents);
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
  surface.sync(agents.map(data => data.id === "one" ? { ...data, dynamic_title: "Updated" } : data));
  assert.match(host.textContent, /Updated/);
  assert.equal(previews.size, 3);
  document.querySelector('[aria-label="Close Updated tab"]').click();
  assert.equal(host.querySelector('[data-agent-id="two"]').hidden, false, "closing the shared selection returns both panes to All agents");
  const columns = surface.element.querySelector("input");
  columns.value = "3";
  columns.dispatchEvent(new window.Event("input"));
  assert.equal(host.querySelector(".agents-grid").style.gridTemplateColumns, "repeat(3, minmax(0, 1fr))");
  surface.sync(agents.slice(0, 1));
  assert.equal(previews.size, 1);
  assert.equal(host.querySelectorAll(".agent-tile").length, 1);
  surface.setPreviewHost(null);
  assert.equal(previews.size, 0);
  assert.equal(host.childNodes.length, 0);
  assert.equal(surface.element.querySelector("textarea"), textarea);
});

test("tiles keep terminal input as the only input path and report unavailable sessions", async () => {
  const { surface, document } = await fixture();
  surface.sync(agents);
  const first = document.querySelector("[data-agent-id=one]");
  assert.equal(document.querySelectorAll(".agent-tile form, .agent-tile__input").length, 0);
  assert.ok(first.querySelector(".terminal-root textarea"));
  assert.match(document.querySelector("[data-agent-id=three]").textContent, /Input unavailable/);
  assert.match(first.textContent, /Interactive/);
});

test("agent tabs select existing terminals, navigate with keys and return to the equal grid", async () => {
  const { surface, document, window, focused } = await fixture();
  surface.sync(agents);
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
  const { surface, document, layouts } = await fixture();
  surface.sync(agents);
  const tab = [...document.querySelectorAll('[role="tab"]')].find(tab => tab.textContent === "Two");
  assert.ok(tab, "each agent has a tab");
  tab.click();
  const layoutCount = layouts.length;
  surface.sync(agents.map(data => data.id === "two" ? { ...data, dynamic_title: "Updated" } : data));
  assert.equal(layouts.length, layoutCount, "metadata updates do not trigger resize feedback");
  assert.equal(tab.textContent, "Updated");
  assert.equal(tab.getAttribute("aria-selected"), "true");
  surface.sync(agents.filter(data => data.id !== "two"));
  assert.equal(document.querySelector('[role="tab"]').getAttribute("aria-selected"), "true");
  assert.equal(surface.isVisible("one"), true);
  surface.sync([]);
  assert.equal(document.querySelectorAll('[role="tab"]').length, 1);
  assert.equal(document.querySelector(".agents-empty").hidden, false);
});

test("closing agent tabs preserves the live grid and reopens the same terminal", async () => {
  const { surface, document, window } = await fixture();
  surface.sync(agents);
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
  surface.sync(agents);
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
  surface.sync(agents.filter(data => data.id !== "two"));
  surface.sync(agents);
  assert.ok(tab("two"), "a newly arriving agent has an open tab");
  assert.equal(document.querySelectorAll('[aria-label="Close All agents tab"]').length, 0);
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
