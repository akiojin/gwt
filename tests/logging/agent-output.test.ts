import { describe, expect, it } from "bun:test";
import {
  createAgentOutputLineBuffer,
  shouldCaptureAgentOutput,
  stripAnsi,
} from "../../src/logging/agentOutput.js";

describe("shouldCaptureAgentOutput", () => {
  it("defaults to false and allows opt-in with true/1", () => {
    // Default is false to avoid PTY stdin/stdout conflicts with OpenTUI
    expect(shouldCaptureAgentOutput({})).toBe(false);
    expect(shouldCaptureAgentOutput({ GWT_CAPTURE_AGENT_OUTPUT: "true" })).toBe(
      true,
    );
    expect(shouldCaptureAgentOutput({ GWT_CAPTURE_AGENT_OUTPUT: "1" })).toBe(
      true,
    );
    expect(shouldCaptureAgentOutput({ GWT_CAPTURE_AGENT_OUTPUT: "TRUE" })).toBe(
      true,
    );
    expect(
      shouldCaptureAgentOutput({ GWT_CAPTURE_AGENT_OUTPUT: "false" }),
    ).toBe(false);
    expect(shouldCaptureAgentOutput({ GWT_CAPTURE_AGENT_OUTPUT: "0" })).toBe(
      false,
    );
    expect(
      shouldCaptureAgentOutput({ GWT_CAPTURE_AGENT_OUTPUT: "FALSE" }),
    ).toBe(false);
    // Empty string defaults to false
    expect(shouldCaptureAgentOutput({ GWT_CAPTURE_AGENT_OUTPUT: "" })).toBe(
      false,
    );
  });
});

describe("createAgentOutputLineBuffer", () => {
  it("buffers chunks and flushes complete lines", () => {
    const lines: string[] = [];
    const buffer = createAgentOutputLineBuffer((line) => lines.push(line));

    buffer.push("hello\nworld");
    buffer.push("\nnext");
    buffer.flush();

    expect(lines).toEqual(["hello", "world", "next"]);
  });
});

describe("stripAnsi", () => {
  it("removes ANSI escape codes", () => {
    const input = "\u001b[31merror\u001b[0m";
    expect(stripAnsi(input)).toBe("error");
  });
});
