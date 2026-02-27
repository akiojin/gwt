import { describe, expect, it } from "vitest";
import {
  deduplicateByProjectPath,
  getWindowSession,
  loadWindowSessions,
  persistWindowSessions,
  pruneWindowSessions,
  removeWindowSession,
  upsertWindowSession,
  WINDOW_SESSIONS_STORAGE_KEY,
} from "./windowSessions";

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

describe("windowSessions", () => {
  it("sanitizes and persists session order while deduplicating labels", () => {
    const store = createMockStorage();

    persistWindowSessions(
      [
        { label: "main", projectPath: " /tmp/main " },
        { label: "  ", projectPath: "/tmp/ignore" },
        { label: "main", projectPath: "/tmp/main-updated" },
        { label: "project-1", projectPath: "/tmp/project-1" },
      ],
      store,
    );

    const sessions = loadWindowSessions(store);
    expect(sessions).toEqual([
      { label: "main", projectPath: "/tmp/main-updated" },
      { label: "project-1", projectPath: "/tmp/project-1" },
    ]);
  });

  it("upserts and removes windows by label", () => {
    const store = createMockStorage();

    upsertWindowSession("main", "/tmp/main", store);
    upsertWindowSession("project-1", "/tmp/project-1", store);
    upsertWindowSession("main", "/tmp/main-2", store);

    expect(loadWindowSessions(store)).toEqual([
      { label: "project-1", projectPath: "/tmp/project-1" },
      { label: "main", projectPath: "/tmp/main-2" },
    ]);

    removeWindowSession("project-1", store);
    expect(getWindowSession("project-1", store)).toBeNull();
  });
});

describe("deduplicateByProjectPath", () => {
  it("keeps only the first entry per projectPath", () => {
    const result = deduplicateByProjectPath([
      { label: "main", projectPath: "/tmp/gwt" },
      { label: "main-1", projectPath: "/tmp/gwt" },
      { label: "main-1-1", projectPath: "/tmp/gwt" },
      { label: "project-1", projectPath: "/tmp/llmlb" },
      { label: "project-1-1", projectPath: "/tmp/llmlb" },
    ]);
    expect(result).toEqual([
      { label: "main", projectPath: "/tmp/gwt" },
      { label: "project-1", projectPath: "/tmp/llmlb" },
    ]);
  });

  it("returns an empty array when given an empty array", () => {
    expect(deduplicateByProjectPath([])).toEqual([]);
  });

  it("returns the same single entry unchanged", () => {
    const input = [{ label: "main", projectPath: "/tmp/project" }];
    expect(deduplicateByProjectPath(input)).toEqual(input);
  });

  it("preserves entries with distinct projectPaths", () => {
    const input = [
      { label: "a", projectPath: "/a" },
      { label: "b", projectPath: "/b" },
      { label: "c", projectPath: "/c" },
    ];
    expect(deduplicateByProjectPath(input)).toEqual(input);
  });
});

describe("windowSessions – corrupted data", () => {
  it("returns empty array when storage contains invalid JSON", () => {
    const store = createMockStorage();
    store.setItem(WINDOW_SESSIONS_STORAGE_KEY, "not-json{{{");
    expect(loadWindowSessions(store)).toEqual([]);
  });

  it("returns empty array when storage contains a non-array JSON value", () => {
    const store = createMockStorage();
    store.setItem(WINDOW_SESSIONS_STORAGE_KEY, JSON.stringify({ foo: "bar" }));
    expect(loadWindowSessions(store)).toEqual([]);
  });

  it("filters out entries with missing or non-string fields", () => {
    const store = createMockStorage();
    const data = [
      { label: "valid", projectPath: "/tmp/valid" },
      { label: 123, projectPath: "/tmp/bad-label" },
      { label: "no-path" },
      null,
      42,
      { label: "ok", projectPath: "/tmp/ok" },
    ];
    store.setItem(WINDOW_SESSIONS_STORAGE_KEY, JSON.stringify(data));
    expect(loadWindowSessions(store)).toEqual([
      { label: "valid", projectPath: "/tmp/valid" },
      { label: "ok", projectPath: "/tmp/ok" },
    ]);
  });

  it("handles entries where label or projectPath is empty after trim", () => {
    const store = createMockStorage();
    const data = [
      { label: "   ", projectPath: "/tmp/blank-label" },
      { label: "empty-path", projectPath: "   " },
      { label: "good", projectPath: "/tmp/good" },
    ];
    store.setItem(WINDOW_SESSIONS_STORAGE_KEY, JSON.stringify(data));
    expect(loadWindowSessions(store)).toEqual([
      { label: "good", projectPath: "/tmp/good" },
    ]);
  });
});

describe("windowSessions – multi-window operations", () => {
  it("upsert with empty label is a no-op", () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/main", store);
    upsertWindowSession("", "/tmp/empty-label", store);
    upsertWindowSession("  ", "/tmp/blank-label", store);
    expect(loadWindowSessions(store)).toEqual([
      { label: "main", projectPath: "/tmp/main" },
    ]);
  });

  it("upsert with empty projectPath is a no-op", () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/main", store);
    upsertWindowSession("no-path", "", store);
    upsertWindowSession("blank-path", "  ", store);
    expect(loadWindowSessions(store)).toEqual([
      { label: "main", projectPath: "/tmp/main" },
    ]);
  });

  it("removeWindowSession with empty label is a no-op", () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/main", store);
    removeWindowSession("", store);
    removeWindowSession("  ", store);
    expect(loadWindowSessions(store)).toEqual([
      { label: "main", projectPath: "/tmp/main" },
    ]);
  });

  it("getWindowSession returns null for empty label", () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/main", store);
    expect(getWindowSession("", store)).toBeNull();
    expect(getWindowSession("  ", store)).toBeNull();
  });

  it("getWindowSession returns matching entry", () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/main", store);
    upsertWindowSession("dev", "/tmp/dev", store);
    expect(getWindowSession("dev", store)).toEqual({
      label: "dev",
      projectPath: "/tmp/dev",
    });
  });

  it("getWindowSession returns null for non-existent label", () => {
    const store = createMockStorage();
    upsertWindowSession("main", "/tmp/main", store);
    expect(getWindowSession("unknown", store)).toBeNull();
  });
});

describe("windowSessions – session restore", () => {
  it("loadWindowSessions returns empty without storage", () => {
    expect(loadWindowSessions(null)).toEqual([]);
  });

  it("loadWindowSessions returns empty with empty storage key", () => {
    const store = createMockStorage();
    expect(loadWindowSessions(store)).toEqual([]);
  });

  it("persistWindowSessions silently ignores null storage", () => {
    // Should not throw
    persistWindowSessions(
      [{ label: "main", projectPath: "/tmp/main" }],
      null,
    );
  });

  it("preserves insertion order across multiple upserts", () => {
    const store = createMockStorage();
    upsertWindowSession("a", "/tmp/a", store);
    upsertWindowSession("b", "/tmp/b", store);
    upsertWindowSession("c", "/tmp/c", store);
    upsertWindowSession("a", "/tmp/a-new", store); // moves 'a' to end

    expect(loadWindowSessions(store)).toEqual([
      { label: "b", projectPath: "/tmp/b" },
      { label: "c", projectPath: "/tmp/c" },
      { label: "a", projectPath: "/tmp/a-new" },
    ]);
  });
});

describe("pruneWindowSessions", () => {
  it("removes duplicate projectPath entries from storage", () => {
    const store = createMockStorage();
    const staleData = [
      { label: "main", projectPath: "/tmp/gwt" },
      { label: "main-1", projectPath: "/tmp/gwt" },
      { label: "main-1-1", projectPath: "/tmp/gwt" },
      { label: "project-1", projectPath: "/tmp/llmlb" },
      { label: "project-1-1", projectPath: "/tmp/llmlb" },
    ];
    store.setItem(WINDOW_SESSIONS_STORAGE_KEY, JSON.stringify(staleData));

    pruneWindowSessions(store);

    const result = loadWindowSessions(store);
    expect(result).toEqual([
      { label: "main", projectPath: "/tmp/gwt" },
      { label: "project-1", projectPath: "/tmp/llmlb" },
    ]);
  });

  it("does not write to storage when no duplicates exist", () => {
    const store = createMockStorage();
    const cleanData = [
      { label: "main", projectPath: "/tmp/gwt" },
      { label: "project-1", projectPath: "/tmp/llmlb" },
    ];
    store.setItem(WINDOW_SESSIONS_STORAGE_KEY, JSON.stringify(cleanData));

    const originalValue = store.getItem(WINDOW_SESSIONS_STORAGE_KEY);
    pruneWindowSessions(store);
    expect(store.getItem(WINDOW_SESSIONS_STORAGE_KEY)).toBe(originalValue);
  });

  it("handles empty storage gracefully", () => {
    const store = createMockStorage();
    pruneWindowSessions(store);
    expect(store.getItem(WINDOW_SESSIONS_STORAGE_KEY)).toBeNull();
  });
});
