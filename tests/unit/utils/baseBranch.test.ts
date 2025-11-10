import { describe, it, expect, vi } from "vitest";
import {
  resolveBaseBranchRef,
  resolveBaseBranchLabel,
} from "../../../src/cli/ui/utils/baseBranch.js";
import type { SelectedBranchState } from "../../../src/cli/ui/types.js";

const localBranch: SelectedBranchState = {
  name: "feature/feature1",
  displayName: "feature/feature1",
  branchType: "local",
};

const remoteBranch: SelectedBranchState = {
  name: "feature/feature1",
  displayName: "origin/feature/feature1",
  branchType: "remote",
  remoteBranch: "origin/feature/feature1",
};

describe("resolveBaseBranchRef", () => {
  it("prefers creation source branch reference when available", () => {
    const fallback = vi.fn(() => "develop");

    const result = resolveBaseBranchRef(localBranch, null, fallback);

    expect(result).toBe("feature/feature1");
    expect(fallback).not.toHaveBeenCalled();
  });

  it("uses remote reference when creation source branch tracks a remote", () => {
    const fallback = vi.fn(() => "develop");

    const result = resolveBaseBranchRef(remoteBranch, null, fallback);

    expect(result).toBe("origin/feature/feature1");
  });

  it("falls back to selected branch when creation source is null", () => {
    const fallback = vi.fn(() => "develop");

    const result = resolveBaseBranchRef(null, remoteBranch, fallback);

    expect(result).toBe("origin/feature/feature1");
    expect(fallback).not.toHaveBeenCalled();
  });

  it("uses default resolver when no branch information is available", () => {
    const fallback = vi.fn(() => "develop");

    const result = resolveBaseBranchRef(null, null, fallback);

    expect(result).toBe("develop");
    expect(fallback).toHaveBeenCalledTimes(1);
  });
});

describe("resolveBaseBranchLabel", () => {
  it("prefers creation source label when provided", () => {
    const fallback = vi.fn(() => "develop");

    const result = resolveBaseBranchLabel(localBranch, remoteBranch, fallback);

    expect(result).toBe("feature/feature1");
  });

  it("falls back to selected branch label when creation source is null", () => {
    const fallback = vi.fn(() => "develop");

    const result = resolveBaseBranchLabel(null, remoteBranch, fallback);

    expect(result).toBe("origin/feature/feature1");
  });

  it("falls back to default label when neither branch is available", () => {
    const fallback = vi.fn(() => "develop");

    const result = resolveBaseBranchLabel(null, null, fallback);

    expect(result).toBe("develop");
    expect(fallback).toHaveBeenCalledTimes(1);
  });
});
