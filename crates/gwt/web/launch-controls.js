// SPEC-2014 2026-05-29 amendment (FR-105..FR-110) — Launch Agent setting
// controls. Standalone, behaviorally-testable builders that replace the
// "wall of native <select>" with operation-appropriate controls:
//
//   - reasoning  -> snapped slider over the ordinal stops, with the Claude
//                   "Auto" option lifted out into a separate toggle so it is
//                   not mistaken for the weakest effort level.
//   - few short  -> accessible segmented radiogroup (Target / Runtime /
//     exclusive    Execution mode / Windows shell, and Agent while the
//     choices      candidate count stays small).
//   - many/long  -> native <select> fallback (Model / Version / Docker, and
//                   Agent once custom agents push the count past the budget).
//   - booleans   -> toggle switch that keeps a real <input type="checkbox">.
//
// Pure logic (chooseLaunchControlKind / reasoningSliderModel) is split from
// DOM building so it can be unit-tested directly. All styling lives in
// styles/app.css using SPEC-2356 design tokens; this module only sets classes,
// ARIA, and behavior. Selected values and onChange payloads are identical to
// the prior native controls, so the existing set_* wizard actions are unchanged.

const DEFAULT_MAX_SEGMENTS = 4;
const DEFAULT_MAX_LABEL_LENGTH = 16;

// --- pure logic ------------------------------------------------------------

// Decide whether a mutually-exclusive option list should render as a segmented
// control (all candidates visible, one-tap) or fall back to a dropdown.
export function chooseLaunchControlKind(options, opts = {}) {
  const list = Array.isArray(options) ? options : [];
  const maxSegments = opts.maxSegments ?? DEFAULT_MAX_SEGMENTS;
  const maxLabelLength = opts.maxLabelLength ?? DEFAULT_MAX_LABEL_LENGTH;
  if (list.length < 2) return "select";
  if (list.length > maxSegments) return "select";
  if (list.some((option) => (option?.label ?? "").length > maxLabelLength)) {
    return "select";
  }
  return "segmented";
}

// Derive the slider model for reasoning options. "auto" (Claude only) is lifted
// out of the ordinal scale; while it is active the slider parks at a sane
// middle fallback so toggling Auto off lands on a reasonable effort level.
export function reasoningSliderModel(options, selectedValue) {
  const list = Array.isArray(options) ? options : [];
  const autoOption = list.find((option) => option?.value === "auto") ?? null;
  const stops = list.filter((option) => option?.value !== "auto");
  const isAuto = selectedValue === "auto" && Boolean(autoOption);

  let ordinalIndex = stops.findIndex((option) => option?.value === selectedValue);
  if (ordinalIndex < 0) {
    ordinalIndex = stops.length > 0 ? Math.floor((stops.length - 1) / 2) : 0;
  }

  const activeStop = stops[ordinalIndex] ?? null;
  return {
    stops,
    autoOption,
    hasAuto: Boolean(autoOption),
    isAuto,
    ordinalIndex,
    activeValue: isAuto ? "auto" : activeStop?.value ?? null,
    activeDescription: isAuto
      ? autoOption?.description ?? ""
      : activeStop?.description ?? "",
  };
}

// --- DOM helpers -----------------------------------------------------------

function el(doc, tag, className, text) {
  const node = doc.createElement(tag);
  if (className) node.className = className;
  if (text != null) node.textContent = text;
  return node;
}

function makeField(doc, label, wide) {
  const field = el(doc, "div", wide ? "launch-field wide" : "launch-field");
  field.appendChild(el(doc, "div", "launch-field-label", label));
  return field;
}

// A toggle switch whose real <input type="checkbox"> overlays the visual track
// at full size (opacity 0). This keeps native checkbox semantics for screen
// readers and keyboard, and stays click/setChecked-actionable for Playwright,
// while the styled track/knob render behind it.
function makeSwitch(doc, { checked, ariaLabel }) {
  const control = el(doc, "span", "launch-toggle__control");
  const input = doc.createElement("input");
  input.setAttribute("type", "checkbox");
  input.className = "launch-toggle__input";
  input.checked = Boolean(checked);
  if (ariaLabel) input.setAttribute("aria-label", ariaLabel);
  const track = el(doc, "span", "launch-toggle__track");
  track.appendChild(el(doc, "span", "launch-toggle__knob"));
  control.appendChild(input);
  control.appendChild(track);
  return { control, input };
}

// --- segmented radiogroup --------------------------------------------------

export function buildSegmentedField(doc, { label, options, selectedValue, onChange, wide }) {
  const field = makeField(doc, label, wide);
  const group = el(doc, "div", "launch-segmented");
  group.setAttribute("role", "radiogroup");
  group.setAttribute("aria-label", label);

  const list = Array.isArray(options) ? options : [];
  const radios = list.map((option) => {
    const radio = el(doc, "button", "launch-segmented__option");
    radio.type = "button";
    radio.dataset.value = option.value;
    radio.setAttribute("role", "radio");
    const selected = option.value === selectedValue;
    radio.setAttribute("aria-checked", selected ? "true" : "false");
    radio.setAttribute("tabindex", selected ? "0" : "-1");
    if (option.color) {
      radio.dataset.agentColor = option.color;
      radio.appendChild(el(doc, "span", "agent-dot"));
    }
    radio.appendChild(doc.createTextNode(option.label));
    group.appendChild(radio);
    return radio;
  });

  const commit = (value) => {
    for (const radio of radios) {
      const isOn = radio.dataset.value === value;
      radio.setAttribute("aria-checked", isOn ? "true" : "false");
      radio.setAttribute("tabindex", isOn ? "0" : "-1");
    }
    onChange(value);
  };

  group.addEventListener("click", (event) => {
    const target = event.target;
    const radio = typeof target?.closest === "function"
      ? target.closest("[data-value]")
      : null;
    if (!radio || !group.contains(radio)) return;
    commit(radio.dataset.value);
  });

  group.addEventListener("keydown", (event) => {
    const key = event.key;
    const focused = doc.activeElement;
    const idx = radios.indexOf(focused);
    if (idx < 0) return;
    if (key === "ArrowLeft" || key === "ArrowUp" || key === "ArrowRight" || key === "ArrowDown") {
      event.preventDefault();
      const dir = key === "ArrowLeft" || key === "ArrowUp" ? -1 : 1;
      radios[(idx + dir + radios.length) % radios.length].focus();
    } else if (key === "Enter" || key === " ") {
      event.preventDefault();
      commit(focused.dataset.value);
    }
  });

  field.appendChild(group);
  return field;
}

// --- native select fallback ------------------------------------------------

export function buildSelectField(
  doc,
  { label, options, selectedValue, onChange, wide, emptyLabel = "Unavailable" },
) {
  const field = makeField(doc, label, wide);
  const select = el(doc, "select", "launch-select");
  select.setAttribute("aria-label", label);
  const list = Array.isArray(options) ? options : [];
  if (list.length === 0) {
    select.disabled = true;
    const option = doc.createElement("option");
    option.value = "";
    option.textContent = emptyLabel;
    select.appendChild(option);
  } else {
    for (const item of list) {
      const option = doc.createElement("option");
      option.value = item.value;
      option.textContent = item.label;
      select.appendChild(option);
    }
    const hasSelected = list.some((item) => item.value === selectedValue);
    const chosen = hasSelected ? selectedValue : list[0].value;
    for (const option of select.children) {
      if (option.getAttribute("value") === chosen) option.setAttribute("selected", "");
    }
    select.addEventListener("change", () => onChange(select.value));
  }
  field.appendChild(select);
  return field;
}

// --- count-adaptive dispatcher ---------------------------------------------

export function buildChoiceOrSelectField(doc, spec) {
  const kind = chooseLaunchControlKind(spec.options, spec.controlOpts ?? {});
  return kind === "segmented"
    ? buildSegmentedField(doc, spec)
    : buildSelectField(doc, spec);
}

// --- boolean toggle switch -------------------------------------------------

export function buildToggleField(doc, { label, copy, checked, onChange, wide }) {
  const field = makeField(doc, label, wide);
  const toggle = el(doc, "label", "launch-toggle");
  const { control, input } = makeSwitch(doc, { checked, ariaLabel: copy });
  input.addEventListener("change", () => onChange(input.checked));
  toggle.appendChild(control);
  toggle.appendChild(el(doc, "span", "launch-toggle__copy", copy));
  field.appendChild(toggle);
  return field;
}

// --- reasoning slider + Auto toggle ----------------------------------------

export function buildReasoningField(doc, { label, options, selectedValue, onChange }) {
  const model = reasoningSliderModel(options, selectedValue);
  const field = makeField(doc, label, true);

  // Auto lives next to the label so it reads as a separate mode, not a stop.
  if (model.hasAuto) {
    const labelEl = field.querySelector(".launch-field-label");
    const labelRow = el(doc, "div", "launch-field-label-row");
    field.replaceChild(labelRow, labelEl);
    labelRow.appendChild(labelEl);
    const auto = el(doc, "label", "launch-toggle launch-toggle--auto");
    auto.dataset.reasoningAuto = "true";
    const { control: autoControl, input: autoInput } = makeSwitch(doc, {
      checked: model.isAuto,
      ariaLabel: "Auto reasoning effort",
    });
    auto.appendChild(el(doc, "span", "launch-toggle__copy", "Auto"));
    auto.appendChild(autoControl);
    labelRow.appendChild(auto);
    field.__autoInput = autoInput;
  }

  const rangeWrap = el(doc, "div", "launch-range");
  const range = doc.createElement("input");
  range.setAttribute("type", "range");
  range.className = "launch-range__input";
  range.setAttribute("min", "0");
  range.setAttribute("max", String(Math.max(0, model.stops.length - 1)));
  range.setAttribute("step", "1");
  range.value = String(model.ordinalIndex);
  range.disabled = model.isAuto;
  range.setAttribute("aria-label", label);
  const ticks = el(doc, "div", "launch-range__ticks");
  model.stops.forEach((stop, index) => {
    const tick = el(doc, "span", "launch-range__tick", stop.label);
    tick.dataset.value = stop.value;
    if (index === model.ordinalIndex && !model.isAuto) tick.dataset.active = "true";
    ticks.appendChild(tick);
  });
  rangeWrap.appendChild(range);
  rangeWrap.appendChild(ticks);

  const description = el(doc, "div", "launch-range__description", model.activeDescription);
  description.dataset.reasoningDescription = "true";

  let currentIndex = model.ordinalIndex;
  const syncTicks = () => {
    const tickEls = ticks.querySelectorAll(".launch-range__tick");
    tickEls.forEach((tick, index) => {
      if (index === currentIndex && !range.disabled) tick.dataset.active = "true";
      else delete tick.dataset.active;
    });
  };
  const setDescription = (text) => {
    description.textContent = text;
  };
  const stopValue = (index) => model.stops[index]?.value ?? null;
  const stopDescription = (index) => model.stops[index]?.description ?? "";

  range.addEventListener("input", () => {
    currentIndex = Number(range.value) || 0;
    range.setAttribute("aria-valuetext", model.stops[currentIndex]?.label ?? "");
    setDescription(stopDescription(currentIndex));
    syncTicks();
    const value = stopValue(currentIndex);
    if (value != null) onChange(value);
  });

  if (field.__autoInput) {
    field.__autoInput.addEventListener("change", () => {
      const on = field.__autoInput.checked;
      range.disabled = on;
      rangeWrap.dataset.suspended = on ? "true" : "";
      if (on) {
        setDescription(model.autoOption?.description ?? "");
        syncTicks();
        onChange("auto");
      } else {
        setDescription(stopDescription(currentIndex));
        syncTicks();
        const value = stopValue(currentIndex);
        if (value != null) onChange(value);
      }
    });
    if (model.isAuto) rangeWrap.dataset.suspended = "true";
  }

  field.appendChild(rangeWrap);
  field.appendChild(description);
  return field;
}
