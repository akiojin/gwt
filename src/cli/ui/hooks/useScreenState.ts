import { useState, useCallback } from "react";
import type { ScreenType } from "../types.js";

export interface ScreenStateResult {
  currentScreen: ScreenType;
  navigateTo: (screen: ScreenType) => void;
  goBack: () => void;
  reset: () => void;
}

const INITIAL_SCREEN: ScreenType = "branch-list";

/**
 * Hook to manage screen navigation state with history
 */
export function useScreenState(): ScreenStateResult {
  const [history, setHistory] = useState<ScreenType[]>([INITIAL_SCREEN]);

  const currentScreen = history[history.length - 1] ?? INITIAL_SCREEN;

  const navigateTo = useCallback((screen: ScreenType) => {
    setHistory((prev) => [...prev, screen]);
  }, []);

  const goBack = useCallback(() => {
    setHistory((prev) => {
      if (prev.length <= 1) {
        return prev; // Stay at initial screen
      }
      return prev.slice(0, -1);
    });
  }, []);

  const reset = useCallback(() => {
    setHistory([INITIAL_SCREEN]);
  }, []);

  return {
    currentScreen,
    navigateTo,
    goBack,
    reset,
  };
}
