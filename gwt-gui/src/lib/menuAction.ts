import type { UnlistenFn, Event } from "@tauri-apps/api/event";

export interface MenuActionPayload {
  action: string;
}

/**
 * Set up a window-scoped menu action listener.
 * Uses getCurrentWebviewWindow().listen() to ensure menu actions
 * are only received by the focused window.
 */
export async function setupMenuActionListener(
  handler: (action: string) => void,
): Promise<UnlistenFn> {
  const { getCurrentWebviewWindow } = await import(
    "@tauri-apps/api/webviewWindow"
  );
  return getCurrentWebviewWindow().listen<MenuActionPayload>(
    "menu-action",
    (event: Event<MenuActionPayload>) => {
      handler(event.payload.action);
    },
  );
}
