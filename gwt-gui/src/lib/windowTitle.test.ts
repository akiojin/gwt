import { describe, expect, it } from "vitest";
import { formatAboutVersion, formatWindowTitle } from "./windowTitle";

describe("formatWindowTitle", () => {
  it("shows only app name when no project is open", () => {
    expect(
      formatWindowTitle({
        appName: "gwt",
        projectPath: null,
      })
    ).toBe("gwt");
  });

  it("shows project path in window title", () => {
    expect(
      formatWindowTitle({
        appName: "gwt",
        projectPath: "/tmp/repo",
      })
    ).toBe("/tmp/repo");
  });
});

describe("formatAboutVersion", () => {
  it("shows concrete version when available", () => {
    expect(formatAboutVersion("6.30.3")).toBe("Version 6.30.3");
  });

  it("falls back to unknown when version is missing", () => {
    expect(formatAboutVersion(null)).toBe("Version unknown");
    expect(formatAboutVersion("   ")).toBe("Version unknown");
  });
});
