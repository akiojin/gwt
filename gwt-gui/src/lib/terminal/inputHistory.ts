/**
 * Per-pane input history with localStorage persistence.
 */

const STORAGE_KEY_PREFIX = "gwt.inputHistory.v1.";
const MAX_ENTRIES = 100;

export class InputHistory {
  private entries: string[] = [];
  /** Index into entries for navigation. -1 = draft (current input). */
  private index = -1;
  private readonly storageKey: string;
  private readonly storage: Storage;

  constructor(paneId: string, storage: Storage = localStorage) {
    this.storageKey = STORAGE_KEY_PREFIX + paneId;
    this.storage = storage;
    this.load();
  }

  /** Push a new entry. Resets navigation index. */
  push(text: string): void {
    if (!text) return;

    // Avoid consecutive duplicates
    if (this.entries.length > 0 && this.entries[this.entries.length - 1] === text) {
      this.index = -1;
      return;
    }

    this.entries.push(text);

    // Enforce max limit (drop oldest)
    if (this.entries.length > MAX_ENTRIES) {
      this.entries = this.entries.slice(this.entries.length - MAX_ENTRIES);
    }

    this.index = -1;
    this.persist();
  }

  /** Navigate to older entry. Returns the entry text. */
  back(): string {
    if (this.entries.length === 0) return "";

    if (this.index === -1) {
      // Start from most recent
      this.index = this.entries.length - 1;
    } else if (this.index > 0) {
      this.index--;
    }
    // else: already at oldest, stay

    return this.entries[this.index] ?? "";
  }

  /** Navigate to newer entry. Returns entry text or "" for draft. */
  forward(): string {
    if (this.index === -1) return "";

    if (this.index < this.entries.length - 1) {
      this.index++;
      return this.entries[this.index] ?? "";
    }

    // Back to draft
    this.index = -1;
    return "";
  }

  /** Get current entry at navigation position. */
  current(): string {
    if (this.index === -1) return "";
    return this.entries[this.index] ?? "";
  }

  /** Remove persisted data for this pane. */
  dispose(): void {
    this.storage.removeItem(this.storageKey);
    this.entries = [];
    this.index = -1;
  }

  private load(): void {
    try {
      const raw = this.storage.getItem(this.storageKey);
      if (raw) {
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed)) {
          this.entries = parsed.filter(
            (e): e is string => typeof e === "string",
          );
        }
      }
    } catch {
      // Corrupted storage — start fresh
      this.entries = [];
    }
  }

  private persist(): void {
    this.storage.setItem(this.storageKey, JSON.stringify(this.entries));
  }
}
