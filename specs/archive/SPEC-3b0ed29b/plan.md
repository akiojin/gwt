# 実装計画: 進捗モーダル（ユーザーストーリー15）

**仕様ID**: `SPEC-3b0ed29b` | **日付**: 2026-01-25 | **仕様書**: [spec.md](./spec.md)
**入力**: `/specs/SPEC-3b0ed29b/spec.md` ユーザーストーリー15（FR-041, FR-044〜FR-060）

**スコープ**: このplan.mdはユーザーストーリー15「起動準備の進捗がセンターモーダルで見える」のみを対象とする。

## 概要

Worktree作成処理の進捗をセンターモーダルで表示する機能を実装する。現在のステータスバー表示（`launch_status`）からモーダル表示に変更し、ステップリスト形式でチェックマーク表示を行う。

## 技術コンテキスト

**言語/バージョン**: Rust 2021 Edition (stable)
**主要な依存関係**: ratatui 0.29, crossterm 0.28
**ストレージ**: N/A（メモリのみ）
**テスト**: cargo test
**ターゲットプラットフォーム**: Linux, macOS, Windows
**プロジェクトタイプ**: 単一CLIアプリケーション
**パフォーマンス目標**: モーダル表示まで1秒以内
**制約**: ASCII文字のみ（絵文字不使用）、CLIは英語のみ

## 原則チェック

| 原則 | 状態 | 備考 |
|-----|------|-----|
| I. シンプルさの追求 | ✅ | 既存のLaunchProgress拡張で実現 |
| II. テストファースト | ✅ | TDDでユニットテスト先行 |
| III. 既存コードの尊重 | ✅ | main.rs, app.rsを改修、新規ファイル最小限 |
| IV. 品質ゲート | ✅ | clippy, fmt, testを通過させる |
| V. 自動化の徹底 | ✅ | Conventional Commits遵守 |

## プロジェクト構造

### ドキュメント（この機能）

```text
specs/SPEC-3b0ed29b/
├── spec.md              # 機能仕様
├── plan.md              # このファイル
├── research.md          # フェーズ0出力
├── data-model.md        # フェーズ1出力
├── quickstart.md        # フェーズ1出力
└── tasks.md             # フェーズ2出力
```

### ソースコード（リポジトリルート）

```text
crates/gwt-cli/src/
├── main.rs                      # LaunchProgress拡張、ProgressStep追加
├── tui/
│   ├── app.rs                   # App状態追加、モーダル描画統合
│   └── widgets/
│       ├── mod.rs               # モジュール追加
│       └── progress_modal.rs    # 新規: モーダルウィジェット
```

## フェーズ0: 調査（技術スタック選定）

**出力**: `specs/SPEC-3b0ed29b/research.md`

### 調査結果

#### 1. 既存のコードベース分析

**現在の実装**:
- `LaunchProgress`列挙型（main.rs:803-820）: 4つのバリアント（ResolvingWorktree, BuildingCommand, CheckingDependencies, InstallingDependencies）
- `LaunchUpdate`列挙型（app.rs:267-271）: Progress, WorktreeReady, Ready, Failed
- `launch_status: Option<String>`（app.rs:321）: 現在の進捗メッセージ
- `apply_launch_updates()`（app.rs:1686-1733）: チャネル経由で更新を受信

**既存パターン**:
- バックグラウンドスレッドで処理、`mpsc::channel`でTUIに通知
- `LaunchProgress.message()`で英語メッセージを生成
- ステータスバーに表示（`active_status_message()`経由）

#### 2. 技術的決定

| 決定 | 選択 | 理由 |
|-----|------|-----|
| ステップ状態管理 | `ProgressStep`構造体を新規追加 | 既存の`LaunchProgress`は単一メッセージ用、複数ステップ状態が必要 |
| モーダル描画 | `ratatui::widgets::Clear` + `Block` | 既存のポップアップパターン（help画面等）に準拠 |
| 経過時間計測 | `std::time::Instant` | 標準ライブラリ、依存追加不要 |
| スキップ表示 | `StepStatus::Skipped`追加 | 既存worktree再利用時のスキップ表示に必要 |

#### 3. 制約と依存関係

- ratatui 0.29のモーダル描画：`Clear`ウィジェットで背景クリア後、`Block`で枠描画
- 半透明オーバーレイ：ratatuiはANSI色のみ対応、疑似半透明（暗い背景色）で代用
- キャンセル処理：`std::process::Child::kill()`またはスレッド中断フラグ

## フェーズ1: 設計（アーキテクチャと契約）

### 1.1 データモデル設計

**ファイル**: `data-model.md`

```rust
/// ステップの種類（FR-048）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressStepKind {
    FetchRemote,       // 1. Fetching remote...
    ValidateBranch,    // 2. Validating branch...
    GeneratePath,      // 3. Generating path...
    CheckConflicts,    // 4. Checking conflicts...
    CreateWorktree,    // 5. Creating worktree...
    CheckDependencies, // 6. Checking dependencies...
}

/// ステップの状態（FR-047）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,    // [ ]
    Running,    // [>]
    Completed,  // [x]
    Failed,     // [!]
    Skipped,    // [skip]
}

/// 進捗ステップ（FR-049）
#[derive(Debug, Clone)]
pub struct ProgressStep {
    pub kind: ProgressStepKind,
    pub status: StepStatus,
    pub started_at: Option<Instant>,
    pub error_message: Option<String>,
}

/// モーダル状態
pub struct ProgressModalState {
    pub visible: bool,
    pub steps: Vec<ProgressStep>,
    pub start_time: Instant,
    pub cancellation_requested: bool,
}
```

### 1.2 クイックスタートガイド

**ファイル**: `quickstart.md`

開発者向けの実装ガイド：
1. `ProgressStepKind`と`ProgressStep`を`main.rs`に追加
2. `ProgressModalState`を`App`構造体に追加
3. `progress_modal.rs`ウィジェットを作成
4. `ui()`関数でモーダル描画を追加
5. ESCキーハンドリングを追加

### 1.3 契約/インターフェース

**ファイル**: `contracts/progress_modal.rs`

```rust
impl ProgressStep {
    pub fn new(kind: ProgressStepKind) -> Self;
    pub fn start(&mut self);
    pub fn complete(&mut self);
    pub fn fail(&mut self, message: String);
    pub fn skip(&mut self);
    pub fn marker(&self) -> &'static str;
    pub fn elapsed_secs(&self) -> Option<f64>;
    pub fn should_show_elapsed(&self) -> bool; // 3秒以上
}

impl Widget for ProgressModal<'_> {
    fn render(self, area: Rect, buf: &mut Buffer);
}
```

## テスト戦略

- **ユニットテスト**: ProgressStep状態遷移、経過時間計算、マーカー文字列
- **統合テスト**: モーダル表示/非表示遷移、ESCキャンセル
- **手動テスト**: 実際のWorktree作成での動作確認

## リスクと緩和策

### 技術的リスク

1. **モーダル描画のちらつき**
   - **緩和策**: ratatuiのダブルバッファリングを確認、必要に応じて描画最適化

2. **キャンセル時のgit操作中断**
   - **緩和策**: キャンセルフラグでループ脱出、不完全worktreeのクリーンアップ処理

### 依存関係リスク

1. **既存のLaunchProgressとの互換性**
   - **緩和策**: 段階的移行、既存バリアントを維持しつつ新しいステップ管理を追加

## 次のステップ

1. ✅ フェーズ0完了: 調査と技術スタック決定
2. ✅ フェーズ1完了: 設計とアーキテクチャ定義
3. ⏭️ `/speckit.tasks` を実行してタスクを生成
4. ⏭️ `/speckit.implement` で実装を開始
