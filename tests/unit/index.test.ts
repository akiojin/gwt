import { describe, it, expect, beforeEach, vi } from 'vitest';

vi.mock('../../src/worktree.js', () => ({
  createWorktree: vi.fn(),
  worktreeExists: vi.fn(),
}));

vi.mock('../../src/config/index.js', () => ({
  saveSession: vi.fn(),
}));

vi.mock('../../src/claude.js', () => ({
  launchClaudeCode: vi.fn(),
  ClaudeError: class extends Error {},
}));

vi.mock('../../src/codex.js', () => ({
  launchCodexCLI: vi.fn(),
  CodexError: class extends Error {},
}));

import { ensureWorktreeExists, launchTool } from '../../src/index.js';
import type { LaunchRequest } from '../../src/ui/components/App.js';
import { createWorktree, worktreeExists } from '../../src/worktree.js';
import { saveSession } from '../../src/config/index.js';
import { launchClaudeCode } from '../../src/claude.js';
import { launchCodexCLI } from '../../src/codex.js';
describe('CLI launch orchestration', () => {
  const baseLaunch: LaunchRequest = {
    repoRoot: '/repo',
    branchName: 'feature/test',
    worktreePath: '/repo/.git/worktree/feature-test',
    mode: 'normal',
    skipPermissions: false,
    createWorktree: true,
    isNewBranch: false,
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('creates a worktree when required', async () => {
    (worktreeExists as unknown as ReturnType<typeof vi.fn>).mockResolvedValue(
      baseLaunch.worktreePath,
    );

    await ensureWorktreeExists(baseLaunch);

    expect(createWorktree).toHaveBeenCalledWith({
      branchName: baseLaunch.branchName,
      worktreePath: baseLaunch.worktreePath,
      repoRoot: baseLaunch.repoRoot,
      isNewBranch: false,
      baseBranch: baseLaunch.branchName,
    });
  });

  it('skips worktree creation when not needed', async () => {
    const launch: LaunchRequest = {
      ...baseLaunch,
      createWorktree: false,
    };

    await ensureWorktreeExists(launch);

    expect(createWorktree).not.toHaveBeenCalled();
  });

  it('launches Claude Code with saved session metadata', async () => {
    const launch: LaunchRequest = {
      ...baseLaunch,
      createWorktree: false,
      tool: 'claude-code',
      skipPermissions: true,
    };

    await launchTool(launch);

    expect(saveSession).toHaveBeenCalledWith(
      expect.objectContaining({
        lastBranch: launch.branchName,
        lastWorktreePath: launch.worktreePath,
        repositoryRoot: launch.repoRoot,
      }),
    );

    expect(launchClaudeCode).toHaveBeenCalledWith(launch.worktreePath, {
      mode: launch.mode,
      skipPermissions: true,
    });
  });

  it('launches Codex CLI with resume mode', async () => {
    const launch: LaunchRequest = {
      ...baseLaunch,
      createWorktree: false,
      tool: 'codex-cli',
      mode: 'resume',
      skipPermissions: false,
    };

    await launchTool(launch);

    expect(saveSession).toHaveBeenCalled();
    expect(launchCodexCLI).toHaveBeenCalledWith(launch.worktreePath, {
      mode: 'resume',
      bypassApprovals: false,
    });
  });
});
