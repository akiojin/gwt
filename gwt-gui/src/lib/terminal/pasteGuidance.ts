export const AGENT_PASTE_HINT_DISMISSED_KEY = "gwt.agentPasteHint.dismissed.v1";

export function platformName(navigatorLike: {
  platform?: string | null;
  userAgentData?: { platform?: string | null } | null;
}): string {
  const platformFromUAData = navigatorLike.userAgentData?.platform?.trim();
  if (platformFromUAData) return platformFromUAData;
  return navigatorLike.platform?.trim() ?? "";
}

export function isMacPlatform(platform: string): boolean {
  const value = platform.toLowerCase();
  return (
    value.includes("mac") ||
    value.includes("iphone") ||
    value.includes("ipad") ||
    value.includes("ipod")
  );
}

export function isWindowsOrLinuxPlatform(platform: string): boolean {
  const value = platform.toLowerCase();
  if (!value) return false;
  if (isMacPlatform(value)) return false;
  return value.includes("win") || value.includes("linux") || value.includes("x11");
}

export function shouldShowAgentPasteHint(input: {
  activeTabType: string | undefined;
  platform: string;
  dismissed: boolean;
  shownInSession: boolean;
}): boolean {
  if (input.activeTabType !== "agent") return false;
  if (input.dismissed || input.shownInSession) return false;
  return isWindowsOrLinuxPlatform(input.platform);
}
