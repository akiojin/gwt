import type { SettingsData, Tab, VoiceInputSettings } from "./types";
import { applyMenuPasteText } from "./terminal/menuPaste";

export function toErrorMessageRuntime(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    const msg = (err as { message?: unknown }).message;
    if (typeof msg === "string") return msg;
  }
  return String(err);
}

export function isTauriRuntimeAvailableRuntime(): boolean {
  if (typeof window === "undefined") return false;
  return (
    typeof (window as Window & { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__ !== "undefined"
  );
}

export function shouldHandleExternalLinkClickRuntime(
  event: MouseEvent,
  anchor: HTMLAnchorElement,
  isAllowedExternalHttpUrl: (href: string) => boolean,
): boolean {
  if (event.defaultPrevented) return false;
  if (event.button !== 0) return false;
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
    return false;
  }
  if (anchor.hasAttribute("download")) return false;

  const rawHref = anchor.getAttribute("href");
  if (!rawHref) return false;
  return isAllowedExternalHttpUrl(rawHref);
}

export function clampFontSizeRuntime(size: number): number {
  return Math.max(8, Math.min(24, Math.round(size)));
}

export function normalizeVoiceInputSettingsRuntime(
  value: Partial<VoiceInputSettings> | null | undefined,
  defaults: VoiceInputSettings,
): VoiceInputSettings {
  const engine = (value?.engine ?? "").trim().toLowerCase();
  const language = (value?.language ?? "").trim().toLowerCase();
  const quality = (value?.quality ?? "").trim().toLowerCase();
  const model = (value?.model ?? "").trim();
  const normalizedQuality =
    quality === "fast" || quality === "balanced" || quality === "accurate"
      ? (quality as VoiceInputSettings["quality"])
      : defaults.quality;
  const defaultModel =
    normalizedQuality === "fast"
      ? "Qwen/Qwen3-ASR-0.6B"
      : "Qwen/Qwen3-ASR-1.7B";

  return {
    enabled: !!value?.enabled,
    engine:
      engine === "qwen3-asr" || engine === "qwen" || engine === "whisper"
        ? "qwen3-asr"
        : defaults.engine,
    language:
      language === "ja" || language === "en" || language === "auto"
        ? (language as VoiceInputSettings["language"])
        : defaults.language,
    quality: normalizedQuality,
    model: model.length > 0 ? model : defaultModel,
  };
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

export function normalizeFontFamilyRuntime(
  value: string | null | undefined,
  fallback: string,
): string {
  const family = (value ?? "").trim();
  return family.length > 0 ? family : fallback;
}

export function getActiveTerminalPaneIdRuntime(args: {
  tabs: Tab[];
  activeTabId: string;
  selectedCanvasSessionTabId: string | null;
}): string | null {
  const active = args.tabs.find((t) => t.id === args.activeTabId) ?? null;
  if (active?.type === "agent" || active?.type === "terminal") {
    return active.paneId && active.paneId.length > 0 ? active.paneId : null;
  }
  if (active?.type === "agentCanvas" && args.selectedCanvasSessionTabId) {
    const selected =
      args.tabs.find((t) => t.id === args.selectedCanvasSessionTabId) ?? null;
    if (selected?.paneId) {
      return selected.paneId;
    }
  }
  return null;
}

export function getActiveEditableElementRuntime(
  mode: "copy" | "paste" = "paste",
): HTMLInputElement | HTMLTextAreaElement | HTMLElement | null {
  if (typeof document === "undefined") return null;
  const el = document.activeElement;
  if (!el) return null;

  if (el instanceof HTMLInputElement && !el.disabled) {
    if (mode === "copy" || !el.readOnly) return el;
  }
  if (el instanceof HTMLTextAreaElement && !el.disabled) {
    if (mode === "copy" || !el.readOnly) return el;
  }
  if (el instanceof HTMLElement && el.isContentEditable) {
    return el;
  }

  return null;
}

export function getEditableSelectionTextRuntime(
  target: HTMLInputElement | HTMLTextAreaElement | HTMLElement,
): string {
  if (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement
  ) {
    const start = target.selectionStart;
    const end = target.selectionEnd;
    if (start === null || end === null || start === end) return "";
    const from = Math.min(start, end);
    const to = Math.max(start, end);
    return target.value.slice(from, to);
  }

  const selection = window.getSelection();
  if (!selection || selection.rangeCount === 0) return "";
  const range = selection.getRangeAt(0);
  if (!target.contains(range.commonAncestorContainer)) return "";
  return selection.toString();
}

export async function fallbackMenuEditActionRuntime(action: "copy" | "paste") {
  const target = getActiveEditableElementRuntime(action);
  if (!target) {
    if (action === "copy") {
      const sel = window.getSelection()?.toString();
      if (sel && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(sel);
      }
    }
    return;
  }

  if (action === "copy") {
    const sel = getEditableSelectionTextRuntime(target);
    if (sel && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(sel);
    }
    return;
  }

  if (!navigator.clipboard?.readText) return;
  let text: string;
  try {
    text = await navigator.clipboard.readText();
  } catch {
    return;
  }
  if (!text) return;

  applyMenuPasteText(target, text);
}
