import { describe, it, expect, beforeAll, afterAll, spyOn } from "bun:test";
import { createLogger, formatDate } from "../../src/logging/logger.js";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const TMP_DIR = path.join(process.cwd(), ".tmp-log-test");
const TMP_HOME = path.join(process.cwd(), ".tmp-log-home");

function readLastLine(file: string): Record<string, unknown> {
  const content = fs.readFileSync(file, "utf-8").trim().split("\n");
  const lines = content.filter(Boolean);
  const last = lines[lines.length - 1];
  if (!last) {
    throw new Error("Expected log line");
  }
  return JSON.parse(last);
}

describe("createLogger", () => {
  beforeAll(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
    fs.mkdirSync(TMP_DIR, { recursive: true });
  });

  afterAll(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
  });

  it("writes to default path ~/.gwt/logs/<cwd>/<YYYY-MM-DD>.jsonl", () => {
    fs.rmSync(TMP_HOME, { recursive: true, force: true });
    fs.mkdirSync(TMP_HOME, { recursive: true });
    const homeSpy = spyOn(os, "homedir").mockReturnValue(TMP_HOME);

    const today = formatDate(new Date());
    const expectedDir = path.join(
      TMP_HOME,
      ".gwt",
      "logs",
      path.basename(process.cwd()),
    );
    const expectedFile = path.join(expectedDir, `${today}.jsonl`);

    const logger = createLogger({
      category: "cli",
      level: "info",
      sync: true,
    });

    logger.info("hello default");
    logger.flush?.();

    expect(fs.existsSync(expectedFile)).toBe(true);

    homeSpy.mockRestore();
    fs.rmSync(TMP_HOME, { recursive: true, force: true });
  });

  it("writes JSON with required fields and category", () => {
    const logfile = path.join(TMP_DIR, "app.log");
    const logger = createLogger({
      logDir: TMP_DIR,
      filename: "app.log",
      category: "cli",
      level: "info",
      keepDays: 7,
      sync: true,
    });

    logger.info("hello world");
    logger.flush?.(); // noop if not supported

    // pino transport is async; wait briefly
    expect(fs.existsSync(logfile)).toBe(true);
    const record = readLastLine(logfile);
    expect(record).toHaveProperty("time");
    expect(record).toHaveProperty("level");
    expect(record).toHaveProperty("msg");
    expect(record).toHaveProperty("category", "cli");
  });

  it("respects LOG_LEVEL override", () => {
    process.env.LOG_LEVEL = "error";
    const logfile = path.join(TMP_DIR, "level.log");
    const logger = createLogger({
      logDir: TMP_DIR,
      filename: "level.log",
      category: "cli",
      sync: true,
    });

    logger.info("should not appear");
    logger.error("should appear");

    const lines = fs.readFileSync(logfile, "utf-8").trim().split("\n");
    const lastLine = lines[lines.length - 1];
    if (!lastLine) {
      throw new Error("Expected log line");
    }
    const last = JSON.parse(lastLine);
    expect(last.msg).toBe("should appear");
    delete process.env.LOG_LEVEL;
  });

  it("appends to existing log files instead of truncating", () => {
    const logfile = path.join(TMP_DIR, "append.log");
    fs.writeFileSync(logfile, "seed\n");

    const logger = createLogger({
      logDir: TMP_DIR,
      filename: "append.log",
      category: "cli",
      sync: true,
    });

    logger.info("appended");
    logger.flush?.();

    const content = fs.readFileSync(logfile, "utf-8");
    expect(content.startsWith("seed\n")).toBe(true);
    const lines = content.trim().split("\n");
    expect(lines.length).toBeGreaterThanOrEqual(2);
  });

  it("multiple logger instances can write to same log file without corruption", () => {
    const logfile = path.join(TMP_DIR, "multilogger.log");
    fs.writeFileSync(logfile, ""); // Clear file

    const LOGGER_COUNT = 3;
    const LOGS_PER_LOGGER = 10;

    // Create multiple logger instances pointing to same file
    const loggers = Array.from({ length: LOGGER_COUNT }, (_, i) =>
      createLogger({
        logDir: TMP_DIR,
        filename: "multilogger.log",
        category: `logger-${i}`,
        sync: true,
      }),
    );

    // Write logs from all loggers (simulating concurrent writes)
    for (let i = 0; i < LOGS_PER_LOGGER; i++) {
      for (let j = 0; j < LOGGER_COUNT; j++) {
        const logger = loggers[j];
        if (!logger) {
          throw new Error("Expected logger instance");
        }
        logger.info({ loggerId: j, index: i }, `Message ${i} from logger ${j}`);
      }
    }

    // Verify log file integrity
    const content = fs.readFileSync(logfile, "utf-8").trim();
    const lines = content.split("\n").filter(Boolean);

    // Should have expected number of log entries
    expect(lines.length).toBe(LOGGER_COUNT * LOGS_PER_LOGGER);

    // Each line should be valid JSON with correct structure
    let validJsonCount = 0;
    for (const line of lines) {
      try {
        const parsed = JSON.parse(line);
        expect(parsed).toHaveProperty("msg");
        expect(parsed).toHaveProperty("category");
        expect(parsed).toHaveProperty("loggerId");
        expect(parsed).toHaveProperty("index");
        expect(parsed).toHaveProperty("time");
        expect(parsed).toHaveProperty("level");
        validJsonCount++;
      } catch {
        // Log corruption detected - fail the test
        throw new Error(`Invalid JSON line: ${line}`);
      }
    }

    expect(validJsonCount).toBe(LOGGER_COUNT * LOGS_PER_LOGGER);
  });
});
