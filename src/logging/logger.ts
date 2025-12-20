import fs from "node:fs";
import path from "node:path";
import os from "node:os";
import pino, { type LoggerOptions, type Logger } from "pino";
import { pruneOldLogs } from "./rotation.js";

type Category = "cli" | "server" | "worker" | string;

export interface LoggerConfig {
  level?: string;
  logDir?: string;
  filename?: string;
  category?: Category;
  base?: Record<string, unknown>;
  keepDays?: number;
  /** For tests or sync writes use pino.destination sync */
  sync?: boolean;
}

/**
 * Create a pino logger with unified structure and category field.
 * - Writes to a single file stream (no per-component files)
 * - Adds `category` to each log record
 * - Prunes files older than keepDays at startup
 */
export function createLogger(config: LoggerConfig = {}): Logger {
  const level = config.level ?? process.env.LOG_LEVEL ?? "info";
  const cwdBase = path.basename(process.cwd()) || "workspace";
  const defaultLogDir = path.join(os.homedir(), ".gwt", "logs", cwdBase);
  const logDir = config.logDir ?? defaultLogDir;
  const filename = config.filename ?? `${formatDate(new Date())}.jsonl`;
  const category = config.category ?? "default";
  const keepDays = config.keepDays ?? 7;

  if (!fs.existsSync(logDir)) {
    fs.mkdirSync(logDir, { recursive: true });
  }

  // Startup rotation
  pruneOldLogs(logDir, keepDays);

  const destination = path.join(logDir, filename);

  const options: LoggerOptions = {
    level,
    base: {
      category,
      ...(config.base ?? {}),
    },
    timestamp: pino.stdTimeFunctions.isoTime,
  };

  if (config.sync) {
    const destinationStream = pino.destination({
      dest: destination,
      sync: true,
    });
    return pino(options, destinationStream);
  }

  const transport = pino.transport({
    targets: [
      {
        target: "pino/file",
        options: { destination, mkdir: true, append: true },
        level,
      },
    ],
  });

  return pino(options, transport);
}

/** Convenience logger for quick use (category defaults to "default"). */
export const logger = createLogger();

export function formatDate(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}
