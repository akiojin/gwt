import { test } from "node:test";
import assert from "node:assert/strict";
import { createHotkeyManager, parseCombo, comboMatches } from "../hotkey.js";

test("parseCombo recognises platform modifiers", () => {
  assert.deepEqual(parseCombo("cmd+p"), { mod: true, alt: false, shift: false, key: "p" });
  assert.deepEqual(parseCombo("Cmd+Shift+P"), { mod: true, alt: false, shift: true, key: "p" });
  assert.deepEqual(parseCombo("escape"), { mod: false, alt: false, shift: false, key: "escape" });
  assert.deepEqual(parseCombo("cmd+?"), { mod: true, alt: false, shift: false, key: "?" });
});

test("comboMatches treats meta and ctrl as equivalent mod", () => {
  const combo = parseCombo("cmd+p");
  assert.equal(comboMatches(combo, { metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, key: "p" }), true);
  assert.equal(comboMatches(combo, { metaKey: false, ctrlKey: true, altKey: false, shiftKey: false, key: "P" }), true);
  assert.equal(comboMatches(combo, { metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, key: "p" }), false);
});

test("registering same combo twice throws", () => {
  const mgr = createHotkeyManager();
  mgr.register("cmd+p", () => {});
  assert.throws(() => mgr.register("cmd+p", () => {}), /already registered/i);
});

test("dispatch invokes handler when combo matches and stops propagation if returned true", () => {
  const mgr = createHotkeyManager();
  let called = 0;
  mgr.register("cmd+p", () => {
    called += 1;
    return true;
  });

  const event = mockKey({ metaKey: true, key: "p" });
  const handled = mgr.dispatch(event);

  assert.equal(handled, true);
  assert.equal(called, 1);
  assert.equal(event.preventDefaultCalled, true);
  assert.equal(event.stopPropagationCalled, true);
});

test("dispatch ignores unmatched combos", () => {
  const mgr = createHotkeyManager();
  mgr.register("cmd+p", () => true);

  const event = mockKey({ metaKey: true, key: "b" });
  const handled = mgr.dispatch(event);

  assert.equal(handled, false);
  assert.equal(event.preventDefaultCalled, false);
});

test("unregister removes the binding", () => {
  const mgr = createHotkeyManager();
  let called = 0;
  mgr.register("cmd+p", () => {
    called += 1;
    return true;
  });
  mgr.unregister("cmd+p");

  mgr.dispatch(mockKey({ metaKey: true, key: "p" }));
  assert.equal(called, 0);
});

test("dispatch ignores events targeted at editable surfaces", () => {
  const mgr = createHotkeyManager();
  let called = 0;
  mgr.register("cmd+p", () => {
    called += 1;
    return true;
  });

  const target = { tagName: "INPUT" };
  const event = mockKey({ metaKey: true, key: "p", target });
  assert.equal(mgr.dispatch(event), false);
  assert.equal(called, 0);

  const overlay = { tagName: "DIV", dataset: { hotkeyOverride: "true" } };
  const allowedEvent = mockKey({ metaKey: true, key: "p", target: overlay });
  assert.equal(mgr.dispatch(allowedEvent), true);
  assert.equal(called, 1);
});

function mockKey({ metaKey = false, ctrlKey = false, altKey = false, shiftKey = false, key, target = { tagName: "BODY" } } = {}) {
  const e = {
    metaKey, ctrlKey, altKey, shiftKey, key, target,
    preventDefaultCalled: false,
    stopPropagationCalled: false,
    preventDefault() { this.preventDefaultCalled = true; },
    stopPropagation() { this.stopPropagationCalled = true; },
  };
  return e;
}
