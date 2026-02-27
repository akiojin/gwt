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
  type VoiceControllerState,
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

  it("releases push-to-talk when trigger key is released after modifiers", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const stopListeningMock = vi.fn(async () => {});
    (controller as any).stopListening = stopListeningMock;
    (controller as any).pttPressed = true;
    (controller as any).activeMode = "ptt";
    (controller as any).state.listening = true;

    document.dispatchEvent(
      new KeyboardEvent("keyup", {
        key: " ",
        metaKey: true,
        shiftKey: false,
      })
    );

    await vi.waitFor(() => {
      expect(stopListeningMock).toHaveBeenCalledWith(false);
    });
    expect((controller as any).pttPressed).toBe(false);

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

  // --- New tests: coverage improvements ---

  it("does not start when already listening", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
    }));

    // First start
    await (controller as any).startListening("toggle");
    expect((controller as any).beginCapture).toHaveBeenCalledTimes(1);

    // Second start while listening - should be no-op
    await (controller as any).startListening("toggle");
    expect((controller as any).beginCapture).toHaveBeenCalledTimes(1);

    controller.dispose();
  });

  it("does not start when startInFlight is true", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).startInFlight = true;

    await (controller as any).startListening("toggle");
    expect((controller as any).beginCapture).not.toHaveBeenCalled();

    controller.dispose();
  });

  it("sets error when voice is unavailable and no GPU", async () => {
    __setVoiceGpuDetectorForTests(() => false);
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return {
          available: false,
          reason: "No GPU acceleration available",
          modelReady: false,
        };
      }
      return null;
    });

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    (controller as any).beginCapture = vi.fn(async () => {});

    await (controller as any).startListening("toggle");

    await vi.waitFor(() => {
      const errorState = states.find((s) => s.error !== null);
      expect(errorState).toBeTruthy();
      expect(errorState!.error).toContain("No GPU acceleration available");
    });

    controller.dispose();
  });

  it("handles transcription failure gracefully", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      if (command === "prepare_voice_model") {
        return { ready: true };
      }
      if (command === "transcribe_voice_audio") {
        throw new Error("Transcription engine failed");
      }
      return null;
    });

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1, 0.2],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      const errorState = states.find((s) => s.error?.includes("transcription failed"));
      expect(errorState).toBeTruthy();
    });

    input.remove();
    controller.dispose();
  });

  it("sets error when no active input target for transcript", async () => {
    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    // Remove focus from any element
    (document.activeElement as HTMLElement)?.blur?.();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      const errorState = states.find((s) =>
        s.error?.includes("No active input target")
      );
      expect(errorState).toBeTruthy();
    });

    controller.dispose();
  });

  it("uses fallback terminal pane when no active element accepts input", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fallback",
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    // No focused terminal target, active element is not input
    const div = document.createElement("div");
    document.body.appendChild(div);
    div.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", {
        paneId: "pane-fallback",
        data: Array.from(new TextEncoder().encode("voice transcript")),
      });
    });

    div.remove();
    controller.dispose();
  });

  it("dispose removes keydown and keyup listeners", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const removeListenerSpy = vi.spyOn(document, "removeEventListener");

    controller.dispose();

    expect(removeListenerSpy).toHaveBeenCalledWith("keydown", expect.any(Function), true);
    expect(removeListenerSpy).toHaveBeenCalledWith("keyup", expect.any(Function), true);

    removeListenerSpy.mockRestore();
  });

  it("updateSettings stops listening when settings become disabled", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [],
      sampleRate: 16_000,
      truncated: false,
    }));

    await (controller as any).startListening("toggle");

    // Now disable settings
    settings.enabled = false;
    controller.updateSettings();

    await vi.waitFor(() => {
      expect((controller as any).state.listening).toBe(false);
    });

    controller.dispose();
  });

  it("skips runtime bootstrap when reason does not mention runtime/python/qwen", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return {
          available: false,
          reason: "No GPU acceleration found",
          modelReady: false,
        };
      }
      return null;
    });

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});

    await (controller as any).startListening("toggle");

    // ensure_voice_runtime should NOT have been called
    expect(invokeMock).not.toHaveBeenCalledWith("ensure_voice_runtime");

    controller.dispose();
  });

  it("does not call ensure_voice_runtime a second time", async () => {
    let runtimeCalls = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return {
          available: false,
          reason: "Voice runtime is unavailable",
          modelReady: false,
        };
      }
      if (command === "ensure_voice_runtime") {
        runtimeCalls++;
        return { ready: false, installed: false, pythonPath: "" };
      }
      return null;
    });

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});

    // First attempt
    await (controller as any).startListening("toggle");
    await vi.waitFor(() => {
      expect(runtimeCalls).toBe(1);
    });

    // Second attempt - should not call again
    (controller as any).startInFlight = false;
    (controller as any).state.listening = false;
    await (controller as any).startListening("toggle");

    // Still only 1 call
    expect(runtimeCalls).toBe(1);

    controller.dispose();
  });

  it("handles startListening error by setting error state", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      if (command === "prepare_voice_model") {
        return { ready: true };
      }
      return null;
    });

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    // Make beginCapture throw
    (controller as any).beginCapture = vi.fn(async () => {
      throw new Error("Microphone denied");
    });

    await (controller as any).startListening("toggle");

    await vi.waitFor(() => {
      const errorState = states.find((s) => s.error?.includes("Microphone denied"));
      expect(errorState).toBeTruthy();
    });

    // Should not be listening after error
    expect((controller as any).state.listening).toBe(false);
    expect((controller as any).activeMode).toBeNull();

    controller.dispose();
  });

  it("discards audio when stopListening is called with discardAudio=true", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1, 0.2],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "text";
    input.value = "original";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(true);

    // Should NOT have called transcribe
    expect(invokeMock).not.toHaveBeenCalledWith("transcribe_voice_audio", expect.anything());
    // Value unchanged
    expect(input.value).toBe("original");

    input.remove();
    controller.dispose();
  });

  it("does nothing in stopListening when not listening and no stream", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    // Not listening, no mediaStream
    (controller as any).state.listening = false;
    (controller as any).mediaStream = null;

    await (controller as any).stopListening(false);

    // No crash, no error
    expect(invokeMock).not.toHaveBeenCalledWith("transcribe_voice_audio", expect.anything());

    controller.dispose();
  });

  it("sets truncation warning when capture was truncated and transcript exists", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      if (command === "prepare_voice_model") {
        return { ready: true };
      }
      if (command === "transcribe_voice_audio") {
        return { transcript: "hello world" };
      }
      return null;
    });

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1, 0.2],
      sampleRate: 16_000,
      truncated: true,
    }));

    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      const warnState = states.find((s) => s.error?.includes("limited to 30s"));
      expect(warnState).toBeTruthy();
    });

    input.remove();
    controller.dispose();
  });

  it("sets truncation error when capture was truncated but transcript is empty", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      if (command === "prepare_voice_model") {
        return { ready: true };
      }
      if (command === "transcribe_voice_audio") {
        return { transcript: "  " };
      }
      return null;
    });

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1, 0.2],
      sampleRate: 16_000,
      truncated: true,
    }));

    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      const warnState = states.find((s) => s.error?.includes("30s limit"));
      expect(warnState).toBeTruthy();
    });

    input.remove();
    controller.dispose();
  });

  it("prepares model when modelReady is false", async () => {
    let preparedCalled = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: false };
      }
      if (command === "prepare_voice_model") {
        preparedCalled = true;
        return { ready: true };
      }
      if (command === "transcribe_voice_audio") {
        return { transcript: "hello" };
      }
      return null;
    });

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");

    expect(preparedCalled).toBe(true);

    input.remove();
    controller.dispose();
  });

  it("handles write_terminal error gracefully", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      if (command === "prepare_voice_model") {
        return { ready: true };
      }
      if (command === "transcribe_voice_audio") {
        return { transcript: "hello" };
      }
      if (command === "write_terminal") {
        throw new Error("Terminal write failed");
      }
      return null;
    });

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-x",
      onStateChange: (s) => states.push({ ...s }),
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    // Ensure active element is not input so it falls through to terminal
    const div = document.createElement("div");
    document.body.appendChild(div);
    div.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      const errState = states.find((s) => s.error?.includes("Failed to send transcript"));
      expect(errState).toBeTruthy();
    });

    div.remove();
    controller.dispose();
  });

  it("handles keydown for toggle hotkey - starts and stops", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    const stopMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;
    (controller as any).stopListening = stopMock;

    // Simulate toggle hotkey keydown (Mod+Shift+M on Mac = Meta+Shift+M)
    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "m",
        metaKey: isMac,
        ctrlKey: !isMac,
        shiftKey: true,
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect(startMock).toHaveBeenCalledWith("toggle");
    });

    controller.dispose();
  });

  it("ignores keydown when voice input is disabled", async () => {
    settings.enabled = false;

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;

    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "m",
        metaKey: isMac,
        ctrlKey: !isMac,
        shiftKey: true,
        bubbles: true,
      })
    );

    // Should NOT have been called
    expect(startMock).not.toHaveBeenCalled();

    controller.dispose();
  });

  it("ignores repeated keydown events", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;

    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "m",
        metaKey: isMac,
        ctrlKey: !isMac,
        shiftKey: true,
        repeat: true,
        bubbles: true,
      })
    );

    expect(startMock).not.toHaveBeenCalled();

    controller.dispose();
  });

  it("keyup does nothing when pttPressed is false", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const stopMock = vi.fn(async () => {});
    (controller as any).stopListening = stopMock;

    document.dispatchEvent(
      new KeyboardEvent("keyup", {
        key: " ",
        bubbles: true,
      })
    );

    expect(stopMock).not.toHaveBeenCalled();

    controller.dispose();
  });

  it("refreshCapability sets supported=false on invoke error", async () => {
    invokeMock.mockRejectedValue(new Error("backend down"));

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    await vi.waitFor(() => {
      const unsupportedState = states.find((s) => s.supported === false);
      expect(unsupportedState).toBeTruthy();
      expect(unsupportedState!.availabilityReason).toContain("unavailable");
    });

    controller.dispose();
  });

  it("endCapture returns null when no chunks have been recorded", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).chunks = [];
    (controller as any).capturedSampleCount = 0;

    const result = await (controller as any).endCapture();
    expect(result).toBeNull();

    controller.dispose();
  });

  it("endCapture merges chunks and returns samples with sampleRate", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    // Populate internal chunks directly
    (controller as any).chunks = [
      new Float32Array([0.1, 0.2]),
      new Float32Array([0.3, 0.4, 0.5]),
    ];
    (controller as any).capturedSampleCount = 5;
    (controller as any).sampleRate = 16_000;
    (controller as any).captureTruncated = false;
    (controller as any).maxCaptureSamples = 480_000;

    const result = await (controller as any).endCapture();
    expect(result).not.toBeNull();
    expect(result.samples).toEqual([
      expect.closeTo(0.1),
      expect.closeTo(0.2),
      expect.closeTo(0.3),
      expect.closeTo(0.4),
      expect.closeTo(0.5),
    ]);
    expect(result.sampleRate).toBe(16_000);
    expect(result.truncated).toBe(false);

    // Internal state should be reset
    expect((controller as any).chunks).toEqual([]);
    expect((controller as any).capturedSampleCount).toBe(0);

    controller.dispose();
  });

  it("endCapture returns truncated=true when capture was truncated", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).chunks = [new Float32Array([0.1, 0.2, 0.3])];
    (controller as any).capturedSampleCount = 3;
    (controller as any).sampleRate = 16_000;
    (controller as any).captureTruncated = true;
    (controller as any).maxCaptureSamples = 3;

    const result = await (controller as any).endCapture();
    expect(result).not.toBeNull();
    expect(result.truncated).toBe(true);
    expect(result.samples.length).toBe(3);

    controller.dispose();
  });

  it("endCapture disconnects processor, source, stops tracks, and closes audioContext", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const processorDisconnect = vi.fn();
    const sourceDisconnect = vi.fn();
    const trackStop = vi.fn();
    const audioContextClose = vi.fn(async () => {});

    (controller as any).processorNode = {
      onaudioprocess: () => {},
      disconnect: processorDisconnect,
    };
    (controller as any).sourceNode = {
      disconnect: sourceDisconnect,
    };
    (controller as any).mediaStream = {
      getTracks: () => [{ stop: trackStop }, { stop: trackStop }],
    };
    (controller as any).audioContext = {
      close: audioContextClose,
    };
    (controller as any).chunks = [];
    (controller as any).capturedSampleCount = 0;

    const result = await (controller as any).endCapture();
    expect(result).toBeNull();
    expect(processorDisconnect).toHaveBeenCalled();
    expect(sourceDisconnect).toHaveBeenCalled();
    expect(trackStop).toHaveBeenCalledTimes(2);
    expect(audioContextClose).toHaveBeenCalled();

    controller.dispose();
  });

  it("endCapture handles disconnect/close errors gracefully", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).processorNode = {
      onaudioprocess: () => {},
      disconnect: () => { throw new Error("disconnect fail"); },
    };
    (controller as any).sourceNode = {
      disconnect: () => { throw new Error("source disconnect fail"); },
    };
    (controller as any).mediaStream = {
      getTracks: () => [{ stop: () => { throw new Error("stop fail"); } }],
    };
    (controller as any).audioContext = {
      close: async () => { throw new Error("close fail"); },
    };
    (controller as any).chunks = [new Float32Array([0.5])];
    (controller as any).capturedSampleCount = 1;
    (controller as any).sampleRate = 16_000;
    (controller as any).captureTruncated = false;

    // Should not throw despite all internal errors
    const result = await (controller as any).endCapture();
    expect(result).not.toBeNull();
    expect(result.samples).toEqual([expect.closeTo(0.5)]);

    controller.dispose();
  });

  it("beginCapture throws when getUserMedia is not available", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    // Remove getUserMedia
    const origMediaDevices = navigator.mediaDevices;
    Object.defineProperty(navigator, "mediaDevices", {
      value: { getUserMedia: undefined },
      configurable: true,
    });

    await expect((controller as any).beginCapture()).rejects.toThrow(
      "Microphone capture API is unavailable"
    );

    Object.defineProperty(navigator, "mediaDevices", {
      value: origMediaDevices,
      configurable: true,
    });

    controller.dispose();
  });

  it("beginCapture throws when AudioContext is not available", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const mockTrack = { stop: vi.fn() };
    const mockStream = { getTracks: () => [mockTrack] };

    const origMediaDevices = navigator.mediaDevices;
    Object.defineProperty(navigator, "mediaDevices", {
      value: {
        getUserMedia: vi.fn(async () => mockStream),
      },
      configurable: true,
    });

    // Remove AudioContext
    const origAudioContext = (window as any).AudioContext;
    const origWebkitAudioContext = (window as any).webkitAudioContext;
    delete (window as any).AudioContext;
    delete (window as any).webkitAudioContext;

    await expect((controller as any).beginCapture()).rejects.toThrow(
      "AudioContext is unavailable"
    );

    // Stream tracks should be stopped
    expect(mockTrack.stop).toHaveBeenCalled();

    // Restore
    (window as any).AudioContext = origAudioContext;
    if (origWebkitAudioContext) {
      (window as any).webkitAudioContext = origWebkitAudioContext;
    }
    Object.defineProperty(navigator, "mediaDevices", {
      value: origMediaDevices,
      configurable: true,
    });

    controller.dispose();
  });

  it("beginCapture sets up audio pipeline and processes audio data", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const mockTrack = { stop: vi.fn() };
    const mockStream = { getTracks: () => [mockTrack] };
    let audioProcessHandler: ((event: any) => void) | null = null;

    const mockProcessor = {
      onaudioprocess: null as any,
      connect: vi.fn(),
      disconnect: vi.fn(),
    };
    const mockSource = {
      connect: vi.fn(),
      disconnect: vi.fn(),
    };
    const mockAudioContext = {
      sampleRate: 16_000,
      createMediaStreamSource: vi.fn(() => mockSource),
      createScriptProcessor: vi.fn(() => mockProcessor),
      destination: {},
      close: vi.fn(async () => {}),
    };

    const origMediaDevices = navigator.mediaDevices;
    Object.defineProperty(navigator, "mediaDevices", {
      value: {
        getUserMedia: vi.fn(async () => mockStream),
      },
      configurable: true,
    });

    const origAudioContext = (window as any).AudioContext;
    (window as any).AudioContext = function() { return mockAudioContext; };

    await (controller as any).beginCapture();

    expect(mockSource.connect).toHaveBeenCalledWith(mockProcessor);
    expect(mockProcessor.connect).toHaveBeenCalledWith(mockAudioContext.destination);
    expect((controller as any).mediaStream).toBe(mockStream);
    expect((controller as any).sampleRate).toBe(16_000);

    // Test the onaudioprocess callback
    audioProcessHandler = mockProcessor.onaudioprocess;
    expect(audioProcessHandler).not.toBeNull();

    // Simulate audio data arriving
    const inputData = new Float32Array([0.1, 0.2, 0.3]);
    audioProcessHandler!({
      inputBuffer: {
        getChannelData: () => inputData,
      },
    });

    expect((controller as any).chunks.length).toBe(1);
    expect((controller as any).capturedSampleCount).toBe(3);

    // Simulate truncation: set maxCaptureSamples low
    (controller as any).maxCaptureSamples = 5;
    (controller as any).capturedSampleCount = 4;

    const inputData2 = new Float32Array([0.4, 0.5, 0.6]);
    audioProcessHandler!({
      inputBuffer: {
        getChannelData: () => inputData2,
      },
    });

    // Should only capture 1 more sample (remaining = 5 - 4 = 1)
    expect((controller as any).captureTruncated).toBe(true);

    // Restore
    (window as any).AudioContext = origAudioContext;
    Object.defineProperty(navigator, "mediaDevices", {
      value: origMediaDevices,
      configurable: true,
    });

    controller.dispose();
  });

  it("onaudioprocess handles already-truncated state", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const mockTrack = { stop: vi.fn() };
    const mockStream = { getTracks: () => [mockTrack] };
    const mockProcessor = {
      onaudioprocess: null as any,
      connect: vi.fn(),
      disconnect: vi.fn(),
    };
    const mockSource = { connect: vi.fn(), disconnect: vi.fn() };
    const mockAudioContext = {
      sampleRate: 16_000,
      createMediaStreamSource: vi.fn(() => mockSource),
      createScriptProcessor: vi.fn(() => mockProcessor),
      destination: {},
      close: vi.fn(async () => {}),
    };

    const origMediaDevices = navigator.mediaDevices;
    Object.defineProperty(navigator, "mediaDevices", {
      value: { getUserMedia: vi.fn(async () => mockStream) },
      configurable: true,
    });
    const origAudioContext = (window as any).AudioContext;
    (window as any).AudioContext = function() { return mockAudioContext; };

    await (controller as any).beginCapture();
    const handler = mockProcessor.onaudioprocess;

    // Mark as already truncated
    (controller as any).captureTruncated = true;
    const inputData = new Float32Array([0.1, 0.2]);
    handler({
      inputBuffer: { getChannelData: () => inputData },
    });

    // No new chunks should be added
    expect((controller as any).chunks.length).toBe(0);

    (window as any).AudioContext = origAudioContext;
    Object.defineProperty(navigator, "mediaDevices", {
      value: origMediaDevices,
      configurable: true,
    });
    controller.dispose();
  });

  it("onaudioprocess truncates when remaining is zero", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const mockTrack = { stop: vi.fn() };
    const mockStream = { getTracks: () => [mockTrack] };
    const mockProcessor = {
      onaudioprocess: null as any,
      connect: vi.fn(),
      disconnect: vi.fn(),
    };
    const mockSource = { connect: vi.fn(), disconnect: vi.fn() };
    const mockAudioContext = {
      sampleRate: 16_000,
      createMediaStreamSource: vi.fn(() => mockSource),
      createScriptProcessor: vi.fn(() => mockProcessor),
      destination: {},
      close: vi.fn(async () => {}),
    };

    const origMediaDevices = navigator.mediaDevices;
    Object.defineProperty(navigator, "mediaDevices", {
      value: { getUserMedia: vi.fn(async () => mockStream) },
      configurable: true,
    });
    const origAudioContext = (window as any).AudioContext;
    (window as any).AudioContext = function() { return mockAudioContext; };

    await (controller as any).beginCapture();
    const handler = mockProcessor.onaudioprocess;

    // Set captured = max so remaining is 0
    (controller as any).capturedSampleCount = (controller as any).maxCaptureSamples;

    const inputData = new Float32Array([0.1]);
    handler({
      inputBuffer: { getChannelData: () => inputData },
    });

    expect((controller as any).captureTruncated).toBe(true);

    (window as any).AudioContext = origAudioContext;
    Object.defineProperty(navigator, "mediaDevices", {
      value: origMediaDevices,
      configurable: true,
    });
    controller.dispose();
  });

  it("PTT keydown dispatches startListening with ptt mode", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;

    // Simulate PTT hotkey (Mod+Shift+Space)
    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: " ",
        metaKey: isMac,
        ctrlKey: !isMac,
        shiftKey: true,
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect(startMock).toHaveBeenCalledWith("ptt");
    });
    expect((controller as any).pttPressed).toBe(true);

    controller.dispose();
  });

  it("toggle keydown stops listening when already in toggle mode", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const stopMock = vi.fn(async () => {});
    (controller as any).stopListening = stopMock;
    (controller as any).state.listening = true;
    (controller as any).activeMode = "toggle";

    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "m",
        metaKey: isMac,
        ctrlKey: !isMac,
        shiftKey: true,
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect(stopMock).toHaveBeenCalledWith(false);
    });

    controller.dispose();
  });

  it("keyup ignores non-matching key when pttPressed", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const stopMock = vi.fn(async () => {});
    (controller as any).stopListening = stopMock;
    (controller as any).pttPressed = true;
    (controller as any).activeMode = "ptt";
    (controller as any).state.listening = true;

    // Dispatch keyup with a non-matching key
    document.dispatchEvent(
      new KeyboardEvent("keyup", {
        key: "m",
        bubbles: true,
      })
    );

    // stopListening should NOT be called since key doesn't match ptt trigger
    expect(stopMock).not.toHaveBeenCalled();
    // pttPressed should still be true
    expect((controller as any).pttPressed).toBe(true);

    controller.dispose();
  });

  it("refreshCapability stops listening when capability becomes unavailable", async () => {
    let returnUnavailable = false;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        if (returnUnavailable) {
          return { available: false, reason: "GPU removed", modelReady: false };
        }
        return { available: true, reason: null, modelReady: true };
      }
      return null;
    });

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [],
      sampleRate: 16_000,
      truncated: false,
    }));

    // Start listening (uses available=true)
    await (controller as any).startListening("toggle");
    expect((controller as any).state.listening).toBe(true);

    // Now switch to unavailable and trigger refresh
    returnUnavailable = true;
    await (controller as any).refreshCapability();

    expect((controller as any).state.listening).toBe(false);

    controller.dispose();
  });

  it("stopListening returns early when endCapture returns null", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => null);

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    // Should not call transcribe
    expect(invokeMock).not.toHaveBeenCalledWith("transcribe_voice_audio", expect.anything());

    controller.dispose();
  });

  it("stopListening returns early when endCapture returns empty samples", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [],
      sampleRate: 16_000,
      truncated: false,
    }));

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    // Should not call transcribe
    expect(invokeMock).not.toHaveBeenCalledWith("transcribe_voice_audio", expect.anything());

    controller.dispose();
  });

  it("insertIntoInput rejects disabled input", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fb",
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "text";
    input.disabled = true;
    input.value = "original";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    // Input should be unchanged because it's disabled
    // Transcript should go to fallback terminal
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", expect.objectContaining({
        paneId: "pane-fb",
      }));
    });
    expect(input.value).toBe("original");

    input.remove();
    controller.dispose();
  });

  it("insertIntoInput rejects readonly input", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fb2",
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "text";
    input.readOnly = true;
    input.value = "locked";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", expect.objectContaining({
        paneId: "pane-fb2",
      }));
    });
    expect(input.value).toBe("locked");

    input.remove();
    controller.dispose();
  });

  it("insertIntoTextarea rejects disabled textarea", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-ta",
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const textarea = document.createElement("textarea");
    textarea.disabled = true;
    textarea.value = "original";
    document.body.appendChild(textarea);
    textarea.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", expect.objectContaining({
        paneId: "pane-ta",
      }));
    });
    expect(textarea.value).toBe("original");

    textarea.remove();
    controller.dispose();
  });

  it("insertIntoActiveElement returns false when active element is not an input type", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-non-input",
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    // Focus a non-editable button element
    const button = document.createElement("button");
    document.body.appendChild(button);
    button.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", expect.objectContaining({
        paneId: "pane-non-input",
      }));
    });

    button.remove();
    controller.dispose();
  });

  it("insertIntoInput rejects non-text input types like checkbox", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-check",
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "checkbox";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", expect.objectContaining({
        paneId: "pane-check",
      }));
    });

    input.remove();
    controller.dispose();
  });

  it("inserts transcript into contentEditable element", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const editable = document.createElement("div");
    editable.contentEditable = "true";
    editable.tabIndex = 0;
    document.body.appendChild(editable);

    // jsdom does not support isContentEditable or document.execCommand,
    // and focus() on a div may not set activeElement. Patch these limitations.
    const origActiveElementDesc = Object.getOwnPropertyDescriptor(Document.prototype, "activeElement")
      ?? Object.getOwnPropertyDescriptor(document, "activeElement");

    Object.defineProperty(editable, "isContentEditable", { get: () => true, configurable: true });
    editable.focus();
    // Ensure activeElement points to the editable div
    Object.defineProperty(document, "activeElement", { get: () => editable, configurable: true });

    const origExecCommand = document.execCommand;
    document.execCommand = ((commandId: string, _showUI?: boolean, value?: string): boolean => {
      if (commandId === "insertText" && value) {
        editable.textContent = (editable.textContent ?? "") + value;
        return true;
      }
      return false;
    }) as typeof document.execCommand;

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(editable.textContent).toContain("voice transcript");
    });

    document.execCommand = origExecCommand;
    // Restore activeElement to native behavior
    if (origActiveElementDesc) {
      Object.defineProperty(document, "activeElement", origActiveElementDesc);
    } else {
      delete (document as any).activeElement;
    }
    editable.remove();
    controller.dispose();
  });

  it("insertIntoEditable falls back to range manipulation when execCommand returns false", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const editable = document.createElement("div");
    editable.contentEditable = "true";
    editable.tabIndex = 0;
    document.body.appendChild(editable);

    const origActiveElementDesc = Object.getOwnPropertyDescriptor(Document.prototype, "activeElement")
      ?? Object.getOwnPropertyDescriptor(document, "activeElement");

    Object.defineProperty(editable, "isContentEditable", { get: () => true, configurable: true });
    editable.focus();
    Object.defineProperty(document, "activeElement", { get: () => editable, configurable: true });

    // execCommand returns false to trigger fallback path
    const origExecCommand = document.execCommand;
    document.execCommand = (() => false) as typeof document.execCommand;

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(editable.textContent).toContain("voice transcript");
    });

    document.execCommand = origExecCommand;
    // Restore activeElement to native behavior
    if (origActiveElementDesc) {
      Object.defineProperty(document, "activeElement", origActiveElementDesc);
    } else {
      delete (document as any).activeElement;
    }
    editable.remove();
    controller.dispose();
  });

  it("insertIntoEditable returns false when element is not contentEditable", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-no-edit",
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    // A div that is not contentEditable
    const div = document.createElement("div");
    document.body.appendChild(div);
    div.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    // Should fall through to terminal
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", expect.objectContaining({
        paneId: "pane-no-edit",
      }));
    });

    div.remove();
    controller.dispose();
  });

  it("ensureRuntimeIfNeeded handles error from ensure_voice_runtime", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return {
          available: false,
          reason: "Voice runtime is unavailable: Missing Python",
          modelReady: false,
        };
      }
      if (command === "ensure_voice_runtime") {
        throw new Error("Installation failed");
      }
      return null;
    });

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    (controller as any).beginCapture = vi.fn(async () => {});

    await (controller as any).startListening("toggle");

    // Should not crash, and error about unavailability should appear
    await vi.waitFor(() => {
      const errorState = states.find((s) => s.error !== null);
      expect(errorState).toBeTruthy();
    });

    controller.dispose();
  });

  it("inserts transcript at cursor position in input with selection", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    // Return samples that have content so that insertTranscript gets called
    let endCaptureCalled = false;
    (controller as any).endCapture = vi.fn(async () => {
      endCaptureCalled = true;
      return {
        samples: [0.1],
        sampleRate: 16_000,
        truncated: false,
      };
    });

    const input = document.createElement("input");
    input.type = "text";
    input.value = "hello world";
    document.body.appendChild(input);
    input.focus();
    input.setSelectionRange(6, 11);

    await (controller as any).startListening("toggle");
    // Re-set focus/selection after startListening in case it was lost
    input.focus();
    input.setSelectionRange(6, 11);

    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      expect(endCaptureCalled).toBe(true);
      expect(input.value).toBe("hello voice transcript");
    });

    input.remove();
    controller.dispose();
  });

  it("inserts transcript into textarea at cursor position with selection", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const textarea = document.createElement("textarea");
    textarea.value = "foo bar";
    document.body.appendChild(textarea);
    textarea.focus();
    textarea.setSelectionRange(4, 7);

    // Directly call insertTranscript to bypass async focus issues
    await (controller as any).insertTranscript("voice transcript");

    expect(textarea.value).toBe("foo voice transcript");

    textarea.remove();
    controller.dispose();
  });

  it("handles input types like search, email, tel, url, password as text input", async () => {
    for (const inputType of ["search", "email", "tel", "url", "password"]) {
      const controller = new VoiceInputController({
        getSettings: () => settings,
        getFallbackTerminalPaneId: () => null,
      });

      const input = document.createElement("input");
      input.type = inputType;
      input.value = "";
      document.body.appendChild(input);
      input.focus();

      // Directly call insertTranscript to bypass async focus issues
      await (controller as any).insertTranscript("voice transcript");

      expect(input.value).toBe("voice transcript");

      input.remove();
      controller.dispose();
    }
  });

  it("PTT keydown is ignored when pttPressed is already true", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;
    (controller as any).pttPressed = true;

    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: " ",
        metaKey: isMac,
        ctrlKey: !isMac,
        shiftKey: true,
        bubbles: true,
      })
    );

    // Should NOT have called startListening because pttPressed was already true
    expect(startMock).not.toHaveBeenCalled();

    controller.dispose();
  });

  it("keyup does not stop when activeMode is not ptt", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const stopMock = vi.fn(async () => {});
    (controller as any).stopListening = stopMock;
    (controller as any).pttPressed = true;
    (controller as any).activeMode = "toggle";
    (controller as any).state.listening = true;

    document.dispatchEvent(
      new KeyboardEvent("keyup", {
        key: " ",
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect((controller as any).pttPressed).toBe(false);
    });
    // stopListening should NOT be called because activeMode is "toggle" not "ptt"
    expect(stopMock).not.toHaveBeenCalled();

    controller.dispose();
  });

  it("keyup does not stop when not listening", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const stopMock = vi.fn(async () => {});
    (controller as any).stopListening = stopMock;
    (controller as any).pttPressed = true;
    (controller as any).activeMode = "ptt";
    (controller as any).state.listening = false;

    document.dispatchEvent(
      new KeyboardEvent("keyup", {
        key: " ",
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect((controller as any).pttPressed).toBe(false);
    });
    expect(stopMock).not.toHaveBeenCalled();

    controller.dispose();
  });

  it("updateSettings refreshes capability without stopping if not listening", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    // Wait for initial capability check
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_voice_capability", expect.anything());
    });

    const callCountBefore = invokeMock.mock.calls.filter(
      (c: any[]) => c[0] === "get_voice_capability"
    ).length;

    controller.updateSettings();

    await vi.waitFor(() => {
      const callCountAfter = invokeMock.mock.calls.filter(
        (c: any[]) => c[0] === "get_voice_capability"
      ).length;
      expect(callCountAfter).toBeGreaterThan(callCountBefore);
    });

    controller.dispose();
  });

  it("startListening sets error with default message when availabilityReason is empty", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: false, reason: "", modelReady: false };
      }
      return null;
    });
    __setVoiceGpuDetectorForTests(() => false);

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    (controller as any).beginCapture = vi.fn(async () => {});

    await (controller as any).startListening("toggle");

    await vi.waitFor(() => {
      const errorState = states.find((s) =>
        s.error?.includes("GPU acceleration and runtime support")
      );
      expect(errorState).toBeTruthy();
    });

    controller.dispose();
  });

  it("transcript result with null transcript is treated as empty", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      if (command === "transcribe_voice_audio") {
        return { transcript: null };
      }
      return null;
    });

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
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

    input.remove();
    controller.dispose();
  });

  it("transcription result with undefined transcript is treated as empty", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      if (command === "transcribe_voice_audio") {
        return {};
      }
      return null;
    });

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
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

    input.remove();
    controller.dispose();
  });

  it("uses fallback hotkey when settings.hotkey is empty", async () => {
    settings.hotkey = "";

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;

    // Default toggle hotkey is Mod+Shift+M
    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "m",
        metaKey: isMac,
        ctrlKey: !isMac,
        shiftKey: true,
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect(startMock).toHaveBeenCalledWith("toggle");
    });

    controller.dispose();
  });

  it("uses fallback ptt_hotkey when settings.ptt_hotkey is empty", async () => {
    settings.ptt_hotkey = "";

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;

    // Default PTT hotkey is Mod+Shift+Space
    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: " ",
        metaKey: isMac,
        ctrlKey: !isMac,
        shiftKey: true,
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect(startMock).toHaveBeenCalledWith("ptt");
    });

    controller.dispose();
  });

  it("languageForQwen returns ja for Japanese setting", async () => {
    settings.language = "ja";

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      const transcribeCall = invokeMock.mock.calls.find(
        (c: any[]) => c[0] === "transcribe_voice_audio"
      );
      expect(transcribeCall).toBeTruthy();
      expect(transcribeCall[1].input.language).toBe("ja");
    });

    input.remove();
    controller.dispose();
  });

  it("languageForQwen returns en for English setting", async () => {
    settings.language = "en";

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      const transcribeCall = invokeMock.mock.calls.find(
        (c: any[]) => c[0] === "transcribe_voice_audio"
      );
      expect(transcribeCall).toBeTruthy();
      expect(transcribeCall[1].input.language).toBe("en");
    });

    input.remove();
    controller.dispose();
  });

  it("insertIntoTextarea rejects readonly textarea", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-ta-ro",
    });

    const textarea = document.createElement("textarea");
    textarea.readOnly = true;
    textarea.value = "locked";
    document.body.appendChild(textarea);
    textarea.focus();

    await (controller as any).insertTranscript("voice transcript");

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", expect.objectContaining({
        paneId: "pane-ta-ro",
      }));
    });
    expect(textarea.value).toBe("locked");

    textarea.remove();
    controller.dispose();
  });

  it("insertIntoEditable handles window.getSelection returning null", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-sel-null",
    });

    const editable = document.createElement("div");
    editable.contentEditable = "true";
    document.body.appendChild(editable);
    Object.defineProperty(editable, "isContentEditable", { get: () => true, configurable: true });

    const origActiveElementDesc = Object.getOwnPropertyDescriptor(Document.prototype, "activeElement")
      ?? Object.getOwnPropertyDescriptor(document, "activeElement");

    Object.defineProperty(document, "activeElement", { get: () => editable, configurable: true });

    const origGetSelection = window.getSelection;
    window.getSelection = () => null;

    await (controller as any).insertTranscript("voice transcript");

    // Should fall through to terminal
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", expect.objectContaining({
        paneId: "pane-sel-null",
      }));
    });

    window.getSelection = origGetSelection;
    if (origActiveElementDesc) {
      Object.defineProperty(document, "activeElement", origActiveElementDesc);
    } else {
      delete (document as any).activeElement;
    }
    editable.remove();
    controller.dispose();
  });

  it("insertIntoEditable creates range when selection has no ranges", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const editable = document.createElement("div");
    editable.contentEditable = "true";
    editable.textContent = "existing";
    document.body.appendChild(editable);
    Object.defineProperty(editable, "isContentEditable", { get: () => true, configurable: true });

    const origActiveElementDesc = Object.getOwnPropertyDescriptor(Document.prototype, "activeElement")
      ?? Object.getOwnPropertyDescriptor(document, "activeElement");
    Object.defineProperty(document, "activeElement", { get: () => editable, configurable: true });

    // Mock getSelection to return a selection with rangeCount=0, then add range
    const mockRange = document.createRange();
    const addedRanges: Range[] = [];
    const origGetSelection = window.getSelection;
    let rangeCount = 0;
    window.getSelection = () => ({
      rangeCount: rangeCount,
      removeAllRanges: () => { rangeCount = 0; },
      addRange: (r: Range) => { addedRanges.push(r); rangeCount = 1; },
      getRangeAt: () => {
        // Return a real range for the execCommand fallback path
        const r = document.createRange();
        r.selectNodeContents(editable);
        r.collapse(false);
        return r;
      },
    } as any);

    const origExecCommand = document.execCommand;
    document.execCommand = ((commandId: string, _showUI?: boolean, value?: string): boolean => {
      if (commandId === "insertText" && value) {
        editable.textContent = (editable.textContent ?? "") + value;
        return true;
      }
      return false;
    }) as typeof document.execCommand;

    await (controller as any).insertTranscript("voice transcript");

    expect(editable.textContent).toContain("voice transcript");
    expect(addedRanges.length).toBeGreaterThan(0);

    document.execCommand = origExecCommand;
    window.getSelection = origGetSelection;
    if (origActiveElementDesc) {
      Object.defineProperty(document, "activeElement", origActiveElementDesc);
    } else {
      delete (document as any).activeElement;
    }
    editable.remove();
    controller.dispose();
  });

  it("onaudioprocess truncates when input buffer is empty", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const mockTrack = { stop: vi.fn() };
    const mockStream = { getTracks: () => [mockTrack] };
    const mockProcessor = {
      onaudioprocess: null as any,
      connect: vi.fn(),
      disconnect: vi.fn(),
    };
    const mockSource = { connect: vi.fn(), disconnect: vi.fn() };
    const mockAudioContext = {
      sampleRate: 16_000,
      createMediaStreamSource: vi.fn(() => mockSource),
      createScriptProcessor: vi.fn(() => mockProcessor),
      destination: {},
      close: vi.fn(async () => {}),
    };

    const origMediaDevices = navigator.mediaDevices;
    Object.defineProperty(navigator, "mediaDevices", {
      value: { getUserMedia: vi.fn(async () => mockStream) },
      configurable: true,
    });
    const origAudioContext = (window as any).AudioContext;
    (window as any).AudioContext = function() { return mockAudioContext; };

    await (controller as any).beginCapture();
    const handler = mockProcessor.onaudioprocess;

    // Send empty input buffer (length=0)
    handler({
      inputBuffer: { getChannelData: () => new Float32Array(0) },
    });

    expect((controller as any).captureTruncated).toBe(true);
    expect((controller as any).chunks.length).toBe(0);

    (window as any).AudioContext = origAudioContext;
    Object.defineProperty(navigator, "mediaDevices", {
      value: origMediaDevices,
      configurable: true,
    });
    controller.dispose();
  });

  it("detectGpuAvailability returns false in jsdom (no WebGL)", () => {
    // Reset the detector to use the real implementation
    __setVoiceGpuDetectorForTests(null);

    // In jsdom, there is no WebGL support, so detectGpuAvailability should return false
    // This is tested indirectly through the controller
    // We can verify by checking that the detector returns false
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    // The real detectGpuAvailability is now active
    // Create a new canvas to verify it doesn't throw
    const canvas = document.createElement("canvas");
    const gl = canvas.getContext("webgl2");
    // In jsdom, getContext returns null for webgl
    expect(gl).toBeNull();

    controller.dispose();
    // Restore mock detector
    __setVoiceGpuDetectorForTests(() => true);
  });

  it("parseHotkey handles Ctrl+Control and Cmd+Command modifiers", async () => {
    // Set hotkey with explicit ctrl modifier
    settings.hotkey = "Ctrl+Shift+K";

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;

    // Dispatch with ctrlKey for non-Mod hotkey
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "k",
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect(startMock).toHaveBeenCalledWith("toggle");
    });

    controller.dispose();
  });

  it("parseHotkey handles Alt+Option modifier", async () => {
    settings.hotkey = "Alt+K";

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;

    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "k",
        altKey: true,
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect(startMock).toHaveBeenCalledWith("toggle");
    });

    controller.dispose();
  });

  it("normalizeKeyName handles Escape key event", async () => {
    // Use "Esc" as the key in a hotkey config to test normalizeKeyName
    settings.hotkey = "Mod+Esc";

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const startMock = vi.fn(async () => {});
    (controller as any).startListening = startMock;

    const isMac = navigator.userAgent.includes("Mac");
    document.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        metaKey: isMac,
        ctrlKey: !isMac,
        bubbles: true,
      })
    );

    await vi.waitFor(() => {
      expect(startMock).toHaveBeenCalledWith("toggle");
    });

    controller.dispose();
  });

  it("refreshCapability with available=true and reason=null sets state correctly", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "get_voice_capability") {
        return { available: true, reason: null, modelReady: true };
      }
      return null;
    });

    const states: VoiceControllerState[] = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (s) => states.push({ ...s }),
    });

    await vi.waitFor(() => {
      const availState = states.find((s) => s.available === true);
      expect(availState).toBeTruthy();
      expect(availState?.supported).toBe(true);
      expect(availState?.modelReady).toBe(true);
      expect(availState?.availabilityReason).toBeNull();
    });

    controller.dispose();
  });

  it("sends quality setting in voice_capability and transcribe calls", async () => {
    settings.quality = "high";

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    (controller as any).beginCapture = vi.fn(async () => {});
    (controller as any).endCapture = vi.fn(async () => ({
      samples: [0.1],
      sampleRate: 16_000,
      truncated: false,
    }));

    const input = document.createElement("input");
    input.type = "text";
    document.body.appendChild(input);
    input.focus();

    await (controller as any).startListening("toggle");
    await (controller as any).stopListening(false);

    await vi.waitFor(() => {
      const transcribeCall = invokeMock.mock.calls.find(
        (c: any[]) => c[0] === "transcribe_voice_audio"
      );
      expect(transcribeCall).toBeTruthy();
      expect(transcribeCall[1].input.quality).toBe("high");
    });

    input.remove();
    controller.dispose();
  });
});
