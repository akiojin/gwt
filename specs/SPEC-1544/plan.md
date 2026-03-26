### 技術コンテキスト

- Rust の gh CLI ラッパーから C# の `System.Diagnostics.Process` + UniTask + JSON パースへの移行
- gh CLI の JSON 出力を活用した型安全なデシリアライズ
- VContainer による DI 統合、キャッシュ戦略（TTL ベース）

### 実装アプローチ

- `IGitHubService` インターフェースを定義し、全メソッドシグネチャを確定する
- gh CLI プロセス実行の基盤クラス（`GhCommandRunner`）を実装する（`System.Diagnostics.Process` + UniTask + JSON パース）
- 認証チェック・gh CLI 検出 → Issues → PR → CI/CD → Spec Issue の順に実装
- Lead向けPR操作、クラッシュレポート機能を追加実装
- キャッシュ戦略を実装し、VContainer で DI 登録して統合テストを実施する

### フェーズ分割

- **Phase S（Setup）**: インターフェース定義、基盤クラス、データ型定義
- **Phase F（Foundation）**: GhCommandRunner 実装、認証チェック、gh CLI 検出
- **Phase U（User story）**: Issues 操作、PR 操作（プリフライト含む）、CI/CD、Spec Issue CRUD、Lead PR操作、クラッシュレポート
- **Phase FIN（Finalization）**: キャッシュ戦略、DI 統合、統合テスト
