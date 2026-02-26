<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "$lib/tauriInvoke";

  let visible = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let unlisten: (() => void) | null = null;

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

  function onKeyDown() {
    if (visible) hide();
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
    Press &#x2318;Q again to quit
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
