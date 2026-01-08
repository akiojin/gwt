/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { createSignal } from "solid-js";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";

const makeEntry = (message: string): FormattedLogEntry => ({
  id: "1",
  raw: { level: 30, time: "2026-01-08T00:00:00.000Z", category: "cli" },
  timestamp: 0,
  timeLabel: "00:00:00",
  levelLabel: "INFO",
  category: "cli",
  message,
  summary: `[00:00:00] [INFO] [cli] ${message}`,
  json: JSON.stringify({ message }, null, 2),
});

describe("LogScreen", () => {
  it("updates entries when data changes", async () => {
    const [entries, setEntries] = createSignal<FormattedLogEntry[]>([]);

    const { LogScreen } = await import("../../screens/solid/LogScreen.js");

    const testSetup = await testRender(
      () => (
        <LogScreen
          entries={entries()}
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
      expect(frame).toContain("No logs available.");

      setEntries([makeEntry("Hello")]);
      await testSetup.renderOnce();
      frame = testSetup.captureCharFrame();
      expect(frame).toContain("[00:00:00] [INFO] [cli] Hello");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
