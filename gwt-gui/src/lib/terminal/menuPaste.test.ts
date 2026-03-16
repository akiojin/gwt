import { describe, expect, it, vi } from "vitest";

import { applyMenuPasteText } from "./menuPaste";

describe("applyMenuPasteText", () => {
  it("updates textareas and dispatches an input event", () => {
    const textarea = document.createElement("textarea");
    textarea.value = "hello world";
    textarea.selectionStart = 6;
    textarea.selectionEnd = 11;

    const inputHandler = vi.fn();
    textarea.addEventListener("input", inputHandler);

    const handled = applyMenuPasteText(textarea, "gwt");

    expect(handled).toBe(true);
    expect(textarea.value).toBe("hello gwt");
    expect(textarea.selectionStart).toBe(9);
    expect(textarea.selectionEnd).toBe(9);
    expect(inputHandler).toHaveBeenCalledTimes(1);
  });

  it("updates inputs and dispatches an input event", () => {
    const input = document.createElement("input");
    input.value = "abc";
    input.selectionStart = 1;
    input.selectionEnd = 2;

    const inputHandler = vi.fn();
    input.addEventListener("input", inputHandler);

    const handled = applyMenuPasteText(input, "Z");

    expect(handled).toBe(true);
    expect(input.value).toBe("aZc");
    expect(input.selectionStart).toBe(2);
    expect(input.selectionEnd).toBe(2);
    expect(inputHandler).toHaveBeenCalledTimes(1);
  });

  it("updates non-text inputs without using setRangeText", () => {
    const input = document.createElement("input");
    input.type = "number";
    input.value = "12";

    const setRangeTextSpy = vi.spyOn(input, "setRangeText");
    const inputHandler = vi.fn();
    input.addEventListener("input", inputHandler);

    const handled = applyMenuPasteText(input, "3");

    expect(handled).toBe(true);
    expect(input.value).toBe("123");
    expect(setRangeTextSpy).not.toHaveBeenCalled();
    expect(inputHandler).toHaveBeenCalledTimes(1);
  });

  it("updates contenteditable elements and dispatches an input event", () => {
    const editable = document.createElement("div");
    editable.contentEditable = "true";
    Object.defineProperty(editable, "isContentEditable", {
      configurable: true,
      value: true,
    });
    editable.textContent = "hello ";
    document.body.appendChild(editable);

    const selection = window.getSelection();
    selection?.removeAllRanges();
    const range = document.createRange();
    range.selectNodeContents(editable);
    range.collapse(false);
    selection?.addRange(range);

    const inputHandler = vi.fn();
    editable.addEventListener("input", inputHandler);

    const handled = applyMenuPasteText(editable, "gwt");
    const currentRange = selection?.getRangeAt(0);

    expect(handled).toBe(true);
    expect(editable.textContent).toBe("hello gwt");
    expect(inputHandler).toHaveBeenCalledTimes(1);
    expect(currentRange?.collapsed).toBe(true);
    expect(editable.contains(currentRange?.startContainer ?? null)).toBe(true);

    editable.remove();
    selection?.removeAllRanges();
  });

  it("returns false for unsupported targets", () => {
    const target = document.createElement("div");
    const inputHandler = vi.fn();
    target.addEventListener("input", inputHandler);

    const handled = applyMenuPasteText(target, "gwt");

    expect(handled).toBe(false);
    expect(inputHandler).not.toHaveBeenCalled();
  });
});
