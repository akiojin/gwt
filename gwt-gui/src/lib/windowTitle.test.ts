import { describe, expect, it } from "vitest";
import { formatWindowTitle } from "./windowTitle";

describe("formatWindowTitle", () => {
  it("includes version when available", () => {
    expect(
      formatWindowTitle({
        appName: "gwt",
        version: "6.30.3",
        projectPath: null,
      })
    ).toBe("gwt v6.30.3");
  });

  it("includes version and project path", () => {
    expect(
      formatWindowTitle({
        appName: "gwt",
        version: "6.30.3",
        projectPath: "/tmp/repo",
      })
    ).toBe("gwt v6.30.3 - /tmp/repo");
  });

  it("falls back to the previous format when version is missing", () => {
    expect(
      formatWindowTitle({
        appName: "gwt",
        version: null,
        projectPath: "/tmp/repo",
      })
    ).toBe("gwt - /tmp/repo");

    expect(
      formatWindowTitle({
        appName: "gwt",
        version: null,
        projectPath: null,
      })
    ).toBe("gwt");
  });

  it("treats blank version as missing", () => {
    expect(
      formatWindowTitle({
        appName: "gwt",
        version: "   ",
        projectPath: null,
      })
    ).toBe("gwt");
  });
});

