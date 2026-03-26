import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  clampFontSizeRuntime,
  normalizeAppLanguageRuntime,
  normalizeUiFontFamilyRuntime,
  normalizeTerminalFontFamilyRuntime,
  normalizeVoiceInputSettingsRuntime,
  createAppearanceState,
  applyUiFontSizeRuntime,
  applyUiFontFamilyRuntime,
  applyTerminalFontSizeRuntime,
  applyTerminalFontFamilyRuntime,
  applyFontSettingsRuntime,
  applyAppearanceSettingsRuntime,
  checkOsEnvCaptureOnStartupRuntime,
  DEFAULT_UI_FONT_FAMILY,
  DEFAULT_TERMINAL_FONT_FAMILY,
  DEFAULT_VOICE_INPUT_SETTINGS,
} from "./appAppearanceRuntime";
import type { SettingsData } from "./types";

// ── createAppearanceState ────────────────────────────────────────────────

describe("createAppearanceState", () => {
  it("returns default state", () => {
    const state = createAppearanceState();
    expect(state.appLanguage).toBe("auto");
    expect(state.osEnvReady).toBe(false);
  });
});

// ── clampFontSizeRuntime ─────────────────────────────────────────────────

describe("clampFontSizeRuntime", () => {
  it("clamps below minimum to 8", () => {
    expect(clampFontSizeRuntime(2)).toBe(8);
  });

  it("clamps above maximum to 24", () => {
    expect(clampFontSizeRuntime(100)).toBe(24);
  });

  it("rounds to nearest integer", () => {
    expect(clampFontSizeRuntime(13.7)).toBe(14);
  });

  it("passes through valid sizes unchanged", () => {
    expect(clampFontSizeRuntime(16)).toBe(16);
  });
});

// ── normalizeAppLanguageRuntime ──────────────────────────────────────────

describe("normalizeAppLanguageRuntime", () => {
  it("returns 'auto' for null", () => {
    expect(normalizeAppLanguageRuntime(null)).toBe("auto");
  });

  it("returns 'auto' for undefined", () => {
    expect(normalizeAppLanguageRuntime(undefined)).toBe("auto");
  });

  it("returns 'auto' for unknown value", () => {
    expect(normalizeAppLanguageRuntime("fr")).toBe("auto");
  });

  it("normalizes 'ja'", () => {
    expect(normalizeAppLanguageRuntime("ja")).toBe("ja");
  });

  it("normalizes 'en'", () => {
    expect(normalizeAppLanguageRuntime("en")).toBe("en");
  });

  it("normalizes 'auto'", () => {
    expect(normalizeAppLanguageRuntime("auto")).toBe("auto");
  });

  it("trims and lowercases input", () => {
    expect(normalizeAppLanguageRuntime("  JA  ")).toBe("ja");
  });
});

// ── normalizeUiFontFamilyRuntime ─────────────────────────────────────────

describe("normalizeUiFontFamilyRuntime", () => {
  it("returns default for null", () => {
    expect(normalizeUiFontFamilyRuntime(null)).toBe(DEFAULT_UI_FONT_FAMILY);
  });

  it("returns default for empty string", () => {
    expect(normalizeUiFontFamilyRuntime("  ")).toBe(DEFAULT_UI_FONT_FAMILY);
  });

  it("returns provided family when non-empty", () => {
    expect(normalizeUiFontFamilyRuntime("Arial")).toBe("Arial");
  });
});

// ── normalizeTerminalFontFamilyRuntime ───────────────────────────────────

describe("normalizeTerminalFontFamilyRuntime", () => {
  it("returns default for null", () => {
    expect(normalizeTerminalFontFamilyRuntime(null)).toBe(
      DEFAULT_TERMINAL_FONT_FAMILY,
    );
  });

  it("returns default for empty string", () => {
    expect(normalizeTerminalFontFamilyRuntime("")).toBe(
      DEFAULT_TERMINAL_FONT_FAMILY,
    );
  });

  it("returns provided family when non-empty", () => {
    expect(normalizeTerminalFontFamilyRuntime("Courier New")).toBe(
      "Courier New",
    );
  });
});

// ── normalizeVoiceInputSettingsRuntime ────────────────────────────────────

describe("normalizeVoiceInputSettingsRuntime", () => {
  it("returns defaults for null", () => {
    expect(normalizeVoiceInputSettingsRuntime(null)).toEqual(
      DEFAULT_VOICE_INPUT_SETTINGS,
    );
  });

  it("returns defaults for undefined", () => {
    expect(normalizeVoiceInputSettingsRuntime(undefined)).toEqual(
      DEFAULT_VOICE_INPUT_SETTINGS,
    );
  });

  it("normalizes known engine to qwen3-asr", () => {
    const result = normalizeVoiceInputSettingsRuntime({ engine: "whisper" });
    expect(result.engine).toBe("qwen3-asr");
  });

  it("falls back to default engine for unknown value", () => {
    const result = normalizeVoiceInputSettingsRuntime({
      engine: "unknown-engine",
    });
    expect(result.engine).toBe(DEFAULT_VOICE_INPUT_SETTINGS.engine);
  });

  it("normalizes quality and picks model for fast", () => {
    const result = normalizeVoiceInputSettingsRuntime({ quality: "fast" });
    expect(result.quality).toBe("fast");
    expect(result.model).toBe("Qwen/Qwen3-ASR-0.6B");
  });

  it("preserves explicit model", () => {
    const result = normalizeVoiceInputSettingsRuntime({
      quality: "fast",
      model: "custom/model",
    });
    expect(result.model).toBe("custom/model");
  });

  it("normalizes language", () => {
    const result = normalizeVoiceInputSettingsRuntime({ language: "ja" });
    expect(result.language).toBe("ja");
  });

  it("defaults language for unknown value", () => {
    const result = normalizeVoiceInputSettingsRuntime({ language: "fr" });
    expect(result.language).toBe(DEFAULT_VOICE_INPUT_SETTINGS.language);
  });

  it("preserves enabled flag", () => {
    const result = normalizeVoiceInputSettingsRuntime({ enabled: true });
    expect(result.enabled).toBe(true);
  });
});

// ── DOM apply helpers ────────────────────────────────────────────────────

describe("applyUiFontSizeRuntime", () => {
  it("sets --ui-font-base CSS variable", () => {
    applyUiFontSizeRuntime(16);
    expect(
      document.documentElement.style.getPropertyValue("--ui-font-base"),
    ).toBe("16px");
  });
});

describe("applyUiFontFamilyRuntime", () => {
  it("sets --ui-font-family CSS variable", () => {
    applyUiFontFamilyRuntime("Arial");
    expect(
      document.documentElement.style.getPropertyValue("--ui-font-family"),
    ).toBe("Arial");
  });

  it("uses default when null", () => {
    applyUiFontFamilyRuntime(null);
    expect(
      document.documentElement.style.getPropertyValue("--ui-font-family"),
    ).toBe(DEFAULT_UI_FONT_FAMILY);
  });
});

describe("applyTerminalFontSizeRuntime", () => {
  it("sets window global and dispatches event", () => {
    const handler = vi.fn();
    window.addEventListener("gwt-terminal-font-size", handler);
    applyTerminalFontSizeRuntime(14);
    expect((window as any).__gwtTerminalFontSize).toBe(14);
    expect(handler).toHaveBeenCalledOnce();
    window.removeEventListener("gwt-terminal-font-size", handler);
  });
});

describe("applyTerminalFontFamilyRuntime", () => {
  it("sets CSS variable, window global, and dispatches event", () => {
    const handler = vi.fn();
    window.addEventListener("gwt-terminal-font-family", handler);
    applyTerminalFontFamilyRuntime("Courier New");
    expect(
      document.documentElement.style.getPropertyValue(
        "--terminal-font-family",
      ),
    ).toBe("Courier New");
    expect((window as any).__gwtTerminalFontFamily).toBe("Courier New");
    expect(handler).toHaveBeenCalledOnce();
    window.removeEventListener("gwt-terminal-font-family", handler);
  });
});

// ── applyFontSettingsRuntime ─────────────────────────────────────────────

describe("applyFontSettingsRuntime", () => {
  it("applies all font settings from SettingsData", () => {
    const settings = {
      ui_font_size: 15,
      terminal_font_size: 18,
      ui_font_family: "Helvetica",
      terminal_font_family: "Menlo",
    } as SettingsData;

    applyFontSettingsRuntime(settings);

    expect(
      document.documentElement.style.getPropertyValue("--ui-font-base"),
    ).toBe("15px");
    expect(
      document.documentElement.style.getPropertyValue("--ui-font-family"),
    ).toBe("Helvetica");
    expect((window as any).__gwtTerminalFontSize).toBe(18);
    expect((window as any).__gwtTerminalFontFamily).toBe("Menlo");
  });

  it("clamps font sizes", () => {
    const settings = {
      ui_font_size: 2,
      terminal_font_size: 100,
      ui_font_family: "",
      terminal_font_family: "",
    } as SettingsData;

    applyFontSettingsRuntime(settings);

    expect(
      document.documentElement.style.getPropertyValue("--ui-font-base"),
    ).toBe("8px");
    expect((window as any).__gwtTerminalFontSize).toBe(24);
  });
});

// ── applyAppearanceSettingsRuntime ───────────────────────────────────────

describe("applyAppearanceSettingsRuntime", () => {
  it("loads settings and applies appearance", async () => {
    const state = createAppearanceState();
    const onLanguageChange = vi.fn();
    const onVoiceInputChange = vi.fn();
    const mockSettings: SettingsData = {
      protected_branches: [],
      default_base_branch: "main",
      worktree_root: "/tmp",
      debug: false,
      log_retention_days: 7,
      agent_auto_install_deps: false,
      docker_force_host: false,
      ui_font_size: 14,
      terminal_font_size: 16,
      ui_font_family: "Arial",
      terminal_font_family: "Menlo",
      app_language: "ja",
      voice_input: { ...DEFAULT_VOICE_INPUT_SETTINGS, enabled: true },
    };

    await applyAppearanceSettingsRuntime({
      state,
      onLanguageChange,
      onVoiceInputChange,
      invokeGetSettings: () => Promise.resolve(mockSettings),
    });

    expect(state.appLanguage).toBe("ja");
    expect(onLanguageChange).toHaveBeenCalledWith("ja");
    expect(onVoiceInputChange).toHaveBeenCalledOnce();
    expect(
      document.documentElement.style.getPropertyValue("--ui-font-base"),
    ).toBe("14px");
  });
});

// ── checkOsEnvCaptureOnStartupRuntime ────────────────────────────────────

describe("checkOsEnvCaptureOnStartupRuntime", () => {
  it("resolves immediately when Tauri is unavailable", async () => {
    const state = createAppearanceState();
    const onResolved = vi.fn();

    await checkOsEnvCaptureOnStartupRuntime({
      state,
      isTauriAvailable: () => false,
      invokeIsOsEnvReady: () => Promise.resolve(true),
      onResolved,
    });

    expect(onResolved).toHaveBeenCalledOnce();
    expect(state.osEnvReady).toBe(false);
  });

  it("sets osEnvReady when Tauri reports ready", async () => {
    const state = createAppearanceState();
    const onResolved = vi.fn();

    await checkOsEnvCaptureOnStartupRuntime({
      state,
      isTauriAvailable: () => true,
      invokeIsOsEnvReady: () => Promise.resolve(true),
      onResolved,
    });

    expect(state.osEnvReady).toBe(true);
    expect(onResolved).toHaveBeenCalledOnce();
  });

  it("resolves even on error", async () => {
    const state = createAppearanceState();
    const onResolved = vi.fn();
    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    await checkOsEnvCaptureOnStartupRuntime({
      state,
      isTauriAvailable: () => true,
      invokeIsOsEnvReady: () => Promise.reject(new Error("boom")),
      onResolved,
    });

    expect(onResolved).toHaveBeenCalledOnce();
    expect(state.osEnvReady).toBe(false);
    expect(consoleSpy).toHaveBeenCalled();
    consoleSpy.mockRestore();
  });
});
