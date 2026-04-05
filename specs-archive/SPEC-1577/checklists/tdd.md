### テストシナリオ

| ID | シナリオ | カテゴリ |
|----|---------|---------| 
| TDD-001 | ツールレジストリに全カテゴリのツールが登録されている | Setup |
| TDD-002 | Responses API の tools 形式に準拠した JSON Schema が生成される | Setup |
| TDD-003 | `codebase_read_file` が指定パスのファイル内容を返す | コード参照 |
| TDD-004 | `codebase_search` が検索結果を構造化 JSON で返す | コード参照 |
| TDD-005 | `git_worktree_create` が worktree を作成し結果を返す | Git |
| TDD-006 | `git_worktree_remove` が destructive 権限チェックを要求する | Git |
| TDD-007 | force push / rebase ツールがレジストリに存在しない | Git |
| TDD-008 | `github_read_issue` が Issue 内容を構造化 JSON で返す | GitHub |
| TDD-009 | `github_create_issue` が gwt-spec ラベル付きで Issue を作成する | GitHub |
| TDD-010 | `agent_hire` が Agent プロセスを起動し、セッション ID を返す | Agent |
| TDD-011 | `pty_capture_scrollback` が最新出力を返す | PTY |
| TDD-012 | `spec_generate_section` が指定セクションを gwt-spec-ops テンプレート準拠で生成する | SPEC |
| TDD-013 | `spec_check_consistency` が重複 SPEC を検出する | SPEC |
| TDD-014 | destructive ツール呼び出し時にユーザー承認が要求される | 権限 |
| TDD-015 | ツール実行タイムアウトがエラー構造体を返す | 共通 |
| TDD-016 | 不正な引数でツール呼び出し時にバリデーションエラーが返る | 共通 |

### テストコード（RED 状態）

```rust
// assistant_tool_registry_tests.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_tool_categories() {
        let registry = AssistantToolRegistry::new();
        let categories = [
            "codebase", "git", "github", "agent", "pty", "spec", "session", "assistant",
        ];
        for category in &categories {
            assert!(
                !registry.get_tools_by_category(category).is_empty(),
                "Category '{}' should have at least one tool",
                category
            );
        }
    }

    #[test]
    fn registry_does_not_contain_forbidden_tools() {
        let registry = AssistantToolRegistry::new();
        let all_tools = registry.get_all_tools();
        assert!(!all_tools.iter().any(|t| t.name == "git_force_push"));
        assert!(!all_tools.iter().any(|t| t.name == "git_rebase"));
    }

    #[test]
    fn registry_total_tool_count_does_not_exceed_40() {
        let registry = AssistantToolRegistry::new();
        assert!(registry.get_all_tools().len() <= 40);
    }

    #[test]
    fn generate_json_schema_produces_valid_responses_api_tools_format() {
        let registry = AssistantToolRegistry::new();
        let schemas = registry.generate_tool_schemas();
        for schema in &schemas {
            assert!(schema.get("type").is_some());
            assert!(schema.get("name").is_some());
            assert!(schema.get("description").is_some());
            assert!(schema.get("parameters").is_some());
            let params = &schema["parameters"];
            assert_eq!(params["type"], "object");
        }
    }

    #[test]
    fn destructive_tool_requires_user_approval() {
        let registry = AssistantToolRegistry::new();
        let destructive_tools: Vec<_> = registry
            .get_all_tools()
            .iter()
            .filter(|t| t.permission_level == ToolPermissionLevel::Destructive)
            .collect();
        for tool in destructive_tools {
            assert!(
                tool.requires_user_approval,
                "Destructive tool '{}' must require user approval",
                tool.name
            );
        }
    }

    #[test]
    fn tool_execution_with_invalid_args_returns_error_structure() {
        let handler = CodebaseReadFileHandler::new();
        let result = handler.execute_sync(serde_json::json!({})); // missing required 'path'
        assert!(result.is_error);
        assert!(!result.error_code.is_empty());
    }

    #[test]
    fn tool_execution_timeout_returns_timeout_error() {
        let handler = SlowToolHandler::new(); // test double
        let result = handler.execute_with_timeout(std::time::Duration::from_millis(1));
        assert!(result.is_error);
        assert_eq!(result.error_code, "TIMEOUT");
    }

    #[test]
    fn spec_generate_section_produces_template_compliant_output() {
        let handler = SpecGenerateSectionHandler::new();
        let result = handler.execute_sync(serde_json::json!({
            "section": "Spec",
            "context": { "background": "Test feature", "scenarios": ["US-1"] }
        }));
        assert!(!result.is_error);
        let content = result.data["content"].as_str().unwrap();
        assert!(content.contains("### 背景"));
        assert!(content.contains("### ユーザーシナリオ"));
        assert!(content.contains("### 機能要件"));
        assert!(content.contains("### 成功基準"));
    }

    #[test]
    fn spec_check_consistency_detects_duplicate() {
        let handler = SpecCheckConsistencyHandler::new();
        let result = handler.execute_sync(serde_json::json!({
            "new_spec_title": "HUD & UI システム",
            "existing_specs": [
                { "number": 1548, "title": "機能仕様: HUD & UI システム" }
            ]
        }));
        assert!(!result.is_error);
        assert!(!result.data["duplicates"].as_array().unwrap().is_empty());
    }
}
```

---
