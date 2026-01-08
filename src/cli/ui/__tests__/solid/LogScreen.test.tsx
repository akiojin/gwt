/** @jsxImportSource @opentui/solid */
import { describe, expect, it, mock } from "bun:test";
import { testRender } from "@opentui/solid";
import { createSignal } from "solid-js";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";

const makeEntry = (
  message: string,
  overrides: Partial<FormattedLogEntry> = {},
): FormattedLogEntry => ({
  id: "1",
  raw: { level: 30, time: "2026-01-08T00:00:00.000Z", category: "cli" },
  timestamp: 0,
  timeLabel: "00:00:00",
  levelLabel: "INFO",
  category: "cli",
  message,
  summary: `[00:00:00] [INFO] [cli] ${message}`,
  json: JSON.stringify({ message }, null, 2),
  ...overrides,
});

describe("LogScreen", () => {
  it("updates entries when data changes", async () => {
    const [entries, setEntries] = createSignal<FormattedLogEntry[]>([]);

    const { LogScreen } = await import("../../screens/solid/LogScreen.js");

    const testSetup = await testRender(
      () => (
        <LogScreen
          entries={entries()}
          branchLabel="feature/logs"
          sourceLabel="/tmp/feature-logs"
          onBack={() => {}}
          onSelect={() => {}}
          onCopy={() => {}}
        />
      ),
      { width: 80, height: 24 },
    );

    try {
      await testSetup.renderOnce();
      let frame = testSetup.captureCharFrame();
      expect(frame).toContain("Branch: feature/logs");
      expect(frame).toContain("Source: /tmp/feature-logs");
      expect(frame).toContain("No logs available.");

      setEntries([makeEntry("Hello")]);
      await testSetup.renderOnce();
      frame = testSetup.captureCharFrame();
      expect(frame).toContain("[00:00:00] [INFO ] [cli ] Hello");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("filters entries by query and shows counts", async () => {
    const entries = [
      makeEntry("alpha", {
        id: "1",
        category: "cli",
        raw: { level: 30, time: "2026-01-08T00:00:00.000Z", category: "cli" },
      }),
      makeEntry("beta", {
        id: "2",
        category: "agent.stdout",
        raw: {
          level: 50,
          time: "2026-01-08T00:00:01.000Z",
          category: "agent.stdout",
        },
      }),
    ];

    const { LogScreen } = await import("../../screens/solid/LogScreen.js");

    const testSetup = await testRender(
      () => (
        <LogScreen
          entries={entries}
          onBack={() => {}}
          onSelect={() => {}}
          onCopy={() => {}}
        />
      ),
      { width: 80, height: 24 },
    );

    try {
      await testSetup.renderOnce();
      let frame = testSetup.captureCharFrame();
      expect(frame).toContain("alpha");
      expect(frame).toContain("beta");

      await testSetup.mockInput.typeText("f");
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("agent");
      await testSetup.renderOnce();
      testSetup.mockInput.pressEnter();
      await testSetup.renderOnce();

      frame = testSetup.captureCharFrame();
      expect(frame).toContain("beta");
      expect(frame).not.toContain("alpha");
      expect(frame).toContain("Showing 1 of 2");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("cycles level filter with v and hides lower levels", async () => {
    const entries = [
      makeEntry("info-log", {
        id: "1",
        levelLabel: "INFO",
        raw: { level: 30, time: "2026-01-08T00:00:00.000Z", category: "cli" },
      }),
      makeEntry("error-log", {
        id: "2",
        levelLabel: "ERROR",
        raw: { level: 50, time: "2026-01-08T00:00:01.000Z", category: "cli" },
      }),
    ];

    const { LogScreen } = await import("../../screens/solid/LogScreen.js");

    const testSetup = await testRender(
      () => (
        <LogScreen
          entries={entries}
          onBack={() => {}}
          onSelect={() => {}}
          onCopy={() => {}}
        />
      ),
      { width: 80, height: 24 },
    );

    try {
      await testSetup.renderOnce();
      let frame = testSetup.captureCharFrame();
      expect(frame).toContain("info-log");
      expect(frame).toContain("error-log");

      await testSetup.mockInput.typeText("v");
      await testSetup.renderOnce();
      await testSetup.mockInput.typeText("v");
      await testSetup.renderOnce();

      frame = testSetup.captureCharFrame();
      expect(frame).toContain("error-log");
      expect(frame).not.toContain("info-log");
      expect(frame).toContain("Level: WARN+");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("triggers reload/tail and toggles wrap", async () => {
    const onReload = mock();
    const onToggleTail = mock();
    const longMessage =
      "this-is-a-very-long-log-line-that-should-be-truncated-when-wrap-is-off";
    const entries = [
      makeEntry(longMessage, {
        id: "1",
        raw: { level: 30, time: "2026-01-08T00:00:00.000Z", category: "cli" },
      }),
    ];

    const { LogScreen } = await import("../../screens/solid/LogScreen.js");

    const testSetup = await testRender(
      () => (
        <LogScreen
          entries={entries}
          onBack={() => {}}
          onSelect={() => {}}
          onCopy={() => {}}
          onReload={onReload}
          onToggleTail={onToggleTail}
        />
      ),
      { width: 40, height: 16 },
    );

    try {
      await testSetup.renderOnce();

      await testSetup.mockInput.typeText("r");
      await testSetup.renderOnce();
      expect(onReload).toHaveBeenCalledTimes(1);

      await testSetup.mockInput.typeText("t");
      await testSetup.renderOnce();
      expect(onToggleTail).toHaveBeenCalledTimes(1);

      await testSetup.mockInput.typeText("w");
      await testSetup.renderOnce();
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("...");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
