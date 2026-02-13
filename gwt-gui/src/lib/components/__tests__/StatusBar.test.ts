import { describe, it, expect } from "vitest";
import { renderBar, usageColorClass, formatMemory } from "../statusBarHelpers";

describe("StatusBar helpers", () => {
  describe("renderBar", () => {
    it("renders 50% as [||||    ]", () => {
      expect(renderBar(50)).toBe("[||||    ]");
    });

    it("renders 0% as [        ]", () => {
      expect(renderBar(0)).toBe("[        ]");
    });

    it("renders 100% as [||||||||]", () => {
      expect(renderBar(100)).toBe("[||||||||]");
    });

    it("renders 25% as [||      ]", () => {
      expect(renderBar(25)).toBe("[||      ]");
    });
  });

  describe("usageColorClass", () => {
    it("returns 'ok' for usage below 70%", () => {
      expect(usageColorClass(0)).toBe("ok");
      expect(usageColorClass(50)).toBe("ok");
      expect(usageColorClass(69)).toBe("ok");
    });

    it("returns 'warn' for 70-89%", () => {
      expect(usageColorClass(70)).toBe("warn");
      expect(usageColorClass(75)).toBe("warn");
      expect(usageColorClass(89)).toBe("warn");
    });

    it("returns 'bad' for 90% and above", () => {
      expect(usageColorClass(90)).toBe("bad");
      expect(usageColorClass(95)).toBe("bad");
      expect(usageColorClass(100)).toBe("bad");
    });
  });

  describe("formatMemory", () => {
    it("formats 8 GB correctly", () => {
      expect(formatMemory(8589934592)).toBe("8.0");
    });

    it("formats 16 GB correctly", () => {
      expect(formatMemory(17179869184)).toBe("16.0");
    });

    it("formats 0 bytes correctly", () => {
      expect(formatMemory(0)).toBe("0.0");
    });

    it("formats fractional GB correctly", () => {
      // 4.5 GB = 4831838208
      expect(formatMemory(4831838208)).toBe("4.5");
    });
  });
});
