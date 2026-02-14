import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearTerminalInputTargetsForTests,
  registerTerminalInputTarget,
} from "./inputTargetRegistry";
import {
  __setVoiceGpuDetectorForTests,
  __setVoiceInvokeForTests,
  VoiceInputController,
  type VoiceControllerSettings,
} from "./voiceInputController";

describe("VoiceInputController", () => {
  let settings: VoiceControllerSettings;
  const invokeMock = vi.fn();

  beforeEach(() => {
    settings = {
      enabled: true,
      engine: "qwen3-asr",
      hotkey: "Mod+Shift+M",
      ptt_hotkey: "Mod+Shift+Space",
      language: "auto",
      quality: "balanced",
      model: "base",
    };

    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return {
          available: true,
          reason: null,
          modelReady: true,
        };
      }
      if (command === "prepare_voice_model") {
        return { ready: true };
      }
      if (command === "transcribe_voice_audio") {
        return { transcript: "voice transcript" };
      }
      if (command === "write_terminal") {
        return null;
      }
      return null;
    });

    __setVoiceInvokeForTests(invokeMock);
    __setVoiceGpuDetectorForTests(() => true);
    clearTerminalInputTargetsForTests();
  });

  afterEach(() => {
    __setVoiceInvokeForTests(null);
    __setVoiceGpuDetectorForTests(null);
    clearTerminalInputTargetsForTests();
  });

  it("starts/stops and inserts transcript into focused input", async () => {
    const states: Array<{ listening: boolean; error: string | null }> = [];

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (state) => {
        states.push({ listening: state.listening, error: state.error });
      },
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.2, 0.1, -0.1],
      sampleRate: 16_000,
    }));

    const input = document.createElement("input");
    input.type = "text";
    input.value = "task: ";
    document.body.appendChild(input);
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);

    await (controller as any).startListening("toggle");
    expect((controller as any).beginCapture).toHaveBeenCalled();

    await (controller as any).stopListening(false);
    await vi.waitFor(() => {
      expect(input.value).toBe("task: voice transcript");
    });

    expect(states.some((s) => s.listening)).toBe(true);

    controller.dispose();
    input.remove();
  });

  it("sends transcript to terminal when terminal target is focused", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.3, 0.2],
      sampleRate: 16_000,
    }));

    const root = document.createElement("div");
    root.tabIndex = 0;
    document.body.appendChild(root);

    const unregister = registerTerminalInputTarget("pane-test", root);
    root.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", {
        paneId: "pane-test",
        data: Array.from(new TextEncoder().encode("voice transcript")),
      });
    });

    unregister();
    root.remove();
    controller.dispose();
  });

  it("supports push-to-talk mode with the same capture pipeline", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1, 0.1, 0.1],
      sampleRate: 16_000,
    }));

    const textarea = document.createElement("textarea");
    document.body.appendChild(textarea);
    textarea.focus();

    await (controller as any).startListening("ptt");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(textarea.value).toBe("voice transcript");
    });

    textarea.remove();
    controller.dispose();
  });

  it("keeps input unchanged when transcript is empty", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      if (command === "prepare_voice_model") {
        return { ready: true };
      }
      if (command === "transcribe_voice_audio") {
        return { transcript: "   " };
      }
      return null;
    });

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fallback",
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.2, 0.3],
      sampleRate: 16_000,
    }));

    const input = document.createElement("input");
    input.type = "text";
    input.value = "unchanged";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(input.value).toBe("unchanged");
    });
    expect(invokeMock).not.toHaveBeenCalledWith("write_terminal", expect.anything());

    input.remove();
    controller.dispose();
  });

  it("does not start when voice input is disabled", async () => {
    settings.enabled = false;

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    await (controller as any).startListening("toggle");

    expect((controller as any).beginCapture).not.toHaveBeenCalled();
    controller.dispose();
  });

  it("auto-installs voice runtime once when capability is runtime-unavailable", async () => {
    let runtimeReady = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return {
          available: runtimeReady,
          reason: runtimeReady
            ? null
            : "Voice runtime is unavailable: Missing Python package(s): qwen_asr",
          modelReady: true,
        };
      }
      if (command === "ensure_voice_runtime") {
        runtimeReady = true;
        return { ready: true, installed: true, pythonPath: "/tmp/voice-venv/bin/python3" };
      }
      if (command === "prepare_voice_model") {
        return { ready: true };
      }
      if (command === "transcribe_voice_audio") {
        return { transcript: "voice transcript" };
      }
      if (command === "write_terminal") {
        return null;
      }
      return null;
    });

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});

    await (controller as any).startListening("toggle");
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("ensure_voice_runtime");
      expect((controller as any).beginCapture).toHaveBeenCalled();
    });

    controller.dispose();
  });
});
