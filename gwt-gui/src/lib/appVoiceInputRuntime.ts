import type { VoiceInputSettings } from "./types";
import {
  VoiceInputController,
  type VoiceControllerState,
} from "./voice/voiceInputController";
import {
  DEFAULT_VOICE_INPUT,
  normalizeVoiceInputSettings,
} from "./components/settingsPanelHelpers";

export { DEFAULT_VOICE_INPUT, normalizeVoiceInputSettings };

export interface VoiceInputState {
  settings: VoiceInputSettings;
  listening: boolean;
  preparing: boolean;
  supported: boolean;
  available: boolean;
  availabilityReason: string | null;
  error: string | null;
}

export interface ApplyVoiceInputSettingsDeps {
  controller: VoiceInputController | null;
}

export function applyVoiceInputSettings(
  state: VoiceInputState,
  value: Partial<VoiceInputSettings> | null | undefined,
  deps: ApplyVoiceInputSettingsDeps,
): void {
  state.settings = normalizeVoiceInputSettings(value);
  deps.controller?.updateSettings();
}

export function createVoiceInputState(
  defaults: VoiceInputSettings = DEFAULT_VOICE_INPUT,
): VoiceInputState {
  return {
    settings: { ...defaults },
    listening: false,
    preparing: false,
    supported: true,
    available: false,
    availabilityReason: null,
    error: null,
  };
}

export function resetVoiceInputTransientState(state: VoiceInputState): void {
  state.listening = false;
  state.preparing = false;
  state.error = null;
  state.supported = true;
  state.available = false;
  state.availabilityReason = null;
}

export function syncControllerState(
  state: VoiceInputState,
  cs: VoiceControllerState,
): void {
  state.listening = cs.listening;
  state.preparing = cs.preparing;
  state.supported = cs.supported;
  state.available = cs.available;
  state.availabilityReason = cs.availabilityReason;
  state.error = cs.error;
}

export interface SetupVoiceControllerDeps {
  getSettings: () => VoiceInputSettings;
  getFallbackTerminalPaneId: () => string | null;
  onStateChange: (cs: VoiceControllerState) => void;
}

export interface SetupVoiceControllerResult {
  controller: VoiceInputController;
  cleanup: () => void;
}

export function setupVoiceController(
  deps: SetupVoiceControllerDeps,
): SetupVoiceControllerResult {
  const controller = new VoiceInputController({
    getSettings: deps.getSettings,
    getFallbackTerminalPaneId: deps.getFallbackTerminalPaneId,
    onStateChange: deps.onStateChange,
  });
  controller.updateSettings();

  const handlePttStart = () => controller.pressPushToTalk();
  const handlePttStop = () => controller.releasePushToTalk();

  window.addEventListener("gwt-voice-ptt-start", handlePttStart);
  window.addEventListener("gwt-voice-ptt-stop", handlePttStop);

  const cleanup = () => {
    window.removeEventListener("gwt-voice-ptt-start", handlePttStart);
    window.removeEventListener("gwt-voice-ptt-stop", handlePttStop);
    controller.dispose();
  };

  return { controller, cleanup };
}
