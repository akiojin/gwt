import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import path from "node:path";

// Vitest shim for environments lacking vi.hoisted (e.g., bun)
if (typeof (vi as Record<string, unknown>).hoisted !== "function") {
  // @ts-expect-error injected shim
  vi.hoisted = (factory: () => unknown) => factory();
}

const accessMock = vi.hoisted(() => vi.fn());

vi.mock("node:fs/promises", async () => {
  const actual =
    await vi.importActual<typeof import("node:fs/promises")>(
      "node:fs/promises",
    );

  return {
    ...actual,
    access: accessMock,
    default: {
      ...actual.default,
      access: accessMock,
    },
    constants: actual.constants,
  };
});

vi.mock("execa", () => ({
  execa: vi.fn(),
}));

import { execa } from "execa";
import {
  detectPackageManager,
  installDependenciesForWorktree,
} from "../../src/services/dependency-installer";

const WORKTREE = "/repo/.worktrees/feature-x";

function setupExistingFiles(files: string[]): void {
  accessMock.mockImplementation(async (target: string) => {
    if (!files.includes(target)) {
      const error = new Error(`ENOENT: no such file, open '${target}'`);
      (error as NodeJS.ErrnoException).code = "ENOENT";
      throw error;
    }
    return undefined;
  });
}

describe("dependency installer", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    accessMock.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("detectPackageManager", () => {
    it("prefers bun when bun.lock exists", async () => {
      setupExistingFiles([path.join(WORKTREE, "bun.lock")]);

      const result = await detectPackageManager(WORKTREE);

      expect(result).not.toBeNull();
      expect(result?.manager).toBe("bun");
      expect(result?.lockfile).toBe(path.join(WORKTREE, "bun.lock"));
    });

    it("falls back to pnpm when bun lock is missing", async () => {
      setupExistingFiles([path.join(WORKTREE, "pnpm-lock.yaml")]);

      const result = await detectPackageManager(WORKTREE);

      expect(result).not.toBeNull();
      expect(result?.manager).toBe("pnpm");
    });

    it("falls back to npm when only package-lock exists", async () => {
      setupExistingFiles([path.join(WORKTREE, "package-lock.json")]);

      const result = await detectPackageManager(WORKTREE);

      expect(result).not.toBeNull();
      expect(result?.manager).toBe("npm");
    });

    it("falls back to npm install when only package.json exists", async () => {
      setupExistingFiles([path.join(WORKTREE, "package.json")]);

      const result = await detectPackageManager(WORKTREE);

      expect(result).not.toBeNull();
      expect(result?.manager).toBe("npm");
      expect(result?.command?.[0]).toBe("npm");
      expect(result?.command?.includes("install")).toBe(true);
    });

    it("returns null when no lockfile exists", async () => {
      setupExistingFiles([]);

      const result = await detectPackageManager(WORKTREE);

      expect(result).toBeNull();
    });
  });

  describe("installDependenciesForWorktree", () => {
    it("runs bun install with frozen-lockfile", async () => {
      setupExistingFiles([path.join(WORKTREE, "bun.lock")]);
      (execa as any).mockResolvedValue({ stdout: "", stderr: "" });

      await installDependenciesForWorktree(WORKTREE);

      expect(execa).toHaveBeenCalledWith(
        "bun",
        ["install", "--frozen-lockfile"],
        expect.objectContaining({ cwd: WORKTREE }),
      );
    });

    it("runs pnpm install when pnpm lock is present", async () => {
      setupExistingFiles([path.join(WORKTREE, "pnpm-lock.yaml")]);
      (execa as any).mockResolvedValue({ stdout: "", stderr: "" });

      await installDependenciesForWorktree(WORKTREE);

      expect(execa).toHaveBeenCalledWith(
        "pnpm",
        ["install", "--frozen-lockfile"],
        expect.objectContaining({ cwd: WORKTREE }),
      );
    });

    it("runs npm ci when package-lock exists", async () => {
      setupExistingFiles([path.join(WORKTREE, "package-lock.json")]);
      (execa as any).mockResolvedValue({ stdout: "", stderr: "" });

      await installDependenciesForWorktree(WORKTREE);

      expect(execa).toHaveBeenCalledWith(
        "npm",
        ["ci"],
        expect.objectContaining({ cwd: WORKTREE }),
      );
    });

    it("runs npm install when only package.json exists", async () => {
      setupExistingFiles([path.join(WORKTREE, "package.json")]);
      (execa as any).mockResolvedValue({ stdout: "", stderr: "" });

      await installDependenciesForWorktree(WORKTREE);

      expect(execa).toHaveBeenCalledWith(
        "npm",
        ["install"],
        expect.objectContaining({ cwd: WORKTREE }),
      );
    });

    it("skips installation when no lockfile exists", async () => {
      setupExistingFiles([]);

      await expect(
        installDependenciesForWorktree(WORKTREE),
      ).resolves.toMatchObject({
        skipped: true,
        reason: "missing-lockfile",
        manager: null,
        lockfile: null,
      });
    });

    it("skips when install command fails", async () => {
      const lockfilePath = path.join(WORKTREE, "bun.lock");
      setupExistingFiles([lockfilePath]);
      (execa as any).mockRejectedValue(new Error("boom"));

      await expect(
        installDependenciesForWorktree(WORKTREE),
      ).resolves.toMatchObject({
        skipped: true,
        manager: "bun",
        lockfile: lockfilePath,
        reason: "install-failed",
      });
    });

    it("skips when lockfile access fails", async () => {
      const accessError = Object.assign(new Error("permission denied"), {
        code: "EACCES",
      });
      accessMock.mockImplementation(async () => {
        throw accessError;
      });

      await expect(
        installDependenciesForWorktree(WORKTREE),
      ).resolves.toMatchObject({
        skipped: true,
        reason: "lockfile-access-error",
        manager: null,
        lockfile: null,
      });
    });
    it("skips when package manager binary is missing (ENOENT)", async () => {
      setupExistingFiles([path.join(WORKTREE, "bun.lock")]);
      const enoent = Object.assign(new Error("not found"), { code: "ENOENT" });
      (execa as any).mockRejectedValueOnce(enoent);

      const result = await installDependenciesForWorktree(WORKTREE);

      expect(result.skipped).toBe(true);
      expect(result.manager).toBe("bun");
    });
  });
});
