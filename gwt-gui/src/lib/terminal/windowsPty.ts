import { platformName } from "./pasteGuidance";

export interface WindowsPtyOptions {
  backend: "conpty";
  buildNumber?: number;
}

type NavigatorLike = {
  platform?: string | null;
  userAgentData?: { platform?: string | null } | null;
};

type WindowLike = {
  __gwtWindowsPtyBuildNumber?: unknown;
};

function normalizeWindowsBuildNumber(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isInteger(value) || value <= 0) {
    return undefined;
  }
  return value;
}

export function parseWindowsBuildNumber(
  osVersion: string | null | undefined,
): number | undefined {
  const matches = osVersion?.match(/\d{5,}/g);
  if (!matches || matches.length === 0) return undefined;
  const buildNumber = Number(matches[matches.length - 1]);
  return normalizeWindowsBuildNumber(buildNumber);
}

export function buildWindowsPtyOptions(
  platform: string,
  buildNumber?: unknown,
): WindowsPtyOptions | undefined {
  if (!platform.toLowerCase().includes("win")) return undefined;
  const normalizedBuild = normalizeWindowsBuildNumber(buildNumber);
  if (normalizedBuild === undefined) {
    return { backend: "conpty" };
  }
  return {
    backend: "conpty",
    buildNumber: normalizedBuild,
  };
}

export function resolveWindowsPtyOptions(
  navigatorLike: NavigatorLike,
  windowLike: WindowLike,
): WindowsPtyOptions | undefined {
  return buildWindowsPtyOptions(
    platformName(navigatorLike),
    windowLike.__gwtWindowsPtyBuildNumber,
  );
}
