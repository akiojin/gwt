# Feature Specification: Migrate from Tauri GUI to ratatui TUI

## Background

gwt is a SPEC-driven agent management tool that launches coding agents (Claude Code, Codex, Gemini) against SPECs with git worktrees providing isolated workspaces. The current frontend is a Tauri v2 + Svelte 5 desktop GUI introduced in v7.0.0. Prior to the GUI, gwt had a ratatui-based TUI (gwt-cli, 38,415 lines) that was removed in commit bd74257d.

This SPEC replaces the GUI with a new ratatui-based TUI (gwt-tui), migrating the full feature set from both the previous gwt-cli TUI and the current gwt-tauri GUI. The architecture uses Elm Architecture (Model/View/Update) from gwt-cli, with tmux dependency replaced by native PTY + vt100 terminal emulation.

Design document: `docs/superpowers/specs/2026-03-27-tui-migration-design.md`
Reference code: `crates/gwt-cli/` at commit `becf0aab` (38,415 lines)

## UI Design

### 2-Layer Tab Structure

**メイン画面** (通常時): Agent/Shell セッションのタブのみが表示される。各タブはPTYターミナルエミュレーターとして機能し、全キー入力がPTYに転送される（Ctrl+Gプレフィックス操作を除く）。

```
[claude: feat/x] [codex: fix/y] [shell] [+]
─────────────────────────────────────────────
> Analyzing codebase...
> Modified: src/auth.rs
─────────────────────────────────────────────
W1 | claude | feat/x | running | SPEC-42
```

**管理画面** (Ctrl+G, Ctrl+G でトグル): Branches / Issues / SPECs / Settings / Logs の5タブ。ブランチ一覧がデフォルト表示。

```
[Branches] [Issues] [SPECs] [Settings] [Logs]
─────────────────────────────────────────────
  main              -   ○  #42 open
* feat/x   Claude   *   ●  #38 merged
  fix/y    Codex    *   ○
  develop           -   ○
─────────────────────────────────────────────
Ctrl+G,Ctrl+G でメインに戻る
```

### Wizard (エージェント起動ウィザード)

管理画面上のオーバーレイポップアップとして表示。gwt-cli の15ステップウィザードを完全移植。

## User Stories

### US1 - Launch gwt as terminal TUI (P1)

As a developer, I want to launch gwt from my shell and have it display a ratatui TUI with agent/shell tabs, so that I can manage all coding agents from a single interface.

### US2 - Manage agent/shell sessions as tabs (P1)

As a developer, I want to create, switch between, and close agent and shell tabs within gwt, so that I can run multiple agents simultaneously. Same branch can have multiple agents.

### US3 - Launch agents via wizard (P1)

As a developer, I want to launch agents through a full wizard (15 steps: QuickStart → AgentSelect → ModelSelect → VersionSelect → BranchAction → ExecutionMode → SkipPermissions → Docker → Launch), so that I have full control over launch parameters.

### US4 - Toggle management panel (P1)

As a developer, I want to toggle a management panel (Ctrl+G,Ctrl+G) with Branches/Issues/SPECs/Settings/Logs tabs, so that I can manage branches, view issues, and configure settings without leaving gwt.

### US5 - Branch management with agent status (P1)

As a developer, I want to see all branches with PR status, agent status, safety level, and Quick Start (previous settings recall), so that I can quickly resume work on any branch.

### US6 - Settings management (P1)

As a developer, I want to edit global settings, profiles, environment variables, AI settings, and custom agent definitions within the Settings tab.

### US7 - Docker container support (P2)

As a developer, I want gwt to detect docker-compose.yml/DevContainer configs and launch agents inside containers, with service selection, port conflict detection, and volume mounting.

### US8 - Issue/SPEC management (P2)

As a developer, I want to see GitHub Issues and local SPECs in the Issues tab, with search, and launch agents linked to issues (auto-branch creation).

### US9 - Session conversion (P2)

As a developer, I want to convert a session from one agent to another (e.g., Claude → Codex), continuing the same task with a different tool.

### US10 - AI branch naming (P2)

As a developer, I want AI to suggest branch names based on a task description (e.g., "add authentication" → "feat/add-authentication").

### US11 - Clone/Migration wizards (P2)

As a developer, I want to clone new repositories and migrate to bare repositories from within gwt.

### US12 - SpecKit wizard (P2)

As a developer, I want to create and manage SPEC files through a TUI wizard.

### US13 - Voice input (P3)

As a developer, I want to use whisper-rs for local voice recognition, converting speech to text and injecting it into the active PTY.

### US14 - File paste from clipboard (P3)

As a developer, I want to paste files from clipboard to the agent via a dedicated shortcut, sending the file path or content.

## Acceptance Scenarios

1. gwt 起動 → 管理画面の Branches タブが表示される
2. Branches で Enter → Wizard オーバーレイが開く
3. Wizard で Launch → Agent タブが作成され、メイン画面に自動遷移
4. Agent タブでキー入力 → PTY に転送される
5. Ctrl+G, Ctrl+G → 管理画面 ↔ メイン画面をトグル
6. Ctrl+G, ] / [ → エージェントタブの切替
7. Ctrl+G, c → 新しいシェルタブが作成される
8. Ctrl+G, n → Wizard オーバーレイが開く
9. Ctrl+G, x → アクティブタブが閉じる（ワークツリーの安全チェック付き）
10. Ctrl+C ダブルタップ → gwt が終了する（実行中エージェントがあれば確認）
11. ターミナルリサイズ → 全ペイン + タブバーがリサイズされる
12. エージェントプロセスが終了 → タブは "Completed" ステータスで残る（スクロールバック閲覧可能）
13. Branches タブでブランチの Quick Start → 前回設定でワンクリック起動
14. 同じブランチで複数エージェントを起動可能
15. Docker compose 検出 → サービス選択 → コンテナ内でエージェント起動
16. 管理画面内で Tab キーで Branches/Issues/SPECs/Settings/Logs を切替
17. マウスクリックでブランチ選択、スクロールでリスト操作
18. エラー発生 → 重大エラーはモーダル、軽微はステータスバーに表示
19. エージェント起動中 → 6段階プログレスモーダル + キャンセルボタン

## Edge Cases

- Terminal size below 80x24: display warning
- Agent PTY crash: tab remains with error indicator, restart option
- Worktree creation failure: error shown in modal, tab not created
- GitHub API unreachable: PR/Issue panels show offline state, background retry
- Ctrl+G: prefix key must never be forwarded to PTY
- Ctrl+C on agent tab: always forward to PTY (never quit gwt)
- Ctrl+C double-tap on shell tab: quit gwt (with confirmation if agents running)
- Multiple agents on same branch: allowed, each gets unique pane_id
- Long-running gwt: scrollback file-based (not memory), parser memory bounded

## Functional Requirements

### Core UI

- FR-001: gwt-tui crate using ratatui + crossterm, Elm Architecture (Model/View/Update)
- FR-002: 2-layer tab structure: メイン画面 (Agent/Shell tabs) + 管理画面 (Branches/Issues/SPECs/Settings/Logs tabs)
- FR-003: Ctrl+G,Ctrl+G トグルで メイン ↔ 管理画面 切替
- FR-004: Ctrl+G prefix key system (2s timeout) for all management operations
- FR-005: VT100 emulator buffer to ratatui Cell conversion (renderer)
- FR-006: Full PTY terminal rendering with ANSI color and attribute support
- FR-007: Status bar showing current tab info, branch, SPEC association, agent state

### Agent/Shell Sessions

- FR-010: Agent tab = full PTY terminal emulator (all keys forwarded except Ctrl+G)
- FR-011: Shell tab = plain shell PTY (same as agent but shell command)
- FR-012: Tab creation via Ctrl+G,n (wizard) or Ctrl+G,c (shell)
- FR-013: Tab switching via Ctrl+G,]/[ and Ctrl+G,1-9
- FR-014: Tab close via Ctrl+G,x with safety confirmation
- FR-015: Automatic worktree creation on agent launch and cleanup on close
- FR-016: Session status polling (Running/Completed/Error) via PTY process monitoring

### Wizard (Agent Launch)

- FR-020: 15-step wizard as overlay popup (gwt-cli wizard.rs complete migration)
- FR-021: Steps: QuickStart → BranchAction → AgentSelect → ModelSelect → ReasoningLevel → VersionSelect → CollaborationModes → ExecutionMode → ConvertAgentSelect → ConvertSessionSelect → SkipPermissions (with Codex fast mode option) → BranchTypeSelect → IssueSelect → AIBranchSuggest → BranchNameInput
- FR-022: Quick Start — recall previous launch settings per branch (FR-050)
- FR-023: Session modes: Normal, Continue, Resume, Convert
- FR-024: Agent version detection via which + npm registry fallback (bunx/npx)
- FR-025: Custom agent support (SPEC-71f2742d)
- FR-026: Docker detection and service/port selection
- FR-027: AI branch name suggestion (SPEC-1ad9c07d)
- FR-028: GitHub Issue linked branch creation (SPEC-e4798383)
- FR-029: 6-stage progress modal with cancellation (fetch → validate → worktree → skills → deps → launch)

### Management Panel — Branches Tab

- FR-030: Branch list with PR status, agent status, safety level, divergence
- FR-031: View modes: All, Local, Remote
- FR-032: Sort modes: Default, Name, Updated
- FR-033: Filter/search by branch name
- FR-034: Multi-select for batch operations
- FR-035: Git View sub-view (diff, commits, working tree status)
- FR-036: Branch delete with safety level check (Safe/Warning/Danger/Disabled)
- FR-037: Worktree create/delete operations

### Management Panel — Issues Tab

- FR-040: GitHub Issues and local SPECs list with search
- FR-041: Issue → branch creation → agent launch flow

### Management Panel — Settings Tab

- FR-050: Global settings editor (gwt-cli settings.rs migration)
- FR-051: Profile management (create/edit/delete/switch)
- FR-052: Environment variable editor per profile (KEY=VALUE)
- FR-053: AI settings (endpoint, API key, model)
- FR-054: Custom coding agent registration (SPEC-71f2742d)

### Management Panel — Logs Tab

- FR-060: Log viewer for ~/.gwt/logs/ (gwt-cli logs.rs migration)
- FR-061: init_logger() call on startup

### Additional Features

- FR-070: Clone Wizard (clone new repositories)
- FR-071: Migration Dialog (bare repository migration)
- FR-072: SpecKit Wizard (SPEC file management)
- FR-073: Skill registration auto-execution on startup (CLAUDE.md/AGENTS.md/GEMINI.md)
- FR-074: Voice input via whisper-rs (native audio capture + transcription)
- FR-075: File paste from clipboard (dedicated shortcut, OS-native API)
- FR-076: Mouse support (click selection, scroll, double-click)
- FR-077: Error handling: ErrorQueue + modal (critical) + status bar (minor)

### npm Distribution

- FR-080: npm package publication maintained (postinstall binary download from GitHub Releases)

## Non-Functional Requirements

- NFR-001: Rendering optimized with frame rate limiting, dirty flags, differential updates
- NFR-002: Memory usage bounded (scrollback file-based via gwt-core ScrollbackFile)
- NFR-003: Cross-platform support (macOS, Linux, Windows) via crossterm
- NFR-004: Startup time under 500ms to first interactive frame
- NFR-005: gwt-core API changes limited to new modules (no breaking changes)

## Success Criteria

- SC-001: gwt launches as TUI, displays Branches tab on startup
- SC-002: Users can launch agents via full 15-step wizard with all parameters
- SC-003: Agent tabs are full PTY terminal emulators with ANSI color support
- SC-004: Ctrl+G,Ctrl+G toggles between メイン and 管理画面
- SC-005: All gwt-cli screens migrated (Branches, Issues, SPECs, Settings, Logs, Wizard, etc.)
- SC-006: Docker compose/DevContainer support functional
- SC-007: All existing gwt-core tests pass without modification
- SC-008: gwt-tui has >80% test coverage on core modules
- SC-009: gwt-tauri and gwt-gui fully removed from repository
- SC-010: CI/release pipeline updated for TUI binary + npm distribution
- SC-011: gwt-core Settings/ProfilesConfig/AgentConfig fully utilized by gwt-tui
