### 技術コンテキスト

- Unity 6 + C# (UniTask 非同期)
- VContainer（DI）
- LLM API 直接呼び出し（OpenAI / Anthropic / Google 互換）
- PTY 経由のエージェント通信（既存 gwt-core 踏襲）
- GitHub API（gh CLI 連携）
- JSON シリアライズによるセッション永続化（~/.gwt/）
- LimeZu アセット（Lead スプライト）

### 実装アプローチ

1. Rust 側の既存実装（`gwt-core/src/agent/`）を分析し、状態遷移図を作成する
2. C# のインターフェース設計（`ILeadOrchestrationService`, `ILeadAgent`, `IAgentWorker`）
3. UniTask ベースの非同期オーケストレーションループを実装する（常時アクティブ）
4. Lead 雇用・解雇システムを実装する（候補選択、引継ぎドキュメント生成（全コンテキスト含む））
5. Lead キャラクター性の設定システムを実装する（見た目・声・性格の候補定義、LimeZu アセットから選択）
6. Issue close 判断システムを実装する（Lead AI による状況判断 + ユーザー承認フロー）
7. **Lead 専用 SPEC 内蔵ツールを実装する（LLM function calling 用に専用設計、gwt-spec-ops スキルとは別系統）**
8. セッション永続化（JSON シリアライズ）を実装する
9. スタジオとの連携（キャラクター生成・アニメーション制御）を実装する
10. VContainer での DI 登録を行う
11. ユニットテスト・統合テストを作成する

### フェーズ分割

**Phase S（Setup）:** Rust既存実装分析、インターフェース設計、VContainer DI登録
**Phase F（Foundation）:** LeadAgent C#実装、PTY監視、セッション永続化
**Phase U（User Story）:** Lead雇用/解雇、SPEC生成、Issue close判断、スタジオ連携
**Phase FIN（Finalization）:** 統合テスト、パフォーマンス検証、受け入れテスト

### リスク

- LLM API レイテンシによる応答遅延 → ストリーミング応答 + 非同期処理で対応
- PTY 監視のポーリング負荷 → 4秒固定間隔で安定化（#1574準拠）
- セッション永続化の整合性 → トランザクション的な書き込み + チェックサムで対応
- Lead 引継ぎドキュメントの品質 → プロンプトエンジニアリング + 構造化テンプレートで対応
