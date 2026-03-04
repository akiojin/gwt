import type {
  Profile,
  ProfilesConfig,
  SettingsData,
  SkillRegistrationStatus,
  VoiceInputSettings,
} from "../types";

export const DEFAULT_VOICE_INPUT: VoiceInputSettings = {
  enabled: false,
  engine: "qwen3-asr",
  hotkey: "Mod+Shift+M",
  ptt_hotkey: "Mod+Shift+Space",
  language: "auto",
  quality: "balanced",
  model: "Qwen/Qwen3-ASR-1.7B",
};

export const DEFAULT_UI_FONT_FAMILY =
  'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
export const DEFAULT_TERMINAL_FONT_FAMILY =
  '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace';

export type FontPreset = { label: string; value: string };

export const UI_FONT_PRESETS: FontPreset[] = [
  { label: "System UI (Default)", value: DEFAULT_UI_FONT_FAMILY },
  {
    label: "Inter",
    value: '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
  },
  {
    label: "Noto Sans",
    value: '"Noto Sans", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
  },
  {
    label: "Source Sans 3",
    value:
      '"Source Sans 3", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
  },
];

export const TERMINAL_FONT_PRESETS: FontPreset[] = [
  { label: "JetBrains Mono (Default)", value: DEFAULT_TERMINAL_FONT_FAMILY },
  {
    label: "Cascadia Mono",
    value: '"Cascadia Mono", "Cascadia Code", Consolas, monospace',
  },
  {
    label: "Fira Code",
    value: '"Fira Code", "JetBrains Mono", Menlo, Consolas, monospace',
  },
  {
    label: "SF Mono",
    value: '"SF Mono", Menlo, Monaco, Consolas, monospace',
  },
  {
    label: "Ubuntu Mono",
    value: '"Ubuntu Mono", "DejaVu Sans Mono", Consolas, monospace',
  },
];

export const DEFAULT_APP_LANGUAGE: SettingsData["app_language"] = "auto";

export const DEFAULT_SKILL_STATUS: SkillRegistrationStatus = {
  overall: "failed",
  agents: [],
  last_checked_at: 0,
  last_error_message: null,
};

export function getCurrentProfile(cfg: ProfilesConfig | null, key: string): Profile | null {
  if (!cfg) return null;
  if (!key) return null;
  const p = cfg.profiles?.[key];
  return p ?? null;
}

export function isAiEnabled(profile: Profile | null): boolean {
  if (!profile) return false;
  return !!profile.ai?.endpoint?.trim();
}

export function toErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    const msg = (err as { message?: unknown }).message;
    if (typeof msg === "string") return msg;
  }
  return String(err);
}

export function detectGpuAvailability(): boolean {
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

export function normalizeVoiceInputSettings(
  value: Partial<VoiceInputSettings> | null | undefined,
): VoiceInputSettings {
  const engine = (value?.engine ?? "").trim().toLowerCase();
  const hotkey = (value?.hotkey ?? "").trim();
  const pttHotkey = (value?.ptt_hotkey ?? "").trim();
  const language = (value?.language ?? "").trim().toLowerCase();
  const quality = (value?.quality ?? "").trim().toLowerCase();
  const model = (value?.model ?? "").trim();
  const normalizedQuality =
    quality === "fast" || quality === "balanced" || quality === "accurate"
      ? (quality as VoiceInputSettings["quality"])
      : DEFAULT_VOICE_INPUT.quality;
  const defaultModel =
    normalizedQuality === "fast" ? "Qwen/Qwen3-ASR-0.6B" : "Qwen/Qwen3-ASR-1.7B";

  return {
    enabled: !!value?.enabled,
    engine:
      engine === "qwen3-asr" || engine === "qwen" || engine === "whisper"
        ? "qwen3-asr"
        : DEFAULT_VOICE_INPUT.engine,
    hotkey: hotkey.length > 0 ? hotkey : DEFAULT_VOICE_INPUT.hotkey,
    ptt_hotkey: pttHotkey.length > 0 ? pttHotkey : DEFAULT_VOICE_INPUT.ptt_hotkey,
    language:
      language === "ja" || language === "en" || language === "auto"
        ? (language as VoiceInputSettings["language"])
        : DEFAULT_VOICE_INPUT.language,
    quality: normalizedQuality,
    model: model.length > 0 ? model : defaultModel,
  };
}

export function normalizeAppLanguage(
  value: string | null | undefined,
): SettingsData["app_language"] {
  const language = (value ?? "").trim().toLowerCase();
  if (language === "ja" || language === "en" || language === "auto") {
    return language as SettingsData["app_language"];
  }
  return DEFAULT_APP_LANGUAGE;
}

export function normalizeUiFontFamily(value: string | null | undefined): string {
  const family = (value ?? "").trim();
  if (family.length === 0) return DEFAULT_UI_FONT_FAMILY;
  const match = UI_FONT_PRESETS.find((preset) => preset.value === family);
  return match ? match.value : family;
}

export function normalizeTerminalFontFamily(value: string | null | undefined): string {
  const family = (value ?? "").trim();
  if (family.length === 0) return DEFAULT_TERMINAL_FONT_FAMILY;
  const match = TERMINAL_FONT_PRESETS.find((preset) => preset.value === family);
  return match ? match.value : family;
}

export function normalizeSkillStatus(
  value: Partial<SkillRegistrationStatus> | null | undefined,
): SkillRegistrationStatus {
  const agents = Array.isArray(value?.agents)
    ? value.agents.map((agent) => ({
        agent_id: agent.agent_id ?? "unknown",
        label: agent.label ?? "Unknown",
        skills_path: agent.skills_path ?? null,
        registered: !!agent.registered,
        missing_skills: Array.isArray(agent.missing_skills)
          ? agent.missing_skills.filter((skill) => typeof skill === "string")
          : [],
        error_code: agent.error_code ?? null,
        error_message: agent.error_message ?? null,
      }))
    : [];

  return {
    overall: value?.overall ?? DEFAULT_SKILL_STATUS.overall,
    agents,
    last_checked_at:
      typeof value?.last_checked_at === "number" ? value.last_checked_at : Date.now(),
    last_error_message: value?.last_error_message ?? null,
  };
}

export function skillStatusClass(status: string): "status-ok" | "status-degraded" | "status-failed" {
  if (status === "ok") return "status-ok";
  if (status === "degraded") return "status-degraded";
  return "status-failed";
}

export function skillStatusText(status: string | null | undefined): string {
  return (status ?? "unknown").toUpperCase();
}

export function formatRegistrationCheckedAt(millis: number | null | undefined): string {
  if (typeof millis !== "number" || millis <= 0) {
    return "-";
  }
  try {
    return new Date(millis).toLocaleString();
  } catch {
    return "-";
  }
}

export function clampFontSize(v: number): number {
  return Math.max(8, Math.min(24, Math.round(v)));
}
