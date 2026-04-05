### Phase 1

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-001 | `register_with_default_settings_is_enabled` → `gwt-issue-resolve` が利用可能 | registration |
| TDD-002 | `project_scoped_registration_writes_all_agents` → `.codex/skills/gwt-spec-register/SKILL.md` が生成 | registration |
| TDD-003 | コマンド `.claude/commands/gwt-spec-register.md` が存在 | registration |
| TDD-004 | コマンド `.claude/commands/gwt-spec-ops.md` が参照可能 | registration |
| TDD-005 | `claude_registration_writes_local_assets_even_when_plugin_is_enabled` 通過 | registration |
| TDD-006 | `refresh_skill_registration_for_project_root_repairs_assets_with_default_settings` 通過 | refresh |
| TDD-007 | `python3 -m py_compile .claude/skills/gwt-issue-resolve/scripts/inspect_issue.py` 通過 | script |

### Phase 2: スキル登録

#### Rust テスト(`skill_registration.rs`)

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-P2-01 | `generate_managed_skills_block_contains_all_skills` | 全スキル含有チェック |
| TDD-P2-02 | `inject_managed_skills_block_appends_to_content_without_block` | ブロック追加 |
| TDD-P2-03 | `inject_managed_skills_block_replaces_existing_block` | ブロック置換 |
| TDD-P2-04 | `inject_managed_skills_block_is_idempotent` | 冪等性 |
| TDD-P2-05 | `inject_managed_skills_block_rejects_unterminated_begin` | BEGIN without END |
| TDD-P2-06 | `inject_managed_skills_block_rejects_orphan_end` | END without BEGIN |
| TDD-P2-07 | `inject_managed_skills_block_handles_empty_content` | 空コンテンツ |
| TDD-P2-08 | `skill_catalog_matches_project_skill_assets` | カタログ一致 |

#### Rust テスト(`settings.rs`)

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-P2-09 | `skill_registration_preferences_inject_defaults` | デフォルト設定 |
| TDD-P2-10 | `settings_data_inject_round_trip` | 設定往復 |

#### Rust テスト(`clause_docs.rs`)

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-P2-11 | `check_and_fix_creates_docs_with_skills_block` | CLAUDE.md 新規作成 |
| TDD-P2-12 | `check_and_fix_updates_existing_claude_md_with_skills_block` | CLAUDE.md 更新 |
| TDD-P2-13 | `check_and_fix_replaces_outdated_skills_block` | 古いブロック置換 |
| TDD-P2-14 | `agents_md_not_injected_by_default` | AGENTS.md 非注入 |

#### フロントエンド(`SettingsPanel.test.ts`)

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-P2-15 | `agent_tab_renders_docs_injection_checkboxes` | チェックボックス表示 |
| TDD-P2-16 | `agent_tab_claude_md_checked_by_default` | デフォルトチェック |
| TDD-P2-17 | `agent_tab_saves_injection_settings` | 設定保存 |

#### build.rs 検証

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-P2-18 | `build_script_generates_catalog_from_skill_md` | カタログ自動生成 |

### Phase 5: GitHub Transport Policy (planning-ready)

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-P5-01 | REST token auth probe で `GH_TOKEN` / `GITHUB_TOKEN` を検出 | auth |
| TDD-P5-02 | `gwt-pr` が REST で PR list/create/update/view を実行 | pr |
| TDD-P5-03 | `gwt-pr-check` が REST で head branch PR を検索 | pr-check |
| TDD-P5-04 | `gwt-pr-fix` が REST で checks/reviews/comments を取得 | pr-fix |
| TDD-P5-05 | unresolved review threads と thread reply/resolve が GraphQL のみ | graphql-boundary |

### Phase: Artifact-first Storage/API

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-S-01 | `doc:*` artifact parsing and mixed-mode precedence | storage |
| TDD-S-02 | Legacy body-canonical spec issues remain readable | storage |
| TDD-S-03 | `doc`, `contract`, `checklist` share same CRUD family | storage |

### Phase: Completion Gate

| ID | テスト | スコープ |
|----|---------|---------|
| TDD-C-01 | Completion-gate semantics verified in workflow-owned docs/scripts | completion |
| TDD-C-02 | Malformed or stale checklist blocking behavior | completion |
| TDD-C-03 | #1654 artifact rollback and revalidation as remediation acceptance case | completion |

---
