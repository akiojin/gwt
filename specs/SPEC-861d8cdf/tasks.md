# タスク一覧: エージェント状態の可視化

**仕様ID**: `SPEC-861d8cdf`
**作成日**: 2026-01-20

## 概要

本タスク一覧は、tmuxモードでのエージェント状態可視化機能の実装に必要なタスクを定義します。

## タスク依存関係

```
T-100 (データモデル)
    ├── T-101 (hookサブコマンド)
    │   └── T-102 (Hook設定)
    │       └── T-104 (初回セットアップ)
    ├── T-103 (状態表示UI)
    │   └── T-105 (ステータスバー)
    └── T-106 (他エージェント推測)
```

## タスク一覧

### T-100: AgentStatus列挙型とSessionフィールド追加

**優先度**: P1  
**見積もり**: S  
**前提タスク**: なし  
**関連要件**: FR-100a, FR-100b, FR-100c

**説明**:
- Session構造体に`status: AgentStatus`フィールドを追加
- Session構造体に`last_activity_at: Option<DateTime<Utc>>`フィールドを追加
- AgentStatus列挙型（Unknown, Running, WaitingInput, Stopped）を定義
- 既存の.gwt-session.tomlとの後方互換性を維持（statusフィールドがない場合はUnknownとして扱う）

**受け入れ基準**:
- [ ] AgentStatus列挙型が定義され、Serialize/Deserializeが実装されている
- [ ] Sessionにstatus, last_activity_atフィールドが追加されている
- [ ] 既存のセッションファイル読み込み時にエラーが発生しない
- [ ] 60秒経過時にstoppedに自動更新するロジックが実装されている

---

### T-101: gwt hookサブコマンドの実装

**優先度**: P1  
**見積もり**: M  
**前提タスク**: T-100  
**関連要件**: FR-101a, FR-101b, FR-101c, FR-101d

**説明**:
- `gwt hook <event_name>`サブコマンドを追加
- 標準入力からJSONペイロードを読み取り、パース
- イベント種別（UserPromptSubmit, PreToolUse, PostToolUse, Notification, Stop）に応じて状態を決定
- Notification[permission_prompt]の場合はwaiting_inputに設定
- 該当worktreeの.gwt-session.tomlを更新

**受け入れ基準**:
- [ ] `gwt hook UserPromptSubmit`でstatusがrunningに更新される
- [ ] `gwt hook Stop`でstatusがstoppedに更新される
- [ ] `gwt hook Notification`でpermission_promptの場合、statusがwaiting_inputに更新される
- [ ] 実行時間が100ms以内

---

### T-102: Claude Code Hook設定機能

**優先度**: P1  
**見積もり**: M  
**前提タスク**: T-101  
**関連要件**: FR-102a, FR-102b, FR-102c

**説明**:
- ~/.claude/settings.jsonの読み取り・書き込み機能を実装
- 既存のhooks設定を保持しつつ、gwt hook設定を追加/更新
- settings.jsonが存在しない場合は新規作成

**受け入れ基準**:
- [ ] settings.jsonが存在しない場合、新規作成できる
- [ ] 既存のhooks設定を上書きしない（マージする）
- [ ] 5つのイベント（UserPromptSubmit, PreToolUse, PostToolUse, Notification, Stop）が登録される

---

### T-103: 状態表示UIの実装

**優先度**: P1  
**見積もり**: M  
**前提タスク**: T-100  
**関連要件**: FR-103a, FR-103b, FR-103c, FR-103d, FR-103e

**説明**:
- branch_list.rsの表示ロジックを更新
- running: 緑色の星形スピナー（既存のACTIVE_SPINNER_FRAMES）
- waiting_input: 黄色のアイコン + 500ms点滅
- stopped: 赤色の停止アイコン（■）
- バックグラウンドは輝度を下げた色で表示
- アクティブでもstoppedなら赤色

**受け入れ基準**:
- [ ] running状態で緑色の星形スピナーが表示される
- [ ] waiting_input状態で黄色の点滅表示（500ms間隔）
- [ ] stopped状態で赤色の停止アイコンが表示される
- [ ] バックグラウンドエージェントは低輝度色で表示される

---

### T-104: 初回起動時のHookセットアップ提案

**優先度**: P2  
**見積もり**: M  
**前提タスク**: T-102  
**関連要件**: FR-102b

**説明**:
- gwt起動時に~/.claude/settings.jsonのhooks設定を確認
- gwt hookが未登録の場合、確認ダイアログを表示
- ユーザーが「設定する」を選択した場合、T-102の機能でHookを登録
- 「スキップ」選択時は何もしない

**受け入れ基準**:
- [ ] Hook未設定時に確認ダイアログが表示される
- [ ] 「設定する」選択でHookが登録される
- [ ] Hook登録済みの場合、ダイアログは表示されない

---

### T-105: ステータスバーの実装

**優先度**: P1  
**見積もり**: M  
**前提タスク**: T-103  
**関連要件**: FR-104a, FR-104b, FR-104c

**説明**:
- BranchListスクリーンの下部に1行のステータスバーを追加
- エージェント状態の集計を表示（"2 running | 1 waiting | 1 stopped"）
- waiting_inputカウントが1以上の場合、黄色で強調
- エージェントがない場合は表示しないか「No agents」

**受け入れ基準**:
- [ ] ステータスバーが画面下部に表示される
- [ ] 各状態のカウントが正確に表示される
- [ ] waiting部分が黄色で強調される

---

### T-106: 他エージェントの状態推測機能

**優先度**: P2  
**見積もり**: L  
**前提タスク**: T-100, T-103  
**関連要件**: FR-105a, FR-105b, FR-105c, FR-105d

**説明**:
- プロセス生存確認（is_process_running）でstopped/runningの基本判定
- 60秒間ペイン出力がない場合、stoppedと推測
- ペイン出力末尾のプロンプトパターン検出（>, →, Input:等）
- 推測であることをUIで区別（?マーク等）

**受け入れ基準**:
- [ ] プロセス終了でstoppedと判定される
- [ ] 60秒間出力なしでstoppedと推測される
- [ ] プロンプトパターン検出でwaiting_inputと推測される
- [ ] 推測であることがUIで区別される

---

## 実装順序

1. **Phase 1 - 基盤** (T-100)
   - データモデルの拡張

2. **Phase 2 - Hook基盤** (T-101, T-102)
   - hookサブコマンド実装
   - Claude Code設定機能

3. **Phase 3 - UI** (T-103, T-105)
   - 状態表示の実装
   - ステータスバーの追加

4. **Phase 4 - UX向上** (T-104, T-106)
   - 初回セットアップ提案
   - 他エージェント状態推測
