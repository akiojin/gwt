import { createSignal } from "solid-js";
import { useSelectionHandler } from "@opentui/solid";
import type { Selection } from "@opentui/core";
import { copyToClipboard } from "../../utils/clipboard.js";

export interface UseTextSelectionOptions {
  /** Callback when text is copied to clipboard */
  onCopy?: (text: string) => void;
  /** Callback when copy fails */
  onCopyError?: (error: Error) => void;
  /** Whether to automatically copy on selection end */
  autoCopy?: boolean;
}

export interface UseTextSelectionResult {
  /** Whether a selection is currently active */
  isSelecting: () => boolean;
  /** The currently selected text */
  selectedText: () => string;
  /** Manually copy current selection to clipboard */
  copy: () => Promise<void>;
}

/**
 * Hook for handling mouse text selection and clipboard copy.
 * Uses OpenTUI's useSelectionHandler to track selections.
 */
export function useTextSelection(
  options: UseTextSelectionOptions = {},
): UseTextSelectionResult {
  const { onCopy, onCopyError, autoCopy = true } = options;

  const [isSelecting, setIsSelecting] = createSignal(false);
  const [selectedText, setSelectedText] = createSignal("");

  const copyText = async (text: string): Promise<void> => {
    if (!text) return;

    try {
      await copyToClipboard(text);
      onCopy?.(text);
    } catch (error) {
      const err = error instanceof Error ? error : new Error(String(error));
      onCopyError?.(err);
    }
  };

  useSelectionHandler((selection: Selection) => {
    if (selection.isSelecting) {
      setIsSelecting(true);
      const text = selection.getSelectedText();
      setSelectedText(text);
    } else if (selection.isActive) {
      // Selection just ended
      setIsSelecting(false);
      const text = selection.getSelectedText();
      setSelectedText(text);

      if (autoCopy && text) {
        void copyText(text);
      }
    } else {
      // Selection cleared
      setIsSelecting(false);
      setSelectedText("");
    }
  });

  const copy = async (): Promise<void> => {
    const text = selectedText();
    if (text) {
      await copyText(text);
    }
  };

  return {
    isSelecting,
    selectedText,
    copy,
  };
}
