> **📜 HISTORICAL (SPEC-1776)**: This SPEC was written for the previous GUI stack (Tauri/Svelte/C#). It is retained as a historical reference. The gwt-tui migration (SPEC-1776) supersedes GUI-specific design decisions described here.

### 背景

gwt のワークツリー管理は現在すべて手動操作に依存している。利用者がワークツリーの作成・エージェントへの割り当て・進捗監視・PR 作成・マージ・ワークツリー削除を個別に行う必要があり、大規模開発では「どのワークツリーで何をしていたか」が管理不能になる。

\#1549 で定義された Lead（AI オーケストレーター）は常時アクティブなシステムとしてスタジオ内に存在するが、現状の実装（LeadOrchestrator）はユーザーコマンドのエコー返却と基本的なエージェント監視のみ。LLM を活用したタスク計画機能と、既存サービス（IGitService, IAgentService, IGitHubService）を統合するオーケストレーションロジックが欠けている。

**目標**: 利用者がやりたいことだけ自然言語で伝え、Lead が自律的にタスク分割 → ワークツリー作成 → エージェント雇用 → 進捗監視 → PR 作成 → マージを行う。

### 既存ビルディングブロック（再利用対象）

| サービス | Assembly | 使用 API |
|---------|----------|---------|
| `IGitService` | Gwt.Core | `CreateWorktreeAsync`, `DeleteWorktreeAsync`, `ListWorktreesAsync`, `GetCurrentBranchAsync`, `ListBranchesAsync` |
| `IGitHubService` | Gwt.Core | `CreatePullRequestAsync`, `GetPrStatusAsync` |
| `IAIApiService` | Gwt.Core | `SendRequestAsync`, `ChatAsync` |
| `IAgentService` | Gwt.Agent | `HireAgentAsync`, `FireAgentAsync`, `GetSessionAsync`, `ListSessionsAsync`, `OnAgentOutput` |
| `IConfigService` | Gwt.Core | `LoadSettingsAsync` → `Settings.Profiles.DefaultAI` → `ResolvedAISettings` |

**Assembly 依存**: `Gwt.Agent` → `Gwt.Core` → `Gwt.Shared`。全インターフェースは `Gwt.Core` にあるため、新たな asmdef 参照追加は不要。`IProjectLifecycleService` は `Gwt.Lifecycle` に属し `Gwt.Agent` から参照不可のため、プロジェクト情報は `IGitService` + `LeadSessionData.ProjectRoot` で代替する。

### Lead Git権限

- Leadはworktreeライフサイクル全体を自律的に操作する権限を持つ: worktree作成 → push → PR作成 → merge → worktree削除
- **禁止操作**: force push (`git push --force`) および rebase (`git rebase`) はLeadに許可しない
- これは #1543（Git操作レイヤー）のFR-011と連携する権限設計

### PTYポーリング間隔

- モニタリングループのポーリング間隔: **4秒固定**（正式確定）
- `UniTask.Delay(TimeSpan.FromSeconds(4))` でメインスレッドをブロックしない

### waiting_input検知

- PTY出力が**30秒間停止**した場合、エージェントが入力待ち状態（`waiting_input`）であると推定する
- この推定は確定的判定ではなく、ヒューリスティックとして使用する

### SPEC生成

- SPEC Issue生成はLead専用の内蔵ツールとして実装する
- LLM function calling用のツール定義として提供し、Leadが自律的にSPEC Issueを作成・更新できるようにする

### ユーザーシナリオ

- **US-1** [P0]: ユーザーが Lead に自然言語でリクエストを送信すると、LLM がタスクを分割し構造化計画（LeadTaskPlan）を生成する
  - テスト: 自然言語リクエストから構造化タスク計画が生成されること
  - テスト: 各タスクにTaskId、ブランチ名、WorktreeStrategy等が設定されること
- **US-2** [P0]: ユーザーが計画を確認し、承認すると Lead が自律的に実行を開始する
  - テスト: 承認操作で Status が draft → approved に遷移すること
  - テスト: OnPlanUpdated イベントが発火すること
- **US-3** [P0]: 承認された計画の各タスクに対し、Lead がワークツリーを自動作成し、適切なエージェント（Claude/Codex/Gemini）を雇用する
  - テスト: タスクごとにワークツリーが作成されること
  - テスト: 適切なDetectedAgentTypeのエージェントが雇用されること
- **US-4** [P0]: 依存関係のあるタスクは先行タスク完了後に順次開始される。依存なしタスクは並列実行される
  - テスト: DependsOnで指定された先行タスク完了後にタスクが開始されること
  - テスト: 依存なしタスクが並列で開始されること
- **US-5** [P1]: `shared` WorktreeStrategy のタスクは同一ワークツリーを再利用し、不要なワークツリー生成を防ぐ
  - テスト: shared指定タスクが同一ワークツリーを共有すること
- **US-6** [P1]: Lead がエージェント出力を AgentOutputBuffer 経由で監視し、LLM で完了/失敗を判定する
  - テスト: AgentOutputBuffer に出力が蓄積されること
  - テスト: LLM評価で完了/失敗が正しく判定されること
- **US-7** [P1]: タスク完了後に Lead が自動で PR を作成する
  - テスト: タスク完了時にPRが作成されること
  - テスト: PRタイトルに `[Lead] {task.Title}` が含まれること
- **US-8** [P2]: PR が Mergeable な場合にマージを試行し、成功後にワークツリーを自動削除する
  - テスト: Mergeable判定でマージが実行されること
  - テスト: マージ後にワークツリーが削除されること
- **US-9** [P2]: マージコンフリクト時は LeadQuestion としてユーザーに通知し、手動解決を促す
  - テスト: コンフリクト時にLeadQuestionが生成されること
- **US-10** [P1]: 進捗サマリー（タスク数・完了数・失敗数・PR数）がリアルタイムで更新される
  - テスト: タスク状態変更時にOnProgressChangedイベントが発火すること
  - テスト: GetProgressSummaryの集計値がタスク状態と一致すること
- **US-11** [P1]: ユーザーが計画をキャンセルすると、実行中タスクが停止しリソースが解放される
  - テスト: キャンセル操作で全タスクがfailed状態になること
- **US-12** [P2]: ユーザーが計画にフィードバックを与え、LLM が計画を修正できる（RefinePlanAsync）
  - テスト: フィードバックに基づき計画が修正されること
  - テスト: PlanIdが維持されること

### 機能要件

| ID | 要件 | Phase |
|----|------|-------|
| FR-001 | `ILeadTaskPlanner.CreatePlanAsync` で LLM（IAIApiService.SendRequestAsync）を呼び出し、ユーザーリクエストを `LeadTaskPlan` に変換する | 1 |
| FR-002 | LLM レスポンスから JSON を抽出し、マークダウンコードフェンス・生テキスト両方に対応する（ExtractJson） | 1 |
| FR-003 | 重複 TaskId を自動リナンバーし、一意性を保証する（EnsureUniqueTaskIds） | 1 |
| FR-004 | WorktreeStrategy が未指定の場合、デフォルト値 `"new"` を設定する | 1 |
| FR-005 | `ILeadTaskPlanner.RefinePlanAsync` でユーザーフィードバックに基づき計画を修正し、PlanId を維持する | 1 |
| FR-006 | `ProcessUserCommandAsync` を LLM タスク計画呼び出しに置き換え、会話履歴に記録する | 1 |
| FR-007 | `ApprovePlanAsync` で Status を `draft → approved` に遷移し、OnPlanUpdated イベントを発火する | 2 |
| FR-008 | `ExecutePlanAsync` で依存関係グラフに基づきタスクを順序制御する（DependsOn 参照） | 2 |
| FR-009 | `new` WorktreeStrategy のタスクごとに `IGitService.CreateWorktreeAsync` でワークツリーを作成する | 2 |
| FR-010 | `shared` WorktreeStrategy のタスクは同一ブランチのワークツリーを再利用する | 2 |
| FR-011 | `IAgentService.HireAgentAsync` で適切な DetectedAgentType のエージェントを雇用し、Instructions を渡す | 2 |
| FR-012 | タスク開始時に Status を `running` に更新し、OnTaskStatusChanged イベントを発火する | 2 |
| FR-013 | 先行タスクが `failed` の場合、依存タスクも `failed` にカスケードする | 2 |
| FR-014 | `AgentOutputBuffer` がセッション別に最新 N 行の出力を保持し、容量超過時に古い行を破棄する | 3 |
| FR-015 | エージェント停止時に AgentOutputBuffer の直近出力を LLM で評価し、完了/失敗を判定する（EvaluateTaskCompletionAsync） | 3 |
| FR-016 | LLM 評価失敗時はフォールバックとして出力内のキーワード（completed, done, finished）で判定する | 3 |
| FR-017 | タスク完了後に `ILeadMergeManager.CreateTaskPrAsync` で PR を自動作成する（タイトル: `[Lead] {task.Title}`） | 4 |
| FR-018 | `TryMergeAsync` で `PrStatusInfo.Mergeable == "MERGEABLE"` を判定する | 4 |
| FR-019 | マージ後に `CleanupWorktreeAsync` でワークツリーを削除し、WorktreePath を null に設定する | 4 |
| FR-020 | `GetProgressSummary` で ActivePlan のタスク状態からリアルタイム集計を返す | 5 |
| FR-021 | OnProgressChanged イベントがタスクステータス変更時に自動発火する | 5 |
| FR-022 | `CancelPlanAsync` で実行中/保留中タスクを全て `failed` に設定し、計画を終了する | 2 |
| FR-023 | PTY出力停止30秒でwaiting_input状態を推定する | 3 |
| FR-024 | SPEC Issue生成をLead専用内蔵ツール（LLM function calling用）として実装する | 1 |
| FR-025 | Lead Git操作でforce push/rebaseを禁止するガードを実装する | 2 |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | タスク計画生成（LLM 呼び出し）は 30 秒以内にタイムアウトする |
| NFR-002 | ワークツリー作成・エージェント雇用は並列実行し、独立タスクの直列化を避ける |
| NFR-003 | AgentOutputBuffer のメモリ使用量を制限する（デフォルト maxLines=100） |
| NFR-004 | モニタリングループは**4秒間隔（固定）**でメインスレッドをブロックしない（UniTask.Delay） |
| NFR-005 | 全新規サービスは VContainer で Singleton 登録し、ライフタイム管理をフレームワークに委譲する |
| NFR-006 | LeadSessionData のシリアライズ/デシリアライズは JsonUtility で互換性を維持する |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | ユーザーの自然言語リクエストから構造化タスク計画が正しく生成される |
| SC-002 | 計画承認後、指定数のワークツリーが作成され対応エージェントが雇用される |
| SC-003 | 依存関係のあるタスクが正しい順序で実行され、並列実行可能なタスクは同時に開始される |
| SC-004 | `shared` WorktreeStrategy のタスクが同一ワークツリーを共有する |
| SC-005 | タスク完了後に PR が自動作成され、タイトルにタスク名が含まれる |
| SC-006 | Mergeable でない PR に対して TryMergeAsync が false を返す |
| SC-007 | ワークツリー削除後に task.WorktreePath が null になる |
| SC-008 | 進捗サマリーのカウントがタスク状態と一致する |
| SC-009 | 全 20 TDD テストケースが GREEN |
| SC-010 | force push/rebaseコマンドがLeadから実行された場合に拒否される |
| SC-011 | PTY出力停止30秒でwaiting_input推定が正しく動作する |

### 主要データ型

| 型名 | フィールド | 用途 |
|------|-----------|------|
| `LeadTaskPlan` | PlanId, ProjectRoot, UserRequest, Tasks, CreatedAt, Status | タスク計画の全体管理 |
| `LeadPlannedTask` | TaskId, Title, Description, WorktreeStrategy, SuggestedBranch, AgentType, Instructions, DependsOn, Priority, Status, WorktreePath, Branch, AgentSessionId, PrNumber | 個別タスクの定義と実行状態 |
| `ProjectContext` | ProjectRoot, DefaultBranch, CurrentBranch, AvailableAgents, ExistingBranches | LLM に渡すプロジェクト情報 |
| `LeadProgressSummary` | TotalTasks, CompletedTasks, RunningTasks, FailedTasks, PendingTasks, CreatedPrCount, MergedPrCount | 進捗レポート |

### ステータス遷移

**LeadTaskPlan.Status:**
```
draft → approved → executing → completed / failed
```

**LeadPlannedTask.Status:**
```
pending → running → completed / failed
```

### 実装済みファイル

**新規作成（9 ファイル）:**

| ファイル | 内容 |
|---------|------|
| `Agent/Lead/LeadTaskPlan.cs` | LeadTaskPlan + LeadPlannedTask データモデル |
| `Agent/Lead/ProjectContext.cs` | プロジェクトコンテキストモデル |
| `Agent/Lead/ILeadTaskPlanner.cs` | タスク計画インターフェース |
| `Agent/Lead/LeadTaskPlanner.cs` | LLM タスク計画実装（AI API 呼び出し、JSON パース、リトライ） |
| `Agent/Lead/AgentOutputBuffer.cs` | セッション別出力バッファ（容量制限付き） |
| `Agent/Lead/ILeadMergeManager.cs` | マージ管理インターフェース |
| `Agent/Lead/LeadMergeManager.cs` | PR 作成・マージ判定・ワークツリー削除実装 |
| `Agent/Lead/LeadProgressSummary.cs` | 進捗サマリーモデル |
| `Tests/Editor/LeadOrchestratorTests.cs` | TDD テスト（20 テストケース） |

パス prefix: `gwt/gwt/Assets/Scripts/Gwt/`

**変更（4 ファイル）:**

| ファイル | 変更内容 |
|---------|---------|
| `Agent/Lead/ILeadService.cs` | PlanTasksAsync, ApprovePlanAsync, ExecutePlanAsync, CancelPlanAsync, GetActivePlan, GetProgressSummary メソッド追加。OnTaskStatusChanged, OnPlanUpdated, OnProgressChanged イベント追加 |
| `Agent/Lead/LeadSessionData.cs` | ActivePlan (LeadTaskPlan), CompletedPlans (List) フィールド追加 |
| `Agent/Lead/LeadOrchestrator.cs` | コンストラクタ拡張（6 依存注入）、LLM 計画・実行・監視・マージ・進捗の全フロー実装 |
| `Agent/Installers/GwtAgentInstaller.cs` | LeadTaskPlanner → ILeadTaskPlanner, LeadMergeManager → ILeadMergeManager の DI 登録追加 |

### DI 構成

```
GwtAgentInstaller.Install():
  ├─ AgentDetector (Singleton)
  ├─ SkillRegistrationService → ISkillRegistrationService (Singleton)
  ├─ AgentService → IAgentService (Singleton)
  ├─ LeadTaskPlanner → ILeadTaskPlanner (Singleton)      ← NEW
  ├─ LeadMergeManager → ILeadMergeManager (Singleton)    ← NEW
  └─ LeadOrchestrator → ILeadService (Singleton)
       ├─ IAgentService
       ├─ IGitService (from GwtCoreGitInstaller)
       ├─ IAIApiService (from GwtAIInstaller)
       ├─ IConfigService (from GwtCoreInstaller)
       ├─ ILeadTaskPlanner
       └─ ILeadMergeManager
```
