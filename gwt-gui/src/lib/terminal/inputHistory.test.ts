import { describe, expect, it, beforeEach, vi } from "vitest";
import { InputHistory } from "./inputHistory";

function createMockStorage(): Storage {
  const store = new Map<string, string>();
  return {
    getItem: vi.fn((key: string) => store.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => { store.set(key, value); }),
    removeItem: vi.fn((key: string) => { store.delete(key); }),
    clear: vi.fn(() => { store.clear(); }),
    get length() { return store.size; },
    key: vi.fn(() => null),
  };
}

describe("InputHistory", () => {
  let storage: Storage;

  beforeEach(() => {
    storage = createMockStorage();
  });

  it("starts with empty history", () => {
    const h = new InputHistory("pane-1", storage);
    expect(h.current()).toBe("");
  });

  it("pushes entries and navigates back", () => {
    const h = new InputHistory("pane-1", storage);
    h.push("first");
    h.push("second");

    // Navigate back from empty current
    expect(h.back()).toBe("second");
    expect(h.back()).toBe("first");
    // At oldest entry, stays there
    expect(h.back()).toBe("first");
  });

  it("navigates forward after going back", () => {
    const h = new InputHistory("pane-1", storage);
    h.push("first");
    h.push("second");

    h.back(); // "second"
    h.back(); // "first"
    expect(h.forward()).toBe("second");
    expect(h.forward()).toBe(""); // back to empty draft
  });

  it("does not push duplicate consecutive entries", () => {
    const h = new InputHistory("pane-1", storage);
    h.push("same");
    h.push("same");

    expect(h.back()).toBe("same");
    expect(h.back()).toBe("same"); // only one entry
  });

  it("does not push empty strings", () => {
    const h = new InputHistory("pane-1", storage);
    h.push("");
    expect(h.back()).toBe(""); // no history entry
  });

  it("persists to storage on push", () => {
    const h = new InputHistory("pane-1", storage);
    h.push("hello");

    expect(storage.setItem).toHaveBeenCalledWith(
      "gwt.inputHistory.v1.pane-1",
      expect.any(String),
    );

    const stored = JSON.parse(
      (storage.getItem as ReturnType<typeof vi.fn>).mock.results
        .filter((r: { type: string }) => r.type === "return")
        .pop()?.value ?? "[]",
    );
    // Verify via new instance loading from same storage
    const h2 = new InputHistory("pane-1", storage);
    expect(h2.back()).toBe("hello");
  });

  it("restores history from storage", () => {
    storage.setItem(
      "gwt.inputHistory.v1.pane-1",
      JSON.stringify(["old-entry"]),
    );

    const h = new InputHistory("pane-1", storage);
    expect(h.back()).toBe("old-entry");
  });

  it("limits to 100 entries", () => {
    const h = new InputHistory("pane-1", storage);
    for (let i = 0; i < 120; i++) {
      h.push(`entry-${i}`);
    }

    // Count total entries by navigating back
    let count = 0;
    let prev = "";
    while (true) {
      const val = h.back();
      if (val === prev && count > 0) break;
      prev = val;
      count++;
    }
    expect(count).toBeLessThanOrEqual(100);
  });

  it("uses separate storage per pane", () => {
    const h1 = new InputHistory("pane-1", storage);
    const h2 = new InputHistory("pane-2", storage);

    h1.push("from-pane-1");
    h2.push("from-pane-2");

    const h1b = new InputHistory("pane-1", storage);
    const h2b = new InputHistory("pane-2", storage);

    expect(h1b.back()).toBe("from-pane-1");
    expect(h2b.back()).toBe("from-pane-2");
  });

  it("resets navigation index on push", () => {
    const h = new InputHistory("pane-1", storage);
    h.push("first");
    h.push("second");

    h.back(); // "second"
    h.push("third"); // resets index

    expect(h.back()).toBe("third");
    expect(h.back()).toBe("second");
  });

  it("dispose clears storage for the pane", () => {
    const h = new InputHistory("pane-1", storage);
    h.push("data");
    h.dispose();

    expect(storage.removeItem).toHaveBeenCalledWith("gwt.inputHistory.v1.pane-1");
  });

  it("handles corrupted storage gracefully", () => {
    storage.setItem("gwt.inputHistory.v1.pane-1", "not-valid-json");
    const h = new InputHistory("pane-1", storage);
    // Should not throw, starts with empty history
    expect(h.current()).toBe("");
  });
});
