import { mount } from "svelte";
import App from "./App.svelte";
import "./styles/global.css";

// In production builds, prevent the default webview context menu to avoid exposing
// developer actions like "Inspect Element". (Dev builds keep the menu for debugging.)
if (!import.meta.env.DEV) {
  window.addEventListener("contextmenu", (e) => e.preventDefault());
}

const app = mount(App, { target: document.getElementById("app")! });
export default app;
