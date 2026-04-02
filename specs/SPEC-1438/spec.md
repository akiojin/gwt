### Overview

gwt はエージェント起動時に、起動対象の project/worktree 配下へ gwt 管理の skill/command/hook assets を埋め込む。グローバルな `~/.claude` / `~/.codex` / `~/.gemini` への起動時登録・終了時解除は行わない。

### Canonical specification source

- 仕様 SPEC の正本は GitHub Issue（`gwt-spec` ラベル）とする。
- gwt リポジトリ自身は local `specs/`、`specs/specs.md`、`specs/archive/` を正本として保持しない。
- 仕様検索は `gwt-project-search`、仕様作成/更新は `gwt-issue-spec-ops` を使う。

### Target agents and embed locations

- Claude Code: `./.claude/settings.local.json`, `./.claude/skills`, `./.claude/commands`, `./.claude/hooks` を project-local に使う
- Codex: `./.codex/skills`
- Gemini: `./.gemini/skills`

`./` はその launch で実際に agent が動く target worktree root を指す。

Claude を含む全エージェントで、CLI-visible な gwt asset は launch target worktree 直下の local 配置を正とする。plugin namespace や global marketplace には依存しない。

### Managed gwt skills

- `gwt-agent-dispatch`
- `gwt-project-search`
- `gwt-issue-spec-ops`
- `gwt-pr`
- `gwt-pr-check`
- `gwt-fix-pr`
- `gwt-fix-issue`
- `gwt-spec-to-issue-migration`
- `gwt-sync-base`

### Bundled assets

- `gwt-pr` は PR body template を skill bundle に含める。
- `gwt-fix-pr` は `inspect_pr_checks.py` と `LICENSE.txt` を skill bundle に含める。
- `gwt-fix-issue` は `inspect_issue.py` を skill bundle に含める。
- `gwt-spec-to-issue-migration` は migration script を skill bundle に含め、target project 上で自己完結して動作する。
- `gwt-sync-base` は SKILL.md のみ（補助 asset なし）。

### Claude hook source of truth

- Claude Code が実際に読む hook 設定は `./.claude/settings.local.json` の `hooks` セクションとする。
- `plugins/gwt/hooks/hooks.json` は runtime asset としては廃止し、managed hook definition の正本は `skill_registration.rs` に内包する。
- Claude 向け generated asset で既存更新対象に含めるのは `./.claude/settings.local.json` のみとする。

### Codex hook support

Codex CLI (v0.116.0+) が Hooks フレームワークをサポートしている。gwt は Codex のエージェント起動時にも hook assets を埋め込む対象とする。

#### Codex 対応フックイベント

| Event | Scope | Description |
|-------|-------|-------------|
| `SessionStart` | Session | セッション開始時に発火 |
| `PreToolUse` | Turn | ツール呼び出し前に発火。Block/Modify が可能 |
| `PostToolUse` | Turn | ツール呼び出し後に発火（現在 Bash のみ） |
| `UserPromptSubmit` | Turn | プロンプト送信前に発火。ブロック/拡張が可能 |
| `Stop` | Turn | ターン終了時に発火 |

#### Claude Code との対応

| Claude Code Event | Codex Equivalent | Notes |
|---|---|---|
| `PreToolUse` | `PreToolUse` | 同等。Codex では `systemMessage` をサポート |
| `PostToolUse` | `PostToolUse` | Codex は Bash のみ対応 |
| `Stop` | `Stop` | 同等 |
| `SessionStart` | `SessionStart` | 同等 |
| `UserPromptSubmit` | `UserPromptSubmit` | 同等 |
| `SessionEnd` | — | Codex 未対応 |
| `SubagentStop` | — | Codex 未対応 |
| `PreCompact` | — | Codex 未対応 |
| `Notification` | — | Codex 未対応 |

#### Codex hook の設定場所

- Codex は `hooks.json` をアクティブな設定レイヤー（`.codex/` 等）から検出する
- 各 hook は stdin で JSON を受け取り、stdout で制御結果を返す
- Windows は現在 Hooks 無効

#### gwt embed 対応方針

- Codex 向け hook embed は `./.codex/hooks.json` に配置する
- Claude Code と共通の hook ロジック（gwt-tauri hook 呼び出し等）を Codex 向けに変換して埋め込む
- `skill_registration.rs` で Claude/Codex 両方の hook 定義を管理する
- Codex 未対応イベント（`SessionEnd`, `SubagentStop`, `PreCompact`, `Notification`）は skip する

### Lifecycle

1. agent launch 時に target worktree root へ registration / repair を実行する。
2. repair/status は同じ target root を基準に判定する。
3. gwt app 起動時の global registration は行わない。
4. gwt app 終了時の global unregister は行わない。

### Findings that motivated this update

- launch 時 registration が launch target worktree ではなく window の `project_root` に対して実行されると、bare project root と worktree の間で埋め込み先がズレる。
- Codex/Gemini で `SKILL.md` しか出力しないと、`gwt-pr` / `gwt-fix-pr` / `gwt-fix-issue` / `gwt-spec-to-issue-migration` が必要とする補助 asset を解決できない。
- local `specs/` を読み続けると、Issue-first の正本と実装が分岐する。
- bare repo 配下の linked worktree では generated local asset の ignore は worktree 専用ではなく shared `info/exclude` で管理する必要がある。tracked `.gitignore` を runtime が書き換えるべきではない。

### Functional requirements

| ID | Requirement |
|---|---|
| FR-REG-001 | gwt は agent launch 時に、launch target worktree root を registration 対象にする |
| FR-REG-002 | Claude Code は project-local な `.claude/settings.local.json` を使うこと |
| FR-REG-002A | Claude Code の managed assets は `.claude/skills`, `.claude/commands`, `.claude/hooks` に直接配置されなければならない |
| FR-REG-002B | Claude Code は `gwt@gwt-plugins` marketplace/plugin を必須前提にしてはならない |
| FR-REG-003 | Codex には `./.codex/skills` へ gwt skill bundle 一式を埋め込む |
| FR-REG-004 | Gemini には `./.gemini/skills` へ gwt skill bundle 一式を埋め込む |
| FR-REG-005 | Codex/Gemini の bundle は `gwt-project-search` と `gwt-issue-spec-ops` を含む |
| FR-REG-006 | Codex/Gemini の bundle は `gwt-pr` / `gwt-pr-check` / `gwt-fix-pr` / `gwt-fix-issue` を含む |
| FR-REG-007 | `gwt-spec-to-issue-migration` は gwt 提供 skill として維持し、migration script を bundle に含める |
| FR-REG-008 | bundle 内の `SKILL.md` は agent ごとの実在パス（`.claude` / `.codex` / `.gemini`）へ rewrite される |
| FR-REG-009 | status / repair は missing `SKILL.md` だけでなく missing bundled assets も検出する |
| FR-REG-010 | registration / repair は `git rev-parse --git-path info/exclude` が解決する shared exclude に、gwt 管理 block として local generated asset 用 rule を自動補完しなければならない |
| FR-SPEC-001 | gwt リポジトリ自身は local `specs/`、`specs/specs.md`、`specs/archive/` に依存しない |
| FR-SPEC-002 | sidebar の spec task 表示は最新の `gwt-spec` GitHub Issue を参照する |
| FR-SPEC-003 | repository scanner / prompt builder は local `specs/` を source of truth として扱わない |
| FR-SPEC-004 | spec の探索・更新は GitHub Issue-first とし、`gwt-project-search` / `gwt-issue-spec-ops` を導線にする |
| FR-SYNC-001 | `gwt-sync-base` は現在のブランチのベースブランチ（`develop`, `main`, `master` 等）を自動検出し、`git fetch origin <base> && git merge origin/<base> && git push` を実行するスキルとする |
| FR-SYNC-002 | ベースブランチの検出は、PR のベースブランチ、リポジトリのデフォルトブランチ、ブランチ命名規則等から判定する |
| FR-SYNC-003 | `gwt-sync-base` は「最新にして」「ベースをマージ」「sync base」「update from base」「ベースと同期」等のキーワードでトリガーされる |
| FR-SYNC-004 | `gwt-sync-base` はマージコンフリクト発生時に自動解決せず、ユーザーに報告して停止する |
| FR-SYNC-005 | `gwt-sync-base` は rebase や force-push を使用しない。常に merge を使用する |
| FR-SYNC-006 | `gwt-sync-base` はブランチの作成・切り替えを行わない |
| FR-HOOK-001 | Codex agent launch 時に `./.codex/hooks.json` へ gwt managed hook を埋め込む |
| FR-HOOK-002 | Codex hooks は SessionStart, PreToolUse, PostToolUse, UserPromptSubmit, Stop の 5 イベントに対応する |
| FR-HOOK-003 | Claude Code と Codex で共通の hook ロジックは `skill_registration.rs` で一元管理する |
| FR-HOOK-004 | Codex 未対応イベント (SessionEnd, SubagentStop, PreCompact, Notification) は Codex 向け hook 生成時にスキップする |

### Acceptance scenarios

| # | Scenario | Expected result |
|---|---------|-----------------|
| US1 | bare project から agent を worktree へ launch | launch target worktree に `./.codex/skills` / `./.claude/*` / `./.gemini/skills` が生成される |
| US2 | Codex セッションで `$gwt` を実行 | `gwt-project-search`、`gwt-issue-spec-ops`、`gwt-pr`、`gwt-fix-pr` などの `gwt-*` skill が候補表示される |
| US3 | Claude Code を同じ target worktree で起動 | `.claude/skills`, `.claude/commands`, `.claude/hooks` の local asset だけで `gwt-*` が利用できる |
| US4 | gwt repo に local `specs/` が存在しない状態で sidebar を開く | 最新の `gwt-spec` Issue の `## Tasks` から task が表示される |
| US5 | 別プロジェクトで local `specs/SPEC-*` が残っている | `gwt-spec-to-issue-migration` で Issue-first へ移行できる |
| US6 | registration / repair を実行する | generated な `.claude` / `.codex` / `.gemini` 配下の `gwt-*` asset が shared `info/exclude` により untracked noise にならない |
| US7 | ユーザーが「最新にして」と指示する | `gwt-sync-base` がベースブランチを自動検出し、fetch → merge → push を実行して最新コミットが現在のブランチに取り込まれる |
| US8 | ベースブランチとのマージでコンフリクトが発生する | `gwt-sync-base` がコンフリクトを報告し、ユーザーに解決を委ねる |
| US9 | 現在のブランチが既にベースブランチと同期済み | `gwt-sync-base` が「Already up to date」を報告し、push をスキップする |
| US10 | Codex セッションを worktree で起動 | `./.codex/hooks.json` が生成され、gwt managed hook が 5 イベント分登録されている |
| US11 | Codex セッションで PreToolUse hook が発火 | gwt-tauri (or gwt-server) の hook エンドポイントが呼ばれ、対応するアクションが実行される |

### Success criteria

- `cargo test -p gwt-core skill_registration` が pass する
- `cargo test -p gwt-tauri sessions` が pass する
- 手動確認で、Claude/Codex/Gemini すべて launch target worktree の local assets から `gwt-*` skills が見える
- `git status --short` で generated local asset が PR ノイズとして残らない
