import path from "node:path";
import fs from "node:fs/promises";
import { execa } from "execa";
import { startSpinner } from "../utils/spinner.js";

export type PackageManager = "bun" | "pnpm" | "npm";

interface PackageManagerCandidate {
  manager: PackageManager;
  lockfile: string;
  command: [string, ...string[]];
}

interface DetectedPackageManager {
  manager: PackageManager;
  lockfile: string;
  command: [string, ...string[]];
}

export type DependencySkipReason =
  | "missing-lockfile"
  | "missing-binary"
  | "install-failed"
  | "lockfile-access-error"
  | "unknown-error";

export type DependencyInstallResult =
  | {
      skipped: false;
      manager: PackageManager;
      lockfile: string;
    }
  | {
      skipped: true;
      manager: PackageManager | null;
      lockfile: string | null;
      reason: DependencySkipReason;
      message?: string;
    };

export class DependencyInstallError extends Error {
  constructor(
    message: string,
    public cause?: unknown,
  ) {
    super(message);
    this.name = "DependencyInstallError";
  }
}

const INSTALL_CANDIDATES: PackageManagerCandidate[] = [
  {
    manager: "bun",
    lockfile: "bun.lock",
    command: ["bun", "install", "--frozen-lockfile"],
  },
  {
    manager: "pnpm",
    lockfile: "pnpm-lock.yaml",
    command: ["pnpm", "install", "--frozen-lockfile"],
  },
  {
    manager: "npm",
    lockfile: "package-lock.json",
    command: ["npm", "ci"],
  },
  {
    manager: "npm",
    lockfile: "package.json",
    command: ["npm", "install"],
  },
];

async function fileExists(targetPath: string): Promise<boolean> {
  try {
    await fs.access(targetPath);
    return true;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException)?.code;
    if (code === "ENOENT") {
      return false;
    }
    throw new DependencyInstallError(
      `Lockfile access failed: ${targetPath} (${code ?? "unknown"})`,
      error,
    );
  }
}

export async function detectPackageManager(
  worktreePath: string,
): Promise<DetectedPackageManager | null> {
  const normalized = path.resolve(worktreePath);

  for (const candidate of INSTALL_CANDIDATES) {
    const fullPath = path.join(normalized, candidate.lockfile);
    if (await fileExists(fullPath)) {
      return {
        manager: candidate.manager,
        lockfile: fullPath,
        command: candidate.command,
      };
    }
  }

  return null;
}

export async function installDependenciesForWorktree(
  worktreePath: string,
): Promise<DependencyInstallResult> {
  let detection: DetectedPackageManager | null = null;

  try {
    detection = await detectPackageManager(worktreePath);
  } catch (error) {
    if (error instanceof DependencyInstallError) {
      return {
        skipped: true,
        manager: null,
        lockfile: null,
        reason: "lockfile-access-error",
        message: error.message,
      };
    }

    return {
      skipped: true,
      manager: null,
      lockfile: null,
      reason: "unknown-error",
      message: error instanceof Error ? error.message : String(error),
    };
  }

  if (!detection) {
    return {
      skipped: true,
      manager: null,
      lockfile: null,
      reason: "missing-lockfile",
    };
  }

  const [binary, ...args] = detection.command;

  const spinner = startSpinner(
    `Installing dependencies via ${detection.manager} (${path.basename(detection.lockfile)})`,
  );

  try {
    await execa(binary, args, {
      cwd: worktreePath,
      stdout: "inherit",
      stderr: "inherit",
    });
  } catch (error) {
    const code = (error as NodeJS.ErrnoException)?.code;

    if (code === "ENOENT") {
      console.warn(
        `Package manager '${binary}' was not found; skipping automatic install.`,
      );
      return {
        skipped: true,
        manager: detection.manager,
        lockfile: detection.lockfile,
        reason: "missing-binary",
      };
    }

    const messageParts = [
      `Dependency installation failed (${detection.manager}).`,
      `Command: ${binary} ${args.join(" ")}`,
    ];

    const stderr = (error as { stderr?: string })?.stderr;
    if (stderr) {
      messageParts.push(stderr.trim());
    }
    const failureMessage = messageParts.join("\n");

    return {
      skipped: true,
      manager: detection.manager,
      lockfile: detection.lockfile,
      reason: "install-failed",
      message: failureMessage,
    };
  } finally {
    spinner();
  }

  return {
    skipped: false,
    manager: detection.manager,
    lockfile: detection.lockfile,
  };
}
