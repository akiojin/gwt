import { describe, expect, it } from "vitest";
import {
  ISSUE_BRANCH_LOOKUP_UNKNOWN,
  type ClassifyResult,
  type DockerContext,
} from "../types";
import {
  buildNewBranchName,
  canLaunchFromIssue,
  classifyIssuePrefix,
  dockerStatusHint,
  isIssueSelectable,
  isStaleIssueClassifyRequest,
  parseEnvOverrides,
  parseExtraArgs,
  resolveDockerContextSelection,
  shouldLoadMoreIssues,
  splitBranchNamePrefix,
  supportsModelFor,
  toErrorMessage,
  type BranchPrefix,
  type RuntimeTarget,
} from "./agentLaunchFormHelpers";

const BRANCH_PREFIXES: BranchPrefix[] = ["feature/", "bugfix/", "hotfix/", "release/"];

function dockerContext(overrides: Partial<DockerContext> = {}): DockerContext {
  return {
    file_type: "none",
    compose_services: [],
    docker_available: false,
    compose_available: false,
    daemon_running: false,
    force_host: false,
    ...overrides,
  };
}

describe("agentLaunchFormHelpers", () => {
  it("detects model support by agent id", () => {
    expect(supportsModelFor("codex")).toBe(true);
    expect(supportsModelFor("claude")).toBe(true);
    expect(supportsModelFor("gemini")).toBe(true);
    expect(supportsModelFor("opencode")).toBe(true);
    expect(supportsModelFor("custom")).toBe(false);
  });

  it("formats errors and parses args/env overrides", () => {
    expect(toErrorMessage("plain")).toBe("plain");
    expect(toErrorMessage({ message: "typed" })).toBe("typed");
    expect(toErrorMessage({ message: 42 })).toBe("[object Object]");

    expect(parseExtraArgs("")).toEqual([]);
    expect(parseExtraArgs("--a\n\n  --b  \r\n")).toEqual(["--a", "--b"]);

    expect(parseEnvOverrides("A=1\nB = two")).toEqual({
      env: { A: "1", B: "two" },
      error: null,
    });
    expect(parseEnvOverrides("INVALID")).toEqual({
      env: {},
      error: "Invalid env override at line 1. Use KEY=VALUE.",
    });
    expect(parseEnvOverrides("=value")).toEqual({
      env: {},
      error: "Invalid env override at line 1. Use KEY=VALUE.",
    });
  });

  it("builds and splits branch names", () => {
    expect(buildNewBranchName("feature/", "abc")).toBe("feature/abc");
    expect(buildNewBranchName("feature/", "  ")).toBe("");

    expect(splitBranchNamePrefix(" feature/abc ", BRANCH_PREFIXES)).toEqual({
      prefix: "feature/",
      suffix: "abc",
    });
    expect(splitBranchNamePrefix("unknown/abc", BRANCH_PREFIXES)).toBeNull();
  });

  it("builds docker status hint labels", () => {
    expect(dockerStatusHint("host", dockerContext())).toBe("");
    expect(dockerStatusHint("docker", dockerContext({ images_exist: null, container_status: null }))).toBe(
      "",
    );
    expect(
      dockerStatusHint(
        "docker",
        dockerContext({ images_exist: false, container_status: "not_found" }),
      ),
    ).toContain("will build and create automatically");
    expect(
      dockerStatusHint(
        "docker",
        dockerContext({ images_exist: true, container_status: "stopped" }),
      ),
    ).toContain("Containers stopped - will recreate automatically");
    expect(
      dockerStatusHint(
        "docker",
        dockerContext({ images_exist: true, container_status: "running" }),
      ),
    ).toBe("Images ready / Containers running");
  });

  it("resolves docker context selection for host/compose/dockerfile paths", () => {
    const baseInput = {
      pendingRuntimePreference: null as RuntimeTarget | null,
      pendingDockerServicePreference: "",
      dockerService: "",
    };

    expect(resolveDockerContextSelection({ ...baseInput, context: null })).toEqual({
      runtimeTarget: "host",
      dockerService: "",
      pendingRuntimePreference: null,
      pendingDockerServicePreference: "",
    });

    expect(
      resolveDockerContextSelection({
        ...baseInput,
        context: dockerContext({ file_type: "none", force_host: true }),
      }),
    ).toEqual({
      runtimeTarget: "host",
      dockerService: "",
      pendingRuntimePreference: null,
      pendingDockerServicePreference: "",
    });

    expect(
      resolveDockerContextSelection({
        ...baseInput,
        context: dockerContext({
          file_type: "compose",
          docker_available: true,
          compose_available: true,
          compose_services: [],
        }),
      }),
    ).toEqual({
      runtimeTarget: "docker",
      dockerService: "",
      pendingRuntimePreference: null,
      pendingDockerServicePreference: "",
    });

    expect(
      resolveDockerContextSelection({
        ...baseInput,
        context: dockerContext({
          file_type: "compose",
          docker_available: true,
          compose_available: true,
          compose_services: ["app", "worker"],
        }),
        pendingDockerServicePreference: "worker",
      }),
    ).toEqual({
      runtimeTarget: "docker",
      dockerService: "worker",
      pendingRuntimePreference: null,
      pendingDockerServicePreference: "",
    });

    expect(
      resolveDockerContextSelection({
        ...baseInput,
        context: dockerContext({
          file_type: "compose",
          docker_available: true,
          compose_available: false,
          compose_services: ["app"],
        }),
        pendingRuntimePreference: "docker",
      }),
    ).toEqual({
      runtimeTarget: "host",
      dockerService: "app",
      pendingRuntimePreference: null,
      pendingDockerServicePreference: "",
    });

    expect(
      resolveDockerContextSelection({
        ...baseInput,
        context: dockerContext({
          file_type: "dockerfile",
          docker_available: true,
        }),
        pendingRuntimePreference: "host",
      }),
    ).toEqual({
      runtimeTarget: "host",
      dockerService: "",
      pendingRuntimePreference: null,
      pendingDockerServicePreference: "",
    });
  });

  it("checks issue launch guards and pagination trigger", () => {
    const inFlight = new Set<number>([1]);
    const branchMap = new Map<number, string | null>([
      [1, null],
      [2, "feature/2"],
      [3, null],
      [4, ISSUE_BRANCH_LOOKUP_UNKNOWN],
    ]);

    expect(isIssueSelectable(1, inFlight, branchMap)).toBe(false);
    expect(isIssueSelectable(2, new Set(), branchMap)).toBe(false);
    expect(isIssueSelectable(3, new Set(), branchMap)).toBe(true);
    expect(isIssueSelectable(4, new Set(), branchMap)).toBe(false);
    expect(isIssueSelectable(5, new Set(), branchMap)).toBe(false);

    expect(canLaunchFromIssue(null, new Set(), branchMap)).toBe(false);
    expect(canLaunchFromIssue(3, new Set(), branchMap)).toBe(true);

    expect(shouldLoadMoreIssues(1000, 949, 10, 50, true, false, false)).toBe(true);
    expect(shouldLoadMoreIssues(1000, 700, 200, 50, true, false, false)).toBe(false);
    expect(shouldLoadMoreIssues(1000, 940, 10, 50, false, false, false)).toBe(false);
    expect(shouldLoadMoreIssues(1000, 940, 10, 50, true, true, false)).toBe(false);
    expect(shouldLoadMoreIssues(1000, 940, 10, 50, true, false, true)).toBe(false);
  });

  it("handles issue prefix classification and stale request checks", () => {
    const ok = { status: "ok", prefix: "feature" } as ClassifyResult;
    const invalid = { status: "ok", prefix: "invalid" } as ClassifyResult;
    const error = { status: "error", error: "x" } as ClassifyResult;

    expect(classifyIssuePrefix(ok, BRANCH_PREFIXES)).toBe("feature/");
    expect(classifyIssuePrefix(invalid, BRANCH_PREFIXES)).toBe("");
    expect(classifyIssuePrefix(error, BRANCH_PREFIXES)).toBe("");

    expect(isStaleIssueClassifyRequest(1, 2, 10, 10)).toBe(true);
    expect(isStaleIssueClassifyRequest(1, 1, 11, 10)).toBe(true);
    expect(isStaleIssueClassifyRequest(1, 1, 10, 10)).toBe(false);
  });
});
