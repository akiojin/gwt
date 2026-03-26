/**
 * Appearance / settings state management extracted from App.svelte.
 *
 * Pure-function utilities (clampFontSize, normalization helpers) live here so
 * they can be unit-tested without a Svelte runtime.  The orchestration
 * functions (`applyAppearanceSettingsRuntime`, `applyFontSettingsRuntime`,
 * `checkOsEnvCaptureOnStartupRuntime`) wrap them into higher-level operations
 * that App.svelte delegates to.
 */

import type { SettingsData, VoiceInputSettings } from "./types";

// ── Constants ────────────────────────────────────────────────────────────

export const DEFAULT_UI_FONT_FAMILY =
  'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';

export const DEFAULT_TERMINAL_FONT_FAMILY =
  '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace';

export const DEFAULT_VOICE_INPUT_SETTINGS: VoiceInputSettings = {
  enabled: false,
  engine: "qwen3-asr",
  language: "auto",
  quality: "balanced",
  model: "Qwen/Qwen3-ASR-1.7B",
};

const FONT_SIZE_MIN = 8;
const FONT_SIZE_MAX = 24;

// ── Appearance state ─────────────────────────────────────────────────────

export interface AppearanceState {
  appLanguage: SettingsData["app_language"];
  osEnvReady: boolean;
}

export function createAppearanceState(): AppearanceState {
  return {
    appLanguage: "auto",
    osEnvReady: false,
  };
}

// ── Pure normalisation / clamping ────────────────────────────────────────

export function clampFontSizeRuntime(size: number): number {
  return Math.max(FONT_SIZE_MIN, Math.min(FONT_SIZE_MAX, Math.round(size)));
}

export function normalizeAppLanguageRuntime(
  value: string | null | undefined,
): SettingsData["app_language"] {
  const language = (value ?? "").trim().toLowerCase();
  if (language === "ja" || language === "en" || language === "auto") {
    return language as SettingsData["app_language"];
  }
  return "auto";
}

export function normalizeUiFontFamilyRuntime(
  value: string | null | undefined,
): string {
  const family = (value ?? "").trim();
  return family.length > 0 ? family : DEFAULT_UI_FONT_FAMILY;
}

export function normalizeTerminalFontFamilyRuntime(
  value: string | null | undefined,
): string {
  const family = (value ?? "").trim();
  return family.length > 0 ? family : DEFAULT_TERMINAL_FONT_FAMILY;
}

export function normalizeVoiceInputSettingsRuntime(
  value: Partial<VoiceInputSettings> | null | undefined,
): VoiceInputSettings {
  const engine = (value?.engine ?? "").trim().toLowerCase();
  const language = (value?.language ?? "").trim().toLowerCase();
  const quality = (value?.quality ?? "").trim().toLowerCase();
  const model = (value?.model ?? "").trim();
  const normalizedQuality =
    quality === "fast" || quality === "balanced" || quality === "accurate"
      ? (quality as VoiceInputSettings["quality"])
      : DEFAULT_VOICE_INPUT_SETTINGS.quality;
  const defaultModel =
    normalizedQuality === "fast"
      ? "Qwen/Qwen3-ASR-0.6B"
      : "Qwen/Qwen3-ASR-1.7B";

  return {
    enabled: !!value?.enabled,
    engine:
      engine === "qwen3-asr" || engine === "qwen" || engine === "whisper"
        ? "qwen3-asr"
        : DEFAULT_VOICE_INPUT_SETTINGS.engine,
    language:
      language === "ja" || language === "en" || language === "auto"
        ? (language as VoiceInputSettings["language"])
        : DEFAULT_VOICE_INPUT_SETTINGS.language,
    quality: normalizedQuality,
    model: model.length > 0 ? model : defaultModel,
  };
}

// ── DOM side-effect helpers ──────────────────────────────────────────────

export function applyUiFontSizeRuntime(size: number): void {
  document.documentElement.style.setProperty("--ui-font-base", `${size}px`);
}

export function applyUiFontFamilyRuntime(
  family: string | null | undefined,
): void {
  document.documentElement.style.setProperty(
    "--ui-font-family",
    normalizeUiFontFamilyRuntime(family),
  );
}

export function applyTerminalFontSizeRuntime(size: number): void {
  (window as any).__gwtTerminalFontSize = size;
  window.dispatchEvent(
    new CustomEvent("gwt-terminal-font-size", { detail: size }),
  );
}

export function applyTerminalFontFamilyRuntime(
  family: string | null | undefined,
): void {
  const normalized = normalizeTerminalFontFamilyRuntime(family);
  document.documentElement.style.setProperty(
    "--terminal-font-family",
    normalized,
  );
  (window as any).__gwtTerminalFontFamily = normalized;
  window.dispatchEvent(
    new CustomEvent("gwt-terminal-font-family", { detail: normalized }),
  );
}

// ── Orchestration ────────────────────────────────────────────────────────

/**
 * Apply font-related settings to the DOM (synchronous, no Tauri call).
 */
export function applyFontSettingsRuntime(settings: SettingsData): void {
  applyUiFontSizeRuntime(clampFontSizeRuntime(settings.ui_font_size ?? 13));
  applyTerminalFontSizeRuntime(
    clampFontSizeRuntime(settings.terminal_font_size ?? 13),
  );
  applyUiFontFamilyRuntime(settings.ui_font_family);
  applyTerminalFontFamilyRuntime(settings.terminal_font_family);
}

/**
 * Load settings from the Tauri backend and apply all appearance values.
 *
 * The caller must supply callbacks for state that lives in App.svelte's
 * reactive scope (language, voice-input settings).
 */
export async function applyAppearanceSettingsRuntime(args: {
  state: AppearanceState;
  onLanguageChange: (lang: SettingsData["app_language"]) => void;
  onVoiceInputChange: (settings: VoiceInputSettings) => void;
  invokeGetSettings: () => Promise<SettingsData>;
}): Promise<void> {
  const settings = await args.invokeGetSettings();
  applyFontSettingsRuntime(settings);
  const lang = normalizeAppLanguageRuntime(settings.app_language);
  args.state.appLanguage = lang;
  args.onLanguageChange(lang);
  args.onVoiceInputChange(normalizeVoiceInputSettingsRuntime(settings.voice_input));
}

/**
 * Check OS environment capture readiness on startup.
 */
export async function checkOsEnvCaptureOnStartupRuntime(args: {
  state: AppearanceState;
  isTauriAvailable: () => boolean;
  invokeIsOsEnvReady: () => Promise<boolean>;
  onResolved: () => void;
}): Promise<void> {
  if (!args.isTauriAvailable()) {
    args.onResolved();
    return;
  }

  try {
    args.state.osEnvReady = await args.invokeIsOsEnvReady();
    args.onResolved();
  } catch (err) {
    args.onResolved();
    console.error("Failed to check os env capture status:", err);
  }
}
