import { describe, expect, it } from "vitest";
import type { StructuredError } from "./errorBus";
import {
  generateBugReportBody,
  generateFeatureRequestBody,
  type BugReportData,
  type FeatureRequestData,
} from "./issueTemplate";

function makeError(overrides: Partial<StructuredError> = {}): StructuredError {
  return {
    severity: "error",
    code: "E1001",
    message: "something went wrong",
    command: "open_project",
    category: "Git",
    suggestions: ["Try again", "Check path"],
    timestamp: "2026-01-01T00:00:00.000Z",
    ...overrides,
  };
}

describe("generateBugReportBody", () => {
  it("includes all sections when all fields are provided", () => {
    const data: BugReportData = {
      title: "App crashes on open",
      stepsToReproduce: "1. Open the app\n2. Click start",
      expectedResult: "App starts normally",
      actualResult: "App crashes with error",
      systemInfo: "macOS 15.2, M4 Pro",
      logs: "ERROR: null pointer at line 42",
      screenCapture: "Terminal: [error output here]",
      screenshotPath: "/tmp/screenshot.png",
      error: makeError(),
      gwtVersion: "7.10.0",
      platform: "darwin",
    };

    const body = generateBugReportBody(data);

    expect(body).toContain("## Bug Report");
    expect(body).toContain("### Steps to Reproduce");
    expect(body).toContain("1. Open the app");
    expect(body).toContain("### Expected Result");
    expect(body).toContain("App starts normally");
    expect(body).toContain("### Actual Result");
    expect(body).toContain("App crashes with error");
    expect(body).toContain("### Error Details");
    expect(body).toContain("**Code**: E1001");
    expect(body).toContain("**Severity**: error");
    expect(body).toContain("**Command**: open_project");
    expect(body).toContain("**Category**: Git");
    expect(body).toContain("**Suggestions**: Try again, Check path");
    expect(body).toContain("#### System");
    expect(body).toContain("macOS 15.2, M4 Pro");
    expect(body).toContain("**gwt Version**: 7.10.0");
    expect(body).toContain("**Platform**: darwin");
    expect(body).toContain("#### Screen Capture");
    expect(body).toContain("Terminal: [error output here]");
    expect(body).toContain("#### Screenshot");
    expect(body).toContain("/tmp/screenshot.png");
    expect(body).toContain("#### Application Logs");
    expect(body).toContain("null pointer at line 42");
  });

  it("uses fallback text when only required fields are provided", () => {
    const data: BugReportData = {
      title: "Bug",
      stepsToReproduce: "",
      expectedResult: "",
      actualResult: "",
    };

    const body = generateBugReportBody(data);

    expect(body).toContain("_No steps provided_");
    expect(body).toContain("_Not specified_");
    expect(body).not.toContain("### Error Details");
    expect(body).not.toContain("#### System");
    expect(body).not.toContain("#### Screen Capture");
    expect(body).not.toContain("#### Screenshot");
    expect(body).not.toContain("#### Application Logs");
  });

  it("includes error details section when error is present", () => {
    const data: BugReportData = {
      title: "Error report",
      stepsToReproduce: "Click button",
      expectedResult: "Success",
      actualResult: "Error",
      error: makeError({ suggestions: [] }),
    };

    const body = generateBugReportBody(data);

    expect(body).toContain("### Error Details");
    expect(body).toContain("**Code**: E1001");
    expect(body).not.toContain("**Suggestions**");
  });
});

describe("generateFeatureRequestBody", () => {
  it("includes all sections when all fields are provided", () => {
    const data: FeatureRequestData = {
      title: "Add dark mode",
      description: "Support system-wide dark mode",
      useCase: "Users prefer dark mode at night",
      expectedBenefit: "Better user experience",
      gwtVersion: "7.10.0",
      platform: "darwin",
    };

    const body = generateFeatureRequestBody(data);

    expect(body).toContain("## Feature Request");
    expect(body).toContain("### Description");
    expect(body).toContain("Support system-wide dark mode");
    expect(body).toContain("### Use Case");
    expect(body).toContain("Users prefer dark mode at night");
    expect(body).toContain("### Expected Benefit");
    expect(body).toContain("Better user experience");
    expect(body).toContain("**gwt Version**: 7.10.0");
    expect(body).toContain("**Platform**: darwin");
  });

  it("uses fallback text when only required fields are provided", () => {
    const data: FeatureRequestData = {
      title: "Feature",
      description: "",
      useCase: "",
      expectedBenefit: "",
    };

    const body = generateFeatureRequestBody(data);

    expect(body).toContain("_No description provided_");
    expect(body).toContain("_Not specified_");
    expect(body).toContain("**gwt Version**: unknown");
    expect(body).toContain("**Platform**: unknown");
  });
});
