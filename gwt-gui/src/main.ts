import { mount } from "svelte";
import App from "./App.svelte";
import "./styles/global.css";

const DEFAULT_UI_FONT_FAMILY =
  'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
const DEFAULT_TERMINAL_FONT_FAMILY =
  '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace';

// Apply saved font size settings on startup before mounting to reduce flicker
let settings: {
  ui_font_size: number;
  terminal_font_size: number;
  ui_font_family: string;
  terminal_font_family: string;
} | null = null;
try {
  const { invoke } = await import("@tauri-apps/api/core");
  settings = await invoke<{
    ui_font_size: number;
    terminal_font_size: number;
    ui_font_family: string;
    terminal_font_family: string;
  }>("get_settings");
  if (settings.ui_font_size) {
    document.documentElement.style.setProperty("--ui-font-base", settings.ui_font_size + "px");
  }
  const uiFontFamily =
    typeof settings.ui_font_family === "string" && settings.ui_font_family.trim().length > 0
      ? settings.ui_font_family.trim()
      : DEFAULT_UI_FONT_FAMILY;
  document.documentElement.style.setProperty("--ui-font-family", uiFontFamily);

  if (settings.terminal_font_size) {
    (window as any).__gwtTerminalFontSize = settings.terminal_font_size;
  }
  const terminalFontFamily =
    typeof settings.terminal_font_family === "string" &&
    settings.terminal_font_family.trim().length > 0
      ? settings.terminal_font_family.trim()
      : DEFAULT_TERMINAL_FONT_FAMILY;
  document.documentElement.style.setProperty("--terminal-font-family", terminalFontFamily);
  (window as any).__gwtTerminalFontFamily = terminalFontFamily;
} catch {
  // Settings not available (e.g. dev mode without Tauri runtime)
}

const app = mount(App, { target: document.getElementById("app")! });

if (settings?.terminal_font_size) {
  window.dispatchEvent(new CustomEvent("gwt-terminal-font-size", { detail: settings.terminal_font_size }));
}
if (settings?.terminal_font_family) {
  window.dispatchEvent(
    new CustomEvent("gwt-terminal-font-family", {
      detail: settings.terminal_font_family,
    }),
  );
}

export default app;
