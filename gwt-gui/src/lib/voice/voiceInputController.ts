import { invoke } from "@tauri-apps/api/core";
import { getFocusedTerminalPaneId } from "./inputTargetRegistry";

export interface VoiceControllerSettings {
  enabled: boolean;
  hotkey: string;
  language: string;
  model: string;
}

export interface VoiceControllerState {
  supported: boolean;
  listening: boolean;
  error: string | null;
}

export interface VoiceInputControllerOptions {
  getSettings: () => VoiceControllerSettings;
  getFallbackTerminalPaneId: () => string | null;
  onStateChange?: (state: VoiceControllerState) => void;
}

type SpeechRecognitionAlternativeLike = {
  transcript: string;
};

type SpeechRecognitionResultLike = {
  isFinal: boolean;
  length: number;
  [index: number]: SpeechRecognitionAlternativeLike;
};

type SpeechRecognitionEventLike = Event & {
  resultIndex: number;
  results: ArrayLike<SpeechRecognitionResultLike>;
};

type SpeechRecognitionErrorEventLike = Event & {
  error?: string;
  message?: string;
};

type SpeechRecognitionLike = EventTarget & {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  maxAlternatives: number;
  onresult: ((event: SpeechRecognitionEventLike) => void) | null;
  onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null;
  onend: ((event: Event) => void) | null;
  start: () => void;
  stop: () => void;
};

type SpeechRecognitionConstructor = {
  new (): SpeechRecognitionLike;
};

type WindowWithSpeechRecognition = Window & {
  SpeechRecognition?: SpeechRecognitionConstructor;
  webkitSpeechRecognition?: SpeechRecognitionConstructor;
};

type HotkeyDefinition = {
  useMod: boolean;
  ctrl: boolean;
  meta: boolean;
  shift: boolean;
  alt: boolean;
  key: string;
};

const DEFAULT_HOTKEY = "Mod+Shift+M";

function parseHotkey(rawHotkey: string): HotkeyDefinition {
  const tokens = rawHotkey
    .split("+")
    .map((t) => t.trim().toLowerCase())
    .filter((t) => t.length > 0);

  const modifiers = new Set(tokens);
  const keyToken = tokens.find(
    (t) => t !== "mod" && t !== "ctrl" && t !== "control" && t !== "meta" && t !== "cmd" && t !== "command" && t !== "shift" && t !== "alt" && t !== "option"
  );

  return {
    useMod: modifiers.has("mod"),
    ctrl: modifiers.has("ctrl") || modifiers.has("control"),
    meta: modifiers.has("meta") || modifiers.has("cmd") || modifiers.has("command"),
    shift: modifiers.has("shift"),
    alt: modifiers.has("alt") || modifiers.has("option"),
    key: (keyToken ?? "m").toLowerCase(),
  };
}

function normalizeHotkey(rawHotkey: string | undefined | null): HotkeyDefinition {
  const value = (rawHotkey ?? "").trim();
  if (!value) {
    return parseHotkey(DEFAULT_HOTKEY);
  }
  return parseHotkey(value);
}

function normalizeKey(value: string): string {
  if (value.length === 1) return value.toLowerCase();
  return value.toLowerCase();
}

function eventMatchesHotkey(event: KeyboardEvent, hotkey: HotkeyDefinition): boolean {
  const expectedCtrl = hotkey.useMod
    ? !navigator.userAgent.includes("Mac")
    : hotkey.ctrl;
  const expectedMeta = hotkey.useMod
    ? navigator.userAgent.includes("Mac")
    : hotkey.meta;

  return (
    normalizeKey(event.key) === hotkey.key &&
    event.ctrlKey === expectedCtrl &&
    event.metaKey === expectedMeta &&
    event.shiftKey === hotkey.shift &&
    event.altKey === hotkey.alt
  );
}

function mapLanguage(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "ja") return "ja-JP";
  if (normalized === "en") return "en-US";
  return "";
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
  private recognition: SpeechRecognitionLike | null = null;
  private shouldKeepListening = false;
  private startInFlight = false;
  private state: VoiceControllerState = {
    supported: true,
    listening: false,
    error: null,
  };

  constructor(options: VoiceInputControllerOptions) {
    this.options = options;
    document.addEventListener("keydown", this.handleKeydown, true);
  }

  updateSettings() {
    const settings = this.options.getSettings();
    if (!settings.enabled && this.state.listening) {
      this.stopListening();
    }
  }

  dispose() {
    document.removeEventListener("keydown", this.handleKeydown, true);
    this.shouldKeepListening = false;
    if (this.recognition) {
      this.recognition.onend = null;
      this.recognition.onerror = null;
      this.recognition.onresult = null;
      try {
        this.recognition.stop();
      } catch {
        // Ignore stop errors during cleanup.
      }
    }
    this.recognition = null;
  }

  private emitState() {
    this.options.onStateChange?.({ ...this.state });
  }

  private setError(message: string | null) {
    this.state.error = message;
    this.emitState();
  }

  private handleKeydown = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.repeat) return;

    const settings = this.options.getSettings();
    if (!settings.enabled) return;

    const hotkey = normalizeHotkey(settings.hotkey);
    if (!eventMatchesHotkey(event, hotkey)) return;

    event.preventDefault();
    event.stopPropagation();
    void this.toggleListening();
  };

  private async toggleListening() {
    if (this.state.listening || this.startInFlight) {
      this.stopListening();
      return;
    }

    await this.startListening();
  }

  private resolveSpeechRecognitionConstructor(): SpeechRecognitionConstructor | null {
    const speechWindow = window as WindowWithSpeechRecognition;
    return speechWindow.SpeechRecognition ?? speechWindow.webkitSpeechRecognition ?? null;
  }

  private ensureRecognitionInstance(): SpeechRecognitionLike | null {
    if (this.recognition) return this.recognition;

    const Ctor = this.resolveSpeechRecognitionConstructor();
    if (!Ctor) {
      this.state.supported = false;
      this.setError("Speech recognition is not supported in this runtime.");
      return null;
    }

    const recognition = new Ctor();
    recognition.continuous = true;
    recognition.interimResults = false;
    recognition.maxAlternatives = 1;

    recognition.onresult = (event) => {
      let transcript = "";
      for (let i = event.resultIndex; i < event.results.length; i += 1) {
        const result = event.results[i];
        if (!result?.isFinal || result.length === 0) continue;
        transcript += result[0].transcript ?? "";
      }

      const text = transcript.trim();
      if (!text) return;
      void this.insertTranscript(text);
    };

    recognition.onerror = (event) => {
      const reason = event.error || event.message || "unknown";
      this.state.listening = false;
      this.shouldKeepListening = false;
      this.setError(`Voice recognition error: ${reason}`);
    };

    recognition.onend = () => {
      if (this.shouldKeepListening) {
        window.setTimeout(() => {
          if (!this.shouldKeepListening) return;
          try {
            recognition.start();
            this.state.listening = true;
            this.emitState();
          } catch {
            this.state.listening = false;
            this.emitState();
          }
        }, 150);
        return;
      }

      this.state.listening = false;
      this.emitState();
    };

    this.recognition = recognition;
    this.state.supported = true;
    this.emitState();
    return recognition;
  }

  private async startListening() {
    if (this.startInFlight) return;

    const recognition = this.ensureRecognitionInstance();
    if (!recognition) return;

    const settings = this.options.getSettings();
    const language = mapLanguage(settings.language);
    if (language) {
      recognition.lang = language;
    }

    this.startInFlight = true;
    this.shouldKeepListening = true;

    try {
      recognition.start();
      this.state.listening = true;
      this.setError(null);
    } catch (err) {
      this.shouldKeepListening = false;
      this.state.listening = false;
      this.setError(`Failed to start voice input: ${String(err)}`);
    } finally {
      this.startInFlight = false;
      this.emitState();
    }
  }

  private stopListening() {
    this.shouldKeepListening = false;
    this.state.listening = false;

    if (this.recognition) {
      try {
        this.recognition.stop();
      } catch {
        // Ignore stop errors when already stopped.
      }
    }

    this.emitState();
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
      await invoke("write_terminal", { paneId, data: bytes });
      this.setError(null);
    } catch (err) {
      this.setError(`Failed to send transcript to terminal: ${String(err)}`);
    }
  }
}
