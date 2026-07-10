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

// SPEC-1921 US-20 / FR-122 — Codex reasoning ladders scale per model. The
// backend sends 6 stops for gpt-5.6-sol / gpt-5.6-terra (low..ultra), 5 stops
// for gpt-5.6-luna (low..max), and the existing 4 stops for gpt-5.5 / gpt-5.4 /
// gpt-5.4-mini / gpt-5.3-codex-spark (low..xhigh). There is NO Auto row for
// Codex — the whole ladder is ordinal, so no stop is lifted out of the slider.
const CODEX_REASONING_6 = [
  { value: "low", label: "Low", description: "Fast responses with lighter reasoning" },
  { value: "medium", label: "Medium", description: "Balances speed and reasoning depth" },
  { value: "high", label: "High", description: "Greater reasoning depth" },
  { value: "xhigh", label: "Extra high", description: "Maximum reasoning depth" },
  { value: "max", label: "Max", description: "Maximum reasoning depth for the hardest problems" },
  { value: "ultra", label: "Ultra", description: "Maximum reasoning with automatic task delegation" },
];

const CODEX_REASONING_5 = [
  { value: "low", label: "Low", description: "Fast responses with lighter reasoning" },
  { value: "medium", label: "Medium", description: "Balances speed and reasoning depth" },
  { value: "high", label: "High", description: "Greater reasoning depth" },
  { value: "xhigh", label: "Extra high", description: "Maximum reasoning depth" },
  { value: "max", label: "Max", description: "Maximum reasoning depth for the hardest problems" },
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
  { value: "agy", label: "Antigravity CLI" },
  { value: "gemini", label: "Gemini CLI (legacy)" },
];

const AGENT_OPTIONS_MANY = [
  { value: "claude", label: "Claude Code" },
  { value: "codex", label: "Codex" },
  { value: "agy", label: "Antigravity CLI" },
  { value: "gemini", label: "Gemini CLI (legacy)" },
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
});

test("chooseLaunchControlKind falls back to select for current built-in agent labels", () => {
  assert.equal(chooseLaunchControlKind(AGENT_OPTIONS_FEW), "select");
});

test("chooseLaunchControlKind falls back to select past the count threshold", () => {
  // 6 options exceeds the default 4-option segmented budget (Agent w/ custom).
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

test("reasoningSliderModel maps the 6-stop Codex ladder low->ultra with no Auto lift-out", () => {
  // SPEC-1921 US-20 / FR-122 / SC-030 — gpt-5.6-sol / gpt-5.6-terra send six
  // ordinal Codex stops (low..ultra) and no Auto row, so the whole ladder stays
  // on the ordinal scale and ultra is the top stop.
  const model = reasoningSliderModel(CODEX_REASONING_6, "ultra");
  assert.equal(model.hasAuto, false, "Codex has no Auto row");
  assert.equal(model.stops.length, 6);
  assert.deepEqual(
    model.stops.map((stop) => stop.value),
    ["low", "medium", "high", "xhigh", "max", "ultra"],
    "six ordinal stops in ladder order",
  );
  assert.equal(model.ordinalIndex, 5, "ultra is the top stop");
  assert.equal(model.isAuto, false);
  assert.equal(model.activeValue, "ultra");
  assert.equal(model.activeDescription, "Maximum reasoning with automatic task delegation");
});

test("reasoningSliderModel maps the 5-stop Codex ladder low->max", () => {
  // SPEC-1921 US-20 / FR-122 / SC-030 — gpt-5.6-luna sends five ordinal stops
  // (low..max) with no Auto row, topping out at max.
  const model = reasoningSliderModel(CODEX_REASONING_5, "max");
  assert.equal(model.hasAuto, false, "Codex has no Auto row");
  assert.equal(model.stops.length, 5);
  assert.deepEqual(
    model.stops.map((stop) => stop.value),
    ["low", "medium", "high", "xhigh", "max"],
    "five ordinal stops in ladder order",
  );
  assert.equal(model.ordinalIndex, 4, "max is the top stop");
  assert.equal(model.isAuto, false);
  assert.equal(model.activeValue, "max");
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

test("buildReasoningField renders the 6-stop Codex ladder and commits 'ultra' at the top", () => {
  // SPEC-1921 US-20 / FR-122 / SC-030 — the data-driven slider scales to six
  // Codex stops (low..ultra) with no Auto toggle; releasing on the top stop
  // commits the backend stored value 'ultra'.
  const doc = bootDom();
  const sent = [];
  const field = buildReasoningField(doc, {
    label: "Reasoning",
    options: CODEX_REASONING_6,
    selectedValue: "low",
    onChange: (v) => sent.push(v),
  });
  const range = field.querySelector('input[type="range"]');
  assert.ok(range, "reasoning field must render a range slider");
  assert.equal(range.getAttribute("max"), "5", "six stops -> max index 5");
  assert.equal(field.querySelector("[data-reasoning-auto]"), null, "Codex has no Auto toggle");
  const ticks = Array.from(field.querySelectorAll(".launch-range__tick")).map((t) => t.textContent.trim());
  assert.deepEqual(ticks, ["Low", "Medium", "High", "Extra high", "Max", "Ultra"]);
  assert.equal(ticks.at(-1), "Ultra", "Ultra renders as the top stop, above Max");

  // Drag preview never commits (a commit would re-render and destroy the slider
  // mid-drag); only release/keyboard-step commits, and it commits exactly once.
  dispatchRange(doc, range, 5, "input");
  assert.deepEqual(sent, [], "input (drag) previews locally without committing");
  dispatchRange(doc, range, 5, "change");
  assert.deepEqual(sent, ["ultra"], "committing the top stop commits stored value 'ultra'");
});

test("buildReasoningField renders the 5-stop Codex ladder and commits 'max' at the top", () => {
  // SPEC-1921 US-20 / FR-122 / SC-030 — gpt-5.6-luna's five-stop ladder tops out
  // at Max; releasing on the top stop commits the backend stored value 'max'.
  const doc = bootDom();
  const sent = [];
  const field = buildReasoningField(doc, {
    label: "Reasoning",
    options: CODEX_REASONING_5,
    selectedValue: "low",
    onChange: (v) => sent.push(v),
  });
  const range = field.querySelector('input[type="range"]');
  assert.equal(range.getAttribute("max"), "4", "five stops -> max index 4");
  assert.equal(field.querySelector("[data-reasoning-auto]"), null, "Codex has no Auto toggle");
  const ticks = Array.from(field.querySelectorAll(".launch-range__tick")).map((t) => t.textContent.trim());
  assert.deepEqual(ticks, ["Low", "Medium", "High", "Extra high", "Max"]);
  dispatchRange(doc, range, 4, "change");
  assert.deepEqual(sent, ["max"], "committing the top stop commits stored value 'max'");
});

test("buildReasoningField commits 'xhigh' at the top of the 4-stop Codex ladder", () => {
  // SPEC-1921 US-20 / FR-122 / SC-030 — gpt-5.5 / gpt-5.4 / gpt-5.4-mini /
  // gpt-5.3-codex-spark keep the four-stop ladder that tops out at Extra high /
  // stored value 'xhigh'.
  const doc = bootDom();
  const sent = [];
  const field = buildReasoningField(doc, {
    label: "Reasoning",
    options: CODEX_REASONING,
    selectedValue: "low",
    onChange: (v) => sent.push(v),
  });
  const range = field.querySelector('input[type="range"]');
  assert.equal(range.getAttribute("max"), "3", "four stops -> max index 3");
  assert.equal(field.querySelector("[data-reasoning-auto]"), null, "Codex has no Auto toggle");
  const ticks = Array.from(field.querySelectorAll(".launch-range__tick")).map((t) => t.textContent.trim());
  assert.deepEqual(ticks, ["Low", "Medium", "High", "Extra high"]);
  dispatchRange(doc, range, 3, "change");
  assert.deepEqual(sent, ["xhigh"], "committing the top stop commits stored value 'xhigh'");
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

test("buildChoiceOrSelectField dispatches few short options to a segmented control", () => {
  const doc = bootDom();
  const field = buildChoiceOrSelectField(doc, {
    label: "Target",
    options: TARGET_OPTIONS,
    selectedValue: "agent",
    onChange: () => {},
  });
  assert.ok(field.querySelector('[role="radiogroup"]'), "few short options render as segmented");
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

// --- wizard surface wiring: interaction guard covers the slider -----------
// SPEC-3064 Phase 3 (E5): the wizard chrome listeners moved from app.js to
// launch-wizard-surface.js (installWizardChrome).

const wizardSource = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "../launch-wizard-surface.js"),
  "utf8",
);

test("wizard surface extends the interaction guard to the reasoning slider", () => {
  // The guard previously covered only native <select> (Issue #2698). The
  // slider must be guarded too, or its set_reasoning re-render destroys the
  // slider mid-drag. Activation is on focusin (mouse + keyboard) and release
  // on focusout so the whole interaction is coalesced.
  assert.match(
    wizardSource,
    /isGuardedRange\s*=\s*\(el\)\s*=>[\s\S]*?launch-range__input/,
    "expected a guarded-range predicate keyed on .launch-range__input",
  );
  assert.match(
    wizardSource,
    /addEventListener\(\s*"focusin"[\s\S]*?isGuardedRange\(event\.target\)[\s\S]*?wizardInteractionGuard\.activate\(\)/,
    "expected focusin on the slider to activate the wizard interaction guard",
  );
  assert.match(
    wizardSource,
    /addEventListener\(\s*"focusout"[\s\S]*?isGuardedRange\(target\)[\s\S]*?wizardInteractionGuard\.release\(\)/,
    "expected focusout on the slider to release the wizard interaction guard",
  );
});
