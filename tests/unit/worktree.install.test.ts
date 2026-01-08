import {
  describe,
  it,
  expect,
  mock,
  beforeEach,
  afterEach,
  type Mock,
} from "bun:test";
import path from "node:path";

const accessMock = mock();

mock.module("node:fs/promises", () => ({
  access: accessMock,
  default: {
    access: accessMock,
  },
}));

mock.module("execa", () => ({
  execa: mock(),
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
    accessMock.mockReset();
    (execa as Mock).mockReset();
  });

  afterEach(() => {
    accessMock.mockReset();
    (execa as Mock).mockReset();
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
      (execa as Mock).mockResolvedValue({ stdout: "", stderr: "" });

      await installDependenciesForWorktree(WORKTREE);

      expect(execa).toHaveBeenCalledWith(
        "bun",
        ["install", "--frozen-lockfile"],
        expect.objectContaining({ cwd: WORKTREE }),
      );
    });

    it("runs pnpm install when pnpm lock is present", async () => {
      setupExistingFiles([path.join(WORKTREE, "pnpm-lock.yaml")]);
      (execa as Mock).mockResolvedValue({ stdout: "", stderr: "" });

      await installDependenciesForWorktree(WORKTREE);

      expect(execa).toHaveBeenCalledWith(
        "pnpm",
        ["install", "--frozen-lockfile"],
        expect.objectContaining({ cwd: WORKTREE }),
      );
    });

    it("runs npm ci when package-lock exists", async () => {
      setupExistingFiles([path.join(WORKTREE, "package-lock.json")]);
      (execa as Mock).mockResolvedValue({ stdout: "", stderr: "" });

      await installDependenciesForWorktree(WORKTREE);

      expect(execa).toHaveBeenCalledWith(
        "npm",
        ["ci"],
        expect.objectContaining({ cwd: WORKTREE }),
      );
    });

    it("runs npm install when only package.json exists", async () => {
      setupExistingFiles([path.join(WORKTREE, "package.json")]);
      (execa as Mock).mockResolvedValue({ stdout: "", stderr: "" });

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
      (execa as Mock).mockRejectedValue(new Error("boom"));

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
      (execa as Mock).mockRejectedValueOnce(enoent);

      const result = await installDependenciesForWorktree(WORKTREE);

      expect(result.skipped).toBe(true);
      expect(result.manager).toBe("bun");
    });

    // FR-040a: パッケージマネージャーの出力をそのまま標準出力/標準エラーに表示
    it("passes stdout and stderr as inherit to show package manager output directly (FR-040a)", async () => {
      setupExistingFiles([path.join(WORKTREE, "pnpm-lock.yaml")]);
      (execa as Mock).mockResolvedValue({ stdout: "", stderr: "" });

      await installDependenciesForWorktree(WORKTREE);

      expect(execa).toHaveBeenCalledWith(
        "pnpm",
        ["install", "--frozen-lockfile"],
        expect.objectContaining({
          cwd: WORKTREE,
          stdout: "inherit",
          stderr: "inherit",
        }),
      );
    });

    // FR-040b: スピナー表示を行わない（startSpinnerがインポートされていないことを確認）
    it("does not use spinner during dependency installation (FR-040b)", async () => {
      // このテストは、dependency-installer.tsがstartSpinnerをインポートしていないことを
      // 静的に検証する。実装でスピナーを使用するとインポートが必要になり、
      // そのインポートが削除されていることでFR-040bの遵守を確認できる。
      const moduleSource = await Bun.file(
        path.join(
          import.meta.dir,
          "../../src/services/dependency-installer.ts",
        ),
      ).text();

      expect(moduleSource).not.toContain('from "../utils/spinner');
      expect(moduleSource).not.toContain("startSpinner");
    });
  });
});
