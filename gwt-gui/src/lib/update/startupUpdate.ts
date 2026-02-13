import type { UpdateState } from "../types";

export const STARTUP_UPDATE_INITIAL_DELAY_MS = 3000;
export const STARTUP_UPDATE_RETRY_DELAY_MS = 3000;
export const STARTUP_UPDATE_MAX_RETRIES = 3;

type AvailableUpdateState = Extract<UpdateState, { state: "available" }>;

type SleepFn = (ms: number, signal?: AbortSignal) => Promise<void>;

export interface StartupUpdateCheckOptions {
  checkUpdate: () => Promise<UpdateState>;
  onAvailable: (state: AvailableUpdateState) => void;
  signal?: AbortSignal;
  initialDelayMs?: number;
  retryDelayMs?: number;
  maxRetries?: number;
  sleep?: SleepFn;
  warn?: (message: string) => void;
}

function createAbortError(): Error {
  const error = new Error("Aborted");
  error.name = "AbortError";
  return error;
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

function toErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  return String(error);
}

function sleepWithSignal(ms: number, signal?: AbortSignal): Promise<void> {
  if (signal?.aborted) {
    return Promise.reject(createAbortError());
  }

  return new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);

    const onAbort = () => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      reject(createAbortError());
    };

    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

export async function runStartupUpdateCheck(options: StartupUpdateCheckOptions): Promise<void> {
  const initialDelayMs = options.initialDelayMs ?? STARTUP_UPDATE_INITIAL_DELAY_MS;
  const retryDelayMs = options.retryDelayMs ?? STARTUP_UPDATE_RETRY_DELAY_MS;
  const maxRetries = options.maxRetries ?? STARTUP_UPDATE_MAX_RETRIES;
  const sleep = options.sleep ?? sleepWithSignal;
  const warn = options.warn ?? ((message: string) => console.warn(message));

  const totalAttempts = maxRetries + 1;

  try {
    await sleep(initialDelayMs, options.signal);
  } catch (error) {
    if (isAbortError(error) || options.signal?.aborted) return;
    throw error;
  }

  for (let attempt = 1; attempt <= totalAttempts; attempt++) {
    if (options.signal?.aborted) return;

    let failureMessage: string | null = null;
    try {
      const state = await options.checkUpdate();
      if (options.signal?.aborted) return;

      if (state.state === "available") {
        options.onAvailable(state);
        return;
      }
      if (state.state === "up_to_date") {
        return;
      }

      failureMessage = state.message;
    } catch (error) {
      if (isAbortError(error) || options.signal?.aborted) return;
      failureMessage = toErrorMessage(error);
    }

    if (attempt >= totalAttempts) {
      warn(
        `[update] Startup update check failed after ${attempt} attempts: ${failureMessage}`
      );
      return;
    }

    warn(
      `[update] Startup update check failed (attempt ${attempt}/${totalAttempts}): ${failureMessage}. Retrying in ${retryDelayMs}ms.`
    );

    try {
      await sleep(retryDelayMs, options.signal);
    } catch (error) {
      if (isAbortError(error) || options.signal?.aborted) return;
      throw error;
    }
  }
}
