import fs from "node:fs";
import path from "node:path";
import { pruneOldLogs } from "../../src/logging/rotation.js";

const TMP_DIR = path.join(process.cwd(), ".tmp-rotation-test");

describe("pruneOldLogs", () => {
  beforeAll(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
    fs.mkdirSync(TMP_DIR, { recursive: true });
  });

  afterAll(() => {
    fs.rmSync(TMP_DIR, { recursive: true, force: true });
  });

  it("deletes files older than keepDays on startup", () => {
    const oldFile = path.join(TMP_DIR, "old.log");
    const newFile = path.join(TMP_DIR, "new.log");

    fs.writeFileSync(oldFile, "old");
    fs.writeFileSync(newFile, "new");

    // Backdate old file to 8 days ago
    const eightDaysMs = 8 * 24 * 60 * 60 * 1000;
    const past = Date.now() - eightDaysMs;
    fs.utimesSync(oldFile, past / 1000, past / 1000);

    pruneOldLogs(TMP_DIR, 7);

    expect(fs.existsSync(oldFile)).toBe(false);
    expect(fs.existsSync(newFile)).toBe(true);
  });
});
