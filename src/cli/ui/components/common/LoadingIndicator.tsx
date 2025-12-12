import React, { useEffect, useMemo, useRef, useState } from "react";
import { Box, Text } from "ink";

export interface LoadingIndicatorProps {
  /** true にするとローディング表示を開始する */
  isLoading: boolean;
  /** 表示までの遅延時間 (ms)。デフォルトは 300ms */
  delay?: number;
  /** 表示するメッセージ */
  message?: string;
  /** スピナーの更新間隔 (ms)。デフォルトは 80ms */
  interval?: number;
  /** 使用するスピナーフレーム。ASCII のみを想定 */
  frames?: string[];
}

const DEFAULT_FRAMES = ["|", "/", "-", "\\"];

/**
 * ローディング中に簡易スピナーとメッセージを表示するコンポーネント。
 * delay で指定した時間を超えるまでスピナーを表示しないことで、短時間の処理ではちらつきを抑える。
 */
export function LoadingIndicator({
  isLoading,
  delay = 300,
  message = "Loading... please wait",
  interval = 80,
  frames = DEFAULT_FRAMES,
}: LoadingIndicatorProps) {
  const [visible, setVisible] = useState(false);
  const [frameIndex, setFrameIndex] = useState(0);
  const delayTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // スピナーに使用するフレームをキャッシュ
  const safeFrames = useMemo(
    () => (frames.length > 0 ? frames : DEFAULT_FRAMES),
    [frames],
  );

  useEffect(() => {
    // ローディングが開始したら、delay後に表示を有効化
    if (isLoading) {
      delayTimerRef.current = setTimeout(() => {
        setVisible(true);
      }, delay);
    } else {
      setVisible(false);
      setFrameIndex(0);
    }

    return () => {
      if (delayTimerRef.current) {
        clearTimeout(delayTimerRef.current);
        delayTimerRef.current = null;
      }
    };
  }, [isLoading, delay]);

  useEffect(() => {
    // 表示中のみスピナーを回転
    if (visible && isLoading) {
      intervalRef.current = setInterval(() => {
        setFrameIndex((current) => (current + 1) % safeFrames.length);
      }, interval);
    }

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [visible, isLoading, interval, safeFrames.length]);

  // ローディングが解消されたらタイマーをクリア
  useEffect(() => {
    if (!isLoading && delayTimerRef.current) {
      clearTimeout(delayTimerRef.current);
      delayTimerRef.current = null;
    }
  }, [isLoading]);

  if (!isLoading || !visible) {
    return null;
  }

  return (
    <Box gap={1}>
      <Text color="yellow" data-testid="loading-indicator-frame">
        {safeFrames[frameIndex]}
      </Text>
      <Text color="yellow" data-testid="loading-indicator-message">
        {message}
      </Text>
    </Box>
  );
}
