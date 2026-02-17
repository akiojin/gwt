import { describe, expect, it } from "vitest";
import {
  WINDOW_SESSION_RESTORE_LEAD_KEY,
  WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
  isWindowSessionRestoreLeaderCandidate,
  readWindowSessionRestoreLeader,
  releaseWindowSessionRestoreLead,
  tryAcquireWindowSessionRestoreLead,
} from "./windowSessionRestoreLeader";

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

describe("windowSessionRestoreLeader", () => {
  it("only allows main window as restore leader candidate", () => {
    expect(isWindowSessionRestoreLeaderCandidate("main")).toBe(true);
    expect(isWindowSessionRestoreLeaderCandidate(" project-1 ")).toBe(false);
    expect(isWindowSessionRestoreLeaderCandidate("")).toBe(false);
  });

  it("acquires leader for main window", () => {
    const storage = createMockStorage();
    const now = 1_000;

    expect(tryAcquireWindowSessionRestoreLead(storage, "main", now)).toBe(true);

    expect(readWindowSessionRestoreLeader(storage)).toEqual({
      label: "main",
      expiresAt: now + WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
    });
  });

  it("never acquires leader for non-main windows", () => {
    const storage = createMockStorage();
    const now = 1_000;

    expect(
      tryAcquireWindowSessionRestoreLead(storage, "project-123", now),
    ).toBe(false);
    expect(storage.getItem(WINDOW_SESSION_RESTORE_LEAD_KEY)).toBeNull();
  });

  it("blocks takeover while another leader is active", () => {
    const storage = createMockStorage();
    storage.setItem(
      WINDOW_SESSION_RESTORE_LEAD_KEY,
      JSON.stringify({
        label: "other",
        expiresAt: 20_000,
      }),
    );

    expect(tryAcquireWindowSessionRestoreLead(storage, "main", 10_000)).toBe(
      false,
    );
    expect(readWindowSessionRestoreLeader(storage)).toEqual({
      label: "other",
      expiresAt: 20_000,
    });
  });

  it("reacquires after expiration and releases only matching label", () => {
    const storage = createMockStorage();
    storage.setItem(
      WINDOW_SESSION_RESTORE_LEAD_KEY,
      JSON.stringify({
        label: "old",
        expiresAt: 5_000,
      }),
    );

    expect(tryAcquireWindowSessionRestoreLead(storage, "main", 6_000)).toBe(
      true,
    );
    expect(readWindowSessionRestoreLeader(storage)).toEqual({
      label: "main",
      expiresAt: 6_000 + WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
    });

    releaseWindowSessionRestoreLead(storage, "project-1");
    expect(readWindowSessionRestoreLeader(storage)).toEqual({
      label: "main",
      expiresAt: 6_000 + WINDOW_SESSION_RESTORE_LEAD_TTL_MS,
    });

    releaseWindowSessionRestoreLead(storage, "main");
    expect(readWindowSessionRestoreLeader(storage)).toBeNull();
  });
});
