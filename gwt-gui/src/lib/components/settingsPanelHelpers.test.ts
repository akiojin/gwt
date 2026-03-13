import { describe, it, expect, vi, afterEach } from "vitest";
import type { Profile, ProfilesConfig } from "../types";
import {
  DEFAULT_VOICE_INPUT,
  DEFAULT_UI_FONT_FAMILY,
  DEFAULT_TERMINAL_FONT_FAMILY,
  UI_FONT_PRESETS,
  TERMINAL_FONT_PRESETS,
  getCurrentProfile,
  isDefaultProfileKey,
  isAiEnabled,
  toErrorMessage,
  detectGpuAvailability,
  normalizeVoiceInputSettings,
  normalizeAppLanguage,
  normalizeUiFontFamily,
  normalizeTerminalFontFamily,
  clampFontSize,
} from "./settingsPanelHelpers";

describe("settingsPanelHelpers", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("gets current profile safely", () => {
    const profile: Profile = {
      name: "default",
      description: "",
      env: {},
      disabled_env: [],
      ai: null,
    };
    const cfg: ProfilesConfig = { version: 1, active: "default", profiles: { default: profile } };
    expect(getCurrentProfile(null, "default")).toBeNull();
    expect(getCurrentProfile(cfg, "")).toBeNull();
    expect(getCurrentProfile(cfg, "missing")).toBeNull();
    expect(getCurrentProfile(cfg, "default")).toEqual(profile);
  });

  it("checks AI endpoint enablement", () => {
    expect(isAiEnabled(null)).toBe(false);
    expect(
      isAiEnabled({
        name: "p",
        description: "",
        env: {},
        disabled_env: [],
        ai: { endpoint: "   ", api_key: "", model: "", language: "en" },
      }),
    ).toBe(false);
    expect(
      isAiEnabled({
        name: "p",
        description: "",
        env: {},
        disabled_env: [],
        ai: {
          endpoint: "https://api.openai.com/v1",
          api_key: "",
          model: "",
          language: "en",
        },
      }),
    ).toBe(true);
  });

  it("matches only the exact default profile key", () => {
    expect(isDefaultProfileKey("default")).toBe(true);
    expect(isDefaultProfileKey("default ")).toBe(false);
    expect(isDefaultProfileKey("Default")).toBe(false);
    expect(isDefaultProfileKey("dev")).toBe(false);
  });

  it("converts errors to display messages", () => {
    expect(toErrorMessage("failed")).toBe("failed");
    expect(toErrorMessage({ message: "typed" })).toBe("typed");
    expect(toErrorMessage({ message: 123 })).toBe("[object Object]");
    expect(toErrorMessage(null)).toBe("null");
  });

  it("detects GPU availability for no-webgl/software/hardware/exception", () => {
    const realCreateElement = document.createElement.bind(document);

    const createSpy = vi.spyOn(document, "createElement");
    createSpy.mockImplementation((tagName: string) => {
      if (tagName !== "canvas") {
        return realCreateElement(tagName);
      }
      return {
        getContext: () => null,
      } as unknown as HTMLCanvasElement;
    });
    expect(detectGpuAvailability()).toBe(false);

    createSpy.mockImplementation((tagName: string) => {
      if (tagName !== "canvas") {
        return realCreateElement(tagName);
      }
      const gl = {
        getExtension: () => ({ UNMASKED_RENDERER_WEBGL: 1 }),
        getParameter: () => "SwiftShader",
      } as unknown as WebGLRenderingContext;
      return {
        getContext: () => gl,
      } as unknown as HTMLCanvasElement;
    });
    expect(detectGpuAvailability()).toBe(false);

    createSpy.mockImplementation((tagName: string) => {
      if (tagName !== "canvas") {
        return realCreateElement(tagName);
      }
      const gl = {
        getExtension: () => ({ UNMASKED_RENDERER_WEBGL: 1 }),
        getParameter: () => "Apple M3 GPU",
      } as unknown as WebGLRenderingContext;
      return {
        getContext: () => gl,
      } as unknown as HTMLCanvasElement;
    });
    expect(detectGpuAvailability()).toBe(true);

    createSpy.mockImplementation((tagName: string) => {
      if (tagName !== "canvas") {
        return realCreateElement(tagName);
      }
      const gl = {
        getExtension: () => ({ UNMASKED_RENDERER_WEBGL: 1 }),
        getParameter: () => null,
      } as unknown as WebGLRenderingContext;
      return {
        getContext: () => gl,
      } as unknown as HTMLCanvasElement;
    });
    expect(detectGpuAvailability()).toBe(true);

    createSpy.mockImplementation((tagName: string) => {
      if (tagName !== "canvas") {
        return realCreateElement(tagName);
      }
      const gl = {
        getExtension: () => null,
      } as unknown as WebGLRenderingContext;
      return {
        getContext: () => gl,
      } as unknown as HTMLCanvasElement;
    });
    expect(detectGpuAvailability()).toBe(true);

    createSpy.mockImplementation(() => {
      throw new Error("boom");
    });
    expect(detectGpuAvailability()).toBe(false);
  });

  it("normalizes voice input settings for valid and invalid values", () => {
    expect(normalizeVoiceInputSettings(undefined)).toEqual(DEFAULT_VOICE_INPUT);

    const invalid = normalizeVoiceInputSettings({
      enabled: true,
      engine: "invalid",
      hotkey: "",
      ptt_hotkey: "",
      language: "fr" as any,
      quality: "invalid" as any,
      model: "",
    });
    expect(invalid).toEqual({
      ...DEFAULT_VOICE_INPUT,
      enabled: true,
      model: "Qwen/Qwen3-ASR-1.7B",
    });

    const fast = normalizeVoiceInputSettings({
      enabled: false,
      engine: "qwen",
      hotkey: "Alt+M",
      ptt_hotkey: "Alt+Space",
      language: "ja",
      quality: "fast",
      model: "",
    });
    expect(fast.engine).toBe("qwen3-asr");
    expect(fast.language).toBe("ja");
    expect(fast.quality).toBe("fast");
    expect(fast.model).toBe("Qwen/Qwen3-ASR-0.6B");
  });

  it("normalizes app language and font family values", () => {
    expect(normalizeAppLanguage("ja")).toBe("ja");
    expect(normalizeAppLanguage("EN")).toBe("en");
    expect(normalizeAppLanguage("auto")).toBe("auto");
    expect(normalizeAppLanguage("fr")).toBe("auto");
    expect(normalizeAppLanguage(null)).toBe("auto");

    expect(normalizeUiFontFamily("")).toBe(DEFAULT_UI_FONT_FAMILY);
    expect(normalizeUiFontFamily(null)).toBe(DEFAULT_UI_FONT_FAMILY);
    expect(normalizeUiFontFamily(UI_FONT_PRESETS[1].value)).toBe(UI_FONT_PRESETS[1].value);
    expect(normalizeUiFontFamily('"Custom UI", sans-serif')).toBe('"Custom UI", sans-serif');

    expect(normalizeTerminalFontFamily("")).toBe(DEFAULT_TERMINAL_FONT_FAMILY);
    expect(normalizeTerminalFontFamily(undefined)).toBe(DEFAULT_TERMINAL_FONT_FAMILY);
    expect(normalizeTerminalFontFamily(TERMINAL_FONT_PRESETS[1].value)).toBe(
      TERMINAL_FONT_PRESETS[1].value,
    );
    expect(normalizeTerminalFontFamily('"Custom Mono", monospace')).toBe(
      '"Custom Mono", monospace',
    );
  });

  it("clamps font sizes", () => {
    expect(clampFontSize(1)).toBe(8);
    expect(clampFontSize(8.4)).toBe(8);
    expect(clampFontSize(12.6)).toBe(13);
    expect(clampFontSize(100)).toBe(24);
  });
});
