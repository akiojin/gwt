import fs from "node:fs";
import path from "node:path";

/**
 * Delete log files older than `keepDays` from the given directory.
 * This is called at startup to enforce 7-day retention without size limits.
 */
export function pruneOldLogs(logDir: string, keepDays = 7): void {
  if (!fs.existsSync(logDir)) return;

  const cutoff = Date.now() - keepDays * 24 * 60 * 60 * 1000;

  for (const entry of fs.readdirSync(logDir)) {
    const full = path.join(logDir, entry);
    try {
      const stat = fs.statSync(full);
      if (!stat.isFile()) continue;
      if (stat.mtime.getTime() < cutoff) {
        fs.unlinkSync(full);
      }
    } catch {
      // Ignore individual file errors to avoid breaking startup
    }
  }
}
