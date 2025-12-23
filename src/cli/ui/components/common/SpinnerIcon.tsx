import React, { useEffect, useState, useRef } from "react";
import { Text } from "ink";

const SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

export interface SpinnerIconProps {
  /** true にするとスピナーを回転させる */
  isSpinning: boolean;
  /** スピナーの色 */
  color?: "cyan" | "green" | "yellow" | "red";
  /** スピナーの更新間隔 (ms)。デフォルトは 120ms */
  interval?: number;
}

/**
 * インラインで使用できるスピナーアイコンコンポーネント。
 * 自身の内部で状態を管理するため、親コンポーネントの再レンダリングを引き起こさない。
 */
export function SpinnerIcon({
  isSpinning,
  color = "cyan",
  interval = 120,
}: SpinnerIconProps) {
  const [frameIndex, setFrameIndex] = useState(0);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (isSpinning) {
      intervalRef.current = setInterval(() => {
        setFrameIndex((current) => (current + 1) % SPINNER_FRAMES.length);
      }, interval);
    } else {
      setFrameIndex(0);
    }

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [isSpinning, interval]);

  if (!isSpinning) {
    return null;
  }

  const frame = SPINNER_FRAMES[frameIndex] ?? SPINNER_FRAMES[0];

  return <Text color={color}>{frame}</Text>;
}

/**
 * スピナーを文字列として取得するためのフック。
 * Textコンポーネントでラップせず、文字列として使用したい場合に使用。
 */
export function useSpinnerFrame(
  isSpinning: boolean,
  interval = 120,
): string | null {
  const [frameIndex, setFrameIndex] = useState(0);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (isSpinning) {
      intervalRef.current = setInterval(() => {
        setFrameIndex((current) => (current + 1) % SPINNER_FRAMES.length);
      }, interval);
    } else {
      setFrameIndex(0);
    }

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [isSpinning, interval]);

  if (!isSpinning) {
    return null;
  }

  return SPINNER_FRAMES[frameIndex] ?? SPINNER_FRAMES[0] ?? null;
}
