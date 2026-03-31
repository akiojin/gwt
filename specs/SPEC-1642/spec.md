> **ℹ️ TUI MIGRATION NOTE**: This SPEC describes backend/gwt-core functionality unaffected by the gwt-tui migration (SPEC-1776). No changes required.

### 背景
GWT内で全面的なDocker管理機能を提供する。コンテナライフサイクル、リソース監視、ネットワーク設定を包含。docker-compose検出、サービス選択、エージェントDocker内実行は実装済み。Studio時代の #1552（Docker/DevContainer サポート）の機能概念を現行スタックで再定義。

### ユーザーシナリオとテスト

**S1: docker-compose検出と表示**
- Given: プロジェクトにdocker-compose.ymlが存在
- When: プロジェクトを開く
- Then: Docker環境が検出されサービス一覧が表示される

**S2: コンテナライフサイクル管理**
- Given: Docker環境が検出済み
- When: コンテナの起動/停止/再起動を操作
- Then: 操作が実行されステータスが更新される

**S3: リソース監視**
- Given: コンテナが稼働中
- When: リソース使用状況を表示
- Then: CPU/メモリ/ネットワーク使用量が表示される

**S4: エージェントのDocker内実行**
- Given: Docker環境が設定済み
- When: エージェントを起動
- Then: エージェントがDocker内で実行される

### 機能要件

**FR-01: コンテナライフサイクル**
- 起動/停止/再起動/削除
- docker-compose対応

**FR-02: リソース監視**
- CPU/メモリ/ネットワーク使用量表示
- リアルタイム更新

**FR-03: ネットワーク設定**
- ポートマッピング表示
- ネットワーク設定管理

**FR-04: エージェント連携**
- Docker内でのエージェント実行サポート
- Agent管理SPECと連携

### 成功基準

1. docker-compose.ymlの自動検出が動作する
2. コンテナのライフサイクル操作が正常に動作する
3. リソース監視がリアルタイムで表示される

---
