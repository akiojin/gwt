> **⚠️ DEPRECATED (SPEC-1776)**: This SPEC describes GUI-only functionality (Tauri/Svelte/xterm.js) that has been superseded by the gwt-tui migration. The gwt-tui equivalent is defined in SPEC-1776.

### Background

Launch Agent ダイアログは gwt の主要機能であり、Worktree 選択時のエージェント起動設定 UI として完全に実装済みである。
本 SPEC は、個別の増分機能として実装された以下の CLOSED SPECs を統合し、Launch Agent ダイアログ全体の canonical reference として機能する。

**統合済み CLOSED SPECs:**

- **#1317** — Launch Agent のデフォルト設定保持（前回成功起動値の localStorage 永続化・復元・サニタイズ）
- **#1306** — Windows シェル選択（Launch Agent / New Terminal 向けシェル選択 UI・WSL パス変換・永続化）
- **#1337** — Launch Agent From Issue ブランチプレフィックス AI 判定（ラベル優先・AI フォールバック方式のプレフィックス自動分類）

**関連 SPEC:**

- **#1646** — Agent 管理（エージェント検出・起動・ライフサイクル・バージョン管理・異常監視の包括的 SPEC）。本 SPEC は Launch Agent ダイアログの UI・バリデーション・パイプラインに特化し、#1646 はエージェントのライフサイクル管理に特化する。

---

### User Scenarios & Tests

#### US-01: 既存ブランチでの基本起動 [P0]

**前提:**
- サイドバーで既存ブランチが選択されている
- `detect_agents` で少なくとも1つのエージェントが available

**テスト手順:**
1. サイドバーでブランチを選択
2. Launch Agent ダイアログを開く
3. エージェントをドロップダウンから選択
4. Launch ボタンを押下

**検証項目:**
- [ ] `detect_agents` が呼ばれ、利用可能エージェントがドロップダウンに表示される
- [ ] `start_launch_job` が UUID ジョブ ID を返却する
- [ ] `launch-progress` イベントで 7 ステップ（fetch→validate→paths→conflicts→create→skills→deps）が順次発火する
- [ ] `launch-finished` イベントで `status: "ok"` と `paneId` が返却される
- [ ] 選択済みブランチの Worktree でエージェントが起動する

**関連 FR:** FR-001, FR-030, FR-031, FR-039, FR-040

---

#### US-02: 新規ブランチ作成（Direct 命名） [P0]

**前提:**
- "New Branch" モードを選択

**テスト手順:**
1. Launch Agent ダイアログを開く
2. "New Branch" モードを選択
3. Manual タブ → Direct モードを選択
4. プレフィックスドロップダウンから `feature/` を選択
5. suffix に `my-feature` を入力
6. ベースブランチを選択
7. Launch ボタンを押下

**検証項目:**
- [ ] `BRANCH_PREFIXES`（`feature/`, `bugfix/`, `hotfix/`, `release/`）がドロップダウンに表示される
- [ ] `splitBranchNamePrefix()` でフルネームペースト時に自動分割される
- [ ] `buildNewBranchName("feature/", "my-feature")` → `"feature/my-feature"` が生成される
- [ ] `list_worktree_branches` + `list_remote_branches` でベースブランチが optgroup 分離表示される
- [ ] `createBranch: { name: "feature/my-feature", base: "main" }` が LaunchAgentRequest に含まれる
- [ ] create ステップで Worktree が作成され、`worktrees-changed` イベントが発火する

**関連 FR:** FR-014, FR-016, FR-034

---

#### US-03: GitHub Issue からのブランチ作成（カテゴリ自動分類） [P0]

**前提:**
- `check_gh_cli_status` → `available=true`, `authenticated=true`
- AI プロバイダが設定済み（`is_ai_configured` → true）
- Issue #42 が存在し、ラベル `bug` 付き

**テスト手順:**
1. Launch Agent ダイアログを開く
2. "New Branch" モードを選択
3. "From Issue" タブを選択
4. Issue リストから #42 を選択
5. プレフィックスが自動分類されるのを確認
6. Launch ボタンを押下

**検証項目:**
- [ ] `fetch_github_issues` が呼ばれ、Issue リストがページネーション付きで表示される
- [ ] `find_existing_issue_branches_bulk` で既存ブランチチェックが実行され、既存ブランチありの Issue は選択不可（"Branch exists" 表示）
- [ ] Issue #42 選択時、`determinePrefixFromLabels([{name: "bug"}])` が `"bugfix/"` を即座に返す
- [ ] ラベルで判定できない場合、`classify_issue_branch_prefix` が呼ばれ AI 分類が実行される
- [ ] 分類中は `prefixClassifying` スピナー（`&#x21BB;`）が表示される
- [ ] `classifyIssuePrefix()` が `ClassifyResult` を `BranchPrefix` に変換する
- [ ] `isStaleIssueClassifyRequest()` で Stale リクエストが無視される
- [ ] ブランチ名が `bugfix/issue-42` に設定される
- [ ] Launch 成功後、`bugfix/issue-42` ブランチと Worktree が作成される

**関連 FR:** FR-017, FR-018, FR-019, FR-048

---

#### US-04: AI によるブランチ名提案 [P1]

**前提:**
- "New Branch" → Manual → "AI Suggest" モード
- AI が設定済み（`is_ai_configured` → true）

**テスト手順:**
1. Launch Agent ダイアログを開く
2. "New Branch" → Manual → "AI Suggest" モードを選択
3. description テキストエリアに説明を入力
4. Launch ボタンを押下

**検証項目:**
- [ ] `suggest_branch_name` が `BranchSuggestResult` を返却する
- [ ] `status: "ok"` 時に `suggestion` がブランチ名として設定される
- [ ] `status: "ai-not-configured"` 時にエラーメッセージが表示される（[E2001]）
- [ ] `aiBranchDescription` が LaunchAgentRequest に含まれる

**関連 FR:** FR-015

---

#### US-05: セッション Continue/Resume [P1]

**前提:**
- 既存のエージェントセッション履歴がある

**テスト手順:**
1. Launch Agent ダイアログを開く
2. Session Mode を "Continue" or "Resume" に変更
3. Resume の場合は Session ID を入力
4. Launch ボタンを押下

**検証項目:**
- [ ] Claude: Continue → `["--continue"]`、Resume → `["--resume", {session_id}]` が CLI 引数に追加される
- [ ] Codex: Continue → `["resume", "--last"]`、Resume → `["resume", {session_id}]`
- [ ] Gemini: Continue → `["-r", "latest"]`、Resume → `["-r", {session_id}]`
- [ ] OpenCode: Continue → `["-c"]`（session_id あれば `["-s", {id}]`）、Resume → `["-s", {session_id}]`（必須）
- [ ] Copilot: Continue → `["--continue"]`、Resume → `["--resume", {session_id}]`
- [ ] `mode` フィールドが LaunchAgentRequest に正しく設定される

**関連 FR:** FR-007

---

#### US-06: Docker Compose 起動 [P1]

**前提:**
- プロジェクトに `docker-compose.yml` が存在
- `detect_docker_context` → `file_type: "compose"`, `docker_available: true`, `compose_available: true`

**テスト手順:**
1. Launch Agent ダイアログを開く
2. Runtime Target を "Docker" に切替
3. サービスをドロップダウンから選択
4. Build/Recreate/Keep 設定を調整
5. Launch ボタンを押下

**検証項目:**
- [ ] `detect_docker_context` がブランチ切替時に自動実行され、`DockerContext` が返却される
- [ ] `dockerStatusHint()` が images/containers 状態を表示する
- [ ] `images_exist: false` 時に Build が自動 ON + disabled
- [ ] `container_status: "not_found"` 時に Recreate が自動 ON + disabled
- [ ] `dockerService`, `dockerBuild`, `dockerRecreate`, `dockerKeep` が LaunchAgentRequest に含まれる
- [ ] Shell ドロップダウンが disabled になる

**関連 FR:** FR-024, FR-025, FR-026, FR-027, FR-028

---

#### US-07: Devcontainer 起動 [P2]

**前提:**
- プロジェクトに `.devcontainer/devcontainer.json` が存在
- `detect_docker_context` → `file_type: "devcontainer"`

**テスト手順:**
1. Launch Agent ダイアログを開く
2. Runtime Target を "Docker" に切替
3. サービス選択 → Launch

**検証項目:**
- [ ] Devcontainer の compose 設定が解析され、`compose_services` にサービスが含まれる
- [ ] Docker Compose モードと同様の UI が表示される

**関連 FR:** FR-024, FR-026

---

#### US-08: サイドバーからのクイック起動 [P0]

**前提:**
- サイドバーでブランチが選択済み

**テスト手順:**
1. サイドバーの Launch ボタンを押下

**検証項目:**
- [ ] `AgentLaunchForm` が `selectedBranch` 付きでオープンされる
- [ ] 既存ブランチモードで表示される

**関連 FR:** FR-039

---

#### US-09: Issue タブ「Work on this」からの起動 [P1]

**前提:**
- Issues タブで Issue を閲覧中

**テスト手順:**
1. "Work on this" ボタンを押下

**検証項目:**
- [ ] `AgentLaunchForm` が `prefillIssue` 付きで表示される
- [ ] `branchMode="new"`, `newBranchTab="fromIssue"` が自動設定される
- [ ] 該当 Issue が自動選択された状態になる
- [ ] Issue リストの自動読み込みが発生しない（`prefillIssue` 時）

**関連 FR:** FR-042

---

#### US-10: 起動キャンセル [P1]

**前提:**
- Launch 進捗モーダルが表示中（7ステップ実行中）

**テスト手順:**
1. Esc キー or Cancel ボタンを押下

**検証項目:**
- [ ] `cancel_launch_job` が呼ばれ、AtomicBool フラグがセットされる
- [ ] 次のステップ間チェック（`is_launch_cancelled()`）でキャンセルが検出される
- [ ] `launch-finished` イベントで `status: "cancelled"` が返却される
- [ ] Issue ブランチ作成後のキャンセル時は `rollback_new_issue_branch()` で自動クリーンアップされる

**関連 FR:** FR-032, FR-045

---

#### US-11: ブランチ存在時のリカバリ（E1004） [P1]

**前提:**
- 指定したブランチ名が既に存在する

**テスト手順:**
1. 既存ブランチ名で Launch
2. エラーモーダルに `[E1004]` 表示を確認
3. "Use Existing Branch" ボタンを押下

**検証項目:**
- [ ] エラーメッセージに `[E1004]` が含まれる場合、"Use Existing Branch" ボタンが表示される
- [ ] 非 E1004 エラーでは "Use Existing Branch" ボタンが表示されない
- [ ] ボタン押下で `onUseExisting` コールバックが呼ばれ、既存ブランチでの起動が開始される

**関連 FR:** FR-044

---

#### US-12: 設定の永続化と復元 [P0]

**前提:**
- 以前に Launch Agent を成功実行済み

**テスト手順:**
1. Launch Agent を成功実行
2. ダイアログを閉じる
3. ダイアログを再度開く

**検証項目:**
- [ ] `saveLaunchDefaults()` が Launch 成功時のみ呼ばれる（Close/失敗/キャンセル時は不実行）
- [ ] `localStorage` キー `gwt.launchAgentDefaults.v1` に `StoredLaunchDefaults`（`version: 1` エンベロープ）として保存される
- [ ] `loadLaunchDefaults()` でダイアログ初期化時に復元される
- [ ] `sanitizeLaunchDefaults()` で不正データ時にクラッシュせずフォールバック（無効 Agent → 利用可能 Agent、無効 runtime → Host）
- [ ] 全設定項目（Agent/SessionMode/Model/Version/Skip/Reasoning/FastMode/Resume/Advanced/Runtime/Docker/Shell/BranchNamingMode）が復元される
- [ ] 新規ブランチ入力フィールドはデフォルトに永続化されない

**関連 FR:** FR-009, FR-010, FR-011, FR-012

---

#### US-13: Claude GLM プロバイダ設定 [P1]

**前提:**
- Claude エージェントを選択

**テスト手順:**
1. Provider を "GLM (z.ai)" に切替
2. Base URL, API Token, Model ID を入力
3. Launch ボタンを押下

**検証項目:**
- [ ] `save_agent_config` が呼ばれ、GLM 設定が `~/.gwt/config.toml` に永続化される
- [ ] 起動時に以下の環境変数が設定される:
  - `ANTHROPIC_BASE_URL`（`glm.base_url`）
  - `ANTHROPIC_AUTH_TOKEN`（`glm.auth_token`）
  - `API_TIMEOUT_MS`（optional, `glm.api_timeout_ms`）
  - `ANTHROPIC_DEFAULT_OPUS_MODEL`（optional）
  - `ANTHROPIC_DEFAULT_SONNET_MODEL`（optional）
  - `ANTHROPIC_DEFAULT_HAIKU_MODEL`（optional）
- [ ] Advanced の環境変数オーバーライドが GLM 設定より高い優先度でマージされる
- [ ] Base URL / API Token 空チェックで Launch が中断される
- [ ] Provider を Anthropic に戻した場合、GLM 環境変数が注入されない

**関連 FR:** FR-008, FR-046

---

#### US-14: 環境変数オーバーライド [P2]

**前提:**
- Advanced セクションを展開

**テスト手順:**
1. Environment Variables Override に `KEY=VALUE` を入力
2. Launch ボタンを押下

**検証項目:**
- [ ] `parseEnvOverrides()` で `KEY=VALUE` 形式がパースされる
- [ ] `#` コメント行がスキップされる
- [ ] 不正形式の行にはエラーメッセージ（行番号付き）が表示される
- [ ] `envOverrides` がプロファイル設定・GLM 設定より高い優先度でマージされる

**関連 FR:** FR-022

---

#### US-15: エージェントバージョン選択 [P2]

**前提:**
- エージェントにレジストリバージョンが存在

**テスト手順:**
1. Version ドロップダウンから特定バージョンを選択
2. Launch ボタンを押下

**検証項目:**
- [ ] `list_agent_versions` が `AgentVersionsInfo`（tags/versions/source）を返却する
- [ ] "Installed"/"latest"/レジストリタグ/上位10件が表示される
- [ ] 未インストール時は "latest" にフォールバックし、fallback hint が表示される
- [ ] バージョン選択に応じてパッケージランナー（bunx/npx）の `@package@{version}` が構築される

**関連 FR:** FR-003

---

#### US-16: メニューコマンドからの起動 [P2]

**前提:**
- アプリケーションメニューが利用可能

**テスト手順:**
1. メニューから Launch Agent コマンドを選択

**検証項目:**
- [ ] `AgentLaunchForm` が表示される

**関連 FR:** FR-041

---

### Functional Requirements

#### Agent Configuration

| ID | 要件 | Tauri コマンド | 対象ファイル | 状態 |
|----|------|---------------|-------------|------|
| FR-001 | エージェント選択: `detect_agents` で利用可能なエージェント（Claude/Codex/Gemini/OpenCode/Copilot）を検出し、ドロップダウンで選択。`DetectedAgentInfo`（id/name/version/path/available）を返却 | `detect_agents` | `agents.rs`, `AgentLaunchForm.svelte` | Implemented |
| FR-002 | モデル選択: エージェント毎のハードコードモデルリスト。CLI引数マッピング: Codex `--model={model}`, Claude `--model {model}`, Gemini `-m {model}`, OpenCode `-m {model}`, Copilot `--model {model}` | — | `AgentLaunchForm.svelte:245-278`, `terminal.rs:491-508` | Implemented |
| FR-003 | バージョン選択: `list_agent_versions` でレジストリから `AgentVersionsInfo`（tags/versions/source）取得。"Installed"/"latest"/レジストリタグ/上位10件。バージョンに応じて `bunx @package@{version}` or ローカルコマンドを選択 | `list_agent_versions` | `agents.rs`, `terminal.rs:697-796` | Implemented |
| FR-004 | 推論レベル: Codex 専用（low/medium/high/xhigh の select） | — | `AgentLaunchForm.svelte` | Implemented |
| FR-005 | Fast mode: Codex 専用かつ gpt-5.4 モデル限定。`-c service_tier=fast` CLI引数追加 | — | `AgentLaunchForm.svelte`, `terminal.rs` | Implemented |
| FR-006 | Skip Permissions: 全エージェント共通。CLI フラグマッピング: Claude `--dangerously-skip-permissions`（非Windows では `IS_SANDBOX=1` も設定）, Codex `codex_skip_permissions_flag(version)` (バージョンゲート付き), Gemini `-y`, Copilot `--allow-all-tools`, OpenCode なし | — | `terminal.rs:1631-1778` | Implemented |
| FR-007 | セッションモード: Normal/Continue/Resume の3モード。エージェント毎の CLI 引数マッピング（後述の表参照）。OpenCode Resume は Session ID 必須 | — | `terminal.rs:1641-1777` | Implemented |
| FR-008 | Claude GLM プロバイダ: Anthropic/GLM(z.ai) 切替。GLM 時に Base URL, API Token, API Timeout, Opus/Sonnet/Haiku Model ID 入力。`save_agent_config` で永続化。起動時に `ANTHROPIC_BASE_URL`, `ANTHROPIC_AUTH_TOKEN` 等を環境変数設定 | `get_agent_config`, `save_agent_config` | `AgentLaunchForm.svelte`, `terminal.rs` | Implemented |

**FR-002 モデルリスト詳細:**

| Agent | モデルリスト |
|-------|------------|
| Codex | `gpt-5.3-codex`, `gpt-5.4`★, `gpt-5.3-codex-spark`, `gpt-5.2-codex`, `gpt-5.1-codex-max`, `gpt-5.2`, `gpt-5.1-codex-mini` |
| Claude | `opus`, `sonnet`, `haiku`, `opus[1m]`, `sonnet[1m]`, `opusplan` |
| Gemini | `gemini-3-pro-preview`, `gemini-3-flash-preview`, `gemini-2.5-pro`, `gemini-2.5-flash`, `gemini-2.5-flash-lite` |
| Copilot | `gpt-4.1` |
| OpenCode | フリーテキスト入力（`provider/model` 形式） |

★ `gpt-5.4` のみ Fast mode 対応

**FR-007 セッションモード CLI 引数マッピング:**

| Agent | Normal | Continue | Resume |
|-------|--------|----------|--------|
| Claude | *(なし)* | `--continue` / `--resume {id}` | `--resume {id}` |
| Codex | *(なし)* | `resume --last` / `resume {id}` | `resume {id}` |
| Gemini | *(なし)* | `-r latest` / `-r {id}` | `-r {id}` |
| OpenCode | *(なし)* | `-c` / `-s {id}` | `-s {id}` (必須) |
| Copilot | *(なし)* | `--continue` / `--resume {id}` | `--resume {id}` |

**ローカルコマンド & パッケージ:**

| Agent | ローカルコマンド | bunx パッケージ |
|-------|----------------|----------------|
| Claude | `claude` | `@anthropic-ai/claude-code` |
| Codex | `codex` | `@openai/codex` |
| Gemini | `gemini` | `@google/gemini-cli` |
| OpenCode | `opencode` | `opencode-ai` |
| Copilot | `copilot` | `@github/copilot` |

#### Settings Persistence (ex-#1317)

| ID | 要件 | 対象ファイル | 状態 |
|----|------|-------------|------|
| FR-009 | localStorage 永続化: `gwt.launchAgentDefaults.v1` キーで `StoredLaunchDefaults`（`version: 1` エンベロープ）として保存 | `agentLaunchDefaults.ts` | Implemented |
| FR-010 | 保存タイミング: Launch 成功時のみ `saveLaunchDefaults()` を実行。Close/失敗/キャンセル時はデフォルト値を更新しない | `AgentLaunchForm.svelte` | Implemented |
| FR-011 | 復元: ダイアログ初期化時に `loadLaunchDefaults()` → `applySavedLaunchDefaults()` で復元。全設定項目（Agent/SessionMode/Model/Version/Skip/Reasoning/FastMode/Resume/Advanced/Runtime/Docker/Shell/BranchNamingMode）を対象 | `agentLaunchDefaults.ts`, `AgentLaunchForm.svelte` | Implemented |
| FR-012 | サニタイズ: `sanitizeLaunchDefaults()` で全フィールドを型安全に正規化。文字列 `trim()`, boolean `=== true`, Record キー/値空文字除去。不正データ時はクラッシュせずフォールバック（無効 Agent → 利用可能 Agent、無効 runtime → Host） | `agentLaunchDefaults.ts` | Implemented |

#### Branch Configuration (ex-#1337)

| ID | 要件 | Tauri コマンド | 対象ファイル | 状態 |
|----|------|---------------|-------------|------|
| FR-013 | 既存ブランチ選択: サイドバー選択済みブランチを読み取り専用表示 | — | `AgentLaunchForm.svelte` | Implemented |
| FR-014 | Direct 命名: プレフィックスドロップダウン（`BRANCH_PREFIXES`: `feature/`/`bugfix/`/`hotfix/`/`release/`）+ suffix テキスト入力。フルネームペースト時に `splitBranchNamePrefix()` で自動分割 | — | `AgentLaunchForm.svelte`, `agentLaunchFormHelpers.ts` | Implemented |
| FR-015 | AI ブランチ名提案: description テキスト → `suggest_branch_name` API → `BranchSuggestResult`（status/suggestion/error）。失敗時 [E2001] エラー | `suggest_branch_name` | `branch_suggest.rs`, `AgentLaunchForm.svelte` | Implemented |
| FR-016 | ベースブランチ: `list_worktree_branches` + `list_remote_branches` でローカル/リモートを optgroup 分離表示 | `list_worktree_branches`, `list_remote_branches` | `AgentLaunchForm.svelte` | Implemented |
| FR-017 | Issue 連携: `fetch_github_issues` で Issue リスト取得（ページネーション付き）。選択 → `{prefix}issue-{number}` 形式。`find_existing_issue_branches_bulk` で既存ブランチをバルク検索し、存在する Issue は選択不可 | `fetch_github_issues`, `find_existing_issue_branches_bulk` | `issue.rs`, `AgentLaunchForm.svelte` | Implemented |
| FR-018 | プレフィックス AI 判定: 4段階フォールバック — (1) `determinePrefixFromLabels()`: hotfix→`hotfix/`, bug→`bugfix/` (2) キャッシュ (3) `classify_issue_branch_prefix` AI 分類 → `ClassifyResult`（status/prefix/error）(4) ドロップダウン手動選択。`classifyRequestId` カウンターで Stale リクエスト防止 | `classify_issue_branch_prefix` | `issue.rs`, `agentUtils.ts`, `agentLaunchFormHelpers.ts`, `AgentLaunchForm.svelte` | Implemented |
| FR-019 | ブランチ名コンフリクト検出: `find_existing_issue_branches_bulk` で Issue 選択時に既存ブランチ（`IssueBranchMatch`）を検出し、重複 Issue を選択不可にする | `find_existing_issue_branches_bulk` | `issue.rs`, `AgentLaunchForm.svelte` | Implemented |

#### Shell & Environment (ex-#1306)

| ID | 要件 | Tauri コマンド | 対象ファイル | 状態 |
|----|------|---------------|-------------|------|
| FR-020 | シェル選択: `get_available_shells` で `ShellInfo`（id/name/version）のリスト取得。Advanced セクション内 select。Docker モード時は disabled | `get_available_shells` | `terminal.rs`, `AgentLaunchForm.svelte` | Implemented |
| FR-021 | Extra args: Advanced textarea、改行区切り1行1引数。`parseExtraArgs()` で空行除去 | — | `agentLaunchFormHelpers.ts` | Implemented |
| FR-022 | 環境変数オーバーライド: Advanced textarea、`KEY=VALUE` 形式。`#` コメント対応。`parseEnvOverrides()` でバリデーション（行番号付きエラー）。最高優先度でマージ | — | `agentLaunchFormHelpers.ts` | Implemented |
| FR-023 | OS env キャプチャ: deps ステップで `wait_os_env_ready(2s)` 待機。タイムアウト時は `process_env` フォールバック。マージ優先順: プロファイル → GLM → リクエスト overrides | — | `terminal.rs:4339-4449` | Implemented |

#### Docker Integration

| ID | 要件 | Tauri コマンド | 対象ファイル | 状態 |
|----|------|---------------|-------------|------|
| FR-024 | コンテキスト検出: ブランチ切替時に `detect_docker_context` 自動実行。`DockerContext`（file_type/compose_services/docker_available/compose_available/daemon_running/force_host/container_status/images_exist）を返却 | `detect_docker_context` | `docker.rs`, `AgentLaunchForm.svelte` | Implemented |
| FR-025 | ランタイム選択: Docker 検出時に HostOS/Docker トグル。Docker 未インストール or compose 未利用可の場合は disabled | — | `AgentLaunchForm.svelte` | Implemented |
| FR-026 | サービス選択: compose/devcontainer 時にサービス select 表示。`resolveDockerContextSelection()` で pending preference → 現在選択 → 最初のサービスの優先度で自動選択 | — | `agentLaunchFormHelpers.ts` | Implemented |
| FR-027 | Build/Recreate/Keep: `images_exist: false` 時は Build 自動 ON + disabled、`container_status: "not_found"` 時は Recreate 自動 ON + disabled。Keep はエージェント終了後もコンテナ維持 | — | `AgentLaunchForm.svelte` | Implemented |
| FR-028 | ステータスヒント: `dockerStatusHint()` で images/containers 状態を人間可読表示（例: "No images / No containers - will build and create automatically"） | — | `agentLaunchFormHelpers.ts` | Implemented |
| FR-029 | Force host: Settings の `docker_force_host` or リクエストの `dockerForceHost` で Docker をスキップ | — | `AgentLaunchForm.svelte`, `terminal.rs` | Implemented |

#### Launch Pipeline

| ID | 要件 | Tauri コマンド | 対象ファイル | 状態 |
|----|------|---------------|-------------|------|
| FR-030 | 非同期ジョブ: `start_launch_job` で UUID ジョブ ID を返却し、別スレッドで実行。`launch-progress`/`launch-finished` Tauri イベントで進捗通知 | `start_launch_job` | `terminal.rs` | Implemented |
| FR-031 | 7ステップ進捗（詳細後述） | — | `terminal.rs:4088-4449` | Implemented |
| FR-032 | キャンセル: `cancel_launch_job` で AtomicBool フラグセット。各ステップ間で `is_launch_cancelled()` チェック。Esc / Cancel 対応 | `cancel_launch_job` | `terminal.rs`, `LaunchProgressModal.svelte` | Implemented |
| FR-033 | ポーリング: `poll_launch_job` で `LaunchJobPollResult`（running/finished）を返却。Tauri イベント消失時の復旧用。`launch_results` に結果格納後にイベント発火（順序保証） | `poll_launch_job` | `terminal.rs` | Implemented |
| FR-034 | Worktree 作成: create ステップで新規ブランチ or 既存ブランチの Worktree を作成。Issue 連携時は `create_or_verify_linked_branch` 使用。作成後に `worktrees-changed` イベント発火 | — | `terminal.rs:4140-4237` | Implemented |
| FR-035 | スキル登録: skills ステップで `repair_skill_registration_with_settings_at_project_root()`。CLAUDE.md/AGENTS.md/GEMINI.md へのマネージドスキルブロック挿入 | — | `terminal.rs:4282-4334` | Implemented |
| FR-036 | コマンド解決: deps ステップでエージェント毎の CLI コマンド・引数を構築。`ResolvedAgentLaunchCommand`（command/args/label/tool_version/version_for_gates）。パッケージランナー選択: `preferred_launch_runner()` で bunx/npx を判定 | — | `terminal.rs:697-796` | Implemented |
| FR-037 | セッション履歴: 起動成功時にセッション情報を記録 | — | `terminal.rs` | Implemented |
| FR-038 | Agent Teams: Claude Code 起動時に `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` 環境変数を自動設定 | — | `terminal.rs` | Implemented |

**FR-031 7ステップ進捗詳細:**

| Step | Name | 実行内容 |
|------|------|---------|
| 1 | `fetch` | 初期セットアップ、キャンセルチェック |
| 2 | `validate` | agent_id バリデーション、非推奨 Codex 設定キーチェック |
| 3 | `paths` | `project_root` からリポジトリパス解決 |
| 4 | `conflicts` | ブランチ/Worktree コンフリクトチェック、キャンセルチェック |
| 5 | `create` | ブランチ/Worktree 作成、Issue リンクブランチ処理、AI ブランチ名生成 |
| 6 | `skills` | スキル登録、CLAUDE.md/AGENTS.md/GEMINI.md マネージドブロック挿入 |
| 7 | `deps` | OS 環境キャプチャ待機(2s)、プロファイル環境マージ、OpenAI API Key 注入、GWT_PROJECT_ROOT 設定、GLM プロバイダ設定、envOverrides 適用、IS_SANDBOX/AGENT_TEAMS 設定、TERM/COLORTERM デフォルト設定 |

#### Entry Points

| ID | 要件 | 対象ファイル | 状態 |
|----|------|-------------|------|
| FR-039 | サイドバー activate: ブランチ選択状態で Launch ボタン → `AgentLaunchForm` を `selectedBranch` 付きでオープン | `AgentLaunchForm.svelte` | Implemented |
| FR-040 | Launch ボタン: ダイアログ内の Launch ボタン。各種バリデーション条件充足時のみ有効化 | `AgentLaunchForm.svelte` | Implemented |
| FR-041 | メニューコマンド: アプリケーションメニューから Launch Agent コマンド選択で `AgentLaunchForm` 表示 | `AgentLaunchForm.svelte` | Implemented |
| FR-042 | Work on Issue: Issues タブ "Work on this" → `prefillIssue` 経由でダイアログオープン。`branchMode="new"`, `newBranchTab="fromIssue"`, 該当 Issue 自動選択 | `AgentLaunchForm.svelte` | Implemented |
| FR-043 | Quick launch: 外部コンテキストから `LaunchAgentRequest` を直接構築して `launch_agent` or `start_launch_job` を呼出 | `terminal.rs` | Implemented |

#### Error Handling

| ID | 要件 | 対象ファイル | 状態 |
|----|------|-------------|------|
| FR-044 | ブランチ存在リカバリ（E1004）: エラーメッセージに `[E1004]` 含む場合、"Use Existing Branch" ボタンを表示。既存ブランチでの起動を選択可能 | `LaunchProgressModal.svelte` | Implemented |
| FR-045 | Issue ブランチロールバック: `created_issue_branch_for_cleanup` で追跡。失敗時に `rollback_new_issue_branch()` で Worktree + ブランチを自動クリーンアップ。ロールバック失敗は warning ログのみ | `terminal.rs` | Implemented |
| FR-046 | GLM バリデーション: フロントエンドで Base URL/API Token 空チェック → launch 中断。バックエンドでも環境変数マージ後に空チェック → エラー返却 | `AgentLaunchForm.svelte`, `terminal.rs` | Implemented |
| FR-047 | Docker バリデーション: Docker 未利用可エラー、サービス未選択エラー、compose ファイル内サービス未発見エラー | `AgentLaunchForm.svelte` | Implemented |

#### From Issue 改善

| ID | 要件 | 対象ファイル | 状態 |
|----|------|-------------|------|
| FR-048 | カテゴリ完全自動化: プレフィックス選択ドロップダウンを廃止し、常に自動決定する。判定フロー: (1) `determinePrefixFromLabels()`: hotfix→`hotfix/`, bug→`bugfix/` (2) `classify_issue_branch_prefix` AI 分類 (3) デフォルト: AI 未設定・失敗時は `feature/` を自動適用。**変更対象**: `AgentLaunchForm.svelte`（プレフィックスドロップダウン `<select bind:value={newBranchPrefix}>` 削除、`prefixClassifying` スピナー UI 削除）, `agentLaunchFormHelpers.ts`（`classifyIssuePrefix()` の空文字フォールバック → `"feature/"` に変更） | `AgentLaunchForm.svelte:1370-1385`, `agentLaunchFormHelpers.ts:231` | Implemented |

---

### Success Criteria

| ID | 検証条件 | 関連 FR |
|----|---------|---------|
| SC-001 | 全既存テスト（FR-001〜FR-047）が GREEN | FR-001〜FR-047 |
| SC-002 | FR-048 実装後、新規テスト（AI 未設定時 `feature/` デフォルト、AI エラー時 `feature/` フォールバック、ドロップダウン非表示、分類中インジケーター）が GREEN | FR-048 |
| SC-003 | `cd gwt-gui && pnpm test` で全テスト通過 | ALL |
| SC-004 | `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` で型チェック通過 | ALL |
| SC-005 | FR-048 で削除されるドロップダウン UI 要素がレンダリングされないことを確認するテストが存在する | FR-048 |
| SC-006 | `determinePrefixFromLabels()` → AI 分類 → `feature/` フォールバックの全パスが個別テストでカバーされている | FR-048 |

---
