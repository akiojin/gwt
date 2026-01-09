import { describe, it, expect } from "bun:test";
import { parseLogLines } from "../../src/logging/formatter.js";

const pad = (value: number, length = 2) => String(value).padStart(length, "0");

const formatLocalIso = (value: string): string => {
  const date = new Date(value);
  const year = date.getFullYear();
  const month = pad(date.getMonth() + 1);
  const day = pad(date.getDate());
  const hour = pad(date.getHours());
  const minute = pad(date.getMinutes());
  const second = pad(date.getSeconds());
  const millisecond = pad(date.getMilliseconds(), 3);
  const offsetMinutes = -date.getTimezoneOffset();
  const sign = offsetMinutes >= 0 ? "+" : "-";
  const offsetAbs = Math.abs(offsetMinutes);
  const offsetHour = pad(Math.floor(offsetAbs / 60));
  const offsetMinute = pad(offsetAbs % 60);
  return `${year}-${month}-${day}T${hour}:${minute}:${second}.${millisecond}${sign}${offsetHour}:${offsetMinute}`;
};

describe("parseLogLines", () => {
  it("adds displayJson with local time conversion", () => {
    const sourceTime = "2026-01-08T00:00:00.000Z";
    const line = JSON.stringify({
      level: 30,
      time: sourceTime,
      category: "cli",
      message: "hello",
    });

    const [entry] = parseLogLines([line]);

    const expectedTime = formatLocalIso(sourceTime);
    expect(entry.json).toContain(`"time": "${sourceTime}"`);
    expect(entry.displayJson).toContain(`"time": "${expectedTime}"`);
  });
});
