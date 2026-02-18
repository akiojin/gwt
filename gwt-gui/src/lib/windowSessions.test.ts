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
