import stringWidth from "string-width";
import { describe, it, expect } from "vitest";
import { formatBranchChoiceLine } from "../../../src/ui/legacy/prompts";

const stripAnsi = (value: string) => value.replace(/\u001B\[[0-9;]*m/g, "");

describe("formatBranchChoiceLine", () => {
  const baseLine = "⚡🟢✏️ L  feature/login";
  const maxWidth = stringWidth(baseLine);

  it("highlights the entire line when color is supported", () => {
    const result = formatBranchChoiceLine(baseLine, {
      isSelected: true,
      supportsColor: true,
      maxWidth,
    });

    expect(result.startsWith("> ")).toBe(true);
    expect(result.slice(2)).toBe(baseLine);
  });

  it("adds a simple prefix when colorは利用できない", () => {
    const result = formatBranchChoiceLine(baseLine, {
      isSelected: true,
      supportsColor: false,
      maxWidth,
    });

    expect(result.startsWith("> ")).toBe(true);
    expect(result.slice(2)).toBe(baseLine);
  });

  it("returns the original line when not selected", () => {
    const colorLine = formatBranchChoiceLine(baseLine, {
      isSelected: false,
      supportsColor: true,
      maxWidth,
    });
    const monoLine = formatBranchChoiceLine(baseLine, {
      isSelected: false,
      supportsColor: false,
      maxWidth,
    });

    expect(colorLine.startsWith("  ")).toBe(true);
    expect(colorLine.slice(2)).toBe(baseLine);
    expect(monoLine.startsWith("  ")).toBe(true);
    expect(monoLine.slice(2)).toBe(baseLine);
  });
});
