/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import type { FormattedLogEntry } from "../../../../logging/formatter.js";
import { LogDetailScreen } from "../../../screens/solid/LogDetailScreen.js";

const entry: FormattedLogEntry = {
  id: "1",
  raw: { message: "hello" },
  timestamp: Date.now(),
  timeLabel: "12:00:00",
  levelLabel: "INFO",
  category: "test",
  message: "hello",
  summary: "[12:00] hello",
  json: '{\n  "message": "hello"\n}',
};

const renderScreen = async (props: {
  entry: FormattedLogEntry | null;
  onBack?: () => void;
  onCopy?: (entry: FormattedLogEntry) => void;
}) => {
  const testSetup = await testRender(
    () => (
      <LogDetailScreen
        entry={props.entry}
        onBack={props.onBack ?? (() => {})}
        onCopy={props.onCopy ?? (() => {})}
      />
    ),
    { width: 60, height: 6 },
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

describe("Solid LogDetailScreen", () => {
  it("renders entry json", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({ entry });

    try {
      expect(captureCharFrame()).toContain('"message": "hello"');
    } finally {
      cleanup();
    }
  });

  it("handles copy shortcut", async () => {
    const copied: FormattedLogEntry[] = [];
    const { mockInput, renderOnce, cleanup } = await renderScreen({
      entry,
      onCopy: (selected) => copied.push(selected),
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
