# 実装計画: Worktree作成進捗モーダル

**仕様ID**: `SPEC-bf7fb63e`
**作成日**: 2026-01-25

## 実装概要

Worktree作成処理の進捗をセンターモーダルで表示する機能を実装する。既存の`LaunchProgress`列挙型を拡張し、新しい`WorktreeProgressModal`コンポーネントを追加する。

## アーキテクチャ設計

### コンポーネント構成

```text
gwt-cli/
├── src/
│   ├── tui/
│   │   ├── app.rs              # メインアプリケーション（モーダル状態管理追加）
│   │   ├── widgets/
│   │   │   └── progress_modal.rs  # 新規: 進捗モーダルウィジェット
│   │   └── ...
│   └── main.rs                 # LaunchProgress拡張
└── ...
```

### データフロー

```text
1. ユーザーがエージェント起動を確定
   ↓
2. App::start_launch_preparation() 呼び出し
   ↓
3. show_progress_modal = true に設定
   ↓
4. バックグラウンドスレッドで処理開始
   ↓
5. 各ステップごとに LaunchUpdate::Progress を送信
   ↓
6. App::apply_launch_updates() でモーダル状態を更新
   ↓
7. 完了/エラー時にモーダルを閉じて次画面へ遷移
```

## 実装ステップ

### Phase 1: 型定義とデータ構造

1. **ProgressStepKind列挙型の追加** (`main.rs`)
   - Fetch, ValidateBranch, GeneratePath, CheckConflicts, CreateWorktree, CheckDependencies

2. **ProgressStep構造体の追加** (`main.rs`)
   - kind: ProgressStepKind
   - status: StepStatus (Pending/Running/Completed/Failed)
   - started_at: Option<Instant>
   - error_message: Option<String>

3. **LaunchProgress列挙型の拡張** (`main.rs`)
   - 既存のバリアントに加え、DetailedProgress(Vec<ProgressStep>) を追加

### Phase 2: モーダルウィジェット実装

4. **ProgressModalウィジェット作成** (`widgets/progress_modal.rs`)
   - 半透明オーバーレイの描画
   - 中央配置のボックス（幅80文字以上）
   - 動的タイトル表示
   - ステップリスト描画（チェックマーク形式）
   - 経過時間表示（3秒以上の場合）
   - サマリ/エラー表示

5. **App構造体への状態追加** (`app.rs`)
   - progress_modal_visible: bool
   - progress_steps: Vec<ProgressStep>
   - progress_start_time: Option<Instant>

### Phase 3: イベント処理

6. **ESCキーによるキャンセル実装** (`app.rs`)
   - モーダル表示中のESCキー検出
   - キャンセルシグナルの送信
   - バックグラウンド処理の中断
   - ブランチ一覧への復帰

7. **入力ブロック実装** (`app.rs`)
   - モーダル表示中は他のUI操作を無効化

### Phase 4: 進捗送信の実装

8. **start_launch_preparation()の改修** (`app.rs`)
   - 各ステップ開始/完了時にLaunchUpdate送信
   - 詳細なステップ情報を含むProgressを送信

9. **apply_launch_updates()の改修** (`app.rs`)
   - DetailedProgressの処理追加
   - モーダル状態の更新

### Phase 5: 冗長表示の排除

10. **既存表示の条件分岐追加**
    - ステータスバー: モーダル表示中は「Preparing worktree」非表示
    - ブランチ詳細: モーダル表示中は通常表示
    - セッション要約: モーダル表示中は通常表示

### Phase 6: 描画統合

11. **ui()関数への統合** (`app.rs`)
    - モーダル表示条件の追加
    - 最前面レイヤーとして描画

## ファイル変更一覧

| ファイル | 変更内容 |
|---------|---------|
| `crates/gwt-cli/src/main.rs` | ProgressStepKind, ProgressStep, LaunchProgress拡張 |
| `crates/gwt-cli/src/tui/widgets/mod.rs` | progress_modalモジュール追加 |
| `crates/gwt-cli/src/tui/widgets/progress_modal.rs` | 新規作成: モーダルウィジェット |
| `crates/gwt-cli/src/tui/app.rs` | 状態追加、イベント処理、描画統合 |

## テスト計画

### ユニットテスト

1. ProgressStep状態遷移テスト
2. 経過時間計算テスト（3秒閾値）
3. ステップリスト表示フォーマットテスト

### 統合テスト

1. モーダル表示/非表示遷移テスト
2. ESCキーキャンセルテスト
3. エラー時のステップ状態テスト

### 手動テスト

1. 実際のWorktree作成での進捗表示確認
2. 大規模リポジトリでの長時間処理確認
3. ネットワークエラー時のエラー表示確認

## リスクと緩和策

| リスク | 緩和策 |
|-------|-------|
| モーダル描画のちらつき | ダブルバッファリング確認、描画最適化 |
| キャンセル時のgit操作中断失敗 | プロセスkill、クリーンアップ処理 |
| 既存テストの破損 | LaunchProgress互換性維持、段階的移行 |

## 依存関係

- ratatui 0.29: モーダル描画（既存）
- crossterm 0.28: 端末操作（既存）
- std::time::Instant: 経過時間計測

## 互換性

既存の`LaunchProgress`列挙型を拡張するため、既存コードとの互換性を維持する。`DetailedProgress`バリアントを追加し、既存のバリアント（ResolvingWorktree等）も引き続きサポートする。
