const ALLOWED_EXTERNAL_SCHEMES = new Set(["http:", "https:"]);

function normalizeToAbsoluteUrl(raw: string): string | null {
  const value = raw.trim();
  if (!value) return null;

  try {
    return new URL(value).toString();
  } catch {
    return null;
  }
}

export function isAllowedExternalHttpUrl(raw: string): boolean {
  const normalized = normalizeToAbsoluteUrl(raw);
  if (!normalized) return false;

  try {
    return ALLOWED_EXTERNAL_SCHEMES.has(new URL(normalized).protocol);
  } catch {
    return false;
  }
}

async function tryOpenWithTauriShell(url: string): Promise<boolean> {
  try {
    const { open } = await import("@tauri-apps/plugin-shell");
    await open(url);
    return true;
  } catch {
    return false;
  }
}

function tryOpenWithWindow(url: string): boolean {
  if (typeof window === "undefined" || typeof window.open !== "function") {
    return false;
  }

  const opened = window.open(url, "_blank", "noopener,noreferrer");
  return opened !== null;
}

export async function openExternalUrl(raw: string): Promise<boolean> {
  const normalized = normalizeToAbsoluteUrl(raw);
  if (!normalized) return false;
  if (!isAllowedExternalHttpUrl(normalized)) return false;

  if (await tryOpenWithTauriShell(normalized)) {
    return true;
  }

  return tryOpenWithWindow(normalized);
}
