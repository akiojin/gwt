# タスク一覧: Worktree作成進捗モーダル

**仕様ID**: `SPEC-bf7fb63e`
**作成日**: 2026-01-25

## タスク依存関係

```text
TASK-001 → TASK-002 → TASK-003 → TASK-004
                ↘ TASK-005 ↘
                   TASK-006 → TASK-007 → TASK-008
                               ↓
                            TASK-009 → TASK-010
```

## タスク一覧

### TASK-001: ProgressStepKind列挙型の追加

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/main.rs`

**実装内容**:

- ProgressStepKind列挙型を定義
- バリアント: Fetch, ValidateBranch, GeneratePath, CheckConflicts, CreateWorktree, CheckDependencies
- 各バリアントのラベル文字列を返すメソッド実装

**TDDテスト**:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_step_kind_label() {
        assert_eq!(ProgressStepKind::Fetch.label(), "Fetching remote...");
        assert_eq!(ProgressStepKind::ValidateBranch.label(), "Validating branch...");
        assert_eq!(ProgressStepKind::GeneratePath.label(), "Generating path...");
        assert_eq!(ProgressStepKind::CheckConflicts.label(), "Checking conflicts...");
        assert_eq!(ProgressStepKind::CreateWorktree.label(), "Creating worktree...");
        assert_eq!(ProgressStepKind::CheckDependencies.label(), "Checking dependencies...");
    }

    #[test]
    fn test_progress_step_kind_all_steps() {
        let steps = ProgressStepKind::all();
        assert_eq!(steps.len(), 6);
    }
}
```

**受け入れ基準**:

- [ ] 6種類のステップが定義されている
- [ ] 各ステップのラベル文字列が英語で返される
- [ ] テストが全てパスする

---

### TASK-002: ProgressStep構造体の追加

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/main.rs`
**依存**: TASK-001

**実装内容**:

- ProgressStep構造体を定義
- フィールド: kind, status, started_at, error_message
- StepStatus列挙型: Pending, Running, Completed, Failed

**TDDテスト**:

```rust
#[test]
fn test_progress_step_new() {
    let step = ProgressStep::new(ProgressStepKind::Fetch);
    assert_eq!(step.status, StepStatus::Pending);
    assert!(step.started_at.is_none());
    assert!(step.error_message.is_none());
}

#[test]
fn test_progress_step_start() {
    let mut step = ProgressStep::new(ProgressStepKind::Fetch);
    step.start();
    assert_eq!(step.status, StepStatus::Running);
    assert!(step.started_at.is_some());
}

#[test]
fn test_progress_step_complete() {
    let mut step = ProgressStep::new(ProgressStepKind::Fetch);
    step.start();
    step.complete();
    assert_eq!(step.status, StepStatus::Completed);
}

#[test]
fn test_progress_step_fail() {
    let mut step = ProgressStep::new(ProgressStepKind::Fetch);
    step.start();
    step.fail("Network error".to_string());
    assert_eq!(step.status, StepStatus::Failed);
    assert_eq!(step.error_message, Some("Network error".to_string()));
}

#[test]
fn test_progress_step_elapsed_secs() {
    let mut step = ProgressStep::new(ProgressStepKind::Fetch);
    step.start();
    // テスト用に即座に経過時間を確認（0秒付近）
    assert!(step.elapsed_secs().unwrap() < 1.0);
}
```

**受け入れ基準**:

- [ ] ProgressStep構造体が定義されている
- [ ] 状態遷移メソッド（start, complete, fail）が実装されている
- [ ] 経過時間計算が実装されている
- [ ] テストが全てパスする

---

### TASK-003: ProgressStepの表示記号メソッド追加

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/main.rs`
**依存**: TASK-002

**実装内容**:

- ステータスに応じた表示記号を返すメソッド
- [x] 完了、[>] 進行中、[ ] 待機中、[!] 失敗

**TDDテスト**:

```rust
#[test]
fn test_progress_step_marker() {
    let mut step = ProgressStep::new(ProgressStepKind::Fetch);
    assert_eq!(step.marker(), "[ ]");

    step.start();
    assert_eq!(step.marker(), "[>]");

    step.complete();
    assert_eq!(step.marker(), "[x]");
}

#[test]
fn test_progress_step_marker_failed() {
    let mut step = ProgressStep::new(ProgressStepKind::Fetch);
    step.start();
    step.fail("Error".to_string());
    assert_eq!(step.marker(), "[!]");
}
```

**受け入れ基準**:

- [ ] 各ステータスに応じた記号が返される
- [ ] ASCII文字のみ使用（絵文字不使用）
- [ ] テストが全てパスする

---

### TASK-004: 経過時間表示の閾値判定

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/main.rs`
**依存**: TASK-002

**実装内容**:

- 3秒以上経過したかを判定するメソッド
- 経過時間フォーマット（X.Xs形式）

**TDDテスト**:

```rust
#[test]
fn test_progress_step_should_show_elapsed() {
    let mut step = ProgressStep::new(ProgressStepKind::Fetch);
    step.start();
    // 3秒未満は表示しない
    assert!(!step.should_show_elapsed());
}

#[test]
fn test_progress_step_format_elapsed() {
    // 5.2秒経過を想定
    let elapsed = 5.2;
    let formatted = format_elapsed(elapsed);
    assert_eq!(formatted, "5.2s");
}
```

**受け入れ基準**:

- [ ] 3秒閾値の判定が実装されている
- [ ] 経過時間が「X.Xs」形式でフォーマットされる
- [ ] テストが全てパスする

---

### TASK-005: ProgressModalウィジェット基本構造

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/tui/widgets/progress_modal.rs`（新規）
**依存**: TASK-002

**実装内容**:

- ProgressModalウィジェット構造体
- ratatui Widgetトレイト実装
- 基本的なレイアウト計算（中央配置、幅80文字以上）

**TDDテスト**:

```rust
#[test]
fn test_progress_modal_layout() {
    let modal = ProgressModal::new(&steps, "Preparing Worktree");
    let area = Rect::new(0, 0, 120, 40);
    let modal_area = modal.calculate_area(area);

    // 幅が80以上
    assert!(modal_area.width >= 80);
    // 中央配置
    assert_eq!(modal_area.x, (120 - modal_area.width) / 2);
}
```

**受け入れ基準**:

- [ ] ウィジェット構造体が定義されている
- [ ] 中央配置のレイアウト計算が実装されている
- [ ] テストが全てパスする

---

### TASK-006: ProgressModalの描画実装

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/tui/widgets/progress_modal.rs`
**依存**: TASK-005

**実装内容**:

- 半透明オーバーレイ描画
- ボーダー付きボックス描画
- タイトル描画
- ステップリスト描画

**TDDテスト**:

```rust
#[test]
fn test_progress_modal_render_steps() {
    let steps = vec![
        ProgressStep::completed(ProgressStepKind::Fetch),
        ProgressStep::running(ProgressStepKind::ValidateBranch),
        ProgressStep::pending(ProgressStepKind::GeneratePath),
    ];
    let modal = ProgressModal::new(&steps, "Test");

    // 描画結果の検証（バッファ検査）
    let mut buf = Buffer::empty(Rect::new(0, 0, 100, 20));
    modal.render(Rect::new(10, 5, 80, 10), &mut buf);

    // ステップマーカーが含まれる
    let content = buf_to_string(&buf);
    assert!(content.contains("[x]"));
    assert!(content.contains("[>]"));
    assert!(content.contains("[ ]"));
}
```

**受け入れ基準**:

- [ ] オーバーレイが描画される
- [ ] ステップリストが描画される
- [ ] 色分けが実装されている
- [ ] テストが全てパスする

---

### TASK-007: App構造体への状態追加

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/tui/app.rs`
**依存**: TASK-006

**実装内容**:

- progress_modal_visible: bool
- progress_steps: Vec<ProgressStep>
- progress_start_time: Option<Instant>

**TDDテスト**:

```rust
#[test]
fn test_app_progress_modal_initial_state() {
    let app = App::new();
    assert!(!app.progress_modal_visible);
    assert!(app.progress_steps.is_empty());
}

#[test]
fn test_app_show_progress_modal() {
    let mut app = App::new();
    app.show_progress_modal();
    assert!(app.progress_modal_visible);
    assert_eq!(app.progress_steps.len(), 6); // 全ステップ初期化
}
```

**受け入れ基準**:

- [ ] 状態フィールドが追加されている
- [ ] 初期状態が正しい
- [ ] テストが全てパスする

---

### TASK-008: ESCキーによるキャンセル実装

**ステータス**: pending
**優先度**: P2
**ファイル**: `crates/gwt-cli/src/tui/app.rs`
**依存**: TASK-007

**実装内容**:

- モーダル表示中のESCキー検出
- キャンセルシグナル送信
- モーダル閉じてブランチ一覧復帰

**TDDテスト**:

```rust
#[test]
fn test_app_esc_cancels_progress_modal() {
    let mut app = App::new();
    app.show_progress_modal();

    let event = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());
    app.handle_key_event(event);

    assert!(!app.progress_modal_visible);
}

#[test]
fn test_app_esc_ignored_when_no_modal() {
    let mut app = App::new();
    // モーダルなし
    let event = KeyEvent::new(KeyCode::Esc, KeyModifiers::empty());
    let result = app.handle_key_event(event);
    // 通常のESC処理が行われる
}
```

**受け入れ基準**:

- [ ] モーダル表示中にESCでキャンセル可能
- [ ] キャンセル後にブランチ一覧に戻る
- [ ] テストが全てパスする

---

### TASK-009: 冗長表示の排除

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/tui/app.rs`
**依存**: TASK-007

**実装内容**:

- ステータスバー表示条件分岐
- ブランチ詳細表示条件分岐
- セッション要約表示条件分岐

**TDDテスト**:

```rust
#[test]
fn test_app_no_redundant_status_when_modal() {
    let app = App::new();
    app.show_progress_modal();

    // ステータスバーに「Preparing worktree」が含まれない
    let status = app.get_status_text();
    assert!(!status.contains("Preparing worktree"));
}
```

**受け入れ基準**:

- [ ] モーダル表示中は3箇所から「Preparing worktree」が消える
- [ ] 通常表示が維持される
- [ ] テストが全てパスする

---

### TASK-010: UI描画への統合

**ステータス**: pending
**優先度**: P1
**ファイル**: `crates/gwt-cli/src/tui/app.rs`
**依存**: TASK-009

**実装内容**:

- ui()関数でモーダル描画を追加
- 最前面レイヤーとして描画
- 入力ブロック実装

**TDDテスト**:

```rust
#[test]
fn test_app_modal_blocks_input() {
    let mut app = App::new();
    app.show_progress_modal();

    // カーソル移動が無効
    let event = KeyEvent::new(KeyCode::Down, KeyModifiers::empty());
    let handled = app.handle_key_event(event);

    // 入力がブロックされた
    assert!(handled);
    // カーソル位置が変わっていない
}
```

**受け入れ基準**:

- [ ] モーダルが最前面に描画される
- [ ] モーダル表示中は他の入力がブロックされる
- [ ] テストが全てパスする

---

## 実装順序

1. TASK-001 → TASK-002 → TASK-003 → TASK-004（型定義）
2. TASK-005 → TASK-006（ウィジェット）
3. TASK-007 → TASK-008（状態・イベント）
4. TASK-009 → TASK-010（統合）

## 完了条件

- [ ] 全タスクのテストがパスする
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` がパスする
- [ ] `cargo fmt --check` がパスする
- [ ] 手動テストでモーダル表示を確認
