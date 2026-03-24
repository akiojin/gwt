### 技術コンテキスト

- 現行 Rust 実装の Docker 検出・管理ロジックを C# に移植する
- `docker` / `docker-compose` CLI ラッパーを C# で実装し、コンテナ管理を行う
- docker exec PTY 方式で既存の IPtyService インターフェースと統一する

### 実装アプローチ

- Docker CLI ラッパーによるコンテナ管理（SDK 非依存）
- オーバーレイパネルでサービス選択 UI を提供する

### フェーズ分割

1. 既存 Rust 実装の Docker 検出・管理ロジックを分析する
2. C# の `IDockerService` インターフェースを設計する
3. Docker 設定ファイル検出ロジックを実装する
4. `docker` / `docker-compose` CLI ラッパーを C# で実装する
5. DevContainer 設定パーサーを実装する
6. サービス選択オーバーレイ UI を実装する
7. コンテナ内エージェント起動の連携を実装する（docker exec PTY 方式）
8. コンテナ起動失敗時のフォールバック機構を実装する
9. VContainer での DI 登録を行う
10. テストを作成する
