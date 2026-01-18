# TDDテストケース設計: tmuxマルチモードサポート

**仕様ID**: `SPEC-b7bde3ff`
**作成日**: 2026-01-18
**更新日**: 2026-01-18

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

crates/gwt-cli/src/
├── execution_mode.rs -> tests/execution_mode_tests.rs
└── ui/
    ├── pane_list.rs  -> tests/ui_pane_list_tests.rs
    └── split_layout.rs -> tests/ui_split_layout_tests.rs
```

---

## ユニットテスト

### 1. 環境検出テスト（FR-001〜FR-004）

**ファイル**: `crates/gwt-core/src/tmux/detector.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_tmux_environment_with_env_var() {
        // FR-001: TMUX環境変数が設定されている場合、trueを返す
        std::env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");
        assert!(is_inside_tmux());
        std::env::remove_var("TMUX");
    }

    #[test]
    fn test_detect_tmux_environment_without_env_var() {
        // FR-001: TMUX環境変数が未設定の場合、falseを返す
        std::env::remove_var("TMUX");
        assert!(!is_inside_tmux());
    }

    #[test]
    fn test_tmux_command_exists() {
        // FR-004: tmuxコマンドの存在確認
        let result = check_tmux_installed();
        // 実行環境依存のため、結果のみ確認
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_tmux_version_check() {
        // 制約: tmux 2.0以上が必要
        if let Ok(version) = get_tmux_version() {
            assert!(version.major >= 2);
        }
    }
}
```

---

### 2. 実行モードテスト（FR-002〜FR-003）

**ファイル**: `crates/gwt-core/src/execution_mode.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_mode_single_outside_tmux() {
        // FR-003: tmux環境外ではシングルモード
        std::env::remove_var("TMUX");
        let mode = ExecutionMode::detect();
        assert_eq!(mode, ExecutionMode::Single);
    }

    #[test]
    fn test_execution_mode_multi_inside_tmux() {
        // FR-002: tmux環境内ではマルチモード
        std::env::set_var("TMUX", "/tmp/tmux-1000/default,12345,0");
        let mode = ExecutionMode::detect();
        assert_eq!(mode, ExecutionMode::Multi);
        std::env::remove_var("TMUX");
    }
}
```

---

### 3. セッション命名テスト（FR-010〜FR-011）

**ファイル**: `crates/gwt-core/src/tmux/naming.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_name_from_repo() {
        // FR-010: gwt-{リポジトリ名}形式
        let name = generate_session_name("my-awesome-repo", &[]);
        assert_eq!(name, "gwt-my-awesome-repo");
    }

    #[test]
    fn test_session_name_with_existing_session() {
        // FR-011: 既存セッションがある場合は番号付与
        let existing = vec!["gwt-my-repo".to_string()];
        let name = generate_session_name("my-repo", &existing);
        assert_eq!(name, "gwt-my-repo-2");
    }

    #[test]
    fn test_session_name_with_multiple_existing() {
        // FR-011: 複数の既存セッションがある場合
        let existing = vec![
            "gwt-my-repo".to_string(),
            "gwt-my-repo-2".to_string(),
            "gwt-my-repo-3".to_string(),
        ];
        let name = generate_session_name("my-repo", &existing);
        assert_eq!(name, "gwt-my-repo-4");
    }

    #[test]
    fn test_session_name_sanitization() {
        // リポジトリ名に特殊文字がある場合
        let name = generate_session_name("my/repo@v1", &[]);
        assert_eq!(name, "gwt-my-repo-v1");
    }
}
```

---

### 4. ペイン管理テスト（FR-020〜FR-024）

**ファイル**: `crates/gwt-core/src/tmux/pane.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_pane_creation() {
        // FR-020: AgentPane構造体の作成
        let pane = AgentPane::new(
            "1".to_string(),
            "feature/test".to_string(),
            "claude".to_string(),
            std::time::SystemTime::now(),
            12345,
        );
        assert_eq!(pane.branch_name, "feature/test");
        assert_eq!(pane.agent_name, "claude");
    }

    #[test]
    fn test_pane_uptime_calculation() {
        // FR-031: 稼働時間の計算
        let start = std::time::SystemTime::now() - std::time::Duration::from_secs(3661);
        let pane = AgentPane::new(
            "1".to_string(),
            "main".to_string(),
            "codex".to_string(),
            start,
            12345,
        );
        let uptime = pane.uptime_string();
        assert!(uptime.contains("1h"));
    }

    #[test]
    fn test_parse_pane_list_output() {
        // tmux list-panes出力のパース
        let output = "0:12345:bash\n1:12346:claude\n2:12347:codex";
        let panes = parse_pane_list(output);
        assert_eq!(panes.len(), 3);
        assert_eq!(panes[0].pane_id, "0");
        assert_eq!(panes[1].pane_id, "1");
    }
}
```

---

### 5. フォーカス切り替えテスト（FR-040〜FR-042）

**ファイル**: `crates/gwt-cli/src/ui/focus_manager.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_focus_toggle_branch_to_pane() {
        // FR-042: Tabでブランチ一覧→ペイン一覧
        let mut focus = FocusState::BranchList;
        focus.toggle();
        assert_eq!(focus, FocusState::PaneList);
    }

    #[test]
    fn test_focus_toggle_pane_to_branch() {
        // FR-042: Tabでペイン一覧→ブランチ一覧
        let mut focus = FocusState::PaneList;
        focus.toggle();
        assert_eq!(focus, FocusState::BranchList);
    }
}
```

---

### 6. 終了確認テスト（FR-050〜FR-052, FR-060〜FR-061）

**ファイル**: `crates/gwt-core/src/tmux/terminate.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminate_confirmation_required() {
        // FR-050: 終了前に確認が必要
        let pane = AgentPane::new(/* ... */);
        assert!(pane.requires_termination_confirmation());
    }

    #[test]
    fn test_gwt_exit_confirmation_with_agents() {
        // FR-060: エージェント稼働中のgwt終了確認
        let agents = vec![AgentPane::new(/* ... */)];
        assert!(requires_exit_confirmation(&agents));
    }

    #[test]
    fn test_gwt_exit_no_confirmation_without_agents() {
        // FR-060: エージェントなしの場合は確認不要
        let agents: Vec<AgentPane> = vec![];
        assert!(!requires_exit_confirmation(&agents));
    }
}
```

---

### 7. 多重起動警告テスト（FR-080〜FR-081）

**ファイル**: `crates/gwt-cli/src/ui/dialogs/duplicate_warn.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_duplicate_launch() {
        // FR-080: 同一WT+同一エージェントの検出
        let running = vec![
            AgentPane::new("1".into(), "feature/a".into(), "claude".into(), /* ... */),
        ];
        assert!(is_duplicate_launch("feature/a", "claude", &running));
    }

    #[test]
    fn test_no_duplicate_different_branch() {
        // 異なるブランチは重複ではない
        let running = vec![
            AgentPane::new("1".into(), "feature/a".into(), "claude".into(), /* ... */),
        ];
        assert!(!is_duplicate_launch("feature/b", "claude", &running));
    }

    #[test]
    fn test_no_duplicate_different_agent() {
        // 異なるエージェントは重複ではない
        let running = vec![
            AgentPane::new("1".into(), "feature/a".into(), "claude".into(), /* ... */),
        ];
        assert!(!is_duplicate_launch("feature/a", "codex", &running));
    }
}
```

---

### 8. キーバインド変更テスト（FR-100〜FR-101）

**ファイル**: `crates/gwt-cli/src/ui/keybindings.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn test_mode_switch_key_is_m() {
        // FR-100: モード切り替えはmキー
        let event = KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE);
        assert!(is_mode_switch_key(&event));
    }

    #[test]
    fn test_tab_key_is_focus_switch() {
        // FR-101: Tabはフォーカス切り替え
        let event = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert!(is_focus_switch_key(&event));
    }

    #[test]
    fn test_tab_key_is_not_mode_switch() {
        // FR-100: Tabはモード切り替えではない
        let event = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
        assert!(!is_mode_switch_key(&event));
    }
}
```

---

### 9. エージェント列/行レイアウトテスト（FR-033〜FR-035）

**ファイル**: `crates/gwt-core/src/tmux/pane.rs` / `crates/gwt-cli/src/tui/app.rs`

**テスト観点**:

- 右側列内の最大3行制約が守られる
- 3行到達時に新規列が作成される
- 列内の高さが均等化される
- 列数に応じて幅が均等化される

---

### 10. エージェント表示名短縮テスト（FR-026）

**ファイル**: `crates/gwt-core/src/agent/mod.rs` / `crates/gwt-cli/src/tui/screens/wizard.rs`

**テスト観点**:

- Codex CLI → Codex の表示名統一
- Gemini CLI → Gemini の表示名統一

---

### 11. ペイン表示時のmouse有効化テスト（FR-093）

**ファイル**: `crates/gwt-core/src/tmux/pane.rs`

**テスト観点**:

- ペイン表示時に `tmux set -g mouse on` が実行される

---

## インテグレーションテスト

### 12. tmuxセッション統合テスト

**ファイル**: `tests/tmux_integration_tests.rs`

```rust
#[cfg(test)]
mod tests {
    use gwt_core::tmux::*;

    #[test]
    #[ignore] // tmux環境が必要
    fn test_create_and_destroy_session() {
        let session_name = "gwt-test-session";

        // セッション作成
        let result = create_session(session_name);
        assert!(result.is_ok());

        // セッション存在確認
        let sessions = list_sessions().unwrap();
        assert!(sessions.contains(&session_name.to_string()));

        // セッション削除
        let result = destroy_session(session_name);
        assert!(result.is_ok());

        // 削除確認
        let sessions = list_sessions().unwrap();
        assert!(!sessions.contains(&session_name.to_string()));
    }

    #[test]
    #[ignore] // tmux環境が必要
    fn test_create_pane_in_session() {
        let session_name = "gwt-test-pane";
        create_session(session_name).unwrap();

        // ペイン作成
        let result = create_pane(session_name, "/tmp", "echo test");
        assert!(result.is_ok());

        // ペイン一覧確認
        let panes = list_panes(session_name).unwrap();
        assert!(panes.len() >= 2); // マスター + 新規ペイン

        destroy_session(session_name).unwrap();
    }
}
```

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
