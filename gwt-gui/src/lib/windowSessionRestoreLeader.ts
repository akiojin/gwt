export function isWindowSessionRestoreLeaderCandidate(label: string): boolean {
  return label.trim() === "main";
}

type InvokeFn = <T = unknown>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

export async function tryAcquireWindowSessionRestoreLead(
  label: string,
  invokeFn?: InvokeFn,
): Promise<boolean> {
  const normalizedLabel = label.trim();
  if (!isWindowSessionRestoreLeaderCandidate(normalizedLabel)) {
    return false;
  }

  try {
    const invoke = invokeFn ?? (await import("@tauri-apps/api/core")).invoke;
    const acquired = await invoke<boolean>("try_acquire_window_restore_leader", {
      label: normalizedLabel,
    });
    return acquired === true;
  } catch {
    return false;
  }
}

export async function releaseWindowSessionRestoreLead(
  label: string,
  invokeFn?: InvokeFn,
): Promise<void> {
  const normalizedLabel = label.trim();
  if (!normalizedLabel) {
    return;
  }

  try {
    const invoke = invokeFn ?? (await import("@tauri-apps/api/core")).invoke;
    await invoke("release_window_restore_leader", {
      label: normalizedLabel,
    });
  } catch {
    // Ignore command failures; restore flow is best-effort.
  }
}
