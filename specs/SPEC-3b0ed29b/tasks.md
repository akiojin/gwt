# タスク: 進捗モーダル（ユーザーストーリー15）

**入力**: `/specs/SPEC-3b0ed29b/` からの設計ドキュメント
**前提条件**: plan.md（必須）、spec.md（FR-041〜FR-060, SC-012〜SC-018）
**スコープ**: このtasks.mdはユーザーストーリー15「起動準備の進捗がセンターモーダルで見える」のみを対象とする

## フォーマット: `[ID] [P?] [ストーリー] 説明`

- **[P]**: 並列実行可能（異なるファイル、依存関係なし）
- **[US15]**: このタスクはユーザーストーリー15に属する
- 説明に正確なファイルパスを含める

## Lint最小要件

- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo fmt --check`
- `cargo test`

## 依存関係マップ

```text
T001 ─┐
      ├─> T003 ─> T004 ─> T005 ─> T006 ─┐
T002 ─┘                                  ├─> T007 ─> T008 ─> T009 ─> T010
                                         │
                                         └─> [完了]
```

- T001, T002: 並列可能（型定義）
- T003: T001, T002 に依存（ProgressStepメソッド）
- T004〜T006: 順次（ProgressModal構築）
- T007〜T010: 順次（App統合、キャンセル、冗長排除、UI統合）

## フェーズ1: 型定義（基盤）

**目的**: 進捗モーダルで使用する基本型を定義

### 型定義タスク

- [x] **T001** [P] [US15] `ProgressStepKind`列挙型を追加（6段階のステップ種類定義）

  ```text
  crates/gwt-cli/src/main.rs
  ```

  - FetchRemote, ValidateBranch, GeneratePath, CheckConflicts, CreateWorktree, CheckDependencies
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`
  - **TDD**: 各バリアント生成テスト

- [x] **T002** [P] [US15] `StepStatus`列挙型を追加（ステップ状態定義）

  ```text
  crates/gwt-cli/src/main.rs
  ```

  - Pending, Running, Completed, Failed, Skipped
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]` (Default=Pending)
  - **TDD**: 各バリアント生成テスト、デフォルトテスト

## フェーズ2: ProgressStep構造体

**目的**: 個別ステップの状態管理

### ProgressStep実装タスク

- [x] **T003** [US15] `ProgressStep`構造体と基本メソッドを追加（T001, T002完了後）

  ```text
  crates/gwt-cli/src/main.rs
  ```

  - フィールド: kind, status, started_at, error_message
  - メソッド: `new(kind)`, `start()`, `complete()`, `fail(msg)`, `skip()`
  - **TDD**: 状態遷移テスト（Pending→Running→Completed等）

- [x] **T004** [US15] `ProgressStep::marker()`メソッドを追加（T003完了後）

  ```text
  crates/gwt-cli/src/main.rs
  ```

  - 返却値: `[x]`, `[>]`, `[ ]`, `[!]`, `[skip]`
  - **TDD**: 各状態に対するマーカー文字列テスト

- [x] **T005** [US15] `ProgressStep::elapsed_secs()`と`should_show_elapsed()`を追加（T004完了後）

  ```text
  crates/gwt-cli/src/main.rs
  ```

  - `elapsed_secs()`: started_atからの経過秒数（f64）
  - `should_show_elapsed()`: 3秒以上でtrue
  - **TDD**: 経過時間計算テスト、閾値判定テスト

## フェーズ3: ProgressModalウィジェット

**目的**: モーダル描画ウィジェットの実装

### ウィジェット実装タスク

- [x] **T006** [US15] `progress_modal.rs`ファイルを作成し、基本構造を実装（T005完了後）

  ```text
  crates/gwt-cli/src/tui/widgets/progress_modal.rs
  crates/gwt-cli/src/tui/widgets/mod.rs
  ```

  - `ProgressModal<'a>`構造体
  - `ProgressModalState`構造体（visible, steps, start_time, cancellation_requested）
  - `impl Widget for ProgressModal`
  - 描画内容:
    - 半透明オーバーレイ（暗い背景色で疑似半透明）
    - 幅80文字以上のセンターモーダル
    - 動的タイトル
    - ステップリスト（チェックマーク形式）
    - 経過時間表示（3秒以上のステップ）
    - 色分け（緑=完了、黄=進行中、灰=待機、赤=失敗）
    - サマリ表示（完了時）
    - エラーメッセージ表示（失敗時）
  - **TDD**: ステップリスト描画テスト

## フェーズ4: App統合

**目的**: App構造体へのモーダル状態統合

### App統合タスク

- [x] **T007** [US15] `App`構造体に`ProgressModalState`を追加（T006完了後）

  ```text
  crates/gwt-cli/src/tui/app.rs
  ```

  - フィールド: `progress_modal: Option<ProgressModalState>`
  - メソッド:
    - `show_progress_modal()`: モーダル表示開始
    - `hide_progress_modal()`: モーダル非表示
    - `update_progress_step(kind, status)`: ステップ更新
    - `set_progress_error(kind, message)`: エラー設定
  - `LaunchUpdate`列挙型に`ProgressStep`バリアント追加
  - `apply_launch_updates()`でProgressStep処理を追加
  - **TDD**: モーダル表示/非表示状態遷移テスト

- [x] **T008** [US15] ESCキーによるキャンセル実装（T007完了後）

  ```text
  crates/gwt-cli/src/tui/app.rs
  ```

  - モーダル表示中のESCキー検出
  - `cancellation_requested`フラグ設定
  - キャンセル時のworktreeクリーンアップ（不完全な場合）
  - ブランチ一覧への遷移
  - **TDD**: ESCキー処理テスト、キャンセルフラグ状態テスト

- [x] **T009** [US15] 冗長表示の排除（T008完了後）

  ```text
  crates/gwt-cli/src/tui/app.rs
  ```

  - モーダル表示中は`launch_status`を非表示
  - ステータスバー、ブランチ詳細、セッション要約での「Preparing worktree」を抑制
  - **TDD**: モーダル表示中の他領域表示状態テスト

## フェーズ5: UI描画統合

**目的**: TUI描画への統合

### UI統合タスク

- [x] **T010** [US15] `ui()`関数にモーダル描画を統合（T009完了後）

  ```text
  crates/gwt-cli/src/tui/app.rs
  ```

  - `ui()`の最後（最前面）でProgressModal描画
  - モーダル表示中の他UI操作の無効化
  - サマリ表示（2秒）後のモーダル自動クローズ
  - エラー時はキー入力待ち
  - **TDD**: 描画呼び出しテスト

## フェーズ6: 統合と検証

**目的**: 全体の統合とLint/テスト通過

### 統合タスク

- [x] **T011** [統合] Lint/テスト通過確認

  ```text
  cargo clippy --all-targets --all-features -- -D warnings
  cargo fmt --check
  cargo test
  ```

  - 全テスト通過
  - Clippy警告ゼロ
  - フォーマット適合

- [x] **T012** [統合] PLANS.md更新

  ```text
  PLANS.md
  ```

  - 完了タスクのチェック更新
  - 必要に応じて次ステップ更新

## タスク凡例

**優先度**:

- **P1**: 必須 - モーダル表示に必要

**依存関係**:

- **[P]**: 並列実行可能
- 番号順: 前のタスク完了後に実行

**ストーリータグ**:

- **[US15]**: ユーザーストーリー15（進捗モーダル）
- **[統合]**: 複数タスクにまたがる検証

## 進捗追跡

- **完了したタスク**: `[x]` でマーク
- **進行中のタスク**: タスクIDの横にメモを追加
- **ブロックされたタスク**: ブロッカーを文書化

## 独立テスト条件

ユーザーストーリー15は以下の条件で独立してテスト可能:

1. モーダル表示テスト: Skip Permissions確定後1秒以内にモーダルが表示される
2. ステップ進行テスト: 各ステップが[x]/[>]/[ ]/[!]/[skip]で正しく表示される
3. 経過時間テスト: 3秒以上のステップで経過時間が表示される
4. キャンセルテスト: ESCキーでキャンセル可能、ブランチ一覧に戻る
5. 冗長排除テスト: モーダル表示中に他領域で「Preparing worktree」が表示されない
6. エラー表示テスト: 失敗ステップに[!]マーク（赤色）とエラーメッセージが表示される

## 推奨MVP範囲

T001〜T010の完了でユーザーストーリー15の全機能が実装される。
