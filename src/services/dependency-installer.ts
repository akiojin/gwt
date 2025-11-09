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

export interface DependencyInstallResult {
  manager: PackageManager;
  lockfile: string;
  skipped?: boolean;
}

export class DependencyInstallError extends Error {
  constructor(message: string, public cause?: unknown) {
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
): Promise<(DependencyInstallResult & { command: [string, ...string[]] }) | null> {
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
  const detection = await detectPackageManager(worktreePath);

  if (!detection) {
    throw new DependencyInstallError(
      [
        "依存関係ロックファイル (bun.lock / pnpm-lock.yaml / package-lock.json) または package.json が見つかりません。",
        `ワークツリーディレクトリ: ${worktreePath}`,
        "手動で適切なパッケージマネージャーの install コマンドを実行してください。",
      ].join("\n"),
    );
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
        `パッケージマネージャー '${binary}' が見つからないため自動インストールをスキップします。`,
      );
      return {
        manager: detection.manager,
        lockfile: detection.lockfile,
        skipped: true,
      };
    }

    const messageParts = [
      `依存関係のインストールに失敗しました (${detection.manager}).`,
      `実行コマンド: ${binary} ${args.join(" ")}`,
    ];

    const stderr = (error as { stderr?: string })?.stderr;
    if (stderr) {
      messageParts.push(stderr.trim());
    }

    throw new DependencyInstallError(messageParts.join("\n"), error);
  } finally {
    spinner();
  }

  return {
    manager: detection.manager,
    lockfile: detection.lockfile,
  };
}
