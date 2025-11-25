import { describe, it, expect } from "vitest";
import {
  getModelOptions,
  getDefaultInferenceForModel,
  getDefaultModelOption,
} from "./modelOptions.js";

const byId = (tool: string) => getModelOptions(tool).map((m) => m.id);

describe("modelOptions", () => {
  it("lists Claude official aliases and keeps default as recommended Opus 4.5", () => {
    const options = getModelOptions("claude-code");
    const ids = options.map((m) => m.id);
    expect(ids).toEqual([
      "default",
      "sonnet",
      "opus",
      "haiku",
    ]);
    const defaultModel = getDefaultModelOption("claude-code");
    expect(defaultModel?.id).toBe("default");
    expect(defaultModel?.label).toBe("Default (recommended) — Opus 4.5");

    const opusModel = options.find((m) => m.id === "opus");
    expect(opusModel?.label).toBe("Opus 4.5");
  });

  it("has unique Codex models", () => {
    const ids = byId("codex-cli");
    const unique = new Set(ids);
    expect(unique.size).toBe(ids.length);
    expect(ids).toEqual([
      "gpt-5.1-codex",
      "gpt-5.1-codex-max",
      "gpt-5.1-codex-mini",
      "gpt-5.1",
    ]);
  });

  it("uses medium as default reasoning for codex-max", () => {
    const codexMax = getModelOptions("codex-cli").find((m) => m.id === "gpt-5.1-codex-max");
    expect(getDefaultInferenceForModel(codexMax)).toBe("medium");
  });

  it("lists expected Gemini models", () => {
    expect(byId("gemini-cli")).toEqual([
      "gemini-3-pro-preview",
      "gemini-2.5-pro",
      "gemini-2.5-flash",
      "gemini-2.5-flash-lite",
    ]);
  });

  it("lists expected Qwen models", () => {
    expect(byId("qwen-cli")).toEqual(["coder-model", "vision-model"]);
  });
});
