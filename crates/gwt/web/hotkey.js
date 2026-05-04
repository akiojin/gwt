// SPEC-2356 — Operator Design System: Hotkey Manager.
// Owns the `Hotkey Registry` entity. Combos use the form "cmd+p", "cmd+shift+?", etc.
// "cmd" matches both metaKey (macOS Command) and ctrlKey (Windows/Linux Ctrl).

const EDITABLE_TAGS = new Set(["INPUT", "TEXTAREA", "SELECT"]);

export function parseCombo(combo) {
  const parts = String(combo).toLowerCase().split("+").map((p) => p.trim()).filter(Boolean);
  let mod = false, alt = false, shift = false, key = "";
  for (const p of parts) {
    if (p === "cmd" || p === "ctrl" || p === "meta" || p === "mod") mod = true;
    else if (p === "alt" || p === "option" || p === "opt") alt = true;
    else if (p === "shift") shift = true;
    else key = p;
  }
  if (!key) throw new Error(`hotkey combo missing key: ${combo}`);
  return { mod, alt, shift, key };
}

export function comboMatches(combo, event) {
  const eventMod = !!event.metaKey || !!event.ctrlKey;
  if (combo.mod !== eventMod) return false;
  if (combo.alt !== !!event.altKey) return false;
  if (combo.shift !== !!event.shiftKey) return false;
  const eventKey = String(event.key ?? "").toLowerCase();
  return eventKey === combo.key;
}

export function createHotkeyManager() {
  const bindings = new Map();

  function register(combo, handler) {
    const key = canonical(combo);
    if (bindings.has(key)) throw new Error(`hotkey "${combo}" already registered`);
    bindings.set(key, { combo: parseCombo(combo), handler });
  }

  function unregister(combo) {
    bindings.delete(canonical(combo));
  }

  function dispatch(event) {
    if (isEditableTarget(event.target)) return false;
    for (const { combo, handler } of bindings.values()) {
      if (comboMatches(combo, event)) {
        const consumed = handler(event);
        if (consumed) {
          event.preventDefault?.();
          event.stopPropagation?.();
          return true;
        }
      }
    }
    return false;
  }

  function attach(target) {
    const listener = (e) => { dispatch(e); };
    target.addEventListener("keydown", listener);
    return () => target.removeEventListener("keydown", listener);
  }

  return { register, unregister, dispatch, attach, list: () => Array.from(bindings.keys()) };
}

function canonical(combo) {
  const c = parseCombo(combo);
  const segs = [];
  if (c.mod) segs.push("mod");
  if (c.alt) segs.push("alt");
  if (c.shift) segs.push("shift");
  segs.push(c.key);
  return segs.join("+");
}

function isEditableTarget(target) {
  if (!target) return false;
  if (target.dataset?.hotkeyOverride === "true") return false;
  if (target.isContentEditable) return true;
  return EDITABLE_TAGS.has(target.tagName);
}
