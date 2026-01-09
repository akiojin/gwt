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
  displayJson: string;
}

const LEVEL_LABELS: Record<number, string> = {
  10: "TRACE",
  20: "DEBUG",
  30: "INFO",
  40: "WARN",
  50: "ERROR",
  60: "FATAL",
};

const LOCAL_TIME_FORMATTER = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
});

const TIME_KEYS = new Set([
  "time",
  "timestamp",
  "startedAt",
  "endedAt",
  "updatedAt",
  "createdAt",
  "lastUpdated",
]);

const formatLocalTimeParts = (date: Date): string => {
  const parts = LOCAL_TIME_FORMATTER.formatToParts(date);
  const get = (type: Intl.DateTimeFormatPartTypes) =>
    parts.find((part) => part.type === type)?.value;
  const hour = get("hour");
  const minute = get("minute");
  const second = get("second");

  if (!hour || !minute || !second) {
    return LOCAL_TIME_FORMATTER.format(date);
  }

  return `${hour}:${minute}:${second}`;
};

const pad = (value: number, length = 2) => String(value).padStart(length, "0");

const formatLocalIso = (date: Date): string => {
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

const parseTimestampValue = (value: unknown): Date | null => {
  if (value instanceof Date) {
    return Number.isNaN(value.getTime()) ? null : value;
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    const normalized = value < 1_000_000_000_000 ? value * 1000 : value;
    const date = new Date(normalized);
    return Number.isNaN(date.getTime()) ? null : date;
  }
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (!trimmed) return null;
    if (/^\\d+$/.test(trimmed)) {
      const parsed = Number(trimmed);
      if (Number.isFinite(parsed)) {
        const normalized = parsed < 1_000_000_000_000 ? parsed * 1000 : parsed;
        const date = new Date(normalized);
        return Number.isNaN(date.getTime()) ? null : date;
      }
    }
    const date = new Date(trimmed);
    return Number.isNaN(date.getTime()) ? null : date;
  }
  return null;
};

const formatLogDisplayValue = (value: unknown, key?: string): unknown => {
  if (key && TIME_KEYS.has(key)) {
    const date = parseTimestampValue(value);
    if (date) {
      return formatLocalIso(date);
    }
  }
  if (Array.isArray(value)) {
    return value.map((item) => formatLogDisplayValue(item));
  }
  if (value && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>).map(
      ([entryKey, entryValue]) => [
        entryKey,
        formatLogDisplayValue(entryValue, entryKey),
      ],
    );
    return Object.fromEntries(entries);
  }
  return value;
};

const formatLogDisplayPayload = (
  payload: Record<string, unknown>,
): Record<string, unknown> =>
  Object.fromEntries(
    Object.entries(payload).map(([key, value]) => [
      key,
      formatLogDisplayValue(value, key),
    ]),
  );

const formatTimeLabel = (
  value: unknown,
): { label: string; timestamp: number | null } => {
  if (typeof value === "string" || typeof value === "number") {
    const date = new Date(value);
    if (!Number.isNaN(date.getTime())) {
      return {
        label: formatLocalTimeParts(date),
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
      const displayJson = JSON.stringify(
        formatLogDisplayPayload(parsed),
        null,
        2,
      );
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
        displayJson,
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
