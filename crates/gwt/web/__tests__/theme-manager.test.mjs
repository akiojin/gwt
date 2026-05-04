import { test } from "node:test";
import assert from "node:assert/strict";
import { createThemeManager } from "../theme-manager.js";

test("default preference is auto when storage is empty", () => {
  const env = makeEnv({ stored: null, prefersDark: false });
  const mgr = createThemeManager(env);
  assert.equal(mgr.getPreference(), "auto");
  assert.equal(mgr.getEffective(), "light");
});

test("auto resolves to dark when prefers-color-scheme is dark", () => {
  const env = makeEnv({ stored: null, prefersDark: true });
  const mgr = createThemeManager(env);
  assert.equal(mgr.getEffective(), "dark");
});

test("stored preference overrides prefers-color-scheme", () => {
  const env = makeEnv({ stored: "light", prefersDark: true });
  const mgr = createThemeManager(env);
  assert.equal(mgr.getPreference(), "light");
  assert.equal(mgr.getEffective(), "light");
});

test("setTheme persists and notifies subscribers", () => {
  const env = makeEnv({ stored: null, prefersDark: false });
  const mgr = createThemeManager(env);
  const events = [];
  mgr.subscribe((eff) => events.push(eff));

  mgr.setTheme("dark");
  assert.equal(env.storage.get("gwt:ui:theme"), "dark");
  assert.equal(env.documentTheme, "dark");
  assert.deepEqual(events, ["dark"]);

  mgr.setTheme("light");
  assert.deepEqual(events, ["dark", "light"]);
  assert.equal(env.documentTheme, "light");
});

test("setTheme rejects invalid values, falling back to auto", () => {
  const env = makeEnv({ stored: null, prefersDark: false });
  const mgr = createThemeManager(env);
  mgr.setTheme("nonsense");
  assert.equal(mgr.getPreference(), "auto");
});

test("auto preference reacts to OS scheme changes when no manual override", () => {
  const env = makeEnv({ stored: null, prefersDark: false });
  const mgr = createThemeManager(env);
  const events = [];
  mgr.subscribe((eff) => events.push(eff));

  env.changePrefersDark(true);
  assert.deepEqual(events, ["dark"]);

  env.changePrefersDark(false);
  assert.deepEqual(events, ["dark", "light"]);
});

test("manual preference ignores OS scheme changes", () => {
  const env = makeEnv({ stored: "dark", prefersDark: false });
  const mgr = createThemeManager(env);
  const events = [];
  mgr.subscribe((eff) => events.push(eff));

  env.changePrefersDark(true);
  assert.deepEqual(events, []); // no transition
  assert.equal(mgr.getEffective(), "dark");
});

test("unsubscribe stops notifications", () => {
  const env = makeEnv({ stored: null, prefersDark: false });
  const mgr = createThemeManager(env);
  const events = [];
  const off = mgr.subscribe((eff) => events.push(eff));
  off();
  mgr.setTheme("dark");
  assert.deepEqual(events, []);
});

function makeEnv({ stored = null, prefersDark = false } = {}) {
  const storage = new Map();
  if (stored) storage.set("gwt:ui:theme", stored);
  let documentTheme = null;
  let listener = null;
  let currentDark = prefersDark;

  return {
    storage: {
      get: (k) => (storage.has(k) ? storage.get(k) : null),
      set: (k, v) => storage.set(k, v),
      delete: (k) => storage.delete(k),
    },
    matchMedia: (q) => {
      if (q === "(prefers-color-scheme: dark)") {
        return {
          get matches() { return currentDark; },
          addEventListener: (_t, fn) => { listener = fn; },
          removeEventListener: () => { listener = null; },
        };
      }
      return { matches: false, addEventListener: () => {}, removeEventListener: () => {} };
    },
    setDocumentTheme: (t) => { documentTheme = t; },
    get documentTheme() { return documentTheme; },
    changePrefersDark: (next) => {
      currentDark = next;
      listener?.({ matches: next });
    },
  };
}
