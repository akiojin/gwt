/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi } from "vitest";
import { act } from "@testing-library/react";
import { render as inkRender } from "ink-testing-library";
import React from "react";
import { LogDetailScreen } from "../../../components/screens/LogDetailScreen.js";
import type { FormattedLogEntry } from "../../../../../logging/formatter.js";

const entry: FormattedLogEntry = {
  id: "entry-1",
  raw: {
    time: "2025-12-25T10:00:00.000Z",
    level: 30,
    category: "cli",
    msg: "hello",
  },
  timestamp: 1_767_015_200_000,
  timeLabel: "10:00:00",
  levelLabel: "INFO",
  category: "cli",
  message: "hello",
  summary: "[10:00:00] [INFO] [cli] hello",
  json: '{\n  "msg": "hello"\n}',
};

describe("LogDetailScreen", () => {
  it("renders formatted JSON and handles shortcuts", () => {
    const onBack = vi.fn();
    const onCopy = vi.fn();

    const { stdin, lastFrame } = inkRender(
      <LogDetailScreen entry={entry} onBack={onBack} onCopy={onCopy} />,
    );

    expect(lastFrame()).toContain('"msg": "hello"');

    act(() => {
      stdin.write("c");
    });
    expect(onCopy).toHaveBeenCalledWith(entry);

    act(() => {
      stdin.write("q");
    });
    expect(onBack).toHaveBeenCalled();
  });

  it("shows fallback when entry is missing", () => {
    const { lastFrame } = inkRender(
      <LogDetailScreen entry={null} onBack={vi.fn()} onCopy={vi.fn()} />,
    );

    expect(lastFrame()).toContain("No logs available.");
  });
});
