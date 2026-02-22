import type { UnlistenFn, Event } from "@tauri-apps/api/event";

export interface MenuActionPayload {
  action: string;
}

function toErrorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  return String(error);
}

/**
 * Set up a window-scoped menu action listener.
 * Uses getCurrentWebviewWindow().listen() to ensure menu actions
 * are only received by the focused window.
 */
export async function setupMenuActionListener(
  handler: (action: string) => void,
): Promise<UnlistenFn> {
  let webviewWindowApi: typeof import("@tauri-apps/api/webviewWindow");
  try {
    webviewWindowApi = await import("@tauri-apps/api/webviewWindow");
  } catch (error) {
    throw new Error(
      `webviewWindow API unavailable for menu-action listener: ${toErrorMessage(error)}`,
    );
  }

  try {
    return await webviewWindowApi
      .getCurrentWebviewWindow()
      .listen<MenuActionPayload>(
        "menu-action",
        (event: Event<MenuActionPayload>) => {
          handler(event.payload.action);
        },
      );
  } catch (error) {
    throw new Error(
      `menu-action listener init failed: ${toErrorMessage(error)}`,
    );
  }
}
