# TDDテストケース: エージェント状態の可視化

**仕様ID**: `SPEC-861d8cdf`
**作成日**: 2026-01-20

## テストケース一覧

### T-100: AgentStatus列挙型とSessionフィールド追加

#### T-100-01: AgentStatus列挙型の定義

```rust
#[test]
fn test_agent_status_default() {
    let status = AgentStatus::default();
    assert_eq!(status, AgentStatus::Unknown);
}

#[test]
fn test_agent_status_serialize_deserialize() {
    let status = AgentStatus::Running;
    let json = serde_json::to_string(&status).unwrap();
    assert_eq!(json, "\"running\"");
    
    let deserialized: AgentStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, AgentStatus::Running);
}

#[test]
fn test_agent_status_all_variants() {
    let variants = [
        (AgentStatus::Unknown, "\"unknown\""),
        (AgentStatus::Running, "\"running\""),
        (AgentStatus::WaitingInput, "\"waiting_input\""),
        (AgentStatus::Stopped, "\"stopped\""),
    ];
    
    for (status, expected_json) in variants {
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, expected_json);
    }
}
```

#### T-100-02: Session構造体のフィールド追加

```rust
#[test]
fn test_session_with_status_field() {
    let session = Session::new(PathBuf::from("/test/path"), "test-branch");
    assert_eq!(session.status, AgentStatus::Unknown);
    assert!(session.last_activity_at.is_none());
}

#[test]
fn test_session_status_update() {
    let mut session = Session::new(PathBuf::from("/test/path"), "test-branch");
    session.status = AgentStatus::Running;
    session.last_activity_at = Some(Utc::now());
    
    assert_eq!(session.status, AgentStatus::Running);
    assert!(session.last_activity_at.is_some());
}
```

#### T-100-03: 後方互換性

```rust
#[test]
fn test_session_load_without_status_field() {
    // 古い形式のTOML（statusフィールドなし）
    let toml_content = r#"
id = "test-id"
worktree_path = "/test/path"
branch = "test-branch"
created_at = "2026-01-20T00:00:00Z"
updated_at = "2026-01-20T00:00:00Z"
"#;
    
    let session: Session = toml::from_str(toml_content).unwrap();
    assert_eq!(session.status, AgentStatus::Unknown);
}
```

#### T-100-04: 60秒経過による自動stopped判定

```rust
#[test]
fn test_session_auto_stopped_after_60_seconds() {
    let mut session = Session::new(PathBuf::from("/test/path"), "test-branch");
    session.status = AgentStatus::Running;
    session.last_activity_at = Some(Utc::now() - chrono::Duration::seconds(61));
    
    assert!(session.should_mark_stopped());
}

#[test]
fn test_session_not_stopped_within_60_seconds() {
    let mut session = Session::new(PathBuf::from("/test/path"), "test-branch");
    session.status = AgentStatus::Running;
    session.last_activity_at = Some(Utc::now() - chrono::Duration::seconds(30));
    
    assert!(!session.should_mark_stopped());
}
```

---

### T-101: gwt hookサブコマンドの実装

#### T-101-01: UserPromptSubmitイベント処理

```rust
#[test]
fn test_hook_user_prompt_submit_sets_running() {
    let payload = r#"{"session_id": "test-123", "cwd": "/test/worktree"}"#;
    let result = process_hook_event("UserPromptSubmit", payload);
    
    assert!(result.is_ok());
    let session = Session::load_for_worktree(Path::new("/test/worktree")).unwrap();
    assert_eq!(session.status, AgentStatus::Running);
}
```

#### T-101-02: Stopイベント処理

```rust
#[test]
fn test_hook_stop_sets_stopped() {
    let payload = r#"{"session_id": "test-123", "cwd": "/test/worktree"}"#;
    let result = process_hook_event("Stop", payload);
    
    assert!(result.is_ok());
    let session = Session::load_for_worktree(Path::new("/test/worktree")).unwrap();
    assert_eq!(session.status, AgentStatus::Stopped);
}
```

#### T-101-03: Notification[permission_prompt]イベント処理

```rust
#[test]
fn test_hook_notification_permission_prompt_sets_waiting_input() {
    let payload = r#"{"session_id": "test-123", "cwd": "/test/worktree", "notification_type": "permission_prompt"}"#;
    let result = process_hook_event("Notification", payload);
    
    assert!(result.is_ok());
    let session = Session::load_for_worktree(Path::new("/test/worktree")).unwrap();
    assert_eq!(session.status, AgentStatus::WaitingInput);
}
```

#### T-101-04: PreToolUse/PostToolUseイベント処理

```rust
#[test]
fn test_hook_pre_tool_use_sets_running() {
    let payload = r#"{"session_id": "test-123", "cwd": "/test/worktree"}"#;
    let result = process_hook_event("PreToolUse", payload);
    
    assert!(result.is_ok());
    let session = Session::load_for_worktree(Path::new("/test/worktree")).unwrap();
    assert_eq!(session.status, AgentStatus::Running);
}

#[test]
fn test_hook_post_tool_use_sets_running() {
    let payload = r#"{"session_id": "test-123", "cwd": "/test/worktree"}"#;
    let result = process_hook_event("PostToolUse", payload);
    
    assert!(result.is_ok());
    let session = Session::load_for_worktree(Path::new("/test/worktree")).unwrap();
    assert_eq!(session.status, AgentStatus::Running);
}
```

#### T-101-05: 無効なイベント名の処理

```rust
#[test]
fn test_hook_invalid_event_name() {
    let payload = r#"{"session_id": "test-123", "cwd": "/test/worktree"}"#;
    let result = process_hook_event("InvalidEvent", payload);
    
    assert!(result.is_err());
}
```

---

### T-102: Claude Code Hook設定機能

#### T-102-01: settings.json新規作成

```rust
#[test]
fn test_create_claude_settings_if_not_exists() {
    let temp_dir = tempdir().unwrap();
    let settings_path = temp_dir.path().join(".claude/settings.json");
    
    let result = register_gwt_hooks(&settings_path);
    
    assert!(result.is_ok());
    assert!(settings_path.exists());
}
```

#### T-102-02: 既存hooks設定の保持

```rust
#[test]
fn test_preserve_existing_hooks() {
    let temp_dir = tempdir().unwrap();
    let settings_path = temp_dir.path().join(".claude/settings.json");
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    
    let existing_content = r#"{"hooks": {"CustomHook": "custom-command"}}"#;
    fs::write(&settings_path, existing_content).unwrap();
    
    let result = register_gwt_hooks(&settings_path);
    
    assert!(result.is_ok());
    let content = fs::read_to_string(&settings_path).unwrap();
    assert!(content.contains("CustomHook"));
    assert!(content.contains("UserPromptSubmit"));
}
```

#### T-102-03: 5つのイベント登録

```rust
#[test]
fn test_register_all_five_hooks() {
    let temp_dir = tempdir().unwrap();
    let settings_path = temp_dir.path().join(".claude/settings.json");
    
    let result = register_gwt_hooks(&settings_path);
    
    assert!(result.is_ok());
    let content = fs::read_to_string(&settings_path).unwrap();
    assert!(content.contains("UserPromptSubmit"));
    assert!(content.contains("PreToolUse"));
    assert!(content.contains("PostToolUse"));
    assert!(content.contains("Notification"));
    assert!(content.contains("Stop"));
}
```

---

### T-103: 状態表示UIの実装

#### T-103-01: running状態の表示

```rust
#[test]
fn test_render_running_agent_green_spinner() {
    let agent_pane = AgentPane {
        status: AgentStatus::Running,
        is_background: false,
        ..Default::default()
    };
    
    let (icon, color) = agent_pane.status_icon(0);
    
    assert_eq!(color, Color::Green);
    assert!(ACTIVE_SPINNER_FRAMES.contains(&icon.chars().next().unwrap()));
}
```

#### T-103-02: waiting_input状態の表示

```rust
#[test]
fn test_render_waiting_agent_yellow() {
    let agent_pane = AgentPane {
        status: AgentStatus::WaitingInput,
        is_background: false,
        ..Default::default()
    };
    
    let (_, color) = agent_pane.status_icon(0);
    
    assert_eq!(color, Color::Yellow);
}
```

#### T-103-03: stopped状態の表示

```rust
#[test]
fn test_render_stopped_agent_red() {
    let agent_pane = AgentPane {
        status: AgentStatus::Stopped,
        is_background: false,
        ..Default::default()
    };
    
    let (icon, color) = agent_pane.status_icon(0);
    
    assert_eq!(color, Color::Red);
    assert_eq!(icon, "■");
}
```

#### T-103-04: バックグラウンドエージェントの低輝度表示

```rust
#[test]
fn test_render_background_agent_dim() {
    let agent_pane = AgentPane {
        status: AgentStatus::Running,
        is_background: true,
        ..Default::default()
    };
    
    let (_, color) = agent_pane.status_icon(0);
    
    // バックグラウンドは低輝度色
    assert_eq!(color, Color::DarkGray);
}
```

#### T-103-05: アクティブでもstoppedなら赤

```rust
#[test]
fn test_active_stopped_agent_is_red() {
    let agent_pane = AgentPane {
        status: AgentStatus::Stopped,
        is_background: false, // アクティブ
        ..Default::default()
    };
    
    let (_, color) = agent_pane.status_icon(0);
    
    assert_eq!(color, Color::Red);
}
```

#### T-103-06: waiting_inputの点滅（500ms間隔）

```rust
#[test]
fn test_waiting_input_blink_interval() {
    let agent_pane = AgentPane {
        status: AgentStatus::WaitingInput,
        is_background: false,
        ..Default::default()
    };
    
    // 点滅間隔は500ms = 2 spinner_frames (250ms * 2)
    let visible_at_frame_0 = agent_pane.should_show_icon(0);
    let visible_at_frame_2 = agent_pane.should_show_icon(2);
    
    // 点滅のため、フレームによって表示/非表示が切り替わる
    assert_ne!(visible_at_frame_0, visible_at_frame_2);
}
```

---

### T-104: 初回起動時のHookセットアップ提案

#### T-104-01: Hook未設定時の検出

```rust
#[test]
fn test_detect_missing_gwt_hooks() {
    let temp_dir = tempdir().unwrap();
    let settings_path = temp_dir.path().join(".claude/settings.json");
    
    let result = is_gwt_hooks_registered(&settings_path);
    
    assert!(!result);
}
```

#### T-104-02: Hook設定済みの検出

```rust
#[test]
fn test_detect_existing_gwt_hooks() {
    let temp_dir = tempdir().unwrap();
    let settings_path = temp_dir.path().join(".claude/settings.json");
    fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
    
    let content = r#"{"hooks": {"UserPromptSubmit": "gwt hook UserPromptSubmit"}}"#;
    fs::write(&settings_path, content).unwrap();
    
    let result = is_gwt_hooks_registered(&settings_path);
    
    assert!(result);
}
```

---

### T-105: ステータスバーの実装

#### T-105-01: 状態カウントの集計

```rust
#[test]
fn test_status_bar_count() {
    let agents = vec![
        AgentPane { status: AgentStatus::Running, ..Default::default() },
        AgentPane { status: AgentStatus::Running, ..Default::default() },
        AgentPane { status: AgentStatus::WaitingInput, ..Default::default() },
        AgentPane { status: AgentStatus::Stopped, ..Default::default() },
    ];
    
    let summary = StatusBarSummary::from_agents(&agents);
    
    assert_eq!(summary.running_count, 2);
    assert_eq!(summary.waiting_count, 1);
    assert_eq!(summary.stopped_count, 1);
}
```

#### T-105-02: ステータスバーのテキスト生成

```rust
#[test]
fn test_status_bar_text() {
    let summary = StatusBarSummary {
        running_count: 2,
        waiting_count: 1,
        stopped_count: 0,
    };
    
    let text = summary.to_display_text();
    
    assert!(text.contains("2 running"));
    assert!(text.contains("1 waiting"));
}
```

#### T-105-03: waiting強調表示

```rust
#[test]
fn test_status_bar_waiting_highlight() {
    let summary = StatusBarSummary {
        running_count: 1,
        waiting_count: 2,
        stopped_count: 0,
    };
    
    let spans = summary.to_spans();
    
    // waitingの部分が黄色であることを確認
    let waiting_span = spans.iter().find(|s| s.content.contains("waiting")).unwrap();
    assert_eq!(waiting_span.style.fg, Some(Color::Yellow));
}
```

---

### T-106: 他エージェントの状態推測機能

#### T-106-01: プロセス終了でstopped

```rust
#[test]
fn test_detect_stopped_by_process_exit() {
    let agent_pane = AgentPane {
        pid: Some(99999), // 存在しないPID
        ..Default::default()
    };
    
    let inferred = infer_agent_status(&agent_pane, None);
    
    assert_eq!(inferred, AgentStatus::Stopped);
}
```

#### T-106-02: 60秒間出力なしでstopped

```rust
#[test]
fn test_detect_stopped_by_idle() {
    let agent_pane = AgentPane {
        pid: Some(1), // 存在するPID
        last_output_at: Some(Utc::now() - chrono::Duration::seconds(61)),
        ..Default::default()
    };
    
    let inferred = infer_agent_status(&agent_pane, None);
    
    assert_eq!(inferred, AgentStatus::Stopped);
}
```

#### T-106-03: プロンプトパターン検出でwaiting_input

```rust
#[test]
fn test_detect_waiting_by_prompt_pattern() {
    let pane_output = "Processing...\n> ";
    
    let inferred = infer_agent_status_from_output(pane_output);
    
    assert_eq!(inferred, AgentStatus::WaitingInput);
}

#[test]
fn test_detect_waiting_by_arrow_pattern() {
    let pane_output = "Ready\n→ ";
    
    let inferred = infer_agent_status_from_output(pane_output);
    
    assert_eq!(inferred, AgentStatus::WaitingInput);
}
```

#### T-106-04: 推測状態の区別

```rust
#[test]
fn test_inferred_status_marked_as_estimated() {
    let agent_pane = AgentPane {
        agent_name: "codex".to_string(), // Claude Code以外
        ..Default::default()
    };
    
    let (status, is_estimated) = get_agent_status_with_confidence(&agent_pane);
    
    assert!(is_estimated);
}

#[test]
fn test_hook_based_status_not_estimated() {
    let agent_pane = AgentPane {
        agent_name: "claude".to_string(),
        status: AgentStatus::Running, // Hook経由で設定
        ..Default::default()
    };
    
    let (status, is_estimated) = get_agent_status_with_confidence(&agent_pane);
    
    assert!(!is_estimated);
}
```

---

## テスト実行コマンド

```bash
# 全テスト実行
cargo test --package gwt-core --package gwt-cli

# 特定のテストモジュール実行
cargo test agent_status

# テストカバレッジ
cargo tarpaulin --packages gwt-core gwt-cli
```
