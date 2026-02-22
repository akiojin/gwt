import { describe, it, expect } from "vitest";
import { inferAgentId } from "./agentUtils";

describe("inferAgentId", () => {
  it("maps known agent names to canonical IDs", () => {
    expect(inferAgentId("claude")).toBe("claude");
    expect(inferAgentId("Claude Code")).toBe("claude");
    expect(inferAgentId("codex")).toBe("codex");
    expect(inferAgentId("OpenCode")).toBe("opencode");
    expect(inferAgentId("open-code")).toBe("opencode");
    expect(inferAgentId("Gemini Pro")).toBe("gemini");
  });

  it("returns null for unknown agent names", () => {
    expect(inferAgentId(null)).toBeNull();
    expect(inferAgentId("")).toBeNull();
    expect(inferAgentId("unknown")).toBeNull();
  });
});
