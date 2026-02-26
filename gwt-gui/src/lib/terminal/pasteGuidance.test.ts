import { describe, expect, it } from "vitest";
import {
  isMacPlatform,
  isWindowsOrLinuxPlatform,
  platformName,
  shouldShowAgentPasteHint,
} from "./pasteGuidance";

describe("paste guidance helpers", () => {
  it("prefers userAgentData platform over navigator.platform", () => {
    expect(
      platformName({
        platform: "Win32",
        userAgentData: { platform: "Windows" },
      }),
    ).toBe("Windows");
  });

  it("falls back to navigator.platform when userAgentData is unavailable", () => {
    expect(platformName({ platform: "Linux x86_64", userAgentData: null })).toBe(
      "Linux x86_64",
    );
  });

  it("detects mac platforms", () => {
    expect(isMacPlatform("MacIntel")).toBe(true);
    expect(isMacPlatform("iPhone")).toBe(true);
    expect(isMacPlatform("Windows")).toBe(false);
  });

  it("detects windows and linux platforms while excluding mac", () => {
    expect(isWindowsOrLinuxPlatform("Win32")).toBe(true);
    expect(isWindowsOrLinuxPlatform("Linux x86_64")).toBe(true);
    expect(isWindowsOrLinuxPlatform("X11")).toBe(true);
    expect(isWindowsOrLinuxPlatform("MacIntel")).toBe(false);
  });

  it("shows the agent paste hint only for eligible agent tabs", () => {
    expect(
      shouldShowAgentPasteHint({
        activeTabType: "agent",
        platform: "Win32",
        dismissed: false,
        shownInSession: false,
      }),
    ).toBe(true);

    expect(
      shouldShowAgentPasteHint({
        activeTabType: "terminal",
        platform: "Win32",
        dismissed: false,
        shownInSession: false,
      }),
    ).toBe(false);

    expect(
      shouldShowAgentPasteHint({
        activeTabType: "agent",
        platform: "Win32",
        dismissed: true,
        shownInSession: false,
      }),
    ).toBe(false);

    expect(
      shouldShowAgentPasteHint({
        activeTabType: "agent",
        platform: "MacIntel",
        dismissed: false,
        shownInSession: false,
      }),
    ).toBe(false);
  });
});
