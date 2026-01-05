import { createEffect, createMemo, createSignal } from "solid-js";

export interface LoadingIndicatorScreenProps {
  isLoading?: boolean;
  message?: string;
  delay?: number;
  interval?: number;
  frames?: string[];
}

const DEFAULT_FRAMES = ["|", "/", "-", "\\"];

export function LoadingIndicatorScreen({
  isLoading = true,
  message = "Loading... please wait",
  delay = 300,
  interval = 80,
  frames = DEFAULT_FRAMES,
}: LoadingIndicatorScreenProps) {
  const [visible, setVisible] = createSignal(isLoading && delay <= 0);
  const [frameIndex, setFrameIndex] = createSignal(0);

  const safeFrames = createMemo(() =>
    frames.length > 0 ? frames : DEFAULT_FRAMES,
  );

  createEffect(() => {
    if (!isLoading) {
      setVisible(false);
      setFrameIndex(0);
      return;
    }

    if (delay <= 0) {
      setVisible(true);
      return;
    }

    const timer = setTimeout(() => {
      setVisible(true);
    }, delay);

    return () => {
      clearTimeout(timer);
    };
  });

  createEffect(() => {
    if (!visible() || !isLoading) {
      return;
    }

    const timer = setInterval(() => {
      setFrameIndex((current) => (current + 1) % safeFrames().length);
    }, interval);

    return () => {
      clearInterval(timer);
    };
  });

  const frame = createMemo(
    () => safeFrames()[frameIndex()] ?? safeFrames()[0] ?? DEFAULT_FRAMES[0],
  );

  if (!isLoading || !visible()) {
    return null;
  }

  return (
    <box flexDirection="row">
      <text fg="yellow">{frame()}</text>
      <text fg="yellow">{` ${message}`}</text>
    </box>
  );
}
