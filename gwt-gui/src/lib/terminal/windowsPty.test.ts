import { describe, expect, it } from "vitest";

import {
  buildWindowsPtyOptions,
  parseWindowsBuildNumber,
  resolveWindowsPtyOptions,
} from "./windowsPty";

describe("windowsPty", () => {
  it("parses the Windows build number from a report string", () => {
    expect(
      parseWindowsBuildNumber(
        "Microsoft Windows [Version 10.0.26200.7840]",
      ),
    ).toBe(26200);
  });

  it("ignores non-Windows version strings without a build number", () => {
    expect(parseWindowsBuildNumber("Darwin 24.3.0")).toBeUndefined();
    expect(parseWindowsBuildNumber("")).toBeUndefined();
  });

  it("returns ConPTY options on Windows when the build number is known", () => {
    expect(buildWindowsPtyOptions("Windows", 26200)).toEqual({
      backend: "conpty",
      buildNumber: 26200,
    });
  });

  it("returns ConPTY options on Windows even when the build number is unknown", () => {
    expect(buildWindowsPtyOptions("Win32")).toEqual({
      backend: "conpty",
    });
  });

  it("does not enable Windows PTY options on non-Windows platforms", () => {
    expect(buildWindowsPtyOptions("MacIntel", 26200)).toBeUndefined();
    expect(buildWindowsPtyOptions("Linux x86_64", 26200)).toBeUndefined();
  });

  it("resolves Windows PTY options from navigator and window state", () => {
    expect(
      resolveWindowsPtyOptions(
        {
          platform: "Win32",
          userAgentData: { platform: "Windows" },
        },
        {
          __gwtWindowsPtyBuildNumber: 26200,
        },
      ),
    ).toEqual({
      backend: "conpty",
      buildNumber: 26200,
    });
  });
});
