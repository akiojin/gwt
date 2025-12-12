import { describe, it, expect } from "vitest";
import { resolveSessionId } from "../../../../../src/web/server/websocket/handler.js";

describe("resolveSessionId", () => {
  it("returns params sessionId when available", () => {
    const result = resolveSessionId({
      params: { sessionId: "abc" },
      url: "/api/sessions/abc/terminal",
    });

    expect(result).toBe("abc");
  });

  it("falls back to query string when params missing", () => {
    const result = resolveSessionId({
      url: "/api/sessions/terminal?sessionId=xyz",
    });

    expect(result).toBe("xyz");
  });

  it("returns null when neither params nor query contain sessionId", () => {
    const result = resolveSessionId({
      url: "/api/sessions/terminal",
    });

    expect(result).toBeNull();
  });
});
