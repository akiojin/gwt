import { describe, it, expect } from "vitest";
import {
  getModelOptions,
  getDefaultInferenceForModel,
  getDefaultModelOption,
} from "./modelOptions.js";

const byId = (tool: string) => getModelOptions(tool).map((m) => m.id);

describe("modelOptions", () => {
  it("lists Claude official aliases and sets Default as default", () => {
    const options = getModelOptions("claude-code");
    const ids = options.map((m) => m.id);
    expect(ids).toEqual(["", "opus", "sonnet", "haiku"]);
    const defaultModel = getDefaultModelOption("claude-code");
    expect(defaultModel?.id).toBe("");
    expect(defaultModel?.label).toBe("Default (Auto)");
  });

  it("has unique Codex models", () => {
    const ids = byId("codex-cli");
    const unique = new Set(ids);
    expect(unique.size).toBe(ids.length);
    expect(ids).toEqual([
      "",
      "gpt-5.1-codex",
      "gpt-5.2",
      "gpt-5.1-codex-max",
      "gpt-5.1-codex-mini",
      "gpt-5.1",
    ]);
  });

  it("uses medium as default reasoning for codex-max", () => {
    const codexMax = getModelOptions("codex-cli").find(
      (m) => m.id === "gpt-5.1-codex-max",
    );
    expect(getDefaultInferenceForModel(codexMax)).toBe("medium");
  });

  it("exposes gpt-5.2 with xhigh reasoning and medium default", () => {
    const codex52 = getModelOptions("codex-cli").find(
      (m) => m.id === "gpt-5.2",
    );
    expect(codex52?.inferenceLevels).toEqual([
      "xhigh",
      "high",
      "medium",
      "low",
    ]);
    expect(getDefaultInferenceForModel(codex52)).toBe("medium");
  });

  it("lists expected Gemini models", () => {
    expect(byId("gemini-cli")).toEqual([
      "gemini-3",
      "gemini-2.5",
      "gemini-3-pro-preview",
      "gemini-3-flash-preview",
      "gemini-2.5-pro",
      "gemini-2.5-flash",
      "gemini-2.5-flash-lite",
    ]);
  });

  it("lists expected Qwen models", () => {
    expect(byId("qwen-cli")).toEqual(["", "coder-model", "vision-model"]);
  });
});
