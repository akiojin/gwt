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

  continuous = false;
  interimResults = false;
  lang = "";
  maxAlternatives = 1;

  onresult: ((event: any) => void) | null = null;
  onerror: ((event: any) => void) | null = null;
  onend: ((event: Event) => void) | null = null;

  start = vi.fn(() => {});
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
}

function dispatchVoiceHotkey() {
  const event = new KeyboardEvent("keydown", {
    key: "M",
    ctrlKey: true,
    shiftKey: true,
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
});
