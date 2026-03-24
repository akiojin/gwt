### 技術コンテキスト

- Unity 6 + VContainer（DI）+ UniTask（非同期）
- 既存サービス（IGitService, IAgentService, IGitHubService, IAIApiService, IConfigService）を統合
- Assembly依存: `Gwt.Agent` → `Gwt.Core` → `Gwt.Shared`
- `IProjectLifecycleService` は `Gwt.Lifecycle` に属し直接参照不可 → `IGitService` + `LeadSessionData.ProjectRoot` で代替

### 実装アプローチ

1. LLM によるタスク分割、JSON パース、計画生成
2. 計画承認 → ワークツリー作成 → エージェント雇用 → 依存関係制御
3. AgentOutputBuffer + LLM 評価による完了/失敗判定
4. 自動 PR 作成 → Mergeable 判定 → ワークツリークリーンアップ
5. リアルタイム集計 + イベント通知

### フェーズ分割

```
Phase 1: タスク計画エンジン
  └─ LLM によるタスク分割、JSON パース、計画生成
  └─ SPEC Issue生成ツール（LLM function calling用）

Phase 2: 実行オーケストレーション
  └─ 計画承認 → ワークツリー作成 → エージェント雇用 → 依存関係制御
  └─ force push/rebase禁止ガード

Phase 3: 進捗監視強化
  └─ AgentOutputBuffer + LLM 評価による完了/失敗判定
  └─ PTY出力停止30秒でwaiting_input推定

Phase 4: マージ管理
  └─ 自動 PR 作成 → Mergeable 判定 → ワークツリークリーンアップ

Phase 5: 進捗レポート
  └─ リアルタイム集計 + イベント通知

MVP = Phase 1 + Phase 2
```
