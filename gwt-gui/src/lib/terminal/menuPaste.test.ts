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
});
