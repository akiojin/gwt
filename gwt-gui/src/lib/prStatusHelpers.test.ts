import { describe, it, expect } from "vitest";
import type { WorkflowRunInfo } from "./types";
import { workflowStatusIcon, workflowStatusClass } from "./prStatusHelpers";

function makeRun(
  overrides: Partial<WorkflowRunInfo> = {},
): WorkflowRunInfo {
  return {
    workflowName: "CI",
    runId: 1,
    status: "completed",
    conclusion: "success",
    ...overrides,
  };
}

describe("workflowStatusIcon", () => {
  it("returns check mark for completed success", () => {
    expect(workflowStatusIcon(makeRun({ status: "completed", conclusion: "success" }))).toBe("\u2713");
  });

  it("returns X for completed failure", () => {
    expect(workflowStatusIcon(makeRun({ status: "completed", conclusion: "failure" }))).toBe("\u2717");
  });

  it("returns dash for completed neutral", () => {
    expect(workflowStatusIcon(makeRun({ status: "completed", conclusion: "neutral" }))).toBe("\u2014");
  });

  it("returns dash for completed skipped", () => {
    expect(workflowStatusIcon(makeRun({ status: "completed", conclusion: "skipped" }))).toBe("\u2014");
  });

  it("returns ? for completed with unknown conclusion", () => {
    expect(workflowStatusIcon(makeRun({ status: "completed", conclusion: "cancelled" }))).toBe("?");
  });

  it("returns filled circle for in_progress", () => {
    expect(workflowStatusIcon(makeRun({ status: "in_progress", conclusion: null }))).toBe("\u25C9");
  });

  it("returns empty circle for queued", () => {
    expect(workflowStatusIcon(makeRun({ status: "queued", conclusion: null }))).toBe("\u25CB");
  });
});

describe("workflowStatusClass", () => {
  it("returns pass for completed success", () => {
    expect(workflowStatusClass(makeRun({ status: "completed", conclusion: "success" }))).toBe("pass");
  });

  it("returns fail for completed failure", () => {
    expect(workflowStatusClass(makeRun({ status: "completed", conclusion: "failure" }))).toBe("fail");
  });

  it("returns neutral for completed neutral", () => {
    expect(workflowStatusClass(makeRun({ status: "completed", conclusion: "neutral" }))).toBe("neutral");
  });

  it("returns neutral for completed skipped", () => {
    expect(workflowStatusClass(makeRun({ status: "completed", conclusion: "skipped" }))).toBe("neutral");
  });

  it("returns running for in_progress", () => {
    expect(workflowStatusClass(makeRun({ status: "in_progress", conclusion: null }))).toBe("running");
  });

  it("returns pending for queued", () => {
    expect(workflowStatusClass(makeRun({ status: "queued", conclusion: null }))).toBe("pending");
  });
});
