import { useKeyboard } from "@opentui/solid";
import type { KeyEvent } from "@opentui/core";
import {
  defaultKeybindings,
  getKeybindingsForContext,
  type KeybindingDef,
  type Modifiers,
  type UIAction,
} from "../../core/keybindings.js";

type MaybeAccessor<T> = T | (() => T);

export interface UseKeyHandlerOptions {
  context?: MaybeAccessor<string | undefined>;
  keybindings?: MaybeAccessor<KeybindingDef[] | undefined>;
  isActive?: MaybeAccessor<boolean>;
  onAction: (action: UIAction, event: KeyEvent) => void;
}

const NORMALIZED_KEY_NAMES: Record<string, string> = {
  enter: "return",
  linefeed: "return",
  pageup: "pageUp",
  pagedown: "pageDown",
  esc: "escape",
};

const resolveMaybe = <T>(value: MaybeAccessor<T> | undefined): T | undefined =>
  typeof value === "function" ? (value as () => T)() : value;

const normalizeKeyName = (name: string): string =>
  NORMALIZED_KEY_NAMES[name] ?? name;

const normalizeModifiers = (modifiers?: Modifiers) => ({
  ctrl: modifiers?.ctrl ?? false,
  shift: modifiers?.shift ?? false,
  alt: modifiers?.alt ?? false,
  meta: modifiers?.meta ?? false,
});

const getEventModifiers = (event: KeyEvent) => ({
  ctrl: event.ctrl,
  shift: event.shift,
  alt: Boolean(event.option || event.meta),
  meta: Boolean(event.super || event.hyper),
});

const resolveBindings = (
  context?: string,
  keybindings?: KeybindingDef[],
): KeybindingDef[] => {
  if (keybindings !== undefined) {
    return keybindings;
  }
  if (context) {
    return getKeybindingsForContext(context);
  }
  return defaultKeybindings;
};

const matchesKeybinding = (
  binding: KeybindingDef,
  event: KeyEvent,
): boolean => {
  const eventKey = normalizeKeyName(event.name);
  if (eventKey !== binding.key.key) {
    return false;
  }

  const expected = normalizeModifiers(binding.key.modifiers);
  const actual = getEventModifiers(event);

  return (
    expected.ctrl === actual.ctrl &&
    expected.shift === actual.shift &&
    expected.alt === actual.alt &&
    expected.meta === actual.meta
  );
};

/**
 * OpenTUI keyboard handler that maps key events to UI actions.
 */
export function useKeyHandler(options: UseKeyHandlerOptions): void {
  useKeyboard((event) => {
    if (resolveMaybe(options.isActive) === false) {
      return;
    }

    const context = resolveMaybe(options.context);
    const bindings = resolveBindings(
      context,
      resolveMaybe(options.keybindings),
    );

    for (const binding of bindings) {
      if (matchesKeybinding(binding, event)) {
        options.onAction(binding.action, event);
        return;
      }
    }
  });
}
