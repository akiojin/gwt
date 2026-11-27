import assert from "node:assert/strict";
import test from "node:test";
import { parseHTML } from "linkedom";
import { createElement } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { createUiStateStore } from "../../web/ui-state-store.js";
import { useStoreSelector } from "../src/store/useStoreSelector.js";

const selectCount = state => state.count;
// The development react-dom build reads `window` for event timing.
globalThis.window ??= parseHTML("<html></html>").window;

function mount(store, options) {
  const { document } = parseHTML('<html><body><div id="root"></div></body></html>');
  const container = document.getElementById("root");
  const renders = [];
  function Probe() {
    const value = useStoreSelector(store, selectCount, options);
    renders.push(value);
    return createElement("output", null, String(value));
  }
  const root = createRoot(container);
  flushSync(() => root.render(createElement(Probe)));
  return { container, renders, root };
}

test("store updates re-render synchronously; unchanged selections do not", () => {
  const store = createUiStateStore({ count: 0, other: "a" });
  const { container, renders, root } = mount(store);
  assert.deepEqual(renders, [0]);
  store.update(state => ({ ...state, count: 1 }));
  assert.equal(container.textContent, "1", "DOM matches the store when update returns");
  store.update(state => ({ ...state, other: "b" }));
  assert.deepEqual(renders, [0, 1], "Object.is-equal selection must not re-render");
  root.unmount();
});

test("frame selections wait for the store frame", () => {
  const frames = [];
  const store = createUiStateStore({ count: 0 }, { scheduleFrame: run => { frames.push(run); return () => {}; } });
  const { container, renders, root } = mount(store, { frame: true });
  store.update(() => ({ count: 1 }));
  store.update(() => ({ count: 2 }));
  assert.equal(container.textContent, "0");
  assert.equal(frames.length, 1);
  frames.shift()();
  assert.equal(container.textContent, "2");
  assert.deepEqual(renders, [0, 2]);
  root.unmount();
});

test("unmount unsubscribes from the store", () => {
  const store = createUiStateStore({ count: 0 });
  const original = store.subscribe;
  let active = 0;
  store.subscribe = (select, render, frame) => {
    active++;
    const unsubscribe = original(select, render, frame);
    return () => { active--; unsubscribe(); };
  };
  const { renders, root } = mount(store);
  assert.equal(active, 1);
  root.unmount();
  assert.equal(active, 0);
  store.update(() => ({ count: 5 }));
  assert.deepEqual(renders, [0]);
});

test("a component reads the store, never a push payload", async () => {
  const { readFile } = await import("node:fs/promises");
  const surface = await readFile(new URL("../src/surfaces/WorkspaceStateNotice.jsx", import.meta.url), "utf8");
  assert.match(surface, /useStoreSelector\(store,/);
  assert.doesNotMatch(surface, /\b(event|payload)\b/);
});
