### 背景

現在の gwt は Claude Code, Codex, Gemini, OpenCode の 4 つの AI エージェントをサポートし、検出・設定・起動・セッション管理を行っている。エージェントの起動は多段階プロセス（検出→設定→起動→セッション追跡）で構成され、PTY 経由で入出力を管理する。

Unity 6 移行に伴い、これらのエージェント管理機能を C# で再実装する必要がある。2D 世界ではエージェントが 2D キャラクターとして表現され、ステータス（running, idle, waiting_input, stopped）がリアルタイムに反映される。

セッション情報は `~/.gwt/sessions/` に永続化され、アプリ再起動後のリジューム（再開）をサポートする。Project Mode では Lead エージェントが自動的にサブエージェントを起動・委任する。

> **実装方式変更（SPEC 更新）**: PTY 管理について、当初 SPEC では **Pty.Net** 経由で PTY を起動する計画だったが、実装では **`IPtyService.SpawnAsync()` → `System.Diagnostics.Process` ベース PTY** を採用した（#1540 参照）。
>
> **DI パターン変更**: VContainer の `[Inject]` アトリビュートはシーン上の MonoBehaviour に対して未設定のため、`TerminalOverlayPanel` 等では **`LifetimeScope.Find()` による手動解決パターン**を使用している。

現行の実装構成:

| ファイル | 役割 |
|---|---|
| `AgentService.cs` | `IPtyService` + `ITerminalPaneManager` 注入、`HireAgentAsync` で PTY 起動 + ペイン作成 |
| `TerminalPaneManager.cs` | 複数ペイン管理、`ActiveIndex`、タブ切替 |
| `TerminalPaneState.cs` | `PaneId`, `AgentSessionId`, `PtySessionId`, `XtermSharpTerminalAdapter`, `Status` |
| `TerminalOverlayPanel.cs` | VContainer 手動解決（`LifetimeScope.Find`）、自動シェル起動 |
| `UIManager.cs` | `OpenTerminal`/`ToggleTerminal`、F1 キーショートカット、`Start()` で自動オープン |

#### 雇用メタファー

エージェントのライフサイクルをゲーム的な「雇用」メタファーで表現する:

- **エージェント起動 = 「開発者を雇用」**: worktree に対してエージェントを起動する行為を「雇う（Hire）」と表現する ✅ 実装済み（`HireAgentAsync`）
- **エージェント停止 = 「解雇」**: エージェントを停止する行為を「解雇する（Fire）」と表現する
- UI 上のボタンラベル、ログメッセージ、通知等でこのメタファーを一貫して使用する

#### ジョブタイプ（ペルソナ）

エージェントに「役職」を割り当てる仕組み。カイロソフト「ゲーム発展途上国」の「プログラマー」「デザイナー」に相当:

- **フロントエンド開発者（Frontend Developer）**
- **バックエンド開発者（Backend Developer）**
- **DevOps エンジニア（DevOps Engineer）**
- **QA エンジニア（QA Engineer）** 等
- **初期リリースではジョブタイプは汎用（General）のみ。Phase 5 でジョブタイプ分化を実装する**
- **将来拡張**: ジョブタイプに応じたシステムプロンプト変更等で、エージェントの振る舞いに機能的な影響を与える

#### キャラクターデザイン

- **エージェント種別ごとのデザイン**: Claude Code, Codex, Gemini CLI, OpenCode 等のエージェント種別ごとに異なるピクセルアートスプライトを用意する
- **ジョブタイプごとのバリエーション**: 同一エージェント種別でもジョブタイプ（Frontend, Backend, DevOps 等）ごとに異なるスプライトバリエーションを持つ
- **組み合わせ**: エージェント種別 × ジョブタイプ の掛け合わせでスプライトが決定される

#### 空席デスク

- Worktree は存在するがエージェントが未起動の場合、**デスクはあるがキャラクターがいない（空席）** 状態で表示する
- 空席デスクをクリックすると、エージェント雇用（起動）のオーバーレイ UI が表示される

#### PTY ライフサイクル

- エージェントプロセスは **`IPtyService.SpawnAsync()` → `System.Diagnostics.Process` ベースで** Unity 内の子プロセスとして管理する（#1540 参照）
- **アプリ終了 = 全セッション終了**: アプリケーション終了時に全エージェントプロセスを終了する
- デーモン分離（バックグラウンドプロセスとして独立して動作し続ける）は不要。アプリが閉じれば全エージェントが停止する
- **アプリ終了時に稼働中エージェントがあれば確認ダイアログを表示し、承認後に全エージェントを graceful 停止する**
- **クラッシュ復帰**: Unity クラッシュ時はセッション復元のみ行う。PTY は新規起動し、worktree のコード変更は Git 上に残存する

#### DevContainer 対応

- エージェントのみをコンテナ内で実行する対応をサポートする
- gwt 本体（Unity アプリ）はホスト OS で動作し、エージェントプロセスのみ DevContainer 内で起動する

#### agents.toml 互換

- ClaudeAgentConfig の C# 移植仕様をサポートする（Anthropic/GLM プロバイダー設定）
- 既存の agents.toml 形式との互換性を維持する

#### Claude Code hooks・スキル統合

- Unity アプリと Claude Code hooks 間の双方向通信をサポートする（hooks 経由のコールバック受信）
- launch target worktree への project-scoped skill registration をサポートする
- gwt スキルが各エージェント (Claude Code, Codex 等) の available skills に正しく露出する仕組みを提供する

#### エージェント起動方式

- Lead 経由の自動起動（Project Mode での委任）と、ユーザー手動の Launch Agent の両方をサポートする

#### ターミナルペイン管理（実装済み）

> **新規セクション（SPEC 更新）**: 実装で追加されたターミナルペイン管理の構成を記載。

- **`TerminalPaneManager`**: 複数ペインの管理、`ActiveIndex` によるアクティブペイン追跡、タブ切替
- **`TerminalPaneState`**: ペインごとの状態（PaneId, AgentSessionId, PtySessionId, XtermSharpTerminalAdapter, Status）
- **`TerminalOverlayPanel`**: VContainer 手動解決（`LifetimeScope.Find()`）、初回オープン時に自動シェル起動
- **`UIManager`**: `OpenTerminal`/`ToggleTerminal` メソッド、F1 キーショートカット、`Start()` で自動オープン

#### 再実装対象コマンド

| カテゴリ | コマンド |
|---------|---------|
| Detection | `detect_agents`, `list_agent_versions` |
| Config | `get_agent_config`, `save_agent_config` |
| Launch | `launch_terminal`, `spawn_shell`, `launch_agent`, `start_launch_job`, `cancel_launch_job`, `poll_launch_job` |
| Sessions | `get_branch_quick_start`, `get_agent_sidebar_view`, `get_branch_session_summary`, `rebuild_all_branch_session_summaries` |
| Skills | `check_and_fix_agent_instruction_docs`, `get_skill_registration_status_cmd`, `repair_skill_registration_cmd` |

#### 主要データ型

| 型名 | フィールド |
|------|-----------|
| `AgentStatusValue` | enum: Unknown, Idle, Running, WaitingInput, Stopped |
| `JobType` | enum: FrontendDeveloper, BackendDeveloper, DevOpsEngineer, QAEngineer, General |
| `Session` | id, worktree_path, branch, agent, agent_session_id, tool_version, model, status, job_type, created_at, updated_at, stopped_at |
| `AgentConfig` | mode_args, model_overrides, job_type |
| `PaneLaunchMeta` | agent_id, branch, repo_path, worktree_path, tool_label, tool_version, mode, model, docker_options |
| `LaunchAgentRequest` | agent_id, branch, worktree_path, mode, model, prompt, resume_session_id, docker_options, job_type |
| `AgentInfo` | id, name, version, path, is_available |
| `BranchSessionSummary` | branch, sessions, active_count, total_count |
| `TerminalPaneState` | PaneId, AgentSessionId, PtySessionId, XtermSharpTerminalAdapter, Status — **新規追加（実装済み）** |

#### インタビュー確定事項（2026-03-10追記）

以下はユーザーインタビューで確定した追加仕様:

**1 Issue : N Agent（混在型）:**
- 1つのIssueに対して複数のAgentを同時に割り当て可能
- 異なるエージェント種別の混在が可能（例: Claude Code + Codex を同一Issueに）
- 全Agentは同一worktreeを共有する
- ファイル競合はエージェント側の責任として許容する（gwt側では制御しない）
- Leadがタスク割当時にファイル競合を避ける指示を出すかどうかは技術判断に委任（基本方針: エージェント側の責任を維持）

**プロファイル方式カスタムAgent:**
- ユーザーカスタムAgentはJSON形式のプロファイルで定義
- プロファイルにはCLIパス + 引数を指定
- プロファイルの管理UIはSettings画面内に配置

**Agent上限 = スタジオレベル連動:**
- 同時起動Agent数の上限はスタジオレベルに連動（固定値ではない）
- スタジオレベルはコミット数ベースのレベルシステム（#1555参照）
- 初期レベルでは少数、レベルアップで上限増加

**完了時自動PR作成:**
- Agentがタスク完了時に自動的にPRを作成する
- Lead監視サイクルのPR作成検知と連動

**即時running + アニメーション後追い:**
- Agent雇用時、プロセス起動は即座に実行（running状態に遷移）
- ドアからの入場アニメーションは装飾的に並行再生（プロセス起動を待たない）
- 状態変更の即時性 > アニメーションの完了

**エラー時の挙動:**
- エラー発生時はトースト通知でユーザーに通知
- 自動リトライは行わない（手動リトライをユーザーが選択）
- Leadによる自律リトライ（FR-019）とは別レイヤー

**Agent検出:**
- Agentの検出はgwt経由のみ（外部ツールからの検出は不要）
- gwt起動時に1回検出し、結果をキャッシュ（既存NFR-006と一致）

**オフライン時の挙動:**
- ネットワーク切断時はローカル機能のみ継続（PTY、Git操作）
- GitHub連携機能は一時停止（ポーリング停止、エラー通知なし）
- 再接続時に自動復帰

**クラッシュ復帰:**
- Unityクラッシュ時: セッション復元のみ
- PTYは新規起動、worktreeのコード変更はGit上に残存

### ユーザーシナリオ

| ID | シナリオ | 優先度 | 実装状態 |
|---|---|---|---|
| US-1 | 2D 世界でエージェントが 2D キャラクターとして配置・表示される | P0 | 🔲 未実装 |
| US-2 | エージェントクリック→ターミナル UI 表示→対話可能 | P0 | ✅ 実装済み（`TerminalOverlayPanel` + F1 ショートカット） |
| US-3 | Lead が自動的にエージェントを起動・配置する | P0 | 🔲 未実装 |
| US-4 | エージェントのステータスが 2D 世界で視覚的に表現される（idle, thinking, waiting 等） | P0 | 🔲 未実装 |
| US-5 | アプリ再起動後にセッションを復元できる | P1 | 🔲 未実装 |
| US-6 | エージェント起動時に「Hire」、停止時に「Fire」の雇用メタファーで操作できる | P0 | ⚠️ 部分実装（`HireAgentAsync` メソッド名で採用。UI ボタンラベルは未実装） |
| US-7 | エージェントにジョブタイプ（Frontend, Backend 等）を割り当て、見た目が変わる | P1 | 🔲 未実装（Phase 5） |
| US-8 | Worktree はあるがエージェント未起動の場合、空席デスクが表示される | P0 | 🔲 未実装 |
| US-9 | DevContainer 内でエージェントを実行できる | P1 | 🔲 未実装 |
| US-10 | ユーザーが手動で Launch Agent を実行してエージェントを起動できる | P0 | ✅ 実装済み（`TerminalOverlayPanel` 自動シェル起動） |
| US-11 | アプリ終了時に稼働中エージェントがある場合、確認ダイアログが表示される | P0 | 🔲 未実装 |

### 機能要件

| ID | 要件 | 実装状態 |
|---|---|---|
| FR-001 | システム上の利用可能なエージェント（Claude Code, Codex, Gemini, OpenCode）を自動検出する | 🔲 未実装 |
| FR-002 | エージェントを **`IPtyService.SpawnAsync()` → Process ベース PTY** 経由でプロセスとして起動し、PTY 経由で入出力を管理する | ✅ **SPEC 変更: Pty.Net → Process ベース（#1540 参照）** |
| FR-003 | 起動ジョブの非同期管理（進捗追跡、キャンセル）をサポートする | 🔲 未実装 |
| FR-004 | エージェントのステータス（Idle, Running, WaitingInput, Stopped）をリアルタイム追跡する | 🔲 未実装 |
| FR-005 | セッション情報を `~/.gwt/sessions/` に永続化し、再起動後のリジューム（再開）をサポートする。Unity クラッシュ時はセッション復元のみ行い、PTY は新規起動する（worktree のコード変更は Git 上に残存） | 🔲 未実装 |
| FR-006 | ブランチごとのセッション・エージェント履歴を管理する | 🔲 未実装 |
| FR-007 | エージェント設定（モード引数、モデルオーバーライド）を管理する | 🔲 未実装 |
| FR-008 | Lead からの委任による自動エージェント起動をサポートする | 🔲 未実装 |
| FR-009 | 個別のエージェント起動（非 Project Mode）もサポートする | ✅ 実装済み（`AgentService.HireAgentAsync`） |
| FR-010 | VContainer で `IAgentService`, `ISessionService` として DI 登録する | ⚠️ 部分実装（VContainer DI 登録済み。ただしシーン MonoBehaviour では `LifetimeScope.Find` による手動解決パターンを使用） |
| FR-011 | エージェント起動を「Hire」、停止を「Fire」の雇用メタファーで UI 表現する | ⚠️ 部分実装（メソッド名で採用。UI ラベルは未実装） |
| FR-012 | ジョブタイプ（Frontend Developer, Backend Developer, DevOps Engineer, QA Engineer, General）をエージェントに割り当てる機能を提供する | 🔲 未実装（Phase 5） |
| FR-013 | エージェント種別 × ジョブタイプの組み合わせに応じたピクセルアートスプライトを表示する | 🔲 未実装 |
| FR-014 | Worktree 存在 + エージェント未起動の場合、空席デスク（キャラクターなし）を表示する | 🔲 未実装 |
| FR-015 | 空席デスクのクリックでエージェント雇用（起動）UI を表示する | 🔲 未実装 |
| FR-016 | アプリケーション終了時に全エージェントプロセスを終了する（デーモン分離不要） | 🔲 未実装 |
| FR-017 | エージェントプロセスを DevContainer 内で実行するオプションをサポートする | 🔲 未実装 |
| FR-018 | 初期リリースではジョブタイプは汎用 (General) のみ。Phase 5 でジョブタイプ分化を実装する | — (Phase 5) |
| FR-019 | agents.toml 互換 — ClaudeAgentConfig の C# 移植仕様をサポートする（Anthropic/GLM プロバイダー設定） | 🔲 未実装 |
| FR-020 | エージェント起動は Lead 経由の自動起動 + ユーザー手動の Launch Agent の両方をサポートする | ⚠️ 部分実装（手動起動のみ実装済み） |
| FR-021 | アプリ終了時に稼働中エージェントがあれば確認ダイアログを表示し、承認後に全エージェントを graceful 停止する | 🔲 未実装 |
| FR-022 | Claude Code hooks 連携 — Unity アプリと Claude Code hooks 間の双方向通信をサポートする | 🔲 未実装 |
| FR-023 | スキル埋め込み — launch target worktree への project-scoped skill registration をサポートする | 🔲 未実装 |
| FR-024 | gwt スキルが各エージェント (Claude Code, Codex 等) の available skills に正しく露出する仕組みを提供する | 🔲 未実装 |
| FR-025 | **ターミナルペイン管理（`TerminalPaneManager`）で複数エージェントのペインを管理し、タブ切替を提供する** | ✅ **新規追加（実装済み）** |
| FR-026 | **`TerminalOverlayPanel` でオーバーレイ UI を表示し、自動シェル起動する** | ✅ **新規追加（実装済み）** |
| FR-027 | **`UIManager` で F1 キーショートカットによるターミナル表示切替を提供する** | ✅ **新規追加（実装済み）** |
| FR-028 | 1 Issue に対して複数 Agent（異なる種別の混在可）を同時割り当てする機能を提供する | 🔲 未実装 |
| FR-029 | 全 Agent が同一 worktree を共有する構成をサポートする。ファイル競合はエージェント側の責任として許容し、gwt 側では制御しない。Lead がタスク割当時にファイル競合を避ける指示を出すかどうかは技術判断に委任する | 🔲 未実装 |
| FR-030 | ユーザーカスタム Agent を JSON プロファイル（CLI パス + 引数）で定義・管理する機能を Settings 内に提供する | 🔲 未実装 |
| FR-031 | Agent タスク完了時に自動的に PR を作成する | 🔲 未実装 |
| FR-032 | Agent 雇用時にプロセスを即時起動し、入場アニメーションは装飾的に並行再生する | 🔲 未実装 |

### 非機能要件

| ID | 要件 | 実装状態 |
|---|---|---|
| NFR-001 | エージェントプロセスは Unity メインスレッドとは別スレッドで管理し、UI をブロックしない | ✅ 実装済み（async/await + Process 非同期 I/O） |
| NFR-002 | PTY 入出力は低レイテンシ（< 50ms）で処理し、ターミナル表示の遅延を最小化する | ✅ 実装済み |
| NFR-003 | エージェントプロセスのクラッシュ時は自動検出し、セッション状態を Stopped に更新する。Unity クラッシュ時はセッション復元のみ行い、PTY は新規起動する（worktree のコード変更は Git 上に残存） | ⚠️ 部分実装（`PtySession.ProcessExited` イベントで検出。セッション状態管理は未実装） |
| NFR-004 | 同時起動エージェント数の上限をスタジオレベルに連動させる（レベルアップで上限増加、#1555参照） | 🔲 未実装 |
| NFR-005 | セッション永続化ファイルは JSON 形式で、手動編集・デバッグが容易な構造にする | 🔲 未実装 |
| NFR-006 | エージェント検出は起動時に 1 回実行し、結果をキャッシュする（手動リフレッシュ可能） | 🔲 未実装 |
| NFR-007 | AgentManager は VContainer で Singleton ライフタイムとして DI 登録する | ✅ 実装済み |
| NFR-008 | **シーン MonoBehaviour への DI は `LifetimeScope.Find` による手動解決パターンを使用する** | ✅ **新規追加（実装済み）** |

### 成功基準

| ID | 基準 | 実装状態 |
|---|---|---|
| SC-001 | Claude Code, Codex, Gemini, OpenCode の検出・起動が動作する | ⚠️ 部分実装（シェル起動は動作。エージェント固有の検出・起動は未実装） |
| SC-002 | エージェントのライフサイクルが 2D 世界でリアルタイム反映される | 🔲 未実装 |
| SC-003 | セッション永続化・復元が正しく動作する | 🔲 未実装 |
| SC-004 | 全 FR に対応するユニットテストが存在し、パスする | 🔲 未実装 |
| SC-005 | 雇用メタファー（Hire/Fire）が UI 全体で一貫して使用されている | ⚠️ 部分実装（メソッド名のみ） |
| SC-006 | ジョブタイプごとにスプライトが切り替わる | 🔲 未実装（Phase 5） |
| SC-007 | agents.toml 互換設定が正しく読み込み・保存される | 🔲 未実装 |
| SC-008 | Claude Code hooks 経由のコールバック受信が動作する | 🔲 未実装 |
| SC-009 | gwt スキルが各エージェントの available skills に露出する | 🔲 未実装 |
| SC-010 | **ターミナルペイン管理（タブ切替、複数ペイン）が動作する** | ✅ **新規追加（実装済み）** |
| SC-011 | **F1 キーショートカットでターミナルオーバーレイが表示切替できる** | ✅ **新規追加（実装済み）** |
