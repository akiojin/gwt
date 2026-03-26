### 統合済み CLOSED SPECs 要約

| SPEC | タイトル | 主要要件 | 統合先 FR |
|------|---------|---------|----------|
| #1317 | Launch Agent デフォルト設定保持 | localStorage 永続化（`gwt.launchAgentDefaults.v1`）、Launch 成功時のみ保存、`sanitizeLaunchDefaults()` | FR-009〜FR-012 |
| #1306 | Windows シェル選択 | `get_available_shells` でシェル検出、Advanced セクション select、Docker 時 disabled | FR-020 |
| #1337 | From Issue ブランチプレフィックス AI 判定 | `determinePrefixFromLabels()` → キャッシュ → `classify_issue_branch_prefix` AI → ドロップダウン | FR-018 |

### 関連 SPEC との境界

| SPEC | 本 SPEC (#1692) の責務 | 他 SPEC の責務 |
|------|----------------------|---------------|
| #1646 (Agent 管理) | ダイアログ UI、バリデーション、起動パイプライン、設定永続化 | エージェント検出、ライフサイクル管理、バージョン管理、異常監視 |

### 実装ソースファイル一覧

**Frontend:**

| ファイル | 役割 |
|---------|------|
| `gwt-gui/src/lib/components/AgentLaunchForm.svelte` | メインダイアログコンポーネント |
| `gwt-gui/src/lib/components/agentLaunchFormHelpers.ts` | ヘルパー関数（パース、バリデーション、Docker ヒント） |
| `gwt-gui/src/lib/agentLaunchDefaults.ts` | localStorage 永続化・復元・サニタイズ |
| `gwt-gui/src/lib/agentUtils.ts` | `determinePrefixFromLabels()`, `inferAgentId()` |
| `gwt-gui/src/lib/components/LaunchProgressModal.svelte` | 7ステップ進捗モーダル |

**Backend:**

| ファイル | 役割 |
|---------|------|
| `crates/gwt-tauri/src/commands/terminal.rs` | `start_launch_job`, `cancel_launch_job`, `poll_launch_job`, `launch_agent`, 7ステップパイプライン、CLI フラグ解決 |
| `crates/gwt-tauri/src/commands/agents.rs` | `detect_agents`, `list_agent_versions` |
| `crates/gwt-tauri/src/commands/issue.rs` | `fetch_github_issues`, `check_gh_cli_status`, `find_existing_issue_branches_bulk`, `classify_issue_branch_prefix` |
| `crates/gwt-tauri/src/commands/branch_suggest.rs` | `suggest_branch_name` |
| `crates/gwt-tauri/src/commands/docker.rs` | `detect_docker_context` |

**テスト:**

| ファイル | テスト数 |
|---------|---------|
| `AgentLaunchForm.test.ts` | 38 |
| `AgentLaunchForm.glm.test.ts` | 3 |
| `agentLaunchFormHelpers.test.ts` | 7 |
| `agentLaunchDefaults.test.ts` | 6 |
| `agentUtils.test.ts` | 2 |
| `LaunchProgressModal.test.ts` | 12 |
| **合計** | **68** |

---
