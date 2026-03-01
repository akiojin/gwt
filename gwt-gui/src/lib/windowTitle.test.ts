import { describe, expect, it, vi } from "vitest";
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

describe("formatWindowTitle – additional branches", () => {
  it("shows empty string projectPath as appName", () => {
    expect(
      formatWindowTitle({
        appName: "gwt",
        projectPath: "",
      })
    ).toBe("gwt");
  });
});

describe("getAppVersionSafe", () => {
  it("returns version string when Tauri API is available", async () => {
    vi.doMock("@tauri-apps/api/app", () => ({
      getVersion: () => Promise.resolve("7.0.0"),
    }));
    const { getAppVersionSafe } = await import("./windowTitle");
    const result = await getAppVersionSafe();
    expect(result).toBe("7.0.0");
    vi.doUnmock("@tauri-apps/api/app");
  });

  it("returns null when Tauri API throws", async () => {
    vi.doMock("@tauri-apps/api/app", () => ({
      getVersion: () => Promise.reject(new Error("not in tauri")),
    }));
    const mod = await import("./windowTitle");
    const result = await mod.getAppVersionSafe();
    expect(result).toBeNull();
    vi.doUnmock("@tauri-apps/api/app");
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
