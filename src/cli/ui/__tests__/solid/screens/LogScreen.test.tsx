/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";
import { LogScreen } from "../../../screens/solid/LogScreen.js";

const createEntry = (
  id: string,
  summary: string,
  json = '{\n  "message": "ok"\n}',
): FormattedLogEntry => ({
  id,
  raw: { message: summary },
  timestamp: Date.now(),
  timeLabel: "12:00:00",
  levelLabel: "INFO",
  category: "test",
  message: summary,
  summary,
  json,
});

const renderScreen = async (props: {
  entries: FormattedLogEntry[];
  onBack?: () => void;
  onSelect?: (entry: FormattedLogEntry) => void;
  onCopy?: (entry: FormattedLogEntry) => void;
  selectedDate?: string | null;
}) => {
  const testSetup = await testRender(
    () => (
      <LogScreen
        entries={props.entries}
        onBack={props.onBack ?? (() => {})}
        onSelect={props.onSelect ?? (() => {})}
        onCopy={props.onCopy ?? (() => {})}
        selectedDate={props.selectedDate}
      />
    ),
    { width: 60, height: 10 },
  );

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    cleanup,
  };
};

describe("Solid LogScreen", () => {
  it("renders summary and totals", async () => {
    const entries = [createEntry("1", "[12:00] log A")];
    const { captureCharFrame, cleanup } = await renderScreen({
      entries,
      selectedDate: "2026-01-05",
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("gwt - Log Viewer");
      expect(frame).toContain("Total: 1");
      expect(frame).toContain("2026-01-05");
      expect(frame).toContain("log A");
    } finally {
      cleanup();
    }
  });

  it("handles copy shortcut", async () => {
    const entries = [createEntry("1", "log A")];
    const copied: FormattedLogEntry[] = [];
    const { mockInput, renderOnce, cleanup } = await renderScreen({
      entries,
      onCopy: (entry) => copied.push(entry),
    });

    try {
      mockInput.pressKey("c");
      await renderOnce();
      expect(copied).toHaveLength(1);
      expect(copied[0]?.id).toBe("1");
    } finally {
      cleanup();
    }
  });
});
