import { createLogger } from "../../src/logging/logger.js";
import fs from "node:fs";
import path from "node:path";

const TMP_DIR = path.join(process.cwd(), ".tmp-log-test");

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
    const fileCheck = () => {
      if (!fs.existsSync(logfile)) return false;
      const lines = fs.readFileSync(logfile, "utf-8").trim().split("\n").filter(Boolean);
      return lines.length > 0;
    };
    let attempts = 0;
    while (!fileCheck() && attempts < 20) {
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10);
      attempts += 1;
    }

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

    const fileCheck = () => {
      if (!fs.existsSync(logfile)) return false;
      const lines = fs.readFileSync(logfile, "utf-8").trim().split("\n").filter(Boolean);
      return lines.length > 0;
    };
    let attempts = 0;
    while (!fileCheck() && attempts < 20) {
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10);
      attempts += 1;
    }

    const lines = fs.readFileSync(logfile, "utf-8").trim().split("\n");
    const last = JSON.parse(lines[lines.length - 1]);
    expect(last.msg).toBe("should appear");
    process.env.LOG_LEVEL = "";
  });
});
