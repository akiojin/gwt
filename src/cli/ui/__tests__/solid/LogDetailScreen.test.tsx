/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";

const makeEntry = (
  overrides: Partial<FormattedLogEntry> = {},
): FormattedLogEntry => ({
  id: "1",
  raw: { level: 30, time: "2026-01-08T00:00:00.000Z", category: "cli" },
  timestamp: 0,
  timeLabel: "00:00:00",
  levelLabel: "INFO",
  category: "cli",
  message: "hello",
  summary: "[00:00:00] [INFO] [cli] hello",
  json: JSON.stringify({ time: "2026-01-08T00:00:00.000Z" }, null, 2),
  displayJson: JSON.stringify({ time: "LOCAL" }, null, 2),
  ...overrides,
});

describe("LogDetailScreen", () => {
  it("prefers displayJson when present", async () => {
    const { LogDetailScreen } =
      await import("../../screens/solid/LogDetailScreen.js");

    const testSetup = await testRender(
      () => (
        <LogDetailScreen
          entry={makeEntry()}
          onBack={() => {}}
          onCopy={() => {}}
        />
      ),
      { width: 80, height: 12 },
    );

    try {
      await testSetup.renderOnce();
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("LOCAL");
      expect(frame).not.toContain("2026-01-08T00:00:00.000Z");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
