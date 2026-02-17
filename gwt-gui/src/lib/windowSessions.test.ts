import { describe, expect, it } from "vitest";
import {
  getWindowSession,
  loadWindowSessions,
  persistWindowSessions,
  removeWindowSession,
  upsertWindowSession,
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
