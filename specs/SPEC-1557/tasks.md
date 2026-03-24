### Phase S: セットアップ
- [ ] T001 [S] [US-1,US-2] `IProjectLifecycleService` インターフェース定義
- [ ] T002 [S] [US-1,US-2] データ型定義（ProjectInfo, ProjectOpenResult, MigrationJob, QuitState）

### Phase F: 基盤
- [ ] T003 [F] [US-1] パスプローブ・Git リポジトリ検出実装 + テスト
- [ ] T004 [F] [US-2] プロジェクト情報取得実装 + テスト

### Phase U: ユーザーストーリー実装
- [ ] T005 [U] [US-2] プロジェクトオープン（サービス初期化・worktree 検出）実装 + テスト
- [ ] T006 [U] [US-2] プロジェクトオープンの段階的ロード実装（初回表示5秒以内 + バックグラウンド後追い）
- [ ] T007 [U] [US-5] プロジェクトクローズ（リソース解放）実装 + テスト
- [ ] T008 [U] [US-3] 新規 bare リポジトリ作成実装 + テスト
- [ ] T009 [U] [US-4] 通常リポ → bare リポ移行ジョブ実装 + テスト
- [ ] T010 [U] [US-6] アプリケーション終了シーケンス実装 + テスト
- [ ] T011 [U] [US-7] 終了確認ダイアログ実装 + テスト

### Phase FIN: 最終化
- [ ] T012 [FIN] [US-1] VContainer DI 登録
- [ ] T013 [FIN] [US-1,US-2,US-3,US-4,US-5,US-6,US-7] 統合テスト
- [ ] T014 [FIN] [US-2] 初回表示5秒以内のパフォーマンステスト
