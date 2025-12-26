/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import { renderHook, act, waitFor } from "@testing-library/react";
import { Window } from "happy-dom";

// モジュールをモック
vi.mock("../../../../git.js", () => ({
  getAllBranches: vi.fn(),
  hasUnpushedCommitsInRepo: vi.fn(),
  getRepositoryRoot: vi.fn(),
  fetchAllRemotes: vi.fn(),
  collectUpstreamMap: vi.fn(),
  getBranchDivergenceStatuses: vi.fn(),
  hasUncommittedChanges: vi.fn(),
}));

vi.mock("../../../../worktree.js", () => ({
  listAdditionalWorktrees: vi.fn(),
}));

vi.mock("../../../../github.js", () => ({
  getPullRequestByBranch: vi.fn(),
}));

vi.mock("../../../../config/index.js", () => ({
  getLastToolUsageMap: vi.fn(),
}));

import { useGitData } from "../../hooks/useGitData.js";
import {
  getAllBranches,
  getRepositoryRoot,
  fetchAllRemotes,
  collectUpstreamMap,
  getBranchDivergenceStatuses,
} from "../../../../git.js";
import { listAdditionalWorktrees } from "../../../../worktree.js";
import { getLastToolUsageMap } from "../../../../config/index.js";

const mockGetAllBranches = getAllBranches as ReturnType<typeof vi.fn>;
const mockGetRepositoryRoot = getRepositoryRoot as ReturnType<typeof vi.fn>;
const mockFetchAllRemotes = fetchAllRemotes as ReturnType<typeof vi.fn>;
const mockCollectUpstreamMap = collectUpstreamMap as ReturnType<typeof vi.fn>;
const mockGetBranchDivergenceStatuses =
  getBranchDivergenceStatuses as ReturnType<typeof vi.fn>;
const mockListAdditionalWorktrees = listAdditionalWorktrees as ReturnType<
  typeof vi.fn
>;
const mockGetLastToolUsageMap = getLastToolUsageMap as ReturnType<typeof vi.fn>;

describe("useGitData", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;

    // Reset all mocks
    vi.clearAllMocks();

    // Default mock implementations
    mockGetRepositoryRoot.mockResolvedValue("/mock/repo");
    mockFetchAllRemotes.mockResolvedValue(undefined);
    mockGetAllBranches.mockResolvedValue([
      {
        name: "main",
        type: "local",
        isCurrent: true,
        lastCommitDate: "2025-01-01",
      },
      {
        name: "develop",
        type: "local",
        isCurrent: false,
        lastCommitDate: "2025-01-02",
      },
    ]);
    mockListAdditionalWorktrees.mockResolvedValue([]);
    mockCollectUpstreamMap.mockResolvedValue(new Map());
    mockGetBranchDivergenceStatuses.mockResolvedValue([]);
    mockGetLastToolUsageMap.mockResolvedValue(new Map());
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("キャッシュ機構", () => {
    it("初回マウント時にGitデータを取得する", async () => {
      const { result } = renderHook(() => useGitData());

      // 初回ロード中
      expect(result.current.loading).toBe(true);

      await waitFor(() => {
        expect(result.current.loading).toBe(false);
      });

      // Gitデータが取得されていることを確認
      expect(mockGetAllBranches).toHaveBeenCalledTimes(1);
      expect(result.current.branches).toHaveLength(2);
    });

    it("refresh()を呼び出すとGitデータを再取得する", async () => {
      const { result } = renderHook(() => useGitData());

      await waitFor(() => {
        expect(result.current.loading).toBe(false);
      });

      // 初回ロードで1回呼ばれている
      expect(mockGetAllBranches).toHaveBeenCalledTimes(1);

      // refresh()を呼び出す
      await act(async () => {
        result.current.refresh();
      });

      await waitFor(() => {
        expect(result.current.loading).toBe(false);
      });

      // refresh後に再度呼ばれている（forceRefresh=true）
      expect(mockGetAllBranches).toHaveBeenCalledTimes(2);
    });

    it("キャッシュ済みの場合、再マウント時にGitデータを再取得しない", async () => {
      // 注: useGitData は内部で useRef を使ってキャッシュ状態を管理
      // 同一コンポーネント内での再レンダリングではキャッシュが効く
      const { result, rerender } = renderHook(() => useGitData());

      await waitFor(() => {
        expect(result.current.loading).toBe(false);
      });

      // 初回ロードで1回呼ばれている
      expect(mockGetAllBranches).toHaveBeenCalledTimes(1);

      // 再レンダリング
      rerender();

      // キャッシュされているため追加の呼び出しはない
      expect(mockGetAllBranches).toHaveBeenCalledTimes(1);
    });
  });

  describe("lastUpdated", () => {
    it("データ取得成功後にlastUpdatedが更新される", async () => {
      const { result } = renderHook(() => useGitData());

      // 初期状態ではnull
      expect(result.current.lastUpdated).toBeNull();

      await waitFor(() => {
        expect(result.current.loading).toBe(false);
      });

      // ロード完了後にlastUpdatedが設定される
      expect(result.current.lastUpdated).toBeInstanceOf(Date);
    });
  });

  describe("エラーハンドリング", () => {
    it("getAllBranchesがエラーを投げた場合、フォールバック値（空配列）が使用される", async () => {
      // withTimeout がエラーをキャッチしてフォールバック値を返すため、
      // エラー状態にはならず、空配列が設定される
      mockGetAllBranches.mockRejectedValue(new Error("Git error"));

      const { result } = renderHook(() => useGitData());

      await waitFor(() => {
        expect(result.current.loading).toBe(false);
      });

      // エラーはキャッチされ、フォールバック値が使用される
      expect(result.current.error).toBeNull();
      expect(result.current.branches).toHaveLength(0);
    });

    it("fetchAllRemotesがエラーを投げてもローカル表示は継続される", async () => {
      mockFetchAllRemotes.mockRejectedValue(new Error("Network error"));

      const { result } = renderHook(() => useGitData());

      await waitFor(() => {
        expect(result.current.loading).toBe(false);
      });

      // リモート取得失敗でもローカルブランチは表示される
      expect(result.current.branches).toHaveLength(2);
      expect(result.current.error).toBeNull();
    });
  });
});
