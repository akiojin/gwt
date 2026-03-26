import { listen as tauriListen } from "@tauri-apps/api/event";
import type { EventCallback, UnlistenFn } from "@tauri-apps/api/event";

export async function listen<T>(
  event: string,
  handler: EventCallback<T>,
): Promise<UnlistenFn> {
  return tauriListen<T>(event, handler);
}
