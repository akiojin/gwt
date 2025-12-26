/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, vi } from "vitest";
import { act } from "@testing-library/react";
import { render as inkRender } from "ink-testing-library";
import React from "react";
import { LogListScreen } from "../../../components/screens/LogListScreen.js";
import type { FormattedLogEntry } from "../../../../../logging/formatter.js";

const buildEntry = (
  overrides: Partial<FormattedLogEntry>,
): FormattedLogEntry => ({
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
  ...overrides,
});

describe("LogListScreen", () => {
  it("renders log entries and handles shortcuts", () => {
    const entries: FormattedLogEntry[] = [
      buildEntry({ id: "entry-1" }),
      buildEntry({
        id: "entry-2",
        summary: "[10:01:00] [WARN] [server] warn",
        levelLabel: "WARN",
        category: "server",
        message: "warn",
      }),
    ];

    const onSelect = vi.fn();
    const onBack = vi.fn();
    const onCopy = vi.fn();
    const onPickDate = vi.fn();

    const { stdin, lastFrame } = inkRender(
      <LogListScreen
        entries={entries}
        loading={false}
        error={null}
        onBack={onBack}
        onSelect={onSelect}
        onCopy={onCopy}
        onPickDate={onPickDate}
        selectedDate="2025-12-25"
      />,
    );

    const frame = lastFrame();
    expect(frame).toContain("[10:00:00] [INFO] [cli] hello");
    expect(frame).toContain("[10:01:00] [WARN] [server] warn");

    act(() => {
      stdin.write("\r");
    });
    expect(onSelect).toHaveBeenCalledWith(entries[0]);

    act(() => {
      stdin.write("c");
    });
    expect(onCopy).toHaveBeenCalledWith(entries[0]);

    act(() => {
      stdin.write("d");
    });
    expect(onPickDate).toHaveBeenCalled();

    act(() => {
      stdin.write("q");
    });
    expect(onBack).toHaveBeenCalled();
  });

  it("shows empty message when no logs", () => {
    const { lastFrame } = inkRender(
      <LogListScreen
        entries={[]}
        loading={false}
        error={null}
        onBack={vi.fn()}
        onSelect={vi.fn()}
        onCopy={vi.fn()}
        selectedDate="2025-12-25"
      />,
    );

    expect(lastFrame()).toContain("ログがありません");
  });
});
