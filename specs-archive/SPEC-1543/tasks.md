> **Historical Status**: この closed SPEC の未完了 task は旧 backlog の保存であり、現行の完了条件ではない。

- [ ] T001 [S] [US-1] `IGitService` インターフェース定義
- [ ] T002 [S] [US-1] データ型定義（Worktree, Branch, FileChange, FileDiff, CommitEntry, StashEntry, WorkingTreeEntry, GitChangeSummary）
- [ ] T003 [F] [US-1] `GitCommandRunner` 基盤クラス実装（`System.Diagnostics.Process` + UniTask + CancellationToken + タイムアウト）
- [ ] T004 [U] [US-1] Worktree CRUD 実装 + テスト
- [ ] T005 [U] [US-2] Worktree 作成・削除のスタジオ連携実装 + テスト
- [ ] T006 [U] [US-1] Branch 一覧・現在ブランチ・保護チェック実装 + テスト
- [ ] T007 [U] [US-3] Diff/Status 取得実装 + テスト
- [ ] T008 [U] [US-3] コミット履歴・スタッシュ一覧実装 + テスト
- [ ] T009 [U] [US-3] ベースブランチ候補検出実装 + テスト
- [ ] T010 [U] [US-4] Worktree クリーンアップ実装 + テスト
- [ ] T011 [U] [US-1] バージョン履歴取得実装 + テスト
- [ ] T012 [U] [US-1] bare リポジトリサポート・マイグレーション実装 + テスト
- [ ] T013 [U] [US-2] Lead Git権限制御（force push/rebase禁止）実装 + テスト
- [ ] T014 [FIN] [US-1] VContainer DI 登録
- [ ] T015 [FIN] [US-1] 統合テスト（実リポジトリでの E2E）
