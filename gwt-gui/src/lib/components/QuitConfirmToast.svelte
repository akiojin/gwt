<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "$lib/tauriInvoke";

  let visible = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let unlisten: (() => void) | null = null;
  const quitShortcutLabel = detectQuitShortcutLabel();

  function detectQuitShortcutLabel(): string {
    if (typeof navigator === "undefined") return "Quit";
    const navWithUAData = navigator as Navigator & {
      userAgentData?: { platform?: string };
    };
    const platformHint =
      navWithUAData.userAgentData?.platform ||
      navigator.platform ||
      navigator.userAgent ||
      "";
    const normalized = platformHint.toLowerCase();
    if (
      normalized.includes("mac") ||
      normalized.includes("iphone") ||
      normalized.includes("ipad") ||
      normalized.includes("ipod")
    ) {
      return "\u2318Q";
    }
    return "Alt+F4";
  }

  function isQuitShortcut(event: KeyboardEvent): boolean {
    const key = event.key.toLowerCase();
    return (event.metaKey && key === "q") || (event.altKey && key === "f4");
  }

  function clearTimer() {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function hide() {
    visible = false;
    clearTimer();
    invoke("cancel_quit_confirm").catch(() => {});
  }

  function onMouseDown() {
    if (visible) hide();
  }

  function onKeyDown(event: KeyboardEvent) {
    if (!visible || isQuitShortcut(event)) return;
    hide();
  }

  onMount(async () => {
    unlisten = await listen("quit-confirm-show", () => {
      visible = true;
      clearTimer();
      timer = setTimeout(() => {
        hide();
      }, 3000);
    });

    document.addEventListener("mousedown", onMouseDown);
    document.addEventListener("keydown", onKeyDown);
  });

  onDestroy(() => {
    clearTimer();
    unlisten?.();
    document.removeEventListener("mousedown", onMouseDown);
    document.removeEventListener("keydown", onKeyDown);
  });
</script>

{#if visible}
  <div class="quit-confirm-toast fade-in" data-testid="quit-confirm-toast">
    Press {quitShortcutLabel} again to quit
  </div>
{/if}

<style>
  .quit-confirm-toast {
    position: fixed;
    top: 16px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 9999;
    background: var(--bg-surface, #313244);
    color: var(--text-primary, #cdd6f4);
    border: 1px solid var(--border-color, #45475a);
    font-family: var(--ui-font-family, system-ui, -apple-system, sans-serif);
    font-size: var(--ui-font-md, 12px);
    padding: 8px 20px;
    border-radius: 8px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
    pointer-events: none;
    user-select: none;
  }

  .fade-in {
    animation: fadeIn 250ms ease-out;
  }

  @keyframes fadeIn {
    from {
      opacity: 0;
      transform: translateX(-50%) translateY(-8px);
    }
    to {
      opacity: 1;
      transform: translateX(-50%) translateY(0);
    }
  }
</style>
