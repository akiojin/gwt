import { describe, it, expect, beforeEach, afterEach } from "vitest";

import {
  sanitizeEnvRecord,
  validateEnvRecord,
  mergeWithBootstrapEnv,
  getBootstrapEnvKeys,
  isValidEnvKey,
} from "../../../src/config/shared-env.js";

describe("shared env helpers", () => {
  const originalBootstrap = process.env.CLAUDE_WORKTREE_BOOTSTRAP_ENV_KEYS;

  beforeEach(() => {
    delete process.env.CLAUDE_WORKTREE_BOOTSTRAP_ENV_KEYS;
  });

  afterEach(() => {
    if (originalBootstrap !== undefined) {
      process.env.CLAUDE_WORKTREE_BOOTSTRAP_ENV_KEYS = originalBootstrap;
    } else {
      delete process.env.CLAUDE_WORKTREE_BOOTSTRAP_ENV_KEYS;
    }
  });

  it("sanitizes arbitrary records", () => {
    const result = sanitizeEnvRecord({ FOO: 123, BAR: true, BAZ: null });
    expect(result).toEqual({ FOO: "123", BAR: "true" });
  });

  it("validates env records and rejects invalid keys", () => {
    expect(() => validateEnvRecord({ "INVALID-KEY": "value" })).toThrow();
    expect(() => validateEnvRecord({ VALID_KEY: "" })).toThrow();
  });

  it("checks env key pattern helper", () => {
    expect(isValidEnvKey("VALID_ENV_1")).toBe(true);
    expect(isValidEnvKey("lowercase" as string)).toBe(false);
    expect(isValidEnvKey("INVALID-KEY")).toBe(false);
  });

  it("merges bootstrap env keys without overriding existing values", () => {
    const current = { OPENAI_API_KEY: "persist" };
    const source = {
      OPENAI_API_KEY: "ignored",
      ANTHROPIC_API_KEY: "new-value",
    } as NodeJS.ProcessEnv;

    const { merged, addedKeys } = mergeWithBootstrapEnv(current, source, [
      "OPENAI_API_KEY",
      "ANTHROPIC_API_KEY",
    ]);

    expect(merged).toEqual({
      OPENAI_API_KEY: "persist",
      ANTHROPIC_API_KEY: "new-value",
    });
    expect(addedKeys).toEqual(["ANTHROPIC_API_KEY"]);
  });

  it("respects CLAUDE_WORKTREE_BOOTSTRAP_ENV_KEYS override", () => {
    process.env.CLAUDE_WORKTREE_BOOTSTRAP_ENV_KEYS = "foo_key,bar_key";
    expect(getBootstrapEnvKeys()).toEqual(["FOO_KEY", "BAR_KEY"]);
  });
});
