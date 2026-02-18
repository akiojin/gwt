import { mount } from "svelte";
import App from "./App.svelte";
import "./styles/global.css";

// In production builds, prevent the default webview context menu to avoid exposing
// developer actions like "Inspect Element". (Dev builds keep the menu for debugging.)
if (!import.meta.env.DEV) {
  window.addEventListener("contextmenu", (e) => e.preventDefault());
}

// Apply saved font size settings on startup before mounting to reduce flicker
let settings: { ui_font_size: number; terminal_font_size: number } | null = null;
try {
  const { invoke } = await import("@tauri-apps/api/core");
  settings = await invoke<{ ui_font_size: number; terminal_font_size: number }>("get_settings");
  if (settings.ui_font_size) {
    document.documentElement.style.setProperty("--ui-font-base", settings.ui_font_size + "px");
  }
  if (settings.terminal_font_size) {
    (window as any).__gwtTerminalFontSize = settings.terminal_font_size;
  }
} catch {
  // Settings not available (e.g. dev mode without Tauri runtime)
}

const app = mount(App, { target: document.getElementById("app")! });

if (settings?.terminal_font_size) {
  window.dispatchEvent(new CustomEvent("gwt-terminal-font-size", { detail: settings.terminal_font_size }));
}

export default app;
