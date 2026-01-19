# TDDテストケース設計: tmuxマルチモードサポート

**仕様ID**: `SPEC-b7bde3ff`
**作成日**: 2026-01-18
**更新日**: 2026-01-19

## テストファイル構成

```text
crates/gwt-core/src/tmux/
├── mod.rs
├── detector.rs      -> tests/tmux_detector_tests.rs
├── session.rs       -> tests/tmux_session_tests.rs
├── pane.rs          -> tests/tmux_pane_tests.rs
├── naming.rs        -> tests/tmux_naming_tests.rs
├── keybind.rs       -> tests/tmux_keybind_tests.rs
├── terminate.rs     -> tests/tmux_terminate_tests.rs
├── logging.rs       -> tests/tmux_logging_tests.rs
└── error.rs         -> tests/tmux_error_tests.rs

crates/gwt-core/src/
├── execution_mode.rs -> tests/execution_mode_tests.rs
└── ui/
    ├── pane_list.rs  -> tests/ui_pane_list_tests.rs
    └── split_layout.rs -> tests/ui_split_layout_tests.rs
```

---

## ユニットテスト

### 1. 環境検出テスト（FR-001）

**ファイル**: `crates/gwt-core/src/tmux/detector.rs`

**テスト観点**:
- FR-001: TMUX 環境変数が設定されている場合に `is_inside_tmux()` が `true`
- FR-001: TMUX 環境変数が未設定の場合に `is_inside_tmux()` が `false`
- `check_tmux_installed()` の戻り値が `Ok`/`Err` のいずれでも許容される
- tmux バージョン取得時に major が 2 以上であることを確認（取得可能な場合）

---

### 2. 実行モードテスト（FR-001〜FR-002）

**ファイル**: `crates/gwt-core/src/execution_mode.rs`

**テスト観点**:
- FR-002: tmux 環境外では `ExecutionMode::Single`
- FR-001: tmux 環境内では `ExecutionMode::Multi`

---

### 3. CLI起動モードテスト（FR-001〜FR-003）

**ファイル**: `crates/gwt-cli/src/main.rs`

**テスト観点**:
- FR-001: TMUX 環境変数が設定されている場合にマルチモード判定される
- FR-002: TMUX 環境変数が未設定の場合にシングルモード判定される

---

### 4. セッション命名テスト（FR-010〜FR-011）

**ファイル**: `crates/gwt-core/src/tmux/naming.rs`

**テスト観点**:
- FR-010: `gwt-{repo}` 形式で生成される
- FR-011: 既存セッションがある場合は連番が付与される
- 既存セッションが複数ある場合は最大番号 +1 になる
- リポジトリ名に特殊文字がある場合は安全な表記に正規化される

---

### 5. ペイン管理テスト（FR-020〜FR-024）

**ファイル**: `crates/gwt-core/src/tmux/pane.rs`

**テスト観点**:
- FR-020: `AgentPane::new()` が入力を保持する
- FR-031: `AgentPane::uptime_string()` が期待するフォーマットを返す
- tmux の `list-panes` 出力を正しくパースできる

---

### 6. フォーカス切り替えテスト（FR-040〜FR-042）

**ファイル**: `crates/gwt-cli/src/ui/focus_manager.rs`

**テスト観点**:
- FR-042: Tab でブランチ一覧 → ペイン一覧へ遷移
- FR-042: Tab でペイン一覧 → ブランチ一覧へ遷移

---

### 7. 終了確認テスト（FR-050〜FR-052, FR-060〜FR-061）

**ファイル**: `crates/gwt-core/src/tmux/terminate.rs`

**テスト観点**:
- FR-050: エージェント終了前に確認が必要
- FR-060: エージェント稼働中は gwt 終了確認が必要
- FR-060: エージェントがいない場合は終了確認不要

---

### 8. 多重起動警告テスト（FR-080〜FR-081）

**ファイル**: `crates/gwt-cli/src/ui/dialogs/duplicate_warn.rs`

**テスト観点**:
- FR-080: 同一 worktree + 同一エージェントの重複検出
- 異なるブランチは重複扱いしない
- 異なるエージェントは重複扱いしない

---

### 9. キーバインド変更テスト（FR-100〜FR-101）

**ファイル**: `crates/gwt-cli/src/ui/keybindings.rs`

**テスト観点**:
- FR-100: モード切り替えキーが `m` である
- FR-101: Tab がフォーカス切り替えに使われる
- Tab はモード切り替えとして扱われない

---

### 10. エージェント列/行レイアウトテスト（FR-033〜FR-035）

**ファイル**: `crates/gwt-core/src/tmux/pane.rs` / `crates/gwt-cli/src/tui/app.rs`

**テスト観点**:

- 右側列内の最大3行制約が守られる
- 3行到達時に新規列が作成される
- 列内の高さが均等化される
- 列数に応じて幅が均等化される

---

### 11. エージェント表示名短縮テスト（FR-026）

**ファイル**: `crates/gwt-core/src/agent/mod.rs` / `crates/gwt-cli/src/tui/screens/wizard.rs`

**テスト観点**:

- Codex CLI → Codex の表示名統一
- Gemini CLI → Gemini の表示名統一

---

### 12. ペイン表示時のmouse有効化テスト（FR-093）

**ファイル**: `crates/gwt-core/src/tmux/pane.rs`

**テスト観点**:

- ペイン表示時に `tmux set -g mouse on` が実行される
- join-paneの`-P`/`-F`未対応環境でも再表示が成功し、pane_idが維持される

---

## インテグレーションテスト

### 13. tmuxセッション統合テスト

**ファイル**: `tests/tmux_integration_tests.rs`

**テスト観点**:
- tmux 環境が必要なため `#[ignore]` を付与
- セッション作成 → 存在確認 → 削除までの一連の流れ
- セッション内でのペイン作成と一覧取得

---

## 成功基準との対応

| 成功基準 | テストケース |
|----------|-------------|
| SC-001: 環境検出100ms以内 | test_detect_tmux_environment_* |
| SC-002: ペイン作成500ms以内 | test_create_pane_in_session |
| SC-003: ペイン一覧更新100ms以内 | test_parse_pane_list_output |
| SC-004: Ctrl-g復帰200ms以内 | 手動テスト |
| SC-005: 90%が初回で理解 | ユーザビリティテスト |
| SC-006: Cancel時エージェント継続 | test_gwt_exit_no_confirmation_* |
