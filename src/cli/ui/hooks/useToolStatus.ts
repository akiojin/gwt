/**
 * AIツール状態管理フック
 *
 * 各AIツール（Claude Code、Codex、Gemini）のインストール状態を
 * gwt起動時に検出し、キャッシュして提供します。
 * @see specs/SPEC-3b0ed29b/spec.md FR-017〜FR-021
 */

import { useState, useEffect, useCallback } from "react";
import {
  detectAllToolStatuses,
  type ToolStatus,
} from "../../../utils/command.js";

export interface UseToolStatusResult {
  /** ツール状態の配列（ロード中は空配列） */
  tools: ToolStatus[];
  /** ロード中フラグ */
  loading: boolean;
  /** エラー（なければnull） */
  error: Error | null;
  /** ツール状態を再検出（通常は不要、デバッグ用） */
  refresh: () => Promise<void>;
}

/**
 * AIツール状態管理フック
 *
 * コンポーネントでAIツールのインストール状態を取得するためのフック。
 * 初回マウント時に自動的に全ツールの状態を検出してキャッシュします。
 *
 * キャッシュされた結果は、ブランチ選択時やツール起動時に再利用され、
 * 毎回の検出オーバーヘッドを削減します（FR-020）。
 */
export function useToolStatus(): UseToolStatusResult {
  const [tools, setTools] = useState<ToolStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  // ツール状態を検出
  const refresh = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const statuses = await detectAllToolStatuses();
      setTools(statuses);
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
    } finally {
      setLoading(false);
    }
  }, []);

  // 初回マウント時に検出（FR-017: 起動時に検出してキャッシュ）
  useEffect(() => {
    refresh();
  }, [refresh]);

  return {
    tools,
    loading,
    error,
    refresh,
  };
}

// Re-export ToolStatus type for convenience
export type { ToolStatus };
