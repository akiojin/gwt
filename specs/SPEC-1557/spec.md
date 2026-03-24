### 背景

gwt のプロジェクトライフサイクル管理（プロジェクトの開閉、新規作成、bare リポジトリ移行、アプリケーション終了）は、現行 Rust 版で 8 つの Tauri コマンドとして実装されている。Unity 移行に伴い、これらを C# サービスメソッドとして再実装する必要がある。

この機能は #1542（データ永続化）、#1543（Git 操作）、#1546（スタジオ UI）と連携するが、プロジェクトライフサイクルのビジネスロジック（初期化・クリーンアップ・状態遷移）を統括するサービスとして独立して管理する。

### 再実装対象コマンド → Unity サービスメソッド

| Tauri コマンド | Unity サービスメソッド |
|---------------|---------------------|
| `probe_path` | `IProjectLifecycleService.ProbePathAsync()` |
| `open_project` | `IProjectLifecycleService.OpenProjectAsync()` |
| `get_project_info` | `IProjectLifecycleService.GetProjectInfo()` |
| `close_project` | `IProjectLifecycleService.CloseProjectAsync()` |
| `create_project` | `IProjectLifecycleService.CreateProjectAsync()` |
| `start_migration_job` | `IProjectLifecycleService.StartMigrationJobAsync()` |
| `quit_app` | `IProjectLifecycleService.QuitAppAsync()` |
| `cancel_quit_confirm` | `IProjectLifecycleService.CancelQuitConfirm()` |

### ユーザーシナリオ

- **US-1** [P0]: アプリ起動時にプロジェクト選択画面が表示され、パスを指定してプロジェクトを開ける
  - テスト: プロジェクト選択画面が表示されること
  - テスト: パス指定でプロジェクトが開けること
- **US-2** [P0]: プロジェクトを開くと、全 worktree・ブランチ情報が読み込まれスタジオが生成される
  - テスト: 全 worktree 情報が読み込まれること
  - テスト: スタジオが生成されること
- **US-3** [P0]: 新規リポジトリを gwt プロジェクトとして初期化（bare リポ化）できる
  - テスト: bare リポジトリが作成されること
  - テスト: .gwt ディレクトリと settings.json が生成されること
- **US-4** [P1]: 通常リポジトリを bare リポジトリに移行できる（プログレスバー付き）
  - テスト: 移行ジョブが正常に完了すること
  - テスト: プログレスが更新されること
- **US-5** [P0]: プロジェクトを閉じると全リソースが適切に解放される
  - テスト: 全サービスが解放されること
  - テスト: CurrentProject が null になること
- **US-6** [P0]: アプリ終了時に全エージェント・PTY が停止し、セッション状態が永続化される
  - テスト: 全 PTY プロセスが停止すること
  - テスト: セッション状態が永続化されること
- **US-7** [P1]: 終了時に未保存の変更がある場合、確認ダイアログが表示される
  - テスト: 未保存変更ありで確認ダイアログが表示されること
  - テスト: キャンセルで終了が中止されること

### 機能要件

| ID | 要件 |
|----|------|
| FR-001 | 指定パスの Git リポジトリ検出・bare リポ判定を行う |
| FR-002 | プロジェクトを開く際に全サービス（Git, GitHub, Agent, Session）を初期化する |
| FR-003 | プロジェクト情報（パス、名前、リモート URL、デフォルトブランチ等）を取得する |
| FR-004 | プロジェクトを閉じる際に全リソースを適切に解放する |
| FR-005 | 新規 bare リポジトリの作成・初期 worktree セットアップをサポートする |
| FR-006 | 通常リポジトリ → bare リポジトリの移行（非同期ジョブ、プログレス追跡）をサポートする |
| FR-007 | アプリケーション終了時に全エージェントプロセス停止・PTY 終了・セッション永続化を実行する |
| FR-008 | 終了確認ダイアログ（未保存変更・実行中エージェントの確認）をサポートする |
| FR-009 | VContainer で `IProjectLifecycleService` として DI 登録する |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | プロジェクト開閉の**初回表示（スタジオ基本描画）は5秒以内**に完了する。Issueマーカー・LLM分析等の重い処理はバックグラウンドで後追い実行する |
| NFR-002 | bare リポ移行中もメインスレッドをブロックしない |
| NFR-003 | アプリ異常終了時も次回起動時にデータ整合性を維持する |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | パス指定でプロジェクトを開き、スタジオが生成される |
| SC-002 | 新規 bare リポジトリの作成が動作する |
| SC-003 | 通常リポ → bare リポ移行が完了する |
| SC-004 | アプリ終了時に全リソースが適切に解放される |
| SC-005 | 終了確認ダイアログが正しく動作する |
| SC-006 | プロジェクト開閉の初回表示が5秒以内に完了する |

### インタビュー確定事項（2026-03-10追記）

**プロジェクト終了時の挙動:**
- エージェントプロセスを停止する
- セッションデータ（デスク配置、Agent状態、会話履歴）は保持する
- 次回プロジェクト再開時にセッション復元が可能
- PTYプロセスは失われるが、セッションメタデータは永続化

### 主要データ型

| 型名 | フィールド |
|------|-----------| 
| `ProjectInfo` | path, name, bare_path, worktree_root, remote_url, default_branch, is_bare |
| `ProjectOpenResult` | project_info, worktrees, branches, issues_count |
| `MigrationJob` | id, status, progress, source_path, target_path, error |
| `QuitState` | pending_sessions, unsaved_changes, can_quit |
