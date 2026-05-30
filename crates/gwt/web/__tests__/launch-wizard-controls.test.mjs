// SPEC-2014 2026-05-29 amendment (FR-105..FR-110, SC-063/SC-064) — Launch
// Agent setting controls UI/UX overhaul. Behavioral tests for the standalone
// `launch-controls.js` module: reasoning slider + Auto toggle, count-adaptive
// segmented/select dispatch, and boolean toggle switch (real checkbox inside).
//
// These mirror the linkedom behavioral pattern used by theme-segmented.test.mjs
// so the control logic is exercised, not just grepped from source.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";
import {
  chooseLaunchControlKind,
  reasoningSliderModel,
  buildReasoningField,
  buildSegmentedField,
  buildToggleField,
  buildChoiceOrSelectField,
} from "../launch-controls.js";

// --- fixtures (mirror the Rust DTO values served to the frontend) ----------

const CODEX_REASONING = [
  { value: "low", label: "Low", description: "Fast responses with lighter reasoning" },
  { value: "medium", label: "Medium", description: "Balances speed and reasoning depth" },
  { value: "high", label: "High", description: "Greater reasoning depth" },
  { value: "xhigh", label: "Extra high", description: "Maximum reasoning depth" },
];

const CLAUDE_OPUS_REASONING = [
  { value: "auto", label: "Auto", description: "Let the model choose the effort" },
  { value: "low", label: "Low", description: "Fast responses for simple work" },
  { value: "medium", label: "Medium", description: "Balanced reasoning for everyday work" },
  { value: "high", label: "High", description: "Deeper reasoning for complex work" },
  { value: "xhigh", label: "xHigh", description: "Best results for most coding tasks" },
  { value: "max", label: "Max", description: "Deepest reasoning with no constraint" },
];

const TARGET_OPTIONS = [
  { value: "agent", label: "Agent" },
  { value: "shell", label: "Shell" },
];

const AGENT_OPTIONS_FEW = [
  { value: "claude", label: "Claude Code" },
  { value: "codex", label: "Codex" },
  { value: "gemini", label: "Gemini CLI" },
];

const AGENT_OPTIONS_MANY = [
  { value: "claude", label: "Claude Code" },
  { value: "codex", label: "Codex" },
  { value: "gemini", label: "Gemini CLI" },
  { value: "copilot", label: "GitHub Copilot" },
  { value: "custom-a", label: "My Custom Agent" },
];

// --- linkedom helpers (mirror theme-segmented.test.mjs) --------------------

function bootDom() {
  const { document } = parseHTML("<!doctype html><html><body></body></html>");
  let active = null;
  Object.defineProperty(document, "activeElement", {
    configurable: true,
    get: () => active,
  });
  document.__setActive = (el) => {
    active = el;
  };
  return document;
}

function trackFocus(doc, root) {
  for (const el of root.querySelectorAll("button, [tabindex], input")) {
    const original = el.focus?.bind(el);
    el.focus = () => {
      doc.__setActive(el);
      original?.();
    };
  }
}

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

function dispatchRange(doc, input, value, type = "input") {
  input.value = String(value);
  const ev = new doc.defaultView.Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(ev, "target", { value: input, configurable: true });
  input.dispatchEvent(ev);
}

function changeChecked(doc, input, checked) {
  input.checked = checked;
  const ev = new doc.defaultView.Event("change", { bubbles: true, cancelable: true });
  Object.defineProperty(ev, "target", { value: input, configurable: true });
  input.dispatchEvent(ev);
}

// --- chooseLaunchControlKind ----------------------------------------------

test("chooseLaunchControlKind picks segmented for few short options", () => {
  assert.equal(chooseLaunchControlKind(TARGET_OPTIONS), "segmented");
  assert.equal(chooseLaunchControlKind(AGENT_OPTIONS_FEW), "segmented");
});

test("chooseLaunchControlKind falls back to select past the count threshold", () => {
  // 5 options exceeds the default 4-option segmented budget (Agent w/ custom).
  assert.equal(chooseLaunchControlKind(AGENT_OPTIONS_MANY), "select");
});

test("chooseLaunchControlKind falls back to select for long labels", () => {
  const longLabel = [
    { value: "a", label: "A reasonably short one" },
    { value: "b", label: "B" },
  ];
  assert.equal(chooseLaunchControlKind(longLabel), "select");
});

test("chooseLaunchControlKind keeps a single/empty option on select", () => {
  assert.equal(chooseLaunchControlKind([{ value: "x", label: "X" }]), "select");
  assert.equal(chooseLaunchControlKind([]), "select");
});

// --- reasoningSliderModel --------------------------------------------------

test("reasoningSliderModel maps Codex options without Auto", () => {
  const model = reasoningSliderModel(CODEX_REASONING, "medium");
  assert.equal(model.hasAuto, false);
  assert.equal(model.stops.length, 4);
  assert.equal(model.ordinalIndex, 1);
  assert.equal(model.isAuto, false);
  assert.equal(model.activeValue, "medium");
  assert.equal(model.activeDescription, "Balances speed and reasoning depth");
});

test("reasoningSliderModel separates Auto from the Claude ordinal scale", () => {
  const model = reasoningSliderModel(CLAUDE_OPUS_REASONING, "xhigh");
  assert.equal(model.hasAuto, true);
  assert.equal(model.stops.length, 5);
  assert.equal(model.stops[0].value, "low", "Auto is removed from ordinal stops");
  assert.equal(model.ordinalIndex, 3);
  assert.equal(model.isAuto, false);
  assert.equal(model.activeValue, "xhigh");
});

test("reasoningSliderModel marks Auto selection and parks slider at a sane fallback", () => {
  const model = reasoningSliderModel(CLAUDE_OPUS_REASONING, "auto");
  assert.equal(model.isAuto, true);
  assert.equal(model.activeValue, "auto");
  assert.equal(model.activeDescription, "Let the model choose the effort");
  // 5 ordinal stops -> middle fallback index 2 (High) while Auto is active.
  assert.equal(model.ordinalIndex, 2);
});

// --- buildReasoningField ---------------------------------------------------

test("buildReasoningField renders a snapped range over ordinal stops", () => {
  const doc = bootDom();
  const sent = [];
  const field = buildReasoningField(doc, {
    label: "Reasoning",
    options: CODEX_REASONING,
    selectedValue: "medium",
    onChange: (v) => sent.push(v),
  });
  const range = field.querySelector('input[type="range"]');
  assert.ok(range, "reasoning field must render a range slider");
  assert.equal(range.getAttribute("max"), "3", "max = stops - 1");
  assert.equal(range.value, "1", "slider starts at the selected stop");
  // No Auto toggle for Codex (no auto option).
  assert.equal(field.querySelector('[data-reasoning-auto]'), null);

  // `input` (dragging) previews locally only — it must NOT commit to the
  // backend, or the resulting wizard re-render would destroy the slider
  // mid-drag and break the drag (regression: slider "uns operable").
  dispatchRange(doc, range, 2, "input");
  assert.deepEqual(sent, [], "input (drag) previews locally without committing");
  const desc = field.querySelector("[data-reasoning-description]");
  assert.equal(desc.textContent, "Greater reasoning depth", "description previews live during drag");

  // `change` (release / keyboard step) commits the stop's stored value once.
  dispatchRange(doc, range, 2, "change");
  assert.deepEqual(sent, ["high"], "change commits the stop's stored value");
});

test("buildReasoningField is data-driven: any backend stop count (e.g. Ultracode above Max) is rendered", () => {
  // SPEC-2014 US-34 (handled in the backend by another agent) adds a Claude
  // "Ultracode" reasoning level above Max. The slider must render whatever
  // ordinal stops the backend sends with no frontend change — proving the
  // data-driven contract for this and any future level.
  const doc = bootDom();
  const sent = [];
  const OPUS_WITH_ULTRACODE = [
    ...CLAUDE_OPUS_REASONING, // auto, low, medium, high, xhigh, max
    { value: "ultracode", label: "Ultracode", description: "Maximum effort with extended workflows" },
  ];
  const field = buildReasoningField(doc, {
    label: "Reasoning",
    options: OPUS_WITH_ULTRACODE,
    selectedValue: "max",
    onChange: (v) => sent.push(v),
  });
  const range = field.querySelector('input[type="range"]');
  // Auto is lifted out -> 6 ordinal stops (low..ultracode); max index = 5.
  assert.equal(range.getAttribute("max"), "5", "slider scales to the backend stop count");
  const ticks = Array.from(field.querySelectorAll(".launch-range__tick")).map((t) => t.textContent.trim());
  assert.deepEqual(ticks, ["Low", "Medium", "High", "xHigh", "Max", "Ultracode"]);
  assert.equal(ticks.at(-1), "Ultracode", "Ultracode renders as the top stop, above Max");
  // Committing the top stop sends the backend stored value verbatim.
  dispatchRange(doc, range, 5, "change");
  assert.deepEqual(sent, ["ultracode"], "selecting the top stop commits 'ultracode'");
});

test("buildReasoningField exposes an Auto toggle that suspends the slider for Claude", () => {
  const doc = bootDom();
  const sent = [];
  const field = buildReasoningField(doc, {
    label: "Reasoning",
    options: CLAUDE_OPUS_REASONING,
    selectedValue: "xhigh",
    onChange: (v) => sent.push(v),
  });
  const autoToggle = field.querySelector("[data-reasoning-auto] input[type=\"checkbox\"]");
  assert.ok(autoToggle, "Claude reasoning must offer an Auto toggle");
  const range = field.querySelector('input[type="range"]');
  assert.equal(range.disabled, false, "slider is active when Auto is off");

  changeChecked(doc, autoToggle, true);
  assert.equal(sent.at(-1), "auto", "enabling Auto sends 'auto'");
  assert.equal(range.disabled, true, "slider is suspended while Auto is on");

  changeChecked(doc, autoToggle, false);
  assert.equal(range.disabled, false, "disabling Auto re-enables the slider");
  assert.notEqual(sent.at(-1), "auto", "disabling Auto sends an ordinal stop value");
});

// --- buildSegmentedField ---------------------------------------------------

test("buildSegmentedField builds an accessible radiogroup", () => {
  const doc = bootDom();
  const sent = [];
  const field = buildSegmentedField(doc, {
    label: "Target",
    options: TARGET_OPTIONS,
    selectedValue: "agent",
    onChange: (v) => sent.push(v),
  });
  const group = field.querySelector('[role="radiogroup"]');
  assert.ok(group, "segmented field must expose role=radiogroup");
  const radios = group.querySelectorAll('[role="radio"]');
  assert.equal(radios.length, 2);
  const checked = Array.from(radios).filter((r) => r.getAttribute("aria-checked") === "true");
  assert.equal(checked.length, 1, "exactly one option is checked");
  assert.equal(checked[0].dataset.value, "agent");

  trackFocus(doc, field);
  click(doc, radios[1]);
  assert.deepEqual(sent, ["shell"], "clicking an option emits its value");
});

test("buildSegmentedField moves a roving tabindex with arrow keys", () => {
  const doc = bootDom();
  const field = buildSegmentedField(doc, {
    label: "Target",
    options: TARGET_OPTIONS,
    selectedValue: "agent",
    onChange: () => {},
  });
  trackFocus(doc, field);
  const group = field.querySelector('[role="radiogroup"]');
  const radios = Array.from(group.querySelectorAll('[role="radio"]'));
  assert.equal(radios[0].getAttribute("tabindex"), "0");
  assert.equal(radios[1].getAttribute("tabindex"), "-1");

  radios[0].focus();
  keydown(doc, group, "ArrowRight");
  assert.equal(doc.activeElement?.dataset.value, "shell", "ArrowRight moves focus to the next option");
});

// --- buildToggleField ------------------------------------------------------

test("buildToggleField keeps a real checkbox under the switch styling", () => {
  const doc = bootDom();
  const sent = [];
  const field = buildToggleField(doc, {
    label: "Permissions",
    copy: "Skip permission prompts",
    checked: false,
    onChange: (v) => sent.push(v),
  });
  const input = field.querySelector('input[type="checkbox"]');
  assert.ok(input, "toggle must keep an internal checkbox for a11y");
  assert.equal(input.checked, false);

  changeChecked(doc, input, true);
  assert.deepEqual(sent, [true], "toggling emits the checkbox state");
});

// --- buildChoiceOrSelectField (adaptive dispatch) --------------------------

test("buildChoiceOrSelectField dispatches few options to a segmented control", () => {
  const doc = bootDom();
  const field = buildChoiceOrSelectField(doc, {
    label: "Agent",
    options: AGENT_OPTIONS_FEW,
    selectedValue: "claude",
    onChange: () => {},
  });
  assert.ok(field.querySelector('[role="radiogroup"]'), "few agents render as segmented");
  assert.equal(field.querySelector("select"), null);
});

test("buildChoiceOrSelectField falls back to a dropdown when options overflow", () => {
  const doc = bootDom();
  const field = buildChoiceOrSelectField(doc, {
    label: "Agent",
    options: AGENT_OPTIONS_MANY,
    selectedValue: "claude",
    onChange: () => {},
  });
  assert.ok(field.querySelector("select"), "overflowing agent list falls back to a dropdown");
  assert.equal(field.querySelector('[role="radiogroup"]'), null);
});

// --- app.js wiring: interaction guard covers the slider -------------------

const appSource = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "../app.js"),
  "utf8",
);

test("app.js extends the wizard interaction guard to the reasoning slider", () => {
  // The guard previously covered only native <select> (Issue #2698). The
  // slider must be guarded too, or its set_reasoning re-render destroys the
  // slider mid-drag. Activation is on focusin (mouse + keyboard) and release
  // on focusout so the whole interaction is coalesced.
  assert.match(
    appSource,
    /isGuardedRange\s*=\s*\(el\)\s*=>[\s\S]*?launch-range__input/,
    "expected a guarded-range predicate keyed on .launch-range__input",
  );
  assert.match(
    appSource,
    /addEventListener\(\s*"focusin"[\s\S]*?isGuardedRange\(event\.target\)[\s\S]*?wizardInteractionGuard\.activate\(\)/,
    "expected focusin on the slider to activate the wizard interaction guard",
  );
  assert.match(
    appSource,
    /addEventListener\(\s*"focusout"[\s\S]*?isGuardedRange\(target\)[\s\S]*?wizardInteractionGuard\.release\(\)/,
    "expected focusout on the slider to release the wizard interaction guard",
  );
});
