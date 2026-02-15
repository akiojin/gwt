import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearTerminalInputTargetsForTests,
  registerTerminalInputTarget,
} from "./inputTargetRegistry";
import {
  VoiceInputController,
  type VoiceControllerSettings,
} from "./voiceInputController";

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

type ResultPayload = {
  isFinal: boolean;
  length: number;
  0: { transcript: string };
};

class FakeSpeechRecognition {
  static instances: FakeSpeechRecognition[] = [];
  static throwOnStart = false;

  continuous = false;
  interimResults = false;
  lang = "";
  maxAlternatives = 1;

  onresult: ((event: any) => void) | null = null;
  onerror: ((event: any) => void) | null = null;
  onend: ((event: Event) => void) | null = null;

  start = vi.fn(() => {
    if (FakeSpeechRecognition.throwOnStart) {
      throw new Error("start failed");
    }
  });
  stop = vi.fn(() => {
    this.onend?.(new Event("end"));
  });

  constructor() {
    FakeSpeechRecognition.instances.push(this);
  }

  emitFinalTranscript(text: string) {
    const payload: ResultPayload = {
      isFinal: true,
      length: 1,
      0: { transcript: text },
    };
    this.onresult?.({
      resultIndex: 0,
      results: [payload],
    });
  }

  emitError(reason: string) {
    this.onerror?.({
      error: reason,
    });
  }
}

function dispatchVoiceHotkey(
  key = "M",
  modifiers: Partial<Pick<KeyboardEventInit, "ctrlKey" | "metaKey" | "shiftKey" | "altKey">> = {}
) {
  const event = new KeyboardEvent("keydown", {
    key,
    ctrlKey: modifiers.ctrlKey ?? true,
    metaKey: modifiers.metaKey ?? false,
    shiftKey: modifiers.shiftKey ?? true,
    altKey: modifiers.altKey ?? false,
    bubbles: true,
    cancelable: true,
  });
  document.dispatchEvent(event);
}

describe("VoiceInputController", () => {
  let settings: VoiceControllerSettings;

  beforeEach(() => {
    settings = {
      enabled: true,
      hotkey: "Mod+Shift+M",
      language: "auto",
      model: "base",
    };

    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    clearTerminalInputTargetsForTests();
    FakeSpeechRecognition.instances = [];
    FakeSpeechRecognition.throwOnStart = false;
    (window as any).SpeechRecognition = FakeSpeechRecognition;
  });

  afterEach(() => {
    clearTerminalInputTargetsForTests();
    FakeSpeechRecognition.instances = [];
    delete (window as any).SpeechRecognition;
  });

  it("starts listening by hotkey and inserts transcript into focused input", async () => {
    const states: Array<{ listening: boolean; error: string | null }> = [];

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (state) => {
        states.push({ listening: state.listening, error: state.error });
      },
    });

    const input = document.createElement("input");
    input.type = "text";
    input.value = "task: ";
    document.body.appendChild(input);
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);

    dispatchVoiceHotkey();
    await Promise.resolve();

    expect(FakeSpeechRecognition.instances.length).toBe(1);
    FakeSpeechRecognition.instances[0].emitFinalTranscript("voice transcript");

    expect(input.value).toBe("task: voice transcript");

    controller.dispose();
    input.remove();
    expect(states.some((s) => s.listening)).toBe(true);
  });

  it("sends transcript to terminal when terminal target is focused", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const root = document.createElement("div");
    root.tabIndex = 0;
    document.body.appendChild(root);

    const unregister = registerTerminalInputTarget("pane-test", root);
    root.focus();

    dispatchVoiceHotkey();
    await Promise.resolve();

    FakeSpeechRecognition.instances[0].emitFinalTranscript("ls -la");
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", {
        paneId: "pane-test",
        data: Array.from(new TextEncoder().encode("ls -la")),
      });
    });

    unregister();
    root.remove();
    controller.dispose();
  });

  it("does not start when voice input is disabled", async () => {
    settings.enabled = false;

    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    dispatchVoiceHotkey();
    await Promise.resolve();

    expect(FakeSpeechRecognition.instances.length).toBe(0);
    controller.dispose();
  });

  it("maps language setting to recognition.lang", async () => {
    settings.language = "ja";
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    dispatchVoiceHotkey();
    await Promise.resolve();

    expect(FakeSpeechRecognition.instances[0].lang).toBe("ja-JP");
    controller.dispose();
  });

  it("supports custom non-mod hotkey definitions", async () => {
    settings.hotkey = "Ctrl+Shift+K";
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    dispatchVoiceHotkey("M");
    await Promise.resolve();
    expect(FakeSpeechRecognition.instances.length).toBe(0);

    dispatchVoiceHotkey("k", { ctrlKey: true, shiftKey: true });
    await Promise.resolve();
    expect(FakeSpeechRecognition.instances.length).toBe(1);
    controller.dispose();
  });

  it("reports unsupported state when SpeechRecognition is unavailable", async () => {
    delete (window as any).SpeechRecognition;

    const states: Array<{ supported: boolean; error: string | null }> = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (state) => states.push({ supported: state.supported, error: state.error }),
    });

    dispatchVoiceHotkey();
    await Promise.resolve();

    expect(states.some((s) => s.supported === false)).toBe(true);
    expect(
      states.some((s) => s.error === "Speech recognition is not supported in this runtime.")
    ).toBe(true);
    controller.dispose();
  });

  it("falls back to fallback terminal pane when no active input element exists", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fallback",
    });

    dispatchVoiceHotkey();
    await Promise.resolve();

    FakeSpeechRecognition.instances[0].emitFinalTranscript("echo fallback");
    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", {
        paneId: "pane-fallback",
        data: Array.from(new TextEncoder().encode("echo fallback")),
      });
    });

    controller.dispose();
  });

  it("sets error when no transcript target is available", async () => {
    const states: Array<{ error: string | null }> = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (state) => states.push({ error: state.error }),
    });

    dispatchVoiceHotkey();
    await Promise.resolve();
    FakeSpeechRecognition.instances[0].emitFinalTranscript("no target");

    await vi.waitFor(() => {
      expect(
        states.some((s) => s.error === "No active input target for voice transcript.")
      ).toBe(true);
    });
    controller.dispose();
  });

  it("sets error when terminal write fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "write_terminal") throw new Error("denied");
      return null;
    });

    const states: Array<{ error: string | null }> = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fallback",
      onStateChange: (state) => states.push({ error: state.error }),
    });

    dispatchVoiceHotkey();
    await Promise.resolve();
    FakeSpeechRecognition.instances[0].emitFinalTranscript("will fail");

    await vi.waitFor(() => {
      expect(
        states.some((s) => s.error === "Failed to send transcript to terminal: Error: denied")
      ).toBe(true);
    });
    controller.dispose();
  });

  it("stops listening when settings are updated to disabled", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    dispatchVoiceHotkey();
    await Promise.resolve();

    const instance = FakeSpeechRecognition.instances[0];
    settings.enabled = false;
    controller.updateSettings();

    expect(instance.stop).toHaveBeenCalled();
    controller.dispose();
  });

  it("handles recognition onerror events", async () => {
    const states: Array<{ listening: boolean; error: string | null }> = [];
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (state) => states.push({ listening: state.listening, error: state.error }),
    });

    dispatchVoiceHotkey();
    await Promise.resolve();

    FakeSpeechRecognition.instances[0].emitError("network");
    expect(
      states.some((s) => s.error === "Voice recognition error: network" && s.listening === false)
    ).toBe(true);
    controller.dispose();
  });

  it("supports default hotkey fallback and maps en language", async () => {
    settings.hotkey = "";
    settings.language = "en";
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    dispatchVoiceHotkey("M", { ctrlKey: true, shiftKey: true });
    await Promise.resolve();

    expect(FakeSpeechRecognition.instances.length).toBe(1);
    expect(FakeSpeechRecognition.instances[0].lang).toBe("en-US");
    controller.dispose();
  });

  it("matches non-character hotkeys and ignores repeat/defaultPrevented events", async () => {
    settings.hotkey = "Ctrl+Shift+ArrowUp";
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    const repeated = new KeyboardEvent("keydown", {
      key: "ArrowUp",
      ctrlKey: true,
      shiftKey: true,
      repeat: true,
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(repeated);

    const prevented = new KeyboardEvent("keydown", {
      key: "ArrowUp",
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    prevented.preventDefault();
    document.dispatchEvent(prevented);

    expect(FakeSpeechRecognition.instances.length).toBe(0);

    const event = new KeyboardEvent("keydown", {
      key: "ArrowUp",
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    document.dispatchEvent(event);
    await Promise.resolve();

    expect(FakeSpeechRecognition.instances.length).toBe(1);
    controller.dispose();
  });

  it("inserts transcript into textarea active element", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fallback",
    });

    const textarea = document.createElement("textarea");
    textarea.value = "note: ";
    document.body.appendChild(textarea);
    textarea.focus();
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);

    dispatchVoiceHotkey();
    await Promise.resolve();
    FakeSpeechRecognition.instances[0].emitFinalTranscript("voice");

    expect(textarea.value).toBe("note: voice");
    controller.dispose();
    textarea.remove();
  });

  it("falls back to terminal when active input cannot be edited", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fallback",
    });

    const input = document.createElement("input");
    input.type = "text";
    input.readOnly = true;
    input.value = "readonly";
    document.body.appendChild(input);
    input.focus();

    dispatchVoiceHotkey();
    await Promise.resolve();
    FakeSpeechRecognition.instances[0].emitFinalTranscript(" append");

    await vi.waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("write_terminal", {
        paneId: "pane-fallback",
        data: Array.from(new TextEncoder().encode("append")),
      });
    });
    expect(input.value).toBe("readonly");

    controller.dispose();
    input.remove();
  });

  it("stops listening when hotkey is pressed while already listening", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
    });

    dispatchVoiceHotkey();
    await Promise.resolve();
    const instance = FakeSpeechRecognition.instances[0];

    dispatchVoiceHotkey();
    await Promise.resolve();

    expect(instance.stop).toHaveBeenCalled();
    controller.dispose();
  });

  it("reports start failure and reuses recognition instance on retry", async () => {
    const states: Array<{ error: string | null }> = [];
    FakeSpeechRecognition.throwOnStart = true;
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => null,
      onStateChange: (state) => states.push({ error: state.error }),
    });

    try {
      dispatchVoiceHotkey();
      await Promise.resolve();
      expect(FakeSpeechRecognition.instances.length).toBe(1);
      expect(
        states.some((s) => s.error?.startsWith("Failed to start voice input: Error: start failed"))
      ).toBe(true);

      FakeSpeechRecognition.throwOnStart = false;
      dispatchVoiceHotkey();
      await Promise.resolve();

      expect(FakeSpeechRecognition.instances.length).toBe(1);
    } finally {
      controller.dispose();
    }
  });

  it("restarts recognition on end while keep-listening is enabled", async () => {
    vi.useFakeTimers();
    let controller: VoiceInputController | null = null;
    try {
      const states: Array<{ listening: boolean }> = [];
      controller = new VoiceInputController({
        getSettings: () => settings,
        getFallbackTerminalPaneId: () => null,
        onStateChange: (state) => states.push({ listening: state.listening }),
      });

      dispatchVoiceHotkey();
      await Promise.resolve();
      const instance = FakeSpeechRecognition.instances[0];
      const startCallsBefore = instance.start.mock.calls.length;

      instance.onend?.(new Event("end"));
      await vi.advanceTimersByTimeAsync(200);

      expect(instance.start.mock.calls.length).toBeGreaterThan(startCallsBefore);
      expect(states.some((s) => s.listening)).toBe(true);

      FakeSpeechRecognition.throwOnStart = true;
      instance.onend?.(new Event("end"));
      await vi.advanceTimersByTimeAsync(200);
      expect(states[states.length - 1]?.listening).toBe(false);
    } finally {
      controller?.dispose();
      vi.useRealTimers();
    }
  });

  it("ignores non-final and empty speech recognition results", async () => {
    const controller = new VoiceInputController({
      getSettings: () => settings,
      getFallbackTerminalPaneId: () => "pane-fallback",
    });

    try {
      dispatchVoiceHotkey();
      await Promise.resolve();
      const instance = FakeSpeechRecognition.instances[0];

      instance.onresult?.({
        resultIndex: 0,
        results: [
          {
            isFinal: false,
            length: 1,
            0: { transcript: "ignored" },
          },
        ],
      });
      instance.onresult?.({
        resultIndex: 0,
        results: [
          {
            isFinal: true,
            length: 1,
            0: { transcript: "   " },
          },
        ],
      });

      expect(invokeMock).not.toHaveBeenCalledWith("write_terminal", expect.anything());
    } finally {
      controller.dispose();
    }
  });
});
