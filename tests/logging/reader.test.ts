import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import fs from "node:fs";
import path from "node:path";
import { readLogLinesForDate } from "../../src/logging/reader.js";

const TMP_DIR = path.join(process.cwd(), ".tmp-log-reader");

const writeLogFile = (date: string, lines: string[]): string => {
  const filePath = path.join(TMP_DIR, `${date}.jsonl`);
  const content = lines.length ? `${lines.join("\n")}\n` : "";
  fs.writeFileSync(filePath, content);
  return filePath;
};

const sampleLine = (message: string) =>
  JSON.stringify({
    level: 30,
    time: "2026-01-08T00:00:00.000Z",
    category: "cli",
    message,
  });

describe("readLogLinesForDate", () => {
  beforeEach(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
    fs.mkdirSync(TMP_DIR, { recursive: true });
  });

  afterEach(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
  });

  it("returns the preferred date file when it has content", async () => {
    const line = sampleLine("preferred");
    writeLogFile("2026-01-08", [line]);

    const result = await readLogLinesForDate(TMP_DIR, "2026-01-08");

    expect(result?.date).toBe("2026-01-08");
    expect(result?.lines).toEqual([line]);
  });

  it("falls back to the latest available log file when preferred is missing", async () => {
    const latest = sampleLine("latest");
    const older = sampleLine("older");
    writeLogFile("2026-01-07", [latest]);
    writeLogFile("2026-01-05", [older]);

    const result = await readLogLinesForDate(TMP_DIR, "2026-01-08");

    expect(result?.date).toBe("2026-01-07");
    expect(result?.lines).toEqual([latest]);
  });

  it("falls back to the latest log file with content when preferred is empty", async () => {
    const fallback = sampleLine("fallback");
    writeLogFile("2026-01-08", []);
    writeLogFile("2026-01-07", [fallback]);

    const result = await readLogLinesForDate(TMP_DIR, "2026-01-08");

    expect(result?.date).toBe("2026-01-07");
    expect(result?.lines).toEqual([fallback]);
  });
});
