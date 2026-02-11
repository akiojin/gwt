import { describe, expect, it } from "vitest";
import { isCtrlCShortcut, isPasteShortcut, type ShortcutKeyEvent } from "./shortcuts";

function evt(input: Partial<ShortcutKeyEvent>): ShortcutKeyEvent {
  return {
    key: "",
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    ...input,
  };
}

describe("terminal shortcuts", () => {
  it("detects ctrl+c only when modifier set matches", () => {
    expect(isCtrlCShortcut(evt({ key: "c", ctrlKey: true }))).toBe(true);
    expect(isCtrlCShortcut(evt({ key: "C", ctrlKey: true }))).toBe(true);
    expect(isCtrlCShortcut(evt({ key: "c", ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(isCtrlCShortcut(evt({ key: "c", metaKey: true }))).toBe(false);
  });

  it("detects paste for cmd+v", () => {
    expect(isPasteShortcut(evt({ key: "v", metaKey: true }))).toBe(true);
    expect(isPasteShortcut(evt({ key: "V", metaKey: true }))).toBe(true);
    expect(isPasteShortcut(evt({ key: "v", metaKey: true, shiftKey: true }))).toBe(false);
  });

  it("detects paste for ctrl+shift+v", () => {
    expect(isPasteShortcut(evt({ key: "v", ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(isPasteShortcut(evt({ key: "V", ctrlKey: true, shiftKey: true }))).toBe(true);
    expect(isPasteShortcut(evt({ key: "v", ctrlKey: true }))).toBe(false);
  });
});

