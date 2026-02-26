import { describe, it, expect } from "vitest";
import { renderBar, usageColorClass, formatMemory } from "./statusBarHelpers";

describe("statusBarHelpers", () => {
  describe("renderBar", () => {
    it("renders 0% as all spaces", () => {
      expect(renderBar(0)).toBe("[        ]");
    });

    it("renders 100% as all filled", () => {
      expect(renderBar(100)).toBe("[||||||||]");
    });

    it("renders 50% as half filled", () => {
      expect(renderBar(50)).toBe("[||||    ]");
    });

    it("renders 25% as two filled", () => {
      expect(renderBar(25)).toBe("[||      ]");
    });

    it("renders 75% as six filled", () => {
      expect(renderBar(75)).toBe("[||||||  ]");
    });

    it("renders 12.5% as one filled (rounding)", () => {
      expect(renderBar(12.5)).toBe("[|       ]");
    });
  });

  describe("usageColorClass", () => {
    it("returns ok for 0%", () => {
      expect(usageColorClass(0)).toBe("ok");
    });

    it("returns ok for 69%", () => {
      expect(usageColorClass(69)).toBe("ok");
    });

    it("returns warn for exactly 70%", () => {
      expect(usageColorClass(70)).toBe("warn");
    });

    it("returns warn for 89%", () => {
      expect(usageColorClass(89)).toBe("warn");
    });

    it("returns bad for exactly 90%", () => {
      expect(usageColorClass(90)).toBe("bad");
    });

    it("returns bad for 100%", () => {
      expect(usageColorClass(100)).toBe("bad");
    });
  });

  describe("formatMemory", () => {
    it("formats 0 bytes as 0.0", () => {
      expect(formatMemory(0)).toBe("0.0");
    });

    it("formats 1 GB", () => {
      expect(formatMemory(1073741824)).toBe("1.0");
    });

    it("formats 8 GB", () => {
      expect(formatMemory(8589934592)).toBe("8.0");
    });

    it("formats 16 GB", () => {
      expect(formatMemory(17179869184)).toBe("16.0");
    });

    it("formats fractional GB (4.5 GB)", () => {
      expect(formatMemory(4831838208)).toBe("4.5");
    });

    it("formats small values", () => {
      // 512 MB = 536870912 bytes = 0.5 GB
      expect(formatMemory(536870912)).toBe("0.5");
    });
  });
});
