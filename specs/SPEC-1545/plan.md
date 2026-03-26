### 技術コンテキスト

- Rust の PTY/プロセス管理から C# の `System.Diagnostics.Process` + UniTask 非同期パターンへの移行
- VContainer による DI 統合（Singleton ライフタイム + シーン MonoBehaviour 向け手動解決パターン）
- エージェント検出は PATH 検索 + バージョン取得で実装
- セッション永続化は `~/.gwt/sessions/` に JSON 形式

### 実装アプローチ

- `IAgentService`, `ISessionService` インターフェースを定義する
- エージェント検出機構を実装する（PATH 検索・バージョン取得）
- **`IPtyService.SpawnAsync()` → Process ベースの** PTY プロセス管理の基盤を実装する（起動・入出力・終了検出）✅
- PTY ライフサイクル管理（アプリ終了時の確認ダイアログ + 全プロセス graceful 停止）を実装する
- エージェント起動フロー（検出→設定→起動→追跡）を順次実装する
- 雇用メタファー UI、ジョブタイプシステム、キャラクタースプライト、空席デスクを実装する
- セッション永続化、ステータスリアルタイム追跡を実装する
- Lead 委任、Skills 管理、Claude Code hooks 連携、DevContainer 対応を実装する
- VContainer で DI 登録し、統合テストを実施する

### フェーズ分割

- **Phase S（Setup）**: インターフェース定義、データ型定義、JobType enum
- **Phase F（Foundation）**: PTY プロセス管理基盤（✅実装済み）、エージェント検出、設定管理
- **Phase U（User story）**: エージェント起動フロー、雇用メタファー UI、ジョブタイプ、空席デスク、セッション永続化・復元、ステータス追跡、Lead 委任、ターミナルペイン管理（✅実装済み）、Skills 管理、hooks 連携、DevContainer 対応
- **Phase FIN（Finalization）**: DI 統合（✅実装済み）、統合テスト
