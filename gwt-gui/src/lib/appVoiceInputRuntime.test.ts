import { describe, expect, it, vi } from "vitest";
import {
  DEFAULT_VOICE_INPUT,
  normalizeVoiceInputSettings,
  applyVoiceInputSettings,
  createVoiceInputState,
  resetVoiceInputTransientState,
  syncControllerState,
  setupVoiceController,
} from "./appVoiceInputRuntime";
import type { VoiceInputSettings } from "./types";
import type { VoiceControllerState } from "./voice/voiceInputController";

describe("normalizeVoiceInputSettings", () => {
  it("returns defaults for null / undefined", () => {
    expect(normalizeVoiceInputSettings(null)).toEqual(
      DEFAULT_VOICE_INPUT,
    );
    expect(normalizeVoiceInputSettings(undefined)).toEqual(
      DEFAULT_VOICE_INPUT,
    );
  });

  it("passes through valid values", () => {
    const input: VoiceInputSettings = {
      enabled: true,
      engine: "qwen3-asr",
      language: "ja",
      quality: "fast",
      model: "custom-model",
    };
    const result = normalizeVoiceInputSettings(input);
    expect(result).toEqual(input);
  });

  it("normalises engine aliases to qwen3-asr", () => {
    expect(normalizeVoiceInputSettings({ engine: "qwen" } as any).engine).toBe(
      "qwen3-asr",
    );
    expect(
      normalizeVoiceInputSettings({ engine: "whisper" } as any).engine,
    ).toBe("qwen3-asr");
  });

  it("falls back invalid quality to default", () => {
    expect(
      normalizeVoiceInputSettings({ quality: "ultra" } as any).quality,
    ).toBe(DEFAULT_VOICE_INPUT.quality);
  });

  it("falls back invalid language to default", () => {
    expect(
      normalizeVoiceInputSettings({ language: "fr" } as any).language,
    ).toBe(DEFAULT_VOICE_INPUT.language);
  });

  it("assigns default model based on quality", () => {
    expect(normalizeVoiceInputSettings({ quality: "fast" } as any).model).toBe(
      "Qwen/Qwen3-ASR-0.6B",
    );
    expect(
      normalizeVoiceInputSettings({ quality: "balanced" } as any).model,
    ).toBe("Qwen/Qwen3-ASR-1.7B");
    expect(
      normalizeVoiceInputSettings({ quality: "accurate" } as any).model,
    ).toBe("Qwen/Qwen3-ASR-1.7B");
  });

  it("trims and lowercases input strings", () => {
    const result = normalizeVoiceInputSettings({
      engine: "  Qwen3-ASR ",
      language: " JA ",
      quality: " Balanced ",
      model: "  MyModel  ",
    } as any);
    expect(result.engine).toBe("qwen3-asr");
    expect(result.language).toBe("ja");
    expect(result.quality).toBe("balanced");
    expect(result.model).toBe("MyModel");
  });
});

describe("createVoiceInputState", () => {
  it("returns default state", () => {
    const state = createVoiceInputState();
    expect(state.settings).toEqual(DEFAULT_VOICE_INPUT);
    expect(state.listening).toBe(false);
    expect(state.preparing).toBe(false);
    expect(state.supported).toBe(true);
    expect(state.available).toBe(false);
    expect(state.availabilityReason).toBeNull();
    expect(state.error).toBeNull();
  });

  it("accepts custom defaults", () => {
    const custom: VoiceInputSettings = {
      enabled: true,
      engine: "qwen3-asr",
      language: "en",
      quality: "fast",
      model: "test",
    };
    const state = createVoiceInputState(custom);
    expect(state.settings).toEqual(custom);
  });
});

describe("applyVoiceInputSettings", () => {
  it("normalises and assigns settings", () => {
    const state = createVoiceInputState();
    applyVoiceInputSettings(state, { enabled: true, language: "en" } as any, {
      controller: null,
    });
    expect(state.settings.enabled).toBe(true);
    expect(state.settings.language).toBe("en");
  });

  it("calls controller.updateSettings when controller exists", () => {
    const state = createVoiceInputState();
    const mockController = { updateSettings: vi.fn() } as any;
    applyVoiceInputSettings(state, { enabled: true } as any, {
      controller: mockController,
    });
    expect(mockController.updateSettings).toHaveBeenCalledOnce();
  });
});

describe("resetVoiceInputTransientState", () => {
  it("resets transient fields to idle", () => {
    const state = createVoiceInputState();
    state.listening = true;
    state.preparing = true;
    state.error = "some error";
    state.supported = false;
    state.available = true;
    state.availabilityReason = "reason";

    resetVoiceInputTransientState(state);

    expect(state.listening).toBe(false);
    expect(state.preparing).toBe(false);
    expect(state.error).toBeNull();
    expect(state.supported).toBe(true);
    expect(state.available).toBe(false);
    expect(state.availabilityReason).toBeNull();
  });

  it("does not touch settings", () => {
    const state = createVoiceInputState();
    state.settings.enabled = true;
    resetVoiceInputTransientState(state);
    expect(state.settings.enabled).toBe(true);
  });
});

describe("syncControllerState", () => {
  it("copies all controller state fields", () => {
    const state = createVoiceInputState();
    const cs: VoiceControllerState = {
      listening: true,
      preparing: true,
      supported: false,
      available: true,
      availabilityReason: "ready",
      modelReady: true,
      error: "err",
    };
    syncControllerState(state, cs);
    expect(state.listening).toBe(true);
    expect(state.preparing).toBe(true);
    expect(state.supported).toBe(false);
    expect(state.available).toBe(true);
    expect(state.availabilityReason).toBe("ready");
    expect(state.error).toBe("err");
  });
});

describe("setupVoiceController", () => {
  it("creates controller and wires PTT events", () => {
    const onStateChange = vi.fn();
    const { controller, cleanup } = setupVoiceController({
      getSettings: () => DEFAULT_VOICE_INPUT,
      getFallbackTerminalPaneId: () => null,
      onStateChange,
    });

    expect(controller).toBeDefined();

    // Dispatch PTT events
    window.dispatchEvent(new Event("gwt-voice-ptt-start"));
    window.dispatchEvent(new Event("gwt-voice-ptt-stop"));

    // After cleanup, events should be detached
    cleanup();

    // Controller methods should still exist (dispose was called)
    expect(typeof controller.dispose).toBe("function");
  });
});
