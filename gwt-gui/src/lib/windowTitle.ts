export function formatWindowTitle(opts: {
  appName: string;
  version: string | null;
  projectPath: string | null;
}): string {
  const v = opts.version?.trim();
  const versionPart = v ? ` v${v}` : "";

  if (opts.projectPath) {
    return `${opts.appName}${versionPart} - ${opts.projectPath}`;
  }
  return `${opts.appName}${versionPart}`;
}

export async function getAppVersionSafe(): Promise<string | null> {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    return await getVersion();
  } catch {
    return null;
  }
}
