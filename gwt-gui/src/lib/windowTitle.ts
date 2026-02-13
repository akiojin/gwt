export function formatWindowTitle(opts: {
  appName: string;
  projectPath: string | null;
}): string {
  if (opts.projectPath) {
    return opts.projectPath;
  }
  return opts.appName;
}

export function formatAboutVersion(version: string | null): string {
  const v = version?.trim();
  return `Version ${v || "unknown"}`;
}

export async function getAppVersionSafe(): Promise<string | null> {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    return await getVersion();
  } catch {
    return null;
  }
}
