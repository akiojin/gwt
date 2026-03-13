import { describe, expect, it, vi } from "vitest";
import type { OpenProjectResult, ProbePathResult } from "./types";
import { getWindowSession, upsertWindowSession } from "./windowSessions";
import {
  openAndNormalizeRestoredWindowSession,
  restoreCurrentWindowSession,
} from "./windowSessionRestore";

function createMockStorage(): Storage {
  const entries = new Map<string, string>();
  return {
    get length() {
      return entries.size;
    },
    clear() {
      entries.clear();
    },
    key(index: number): string | null {
      return Array.from(entries.keys())[index] ?? null;
    },
    getItem(name: string): string | null {
      return entries.get(name) ?? null;
    },
    removeItem(name: string) {
      entries.delete(name);
    },
    setItem(name: string, value: string) {
      entries.set(name, value);
    },
  } as Storage;
}

function createOpenProjectResult(
  action: OpenProjectResult["action"],
  path = "/tmp/project",
): OpenProjectResult {
  return {
    action,
    info: {
      path,
      repo_name: "project",
      current_branch: "main",
    },
  };
}

describe("windowSessionRestore", () => {
  it("restores the current window when probe returns a gwt project", async () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/project", store);
    const invoke = vi.fn(async (command: string) => {
      if (command === "probe_path") {
        return {
          kind: "gwtProject",
          projectPath: "/tmp/project-canonical",
        } satisfies ProbePathResult;
      }
      if (command === "open_project") {
        return createOpenProjectResult("opened", "/tmp/project-canonical");
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const result = await restoreCurrentWindowSession("main", invoke as any, store);

    expect(result).toEqual({
      kind: "opened",
      result: createOpenProjectResult("opened", "/tmp/project-canonical"),
    });
    expect(invoke).toHaveBeenNthCalledWith(1, "probe_path", { path: "/tmp/project" });
    expect(invoke).toHaveBeenNthCalledWith(2, "open_project", {
      path: "/tmp/project-canonical",
    });
    expect(getWindowSession("main", store)?.projectPath).toBe("/tmp/project");
  });

  it("opens migration flow for the current window and removes the stale restore session", async () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/project", store);
    const invoke = vi.fn(async (command: string) => {
      if (command === "probe_path") {
        return {
          kind: "migrationRequired",
          migrationSourceRoot: "/tmp/project",
        } satisfies ProbePathResult;
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const result = await restoreCurrentWindowSession("main", invoke as any, store);

    expect(result).toEqual({
      kind: "migrationRequired",
      sourceRoot: "/tmp/project",
    });
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(getWindowSession("main", store)).toBeNull();
  });

  it("clears the current restore session when the project focuses an existing window", async () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/project", store);
    const invoke = vi.fn(async (command: string) => {
      if (command === "probe_path") {
        return {
          kind: "gwtProject",
          projectPath: "/tmp/project",
        } satisfies ProbePathResult;
      }
      if (command === "open_project") {
        return {
          ...createOpenProjectResult("focusedExisting"),
          focusedWindowLabel: "project-1",
        } satisfies OpenProjectResult;
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const result = await restoreCurrentWindowSession("main", invoke as any, store);

    expect(result).toEqual({
      kind: "focusedExisting",
      focusedWindowLabel: "project-1",
    });
    expect(getWindowSession("main", store)).toBeNull();
  });

  it("removes stale restore data when probe reports a missing path", async () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/missing", store);
    const invoke = vi.fn(async (command: string) => {
      if (command === "probe_path") {
        return {
          kind: "notFound",
          message: "Path does not exist: /tmp/missing",
        } satisfies ProbePathResult;
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const result = await restoreCurrentWindowSession("main", invoke as any, store);

    expect(result).toEqual({
      kind: "stale",
      reason: "notFound",
    });
    expect(getWindowSession("main", store)).toBeNull();
  });

  it("opens a secondary restored window and normalizes its session label", async () => {
    const store = createMockStorage();
    upsertWindowSession("project-1", "/tmp/project", store);
    const invoke = vi.fn(async (command: string) => {
      if (command === "probe_path") {
        return {
          kind: "gwtProject",
          projectPath: "/tmp/project-canonical",
        } satisfies ProbePathResult;
      }
      if (command === "open_gwt_window") {
        return "project-1-1";
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const result = await openAndNormalizeRestoredWindowSession(
      "project-1",
      invoke as any,
      store,
    );

    expect(result).toEqual({
      kind: "opened",
      openedLabel: "project-1-1",
    });
    expect(getWindowSession("project-1", store)).toBeNull();
    expect(getWindowSession("project-1-1", store)?.projectPath).toBe(
      "/tmp/project-canonical",
    );
  });

  it("skips secondary window creation for migration-required sessions", async () => {
    const store = createMockStorage();
    upsertWindowSession("project-1", "/tmp/project", store);
    const invoke = vi.fn(async (command: string) => {
      if (command === "probe_path") {
        return {
          kind: "migrationRequired",
          migrationSourceRoot: "/tmp/project",
        } satisfies ProbePathResult;
      }
      if (command === "open_gwt_window") {
        throw new Error("open_gwt_window should not be called");
      }
      throw new Error(`unexpected command: ${command}`);
    });

    const result = await openAndNormalizeRestoredWindowSession(
      "project-1",
      invoke as any,
      store,
    );

    expect(result).toEqual({
      kind: "migrationRequired",
      sourceRoot: "/tmp/project",
    });
    expect(getWindowSession("project-1", store)).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(1);
  });
});
