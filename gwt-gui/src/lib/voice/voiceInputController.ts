import { getFocusedTerminalPaneId } from "./inputTargetRegistry";

export interface VoiceControllerSettings {
  enabled: boolean;
  engine: string;
  hotkey: string;
  ptt_hotkey: string;
  language: string;
  quality: string;
  model: string;
}

export interface VoiceControllerState {
  supported: boolean;
  available: boolean;
  availabilityReason: string | null;
  listening: boolean;
  preparing: boolean;
  modelReady: boolean;
  error: string | null;
}

export interface VoiceInputControllerOptions {
  getSettings: () => VoiceControllerSettings;
  getFallbackTerminalPaneId: () => string | null;
  onStateChange?: (state: VoiceControllerState) => void;
}

type HotkeyDefinition = {
  useMod: boolean;
  ctrl: boolean;
  meta: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
};

type CaptureMode = "toggle" | "ptt";

type VoiceCapabilityResponse = {
  available: boolean;
  reason?: string | null;
  modelReady: boolean;
};

type VoicePrepareModelResponse = {
  ready: boolean;
};

type VoiceRuntimeSetupResponse = {
  ready: boolean;
  installed: boolean;
  pythonPath: string;
};

type VoiceTranscriptionResponse = {
  transcript: string;
};

type TauriInvokeFn = <T>(command: string, payload?: unknown) => Promise<T>;

const DEFAULT_TOGGLE_HOTKEY = "Mod+Shift+M";
const DEFAULT_PTT_HOTKEY = "Mod+Shift+Space";

function parseHotkey(rawHotkey: string): HotkeyDefinition {
  const tokens = rawHotkey
    .split("+")
    .map((t) => t.trim().toLowerCase())
    .filter((t) => t.length > 0);

  const modifiers = new Set(tokens);
  const keyToken = tokens.find(
    (t) =>
      t !== "mod" &&
      t !== "ctrl" &&
      t !== "control" &&
      t !== "meta" &&
      t !== "cmd" &&
      t !== "command" &&
      t !== "shift" &&
      t !== "alt" &&
      t !== "option"
  );

  return {
    useMod: modifiers.has("mod"),
    ctrl: modifiers.has("ctrl") || modifiers.has("control"),
    meta: modifiers.has("meta") || modifiers.has("cmd") || modifiers.has("command"),
    shift: modifiers.has("shift"),
    alt: modifiers.has("alt") || modifiers.has("option"),
    key: normalizeKeyName(keyToken ?? "m"),
  };
}

function normalizeHotkey(
  rawHotkey: string | undefined | null,
  fallback: string
): HotkeyDefinition {
  const value = (rawHotkey ?? "").trim();
  if (!value) {
    return parseHotkey(fallback);
  }
  return parseHotkey(value);
}

function normalizeKey(value: string): string {
  if (value.length === 1) return value.toLowerCase();
  return value.toLowerCase();
}

function normalizeKeyName(value: string): string {
  const key = normalizeKey(value);
  if (key === "space" || key === "spacebar") return " ";
  if (key === "esc") return "escape";
  return key;
}

function eventMatchesHotkey(event: KeyboardEvent, hotkey: HotkeyDefinition): boolean {
  const expectedCtrl = hotkey.useMod
    ? !navigator.userAgent.includes("Mac")
    : hotkey.ctrl;
  const expectedMeta = hotkey.useMod
    ? navigator.userAgent.includes("Mac")
    : hotkey.meta;

  return (
    normalizeKeyName(event.key) === hotkey.key &&
    event.ctrlKey === expectedCtrl &&
    event.metaKey === expectedMeta &&
    event.shiftKey === hotkey.shift &&
    event.altKey === hotkey.alt
  );
}

function languageForQwen(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "ja") return "ja";
  if (normalized === "en") return "en";
  return "auto";
}

function detectGpuAvailability(): boolean {
  try {
    const canvas = document.createElement("canvas");
    const gl =
      canvas.getContext("webgl2") ||
      (canvas.getContext("webgl") as WebGLRenderingContext | null) ||
      (canvas.getContext("experimental-webgl") as WebGLRenderingContext | null);

    if (!gl) return false;

    const ext = gl.getExtension("WEBGL_debug_renderer_info") as {
      UNMASKED_RENDERER_WEBGL: number;
    } | null;
    const renderer = ext ? String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) ?? "") : "";
    const normalized = renderer.toLowerCase();

    if (
      normalized.includes("swiftshader") ||
      normalized.includes("llvmpipe") ||
      normalized.includes("software") ||
      normalized.includes("mesa offscreen")
    ) {
      return false;
    }

    return true;
  } catch {
    return false;
  }
}

async function defaultInvokeTauri<T>(command: string, payload?: unknown): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, payload as Record<string, unknown> | undefined);
}

let invokeTauri: TauriInvokeFn = defaultInvokeTauri;
let gpuAvailabilityDetector: () => boolean = detectGpuAvailability;

export function __setVoiceInvokeForTests(invoker: TauriInvokeFn | null) {
  invokeTauri = invoker ?? defaultInvokeTauri;
}

export function __setVoiceGpuDetectorForTests(detector: (() => boolean) | null) {
  gpuAvailabilityDetector = detector ?? detectGpuAvailability;
}

function isTextInputElement(element: HTMLInputElement): boolean {
  const type = (element.type ?? "text").toLowerCase();
  return (
    type === "" ||
    type === "text" ||
    type === "search" ||
    type === "url" ||
    type === "email" ||
    type === "tel" ||
    type === "password"
  );
}

function dispatchTextInputEvent(target: HTMLElement) {
  target.dispatchEvent(new Event("input", { bubbles: true, cancelable: false }));
}

function insertIntoInput(element: HTMLInputElement, text: string): boolean {
  if (element.disabled || element.readOnly || !isTextInputElement(element)) return false;

  const original = element.value;
  const start = element.selectionStart ?? original.length;
  const end = element.selectionEnd ?? original.length;
  element.value = original.slice(0, start) + text + original.slice(end);

  const cursor = start + text.length;
  try {
    element.setSelectionRange(cursor, cursor);
  } catch {
    // Ignore unsupported selection APIs for certain input types.
  }

  dispatchTextInputEvent(element);
  return true;
}

function insertIntoTextarea(element: HTMLTextAreaElement, text: string): boolean {
  if (element.disabled || element.readOnly) return false;

  const original = element.value;
  const start = element.selectionStart ?? original.length;
  const end = element.selectionEnd ?? original.length;
  element.value = original.slice(0, start) + text + original.slice(end);

  const cursor = start + text.length;
  element.setSelectionRange(cursor, cursor);
  dispatchTextInputEvent(element);
  return true;
}

function insertIntoEditable(element: HTMLElement, text: string): boolean {
  if (!element.isContentEditable) return false;

  element.focus();
  const selection = window.getSelection();
  if (!selection) return false;

  if (selection.rangeCount === 0) {
    const range = document.createRange();
    range.selectNodeContents(element);
    range.collapse(false);
    selection.removeAllRanges();
    selection.addRange(range);
  }

  const ok = document.execCommand("insertText", false, text);
  if (!ok) {
    const range = selection.getRangeAt(0);
    range.deleteContents();
    range.insertNode(document.createTextNode(text));
    range.collapse(false);
    selection.removeAllRanges();
    selection.addRange(range);
  }

  dispatchTextInputEvent(element);
  return true;
}

function insertIntoActiveElement(text: string): boolean {
  const active = document.activeElement;
  if (!active) return false;

  if (active instanceof HTMLInputElement) {
    return insertIntoInput(active, text);
  }

  if (active instanceof HTMLTextAreaElement) {
    return insertIntoTextarea(active, text);
  }

  if (active instanceof HTMLElement) {
    return insertIntoEditable(active, text);
  }

  return false;
}

export class VoiceInputController {
  private readonly options: VoiceInputControllerOptions;
  private state: VoiceControllerState = {
    supported: true,
    available: false,
    availabilityReason: null,
    listening: false,
    preparing: false,
    modelReady: false,
    error: null,
  };

  private startInFlight = false;
  private runtimeBootstrapAttempted = false;
  private pttPressed = false;
  private activeMode: CaptureMode | null = null;

  private mediaStream: MediaStream | null = null;
  private audioContext: AudioContext | null = null;
  private sourceNode: MediaStreamAudioSourceNode | null = null;
  private processorNode: ScriptProcessorNode | null = null;
  private sampleRate = 44_100;
  private chunks: Float32Array[] = [];

  constructor(options: VoiceInputControllerOptions) {
    this.options = options;
    document.addEventListener("keydown", this.handleKeydown, true);
    document.addEventListener("keyup", this.handleKeyup, true);
    void this.refreshCapability();
  }

  updateSettings() {
    const settings = this.options.getSettings();
    if (!settings.enabled && this.state.listening) {
      void this.stopListening(false);
    }
    void this.refreshCapability();
  }

  dispose() {
    document.removeEventListener("keydown", this.handleKeydown, true);
    document.removeEventListener("keyup", this.handleKeyup, true);
    this.pttPressed = false;
    void this.stopListening(true);
  }

  private emitState() {
    this.options.onStateChange?.({ ...this.state });
  }

  private setError(message: string | null) {
    this.state.error = message;
    this.emitState();
  }

  private async refreshCapability() {
    const settings = this.options.getSettings();
    const quality = (settings.quality ?? "balanced").trim().toLowerCase();
    const gpuAvailable = gpuAvailabilityDetector();

    try {
      const capability = await invokeTauri<VoiceCapabilityResponse>(
        "get_voice_capability",
        {
          gpuAvailable,
          quality,
        }
      );

      this.state.supported = true;
      this.state.available = !!capability.available;
      this.state.availabilityReason = capability.reason ?? null;
      this.state.modelReady = !!capability.modelReady;
      if (!this.state.available && this.state.listening) {
        await this.stopListening(true);
      }
      this.emitState();
    } catch {
      this.state.supported = false;
      this.state.available = false;
      this.state.modelReady = false;
      this.state.availabilityReason = "Voice runtime is unavailable in this environment.";
      this.emitState();
    }
  }

  private handleKeydown = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.repeat) return;

    const settings = this.options.getSettings();
    if (!settings.enabled) return;

    const toggleHotkey = normalizeHotkey(settings.hotkey, DEFAULT_TOGGLE_HOTKEY);
    const pttHotkey = normalizeHotkey(settings.ptt_hotkey, DEFAULT_PTT_HOTKEY);

    if (eventMatchesHotkey(event, toggleHotkey)) {
      event.preventDefault();
      event.stopPropagation();
      if (this.state.listening && this.activeMode === "toggle") {
        void this.stopListening(false);
      } else {
        void this.startListening("toggle");
      }
      return;
    }

    if (!this.pttPressed && eventMatchesHotkey(event, pttHotkey)) {
      event.preventDefault();
      event.stopPropagation();
      this.pttPressed = true;
      void this.startListening("ptt");
    }
  };

  private handleKeyup = (event: KeyboardEvent) => {
    if (!this.pttPressed) return;

    const settings = this.options.getSettings();
    const pttHotkey = normalizeHotkey(settings.ptt_hotkey, DEFAULT_PTT_HOTKEY);
    if (!eventMatchesHotkey(event, pttHotkey)) return;

    event.preventDefault();
    event.stopPropagation();
    this.pttPressed = false;

    if (this.state.listening && this.activeMode === "ptt") {
      void this.stopListening(false);
    }
  };

  private async startListening(mode: CaptureMode) {
    if (this.startInFlight || this.state.listening) return;

    const settings = this.options.getSettings();
    if (!settings.enabled) return;

    this.startInFlight = true;

    try {
      await this.refreshCapability();
      if (!this.state.available && gpuAvailabilityDetector()) {
        await this.ensureRuntimeIfNeeded();
      }
      if (!this.state.available) {
        this.setError(
          this.state.availabilityReason ||
            "Voice input is unavailable because GPU acceleration and runtime support are required."
        );
        return;
      }

      this.state.preparing = true;
      this.emitState();

      if (!this.state.modelReady) {
        const prep = await invokeTauri<VoicePrepareModelResponse>(
          "prepare_voice_model",
          {
            gpuAvailable: gpuAvailabilityDetector(),
            quality: settings.quality,
          }
        );
        this.state.modelReady = !!prep.ready;
      }

      await this.beginCapture();
      this.activeMode = mode;
      this.state.listening = true;
      this.setError(null);
    } catch (err) {
      this.state.listening = false;
      this.activeMode = null;
      this.setError(`Failed to start voice input: ${String(err)}`);
    } finally {
      this.state.preparing = false;
      this.startInFlight = false;
      this.emitState();
    }
  }

  private async ensureRuntimeIfNeeded() {
    if (this.runtimeBootstrapAttempted) return;
    this.runtimeBootstrapAttempted = true;

    const reason = (this.state.availabilityReason ?? "").toLowerCase();
    if (
      !reason.includes("runtime") &&
      !reason.includes("python") &&
      !reason.includes("qwen")
    ) {
      return;
    }

    this.state.preparing = true;
    this.emitState();

    try {
      const setupResult = await invokeTauri<VoiceRuntimeSetupResponse>(
        "ensure_voice_runtime"
      );
      if (setupResult.ready) {
        await this.refreshCapability();
      }
    } catch {
      // Fall through to normal unavailable handling.
    } finally {
      this.state.preparing = false;
      this.emitState();
    }
  }

  private async stopListening(discardAudio: boolean) {
    if (!this.state.listening && !this.mediaStream) return;

    this.state.listening = false;
    this.emitState();

    const capture = await this.endCapture();
    this.activeMode = null;

    if (discardAudio || !capture || capture.samples.length === 0) {
      return;
    }

    this.state.preparing = true;
    this.emitState();

    const settings = this.options.getSettings();
    try {
      const result = await invokeTauri<VoiceTranscriptionResponse>(
        "transcribe_voice_audio",
        {
          input: {
            samples: capture.samples,
            sampleRate: capture.sampleRate,
            language: languageForQwen(settings.language),
            quality: settings.quality,
            gpuAvailable: gpuAvailabilityDetector(),
          },
        }
      );

      const transcript = (result?.transcript ?? "").trim();
      if (!transcript) {
        return;
      }
      await this.insertTranscript(transcript);
      this.setError(null);
    } catch (err) {
      this.setError(`Voice transcription failed: ${String(err)}`);
    } finally {
      this.state.preparing = false;
      this.emitState();
    }
  }

  private async beginCapture() {
    if (!navigator.mediaDevices?.getUserMedia) {
      throw new Error("Microphone capture API is unavailable");
    }

    this.chunks = [];

    const stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        channelCount: 1,
        echoCancellation: true,
        noiseSuppression: true,
      },
      video: false,
    });

    const AudioCtx = (window.AudioContext || (window as any).webkitAudioContext) as
      | (new () => AudioContext)
      | undefined;
    if (!AudioCtx) {
      stream.getTracks().forEach((track) => track.stop());
      throw new Error("AudioContext is unavailable");
    }

    const audioContext = new AudioCtx();
    const source = audioContext.createMediaStreamSource(stream);
    const processor = audioContext.createScriptProcessor(4096, 1, 1);

    this.sampleRate = audioContext.sampleRate;

    processor.onaudioprocess = (event) => {
      const input = event.inputBuffer.getChannelData(0);
      this.chunks.push(new Float32Array(input));
    };

    source.connect(processor);
    processor.connect(audioContext.destination);

    this.mediaStream = stream;
    this.audioContext = audioContext;
    this.sourceNode = source;
    this.processorNode = processor;
  }

  private async endCapture(): Promise<{ samples: number[]; sampleRate: number } | null> {
    const processor = this.processorNode;
    const source = this.sourceNode;
    const audioContext = this.audioContext;
    const stream = this.mediaStream;

    this.processorNode = null;
    this.sourceNode = null;
    this.audioContext = null;
    this.mediaStream = null;

    if (processor) {
      processor.onaudioprocess = null;
      try {
        processor.disconnect();
      } catch {
        // Ignore disconnect failures.
      }
    }

    if (source) {
      try {
        source.disconnect();
      } catch {
        // Ignore disconnect failures.
      }
    }

    if (stream) {
      for (const track of stream.getTracks()) {
        try {
          track.stop();
        } catch {
          // Ignore stop failures.
        }
      }
    }

    if (audioContext) {
      try {
        await audioContext.close();
      } catch {
        // Ignore close failures.
      }
    }

    if (this.chunks.length === 0) {
      this.chunks = [];
      return null;
    }

    const totalLength = this.chunks.reduce((sum, chunk) => sum + chunk.length, 0);
    const merged = new Float32Array(totalLength);
    let offset = 0;
    for (const chunk of this.chunks) {
      merged.set(chunk, offset);
      offset += chunk.length;
    }

    this.chunks = [];
    return {
      samples: Array.from(merged),
      sampleRate: this.sampleRate,
    };
  }

  private async insertTranscript(text: string) {
    const focusedTerminalPaneId = getFocusedTerminalPaneId();
    if (focusedTerminalPaneId) {
      await this.sendToTerminal(focusedTerminalPaneId, text);
      return;
    }

    if (insertIntoActiveElement(text)) {
      return;
    }

    const paneId = this.options.getFallbackTerminalPaneId();
    if (!paneId) {
      this.setError("No active input target for voice transcript.");
      return;
    }

    await this.sendToTerminal(paneId, text);
  }

  private async sendToTerminal(paneId: string, text: string) {
    const bytes = Array.from(new TextEncoder().encode(text));
    try {
      await invokeTauri("write_terminal", { paneId, data: bytes });
    } catch (err) {
      this.setError(`Failed to send transcript to terminal: ${String(err)}`);
    }
  }
}
