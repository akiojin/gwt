import { describe, expect, it } from "vitest";
import {
  getAgentInputProfile,
  getAgentInputProfileOrDefault,
  buildSendBytes,
  buildQueueBytes,
  buildInterruptBytes,
  buildImageReference,
  type AgentInputProfile,
} from "./agentInputProfile";

describe("agentInputProfile", () => {
  describe("getAgentInputProfile", () => {
    it("returns claude profile", () => {
      const p = getAgentInputProfile("claude");
      expect(p).toBeDefined();
      expect(p!.agentId).toBe("claude");
      expect(p!.send.suffixBytes).toEqual([0x0a]);
    });

    it("returns codex profile", () => {
      const p = getAgentInputProfile("codex");
      expect(p).toBeDefined();
      expect(p!.agentId).toBe("codex");
      expect(p!.send.suffixBytes).toEqual([0x0d]);
      expect(p!.queue).toBeDefined();
      expect(p!.queue!.suffixBytes).toEqual([0x09]);
    });

    it("returns gemini profile", () => {
      const p = getAgentInputProfile("gemini");
      expect(p).toBeDefined();
      expect(p!.newlineBytes).toEqual([0x5c, 0x0d]);
    });

    it("returns copilot profile", () => {
      const p = getAgentInputProfile("copilot");
      expect(p).toBeDefined();
      expect(p!.send.suffixBytes).toEqual([0x0d]);
    });

    it("returns opencode profile", () => {
      const p = getAgentInputProfile("opencode");
      expect(p).toBeDefined();
      expect(p!.send.suffixBytes).toEqual([0x0d]);
    });

    it("returns undefined for unknown agent", () => {
      expect(getAgentInputProfile("unknown-agent")).toBeUndefined();
    });

    it("returns undefined for empty string", () => {
      expect(getAgentInputProfile("")).toBeUndefined();
    });
  });

  describe("getAgentInputProfileOrDefault", () => {
    it("returns known profile when found", () => {
      const p = getAgentInputProfileOrDefault("claude");
      expect(p.agentId).toBe("claude");
    });

    it("returns default profile for unknown agent", () => {
      const p = getAgentInputProfileOrDefault("unknown-agent");
      expect(p.agentId).toBe("_default");
      expect(p.send.suffixBytes).toEqual([0x0d]);
      expect(p.interrupt.bytes).toEqual([0x1b]);
      expect(p.newlineBytes).toEqual([0x0a]);
    });

    it("returns default profile for empty string", () => {
      const p = getAgentInputProfileOrDefault("");
      expect(p.agentId).toBe("_default");
    });
  });

  describe("buildSendBytes", () => {
    it("encodes single-line text with suffix for claude", () => {
      const profile = getAgentInputProfile("claude")!;
      const bytes = buildSendBytes(profile, "hello");
      const expected = [...new TextEncoder().encode("hello"), 0x0a];
      expect(bytes).toEqual(expected);
    });

    it("encodes single-line text with suffix for codex", () => {
      const profile = getAgentInputProfile("codex")!;
      const bytes = buildSendBytes(profile, "hello");
      const expected = [...new TextEncoder().encode("hello"), 0x0d];
      expect(bytes).toEqual(expected);
    });

    it("converts newlines using profile newlineBytes", () => {
      const profile = getAgentInputProfile("claude")!;
      const bytes = buildSendBytes(profile, "line1\nline2");
      // claude: newlineBytes = [0x0a], so \n stays as 0x0a
      const expected = [
        ...new TextEncoder().encode("line1"),
        0x0a,
        ...new TextEncoder().encode("line2"),
        0x0a,
      ];
      expect(bytes).toEqual(expected);
    });

    it("converts newlines for gemini (backslash + CR)", () => {
      const profile = getAgentInputProfile("gemini")!;
      const bytes = buildSendBytes(profile, "line1\nline2");
      const expected = [
        ...new TextEncoder().encode("line1"),
        0x5c, 0x0d, // \ + Enter
        ...new TextEncoder().encode("line2"),
        0x0d, // send suffix
      ];
      expect(bytes).toEqual(expected);
    });

    it("handles empty text", () => {
      const profile = getAgentInputProfile("claude")!;
      const bytes = buildSendBytes(profile, "");
      expect(bytes).toEqual([0x0a]);
    });

    it("handles multi-byte characters (Japanese)", () => {
      const profile = getAgentInputProfile("claude")!;
      const bytes = buildSendBytes(profile, "こんにちは");
      const expected = [...new TextEncoder().encode("こんにちは"), 0x0a];
      expect(bytes).toEqual(expected);
    });
  });

  describe("buildQueueBytes", () => {
    it("returns queue bytes for codex", () => {
      const profile = getAgentInputProfile("codex")!;
      const bytes = buildQueueBytes(profile, "hello");
      const expected = [...new TextEncoder().encode("hello"), 0x09];
      expect(bytes).toEqual(expected);
    });

    it("returns null for agent without queue support", () => {
      const profile = getAgentInputProfile("claude")!;
      expect(buildQueueBytes(profile, "hello")).toBeNull();
    });
  });

  describe("buildInterruptBytes", () => {
    it("returns ESC for claude", () => {
      const profile = getAgentInputProfile("claude")!;
      expect(buildInterruptBytes(profile)).toEqual([0x1b]);
    });

    it("returns ESC for codex", () => {
      const profile = getAgentInputProfile("codex")!;
      expect(buildInterruptBytes(profile)).toEqual([0x1b]);
    });
  });

  describe("buildImageReference", () => {
    it("returns plain path for claude (path_reference)", () => {
      const profile = getAgentInputProfile("claude")!;
      const ref = buildImageReference(profile, "/tmp/image.png");
      expect(ref).toBe("/tmp/image.png");
    });

    it("returns @path for gemini (command)", () => {
      const profile = getAgentInputProfile("gemini")!;
      const ref = buildImageReference(profile, "/tmp/image.png");
      expect(ref).toBe("@/tmp/image.png");
    });

    it("returns @path for copilot (command)", () => {
      const profile = getAgentInputProfile("copilot")!;
      const ref = buildImageReference(profile, "/tmp/image.png");
      expect(ref).toBe("@/tmp/image.png");
    });

    it("returns null for agent without image support", () => {
      const profile = getAgentInputProfile("opencode")!;
      expect(buildImageReference(profile, "/tmp/image.png")).toBeNull();
    });
  });
});
