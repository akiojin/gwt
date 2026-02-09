# 機能仕様: gwt GUI コーディングエージェント機能のTUI完全移行（Quick Start / Mode / Skip / Reasoning / Version）

**仕様ID**: `SPEC-90217e33`
**作成日**: 2026-02-09
**ステータス**: ドラフト
**カテゴリ**: GUI

**依存仕様**:

- `SPEC-86bb4e7c`（GUI: ブランチフィルター修正・エージェント起動ウィザード・Profiles設定）
- `SPEC-d6210238`（GUI: Phase 1 基盤）

**参照仕様（移植元 / Porting）**:

- `SPEC-3b0ed29b`（コーディングエージェント対応: Mode/Skip/Reasoning/Quick Start/進捗モーダル）
- `SPEC-f47db390`（セッションID永続化とContinue/Resume強化）
- `SPEC-2ca73d7d`（エージェント履歴の永続化）
- `SPEC-fdebd681`（Codex collaboration_modes Support）

**入力**: ユーザー報告: 「TUIから移行済みのはずがGUIでは未移行が多い」「ブランチ選択からエージェント起動後の体験が劣化している（Mode/Skip/モデル/バージョン/設定/履歴）」「OpenCodeが無い」「Quick Startが無い」「Local/Remoteの意味が崩れて見える」「起動が失敗しても理由が分かりづらい」

## 背景

- GUI 版は `Launch Agent...` と Profiles 設定などの最小要件を満たしたが、TUI で成立していた「ブランチごとの作業継続性（Quick Start / Continue）」「エージェント起動オプション（Mode/Skip/Reasoning/Version）」「OpenCode」などが欠落しており、移行完了と見なせない。
- 既存ユーザーは TUI の運用（Worktree 単位でのセッション継続・モデル/推論/権限スキップの切替）に依存しているため、GUI がそれを後退させないことが必要。
- 移行段階では、設定ファイルや履歴ファイルを勝手に初期化（空で保存/意図しない変換/削除）しないことが重要。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Mode/Skip/Reasoning/Version を指定してエージェントを起動 (優先度: P0)

開発者として、GUI の起動ウィザードで TUI と同等の起動オプション（Mode/Skip/Reasoning/Version など）を指定して起動したい。

**独立したテスト**: `launch_agent` の引数組み立てロジックをユニットテストで検証できる。

**受け入れシナリオ**:

1. **前提条件** 起動ウィザードを開いている、**操作** Codex + Mode=Normal で Launch、**期待結果** Codex がデフォルト引数（web search / sandbox / reasoning など）付きで起動する
2. **前提条件** 同上、**操作** Codex + Skip Permissions=ON で Launch、**期待結果** Codex の skip フラグ（バージョンに応じて `--yolo` または `--dangerously-bypass-approvals-and-sandbox`）が付与される
3. **前提条件** 同上、**操作** Codex + Reasoning を変更して Launch、**期待結果** Codex の reasoning 設定が起動引数へ反映される
4. **前提条件** 同上、**操作** Claude Code + Skip Permissions=ON で Launch、**期待結果** `--dangerously-skip-permissions` が付与される（Windows 以外では `IS_SANDBOX=1` を付与）
5. **前提条件** 同上、**操作** Agent Version で `latest` 以外（例: `1.2.3`）を **選択** して Launch、**期待結果** bunx/npx のパッケージ指定が `@...@{version}` になる

---

### ユーザーストーリー 2 - Quick Start でブランチごとの前回設定で再開/新規 (優先度: P0)

開発者として、ブランチを選択した際に、そのブランチで最後に使ったエージェント設定を Quick Start で確認し、ワンクリックで Continue/New を開始したい。

**独立したテスト**: セッション履歴の読み出し（ブランチ→ツール別最新）をユニットテストで検証できる。

**受け入れシナリオ**:

1. **前提条件** ブランチAでCodexを起動した履歴がある、**操作** ブランチAを選択、**期待結果** Summary に Quick Start（Codex 行）が表示される
2. **前提条件** Quick Start に sessionId がある、**操作** Continue を実行、**期待結果** sessionId を指定して Continue 起動される（ツール固有の引数へ反映）
3. **前提条件** Quick Start に sessionId が無い、**操作** Continue を実行、**期待結果** ツールの `--last`/`-c` 等へフォールバックして起動される（UIはクラッシュしない）
4. **前提条件** 同一ブランチで複数ツール利用履歴がある、**操作** Quick Start を確認、**期待結果** ツールごとの行が並び、それぞれ Continue/New を選べる

---

### ユーザーストーリー 3 - ブランチ一覧に直近利用ツールを表示し識別できる (優先度: P1)

開発者として、ブランチ一覧で直近利用したツール（例: `Codex@latest`）が見え、TUI と同等にエージェントごとの色で識別したい。

**独立したテスト**: ブランチ一覧 DTO に `last_tool_usage` を付与するロジックをユニットテストで検証できる。

**受け入れシナリオ**:

1. **前提条件** ブランチAでClaudeを起動した履歴がある、**操作** Sidebar のブランチ一覧を表示、**期待結果** ブランチAに `Claude@...` が表示される
2. **前提条件** 履歴が無いブランチ、**操作** 同上、**期待結果** ツール表示は空である
3. **前提条件** Codex/Claude/Gemini/OpenCode が混在、**操作** 表示色を確認、**期待結果** 定義色（Claude: yellow / Codex: cyan / Gemini: magenta / OpenCode: green）で表示される

---

### ユーザーストーリー 4 - OpenCode をGUIから起動できる (優先度: P1)

開発者として、TUI と同様に OpenCode を選択して起動できるようにしたい。

**独立したテスト**: agent 定義の検出/起動コマンド組み立てをユニットテストで検証できる。

**受け入れシナリオ**:

1. **前提条件** 起動ウィザードを開く、**操作** OpenCode を選択して Launch、**期待結果** OpenCode が起動する（未インストールでも bunx/npx フォールバックが動作する）
2. **前提条件** OpenCode のモデル未指定、**操作** Launch、**期待結果** 起動がブロックされない（空で止まらない）

---

### ユーザーストーリー 5 - Summary に AI セッション要約を表示できる (優先度: P0)

開発者として、TUI と同様に Summary（Session Summary）で **直近のエージェントセッションのAI要約** を確認し、状況把握と再開ができるようにしたい。

**独立したテスト**: AI 要約の取得コマンドは、AI 未設定/無効/セッション無しの各ケースで安定したレスポンスを返し、UI がクラッシュしないことをユニットテストで検証できる。

**受け入れシナリオ**:

1. **前提条件** Profiles に AI 設定（endpoint/model）が設定済みかつ Session Summary=Enabled、対象ブランチに直近の sessionId がある、**操作** Summary を開く、**期待結果** `Generating...` が表示された後、AI 要約（Markdown）が表示される
2. **前提条件** AI が未設定（endpoint/model が空）または無効、**操作** Summary を開く、**期待結果** 「AI設定が必要」である旨のヒントが表示される（UI はクラッシュしない）
3. **前提条件** AI は設定済みだが Session Summary=Disabled、**操作** Summary を開く、**期待結果** 「Session summary disabled」が表示される
4. **前提条件** 対象ブランチにセッション履歴が無い、**操作** Summary を開く、**期待結果** 「No session」が表示される
5. **前提条件** Session Summary タブを閉じている、**操作** Worktree（ブランチ）を選択する、**期待結果** Session Summary タブが再度追加/表示され、選択中ブランチの内容が表示される（既存のエージェントタブは閉じられない）

## エッジケース

- 設定/履歴ファイルの読み込みは **ディスクへの副作用無し**（ユーザーが Save/Launch を実行するまで書き込まない）。
- 旧形式（例: legacy JSON）の存在は読み取りで許容するが、GUI 起動だけで自動変換/自動削除しない。
- 起動直後の異常終了は「起動成功」として扱わず、UI にエラーとして提示する。
- エージェント未インストール時、bunx/npx が無い場合は明確なエラーを提示する。

## 要件 *(必須)*

### 機能要件

#### 起動オプション（Mode/Skip/Reasoning/Version）

- **FR-001**: 起動ウィザードは Session Mode（`normal`/`continue`/`resume`）を選択できなければ**ならない**
- **FR-002**: 起動ウィザードは Skip Permissions（ON/OFF）を選択できなければ**ならない**
- **FR-003**: 起動ウィザードは Codex の Reasoning を選択できなければ**ならない**
- **FR-004**: 起動ウィザードは Agent Version を **選択** できなければ**ならない**。選択肢は `installed`（利用可能な場合）, `latest`, および取得できたバージョン候補である。デフォルトは `installed`（利用可能な場合）で、無い場合は `latest` とする。
- **FR-005**: Agent Version は **選択式のみ** で、自由入力を受け付けては**ならない**。`installed` 以外が選ばれた場合、bunx/npx による起動（`@...@{version}`）が行われなければ**ならない**。
- **FR-006**: 起動ウィザードは Extra Args（任意）と Env Overrides（任意）を指定できなければ**ならない**

#### Quick Start / 履歴

- **FR-010**: Summary は選択中ブランチの Quick Start（ツール別の最新設定）を表示しなければ**ならない**
- **FR-011**: Quick Start は Continue/New の2操作を提供し、選択に応じて起動リクエストへ反映しなければ**ならない**
- **FR-012**: gwt はエージェント起動時にセッション履歴（ブランチ/ツール/設定）を追記しなければ**ならない**
- **FR-013**: gwt はエージェント終了後にセッションID検出を試み、成功時は履歴へ保存しなければ**ならない**（失敗時は警告ログのみ）
- **FR-014**: Summary は選択中ブランチの **直近セッション**（最新の ToolSessionEntry）について AI 要約（Markdown）を表示しなければ**ならない**
- **FR-015**: AI 要約の生成は、Profiles の AI 設定が有効かつ Session Summary=Enabled の場合のみ実行しなければ**ならない**
- **FR-016**: AI 要約の取得/生成は読み取り専用であり、GUI 表示のために設定/履歴ファイルへ自動書き込みを行っては**ならない**
- **FR-017**: Session Summary はメインエリアの **タブ** として表示され、ユーザーはタブを閉じることができなければ**ならない**
- **FR-018**: Worktree（ブランチ）選択時、Session Summary タブが閉じられている場合は自動で再追加し、選択中ブランチの内容を表示しなければ**ならない**

#### タブ表示（Worktree優先）

- **FR-019**: エージェント起動で追加されるタブのラベルは、エージェント名ではなく **Worktree名（ブランチ名）** を表示しなければ**ならない**

#### ブランチ一覧のツール表示

- **FR-020**: BranchInfo は `last_tool_usage`（例: `Codex@latest`）を持てなければ**ならない**
- **FR-021**: Sidebar のブランチ一覧は `last_tool_usage` を表示しなければ**ならない**（存在する場合）
- **FR-022**: ツール表示の色はツール種別で一貫しなければ**ならない**

#### OpenCode

- **FR-030**: ビルトインエージェントとして OpenCode を検出・選択・起動できなければ**ならない**

### インターフェース（フロント/バック間）

#### Tauri Commands

- `detect_agents() -> DetectedAgentInfo[]`（OpenCode を含む）
- `list_agent_versions(agentId: string) -> AgentVersionsInfo`（bunx/npx 用）
- `get_branch_quick_start(projectPath: string, branch: string) -> ToolSessionEntry[]`（Quick Start 表示用）
- `get_branch_session_summary(projectPath: string, branch: string) -> SessionSummaryResult`（Summary 表示用）
- `launch_agent(request: LaunchAgentRequest) -> paneId: string`

#### DTO 変更

- `LaunchAgentRequest` は以下を含む:
  - `mode`: `"normal" | "continue" | "resume"`
  - `skipPermissions`: `boolean`
  - `reasoningLevel?`: `string`
  - `collaborationModes?`: `boolean`
  - `agentVersion?`: `"installed" | string`（`latest`/`1.2.3` 等）
  - `extraArgs?`: `string[]`
  - `envOverrides?`: `Record<string, string>`
  - `resumeSessionId?`: `string`

## 成功基準 *(必須)*

- ブランチ選択後、Quick Start から 2クリック以内で Continue/New が開始できる。
- Codex の skip/reasoning/collaboration_modes が期待通り起動引数へ反映される。
- OpenCode が GUI から起動できる。
- 履歴/設定の読み込み時に、勝手に初期化・上書きが発生しない。

## 制約と仮定

- GUI のユーザー向け表示は英語のみ。
- 実装は `feature/multi-terminal` ブランチ内で完結し、新規ブランチ作成は行わない。

### 制約

- GUI のユーザー向け表示は英語のみ（UI文言に日本語を出さない）。
- 設定/履歴ファイルは読み取り時に副作用を持たない（Save/Launch まで書き込まない）。
- 実装は `feature/multi-terminal` ブランチ内で完結し、新規ブランチ作成は行わない。

### 仮定

- セッション履歴（`~/.gwt/sessions/*.toml`）が存在する場合、Quick Start はそれを唯一の参照元として扱う。
- Claude/Codex/Gemini/OpenCode の CLI オプションは既存の Porting 仕様に従う（互換性が崩れた場合は best-effort でフォールバックする）。

## 範囲外 *(必須)*

<!--
  対応必要: この機能の範囲外であることを明確にしてください。
-->

次の項目は、この機能の範囲外です：

- TUI の全画面/キー操作 UI を GUI にそのまま再現すること（GUI は同等の機能提供を目的とし、UI表現は異なり得る）。
- Web UI の新規実装（本仕様は Tauri GUI の範囲に限定する）。

## セキュリティとプライバシーの考慮事項 *(該当する場合)*

<!--
  対応必要: セキュリティまたはプライバシーの影響がある場合は記入してください。
-->

- `skipPermissions` は危険な起動オプションのため、ユーザーが明示的に選択した場合のみ付与する（デフォルトOFF）。
- セッションID/履歴はホームディレクトリ配下に保存し、外部送信しない。

## 依存関係 *(該当する場合)*

<!--
  対応必要: この機能が依存する他のシステム、サービス、または機能を記載してください。
-->

- `~/.gwt/sessions/` の TypeScript 互換セッション履歴（`ToolSessionEntry`）。
- 各CLIのセッション保存ディレクトリ（Codex: `~/.codex/sessions`、Claude: `~/.claude/projects/`、Gemini/OpenCode: 各ツールの保存形式）。

## 参考資料 *(該当する場合)*

<!--
  対応必要: 関連するドキュメント、設計、または参考資料へのリンクを追加してください。
-->

- `SPEC-3b0ed29b`（Mode/Skip/Reasoning/Quick Start）
- `SPEC-f47db390`（Session ID 永続化/Continue/Resume）

> ⚠️ Markdown整形ヒント：参考資料のURLは必ず `[タイトル](https://example.com)` 形式で記載し、裸URLは使用しないでください。
