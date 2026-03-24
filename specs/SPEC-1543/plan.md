### 技術コンテキスト

- Rust の `std::process::Command` + `tokio` 非同期ランタイムから、C# の `System.Diagnostics.Process` + UniTask 非同期パターンへの移行
- bare リポジトリ + worktree アーキテクチャの C# 実装
- VContainer による DI 統合

### 実装アプローチ

- `IGitService` インターフェースを定義し、全メソッドシグネチャを確定する
- git CLI プロセス実行の基盤クラス（`GitCommandRunner`）を実装する（`System.Diagnostics.Process` + UniTask パターン）
- 各カテゴリの Git 操作を順次実装し、パーサーとモデル変換をテスト駆動で進める
- VContainer で DI 登録し、統合テストを実施する

### フェーズ分割

- **Phase S（Setup）**: インターフェース定義、基盤クラス、データ型定義
- **Phase F（Foundation）**: GitCommandRunner 実装、エラーハンドリング、タイムアウト
- **Phase U（User story）**: 各 Git 操作（Worktree, Branch, Diff, History, Cleanup, Version）の実装
- **Phase FIN（Finalization）**: DI 統合、統合テスト、パフォーマンス検証
