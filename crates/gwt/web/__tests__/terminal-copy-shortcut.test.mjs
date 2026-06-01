import { test } from "node:test";
import assert from "node:assert/strict";
import { classifyTerminalCopyKeyEvent } from "../terminal-copy-shortcut.js";

test("Windows Ctrl+C copies only when a terminal selection exists", () => {
  assert.deepEqual(
    classifyTerminalCopyKeyEvent(keyEvent({ ctrlKey: true, key: "c" }), {
      platform: "Windows",
      hasSelection: true,
    }),
    { copy: true, clearSelectionAfterCopy: true },
  );

  assert.deepEqual(
    classifyTerminalCopyKeyEvent(keyEvent({ ctrlKey: true, key: "c" }), {
      platform: "Windows",
      hasSelection: false,
    }),
    { copy: false, clearSelectionAfterCopy: false },
  );
});

test("Windows Ctrl+C copy ignores keyup so it cannot copy twice", () => {
  assert.deepEqual(
    classifyTerminalCopyKeyEvent(keyEvent({ type: "keyup", ctrlKey: true, key: "c" }), {
      platform: "Win32",
      hasSelection: true,
    }),
    { copy: false, clearSelectionAfterCopy: false },
  );
});

test("Linux keeps Ctrl+C on the PTY path even when a selection exists", () => {
  assert.deepEqual(
    classifyTerminalCopyKeyEvent(keyEvent({ ctrlKey: true, key: "c" }), {
      platform: "Linux x86_64",
      hasSelection: true,
    }),
    { copy: false, clearSelectionAfterCopy: false },
  );
});

test("non-macOS Ctrl+Shift+C remains the explicit copy shortcut", () => {
  assert.deepEqual(
    classifyTerminalCopyKeyEvent(keyEvent({ ctrlKey: true, shiftKey: true, key: "C" }), {
      platform: "Windows",
      hasSelection: true,
    }),
    { copy: true, clearSelectionAfterCopy: false },
  );

  assert.deepEqual(
    classifyTerminalCopyKeyEvent(keyEvent({ ctrlKey: true, shiftKey: true, key: "c" }), {
      platform: "Linux",
      hasSelection: true,
    }),
    { copy: true, clearSelectionAfterCopy: false },
  );
});

test("macOS keeps terminal copy shortcuts out of the Ctrl-based handler", () => {
  assert.deepEqual(
    classifyTerminalCopyKeyEvent(keyEvent({ ctrlKey: true, shiftKey: true, key: "c" }), {
      platform: "MacIntel",
      hasSelection: true,
    }),
    { copy: false, clearSelectionAfterCopy: false },
  );
});

function keyEvent({
  type = "keydown",
  ctrlKey = false,
  shiftKey = false,
  altKey = false,
  metaKey = false,
  key = "",
} = {}) {
  return { type, ctrlKey, shiftKey, altKey, metaKey, key };
}
