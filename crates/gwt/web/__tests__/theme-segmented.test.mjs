import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import { wireThemeToggle } from "../theme-toggle.js";

const here = dirname(fileURLToPath(import.meta.url));
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");

// SPEC-2356 FR-024 / US3-AS6, AS7 — segmented theme toggle
// (AUTO / DARK / LIGHT exposed in parallel, no cycle).

function makeStubManager(initialPref = "auto", initialEff = "light") {
  let preference = initialPref;
  let effective = initialEff;
  const subscribers = new Set();
  return {
    setTheme: (value) => {
      preference = value;
      if (value !== "auto") effective = value;
      for (const fn of subscribers) fn(effective);
    },
    getPreference: () => preference,
    getEffective: () => effective,
    subscribe: (fn) => {
      subscribers.add(fn);
      return () => subscribers.delete(fn);
    },
    _setEffective: (eff) => {
      effective = eff;
      for (const fn of subscribers) fn(effective);
    },
  };
}

function bootDom() {
  const { document, window } = parseHTML(indexHtml);
  // linkedom does not track document.activeElement on focus(); shim it so the
  // segmented toggle handler can read the focused option deterministically.
  let active = null;
  Object.defineProperty(document, "activeElement", {
    configurable: true,
    get: () => active,
  });
  for (const el of document.querySelectorAll("button, [tabindex]")) {
    const original = el.focus?.bind(el);
    el.focus = () => {
      active = el;
      original?.();
    };
  }
  return { doc: document, win: window };
}

// linkedom only ships a generic Event constructor; build typed-ish events by
// instantiating Event then patching the properties our handlers read.
function click(doc, target) {
  const ev = new doc.defaultView.Event("click", { bubbles: true, cancelable: true });
  Object.defineProperty(ev, "target", { value: target, configurable: true });
  target.dispatchEvent(ev);
}

function keydown(doc, target, key) {
  const ev = new doc.defaultView.Event("keydown", { bubbles: true, cancelable: true });
  Object.defineProperty(ev, "key", { value: key, configurable: true });
  Object.defineProperty(ev, "target", { value: target, configurable: true });
  target.dispatchEvent(ev);
}

test("index.html exposes a segmented theme toggle (radiogroup with auto/dark/light)", () => {
  const { doc } = bootDom();
  const root = doc.getElementById("op-theme-toggle");
  assert.ok(root, "#op-theme-toggle root must exist");
  assert.equal(root.getAttribute("role"), "radiogroup", "root must be radiogroup");
  const buttons = root.querySelectorAll("[data-theme-value]");
  assert.equal(buttons.length, 3, "must expose AUTO / DARK / LIGHT buttons");
  const values = Array.from(buttons).map((b) => b.dataset.themeValue);
  assert.deepEqual(values, ["auto", "dark", "light"]);
  for (const btn of buttons) {
    assert.equal(btn.getAttribute("role"), "radio", "each option must be role=radio");
    assert.ok(btn.hasAttribute("aria-checked"), "each option must declare aria-checked");
  }
});

test("wireThemeToggle marks the active preference exclusively via aria-checked", () => {
  const { doc } = bootDom();
  const mgr = makeStubManager("auto");
  wireThemeToggle({ doc, themeManager: mgr });

  const buttons = Array.from(doc.querySelectorAll("#op-theme-toggle [data-theme-value]"));
  const checked = buttons.filter((b) => b.getAttribute("aria-checked") === "true");
  assert.equal(checked.length, 1, "exactly one option must be aria-checked=true");
  assert.equal(checked[0].dataset.themeValue, "auto");
});

test("clicking a segmented option calls setTheme(value) and updates aria-checked", () => {
  const { doc } = bootDom();
  const mgr = makeStubManager("auto");
  wireThemeToggle({ doc, themeManager: mgr });

  const lightBtn = doc.querySelector('#op-theme-toggle [data-theme-value="light"]');
  click(doc, lightBtn);

  assert.equal(mgr.getPreference(), "light");
  const buttons = Array.from(doc.querySelectorAll("#op-theme-toggle [data-theme-value]"));
  const checked = buttons.find((b) => b.getAttribute("aria-checked") === "true");
  assert.equal(checked.dataset.themeValue, "light");
});

test("AUTO can be re-selected after a manual override (no dead-end cycle)", () => {
  const { doc } = bootDom();
  const mgr = makeStubManager("auto");
  wireThemeToggle({ doc, themeManager: mgr });

  const dark = doc.querySelector('#op-theme-toggle [data-theme-value="dark"]');
  click(doc, dark);
  assert.equal(mgr.getPreference(), "dark");

  const auto = doc.querySelector('#op-theme-toggle [data-theme-value="auto"]');
  click(doc, auto);
  assert.equal(mgr.getPreference(), "auto", "AUTO must be reachable directly");
});

test("Arrow keys move roving tabindex across segmented options", () => {
  const { doc } = bootDom();
  const mgr = makeStubManager("auto");
  wireThemeToggle({ doc, themeManager: mgr });

  const buttons = Array.from(doc.querySelectorAll("#op-theme-toggle [data-theme-value]"));
  // Active option (auto) carries tabindex=0; others carry tabindex=-1.
  assert.equal(buttons[0].getAttribute("tabindex"), "0");
  assert.equal(buttons[1].getAttribute("tabindex"), "-1");
  assert.equal(buttons[2].getAttribute("tabindex"), "-1");

  buttons[0].focus();
  keydown(doc, buttons[0], "ArrowRight");
  assert.equal(doc.activeElement?.dataset.themeValue, "dark", "ArrowRight moves focus to next option");

  keydown(doc, doc.activeElement, "ArrowLeft");
  assert.equal(doc.activeElement?.dataset.themeValue, "auto", "ArrowLeft wraps to previous option");
});

test("Enter on a focused option commits that preference", () => {
  const { doc } = bootDom();
  const mgr = makeStubManager("auto");
  wireThemeToggle({ doc, themeManager: mgr });

  const dark = doc.querySelector('#op-theme-toggle [data-theme-value="dark"]');
  dark.focus();
  keydown(doc, dark, "Enter");
  assert.equal(mgr.getPreference(), "dark");
});

test("AUTO option exposes an effective indicator that follows getEffective()", () => {
  const { doc } = bootDom();
  const mgr = makeStubManager("auto", "light");
  wireThemeToggle({ doc, themeManager: mgr });

  const indicator = doc.getElementById("op-theme-effective-indicator");
  assert.ok(indicator, "AUTO option must expose an effective indicator span");
  assert.equal(indicator.textContent, "▯", "light effective renders the unfilled glyph");

  mgr._setEffective("dark");
  assert.equal(indicator.textContent, "▮", "dark effective renders the filled glyph");
});
