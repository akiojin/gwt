export interface FormattedLogEntry {
  id: string;
  raw: Record<string, unknown>;
  timestamp: number | null;
  timeLabel: string;
  levelLabel: string;
  category: string;
  message: string;
  summary: string;
  json: string;
}

const LEVEL_LABELS: Record<number, string> = {
  10: "TRACE",
  20: "DEBUG",
  30: "INFO",
  40: "WARN",
  50: "ERROR",
  60: "FATAL",
};

const formatTimeLabel = (
  value: unknown,
): { label: string; timestamp: number | null } => {
  if (typeof value === "string" || typeof value === "number") {
    const date = new Date(value);
    if (!Number.isNaN(date.getTime())) {
      const hours = String(date.getHours()).padStart(2, "0");
      const minutes = String(date.getMinutes()).padStart(2, "0");
      const seconds = String(date.getSeconds()).padStart(2, "0");
      return {
        label: `${hours}:${minutes}:${seconds}`,
        timestamp: date.getTime(),
      };
    }
  }

  return { label: "--:--:--", timestamp: null };
};

const formatLevelLabel = (value: unknown): string => {
  if (typeof value === "number") {
    return LEVEL_LABELS[value] ?? `LEVEL-${value}`;
  }
  if (typeof value === "string") {
    return value.toUpperCase();
  }
  return "UNKNOWN";
};

const resolveMessage = (entry: Record<string, unknown>): string => {
  if (typeof entry.msg === "string") {
    return entry.msg;
  }
  if (typeof entry.message === "string") {
    return entry.message;
  }
  if (entry.msg !== undefined) {
    return String(entry.msg);
  }
  return "";
};

export function parseLogLines(
  lines: string[],
  options: { limit?: number } = {},
): FormattedLogEntry[] {
  const entries: FormattedLogEntry[] = [];

  lines.forEach((line, index) => {
    if (!line.trim()) return;
    try {
      const parsed = JSON.parse(line) as Record<string, unknown>;
      const { label: timeLabel, timestamp } = formatTimeLabel(parsed.time);
      const levelLabel = formatLevelLabel(parsed.level);
      const category =
        typeof parsed.category === "string" ? parsed.category : "unknown";
      const message = resolveMessage(parsed);
      const summary =
        `[${timeLabel}] [${levelLabel}] [${category}] ${message}`.trim();
      const json = JSON.stringify(parsed, null, 2);
      const id = `${timestamp ?? "unknown"}-${index}`;

      entries.push({
        id,
        raw: parsed,
        timestamp,
        timeLabel,
        levelLabel,
        category,
        message,
        summary,
        json,
      });
    } catch {
      // Skip malformed JSON lines
    }
  });

  const limit = options.limit ?? 100;
  if (entries.length <= limit) {
    return [...entries].reverse();
  }

  return entries.slice(-limit).reverse();
}
