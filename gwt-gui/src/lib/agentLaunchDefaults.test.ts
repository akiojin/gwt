import { beforeEach, describe, expect, it } from "vitest";
import {
  AGENT_LAUNCH_DEFAULTS_STORAGE_KEY,
  loadLaunchDefaults,
  saveLaunchDefaults,
} from "./agentLaunchDefaults";

class MemoryStorage implements Storage {
  private data = new Map<string, string>();

  get length(): number {
    return this.data.size;
  }

  clear(): void {
    this.data.clear();
  }

  getItem(key: string): string | null {
    return this.data.has(key) ? this.data.get(key)! : null;
  }

  key(index: number): string | null {
    return Array.from(this.data.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.data.delete(key);
  }

  setItem(key: string, value: string): void {
    this.data.set(key, value);
  }
}

describe("agentLaunchDefaults", () => {
  let store: MemoryStorage;

  beforeEach(() => {
    store = new MemoryStorage();
  });

  it("returns null when defaults are not stored", () => {
    expect(loadLaunchDefaults(store)).toBeNull();
  });

  it("persists and restores launch defaults", () => {
    saveLaunchDefaults(
      {
        selectedAgent: "codex",
        sessionMode: "continue",
        modelByAgent: { codex: "gpt-5.3-codex-spark" },
        agentVersionByAgent: { codex: "latest" },
        skipPermissions: true,
        reasoningLevel: "high",
        resumeSessionId: "session-123",
        showAdvanced: true,
        extraArgsText: "--foo",
        envOverridesText: "FOO=bar",
        runtimeTarget: "docker",
        dockerService: "app",
        dockerBuild: true,
        dockerRecreate: true,
        dockerKeep: false,
      },
      store,
    );

    expect(loadLaunchDefaults(store)).toEqual({
      selectedAgent: "codex",
      sessionMode: "continue",
      modelByAgent: { codex: "gpt-5.3-codex-spark" },
      agentVersionByAgent: { codex: "latest" },
      skipPermissions: true,
      reasoningLevel: "high",
      resumeSessionId: "session-123",
      showAdvanced: true,
      extraArgsText: "--foo",
      envOverridesText: "FOO=bar",
      runtimeTarget: "docker",
      dockerService: "app",
      dockerBuild: true,
      dockerRecreate: true,
      dockerKeep: false,
    });
  });

  it("returns null for invalid JSON", () => {
    store.setItem(AGENT_LAUNCH_DEFAULTS_STORAGE_KEY, "{bad-json");
    expect(loadLaunchDefaults(store)).toBeNull();
  });

  it("returns null for unknown schema version", () => {
    store.setItem(
      AGENT_LAUNCH_DEFAULTS_STORAGE_KEY,
      JSON.stringify({ version: 99, data: {} }),
    );
    expect(loadLaunchDefaults(store)).toBeNull();
  });

  it("sanitizes invalid values to safe defaults", () => {
    store.setItem(
      AGENT_LAUNCH_DEFAULTS_STORAGE_KEY,
      JSON.stringify({
        version: 1,
        data: {
          selectedAgent: 123,
          sessionMode: "bad",
          modelByAgent: { codex: 123, gemini: " gemini-2.5-pro " },
          agentVersionByAgent: { codex: null, gemini: " installed " },
          skipPermissions: "yes",
          reasoningLevel: 1,
          resumeSessionId: null,
          showAdvanced: "yes",
          extraArgsText: 123,
          envOverridesText: false,
          runtimeTarget: "bad",
          dockerService: 123,
          dockerBuild: 1,
          dockerRecreate: true,
          dockerKeep: "no",
        },
      }),
    );

    expect(loadLaunchDefaults(store)).toEqual({
      selectedAgent: "",
      sessionMode: "normal",
      modelByAgent: { gemini: "gemini-2.5-pro" },
      agentVersionByAgent: { gemini: "installed" },
      skipPermissions: false,
      reasoningLevel: "",
      resumeSessionId: "",
      showAdvanced: false,
      extraArgsText: "",
      envOverridesText: "",
      runtimeTarget: "host",
      dockerService: "",
      dockerBuild: false,
      dockerRecreate: true,
      dockerKeep: false,
    });
  });
});
