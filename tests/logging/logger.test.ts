import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";
import { createLogger, formatDate } from "../../src/logging/logger.js";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const TMP_DIR = path.join(process.cwd(), ".tmp-log-test");
const TMP_HOME = path.join(process.cwd(), ".tmp-log-home");

function readLastLine(file: string): any {
  const content = fs.readFileSync(file, "utf-8").trim().split("\n");
  const lines = content.filter(Boolean);
  const last = lines[lines.length - 1];
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
    const homeSpy = vi.spyOn(os, "homedir").mockReturnValue(TMP_HOME);

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
    const last = JSON.parse(lines[lines.length - 1]);
    expect(last.msg).toBe("should appear");
    process.env.LOG_LEVEL = "";
  });
});
