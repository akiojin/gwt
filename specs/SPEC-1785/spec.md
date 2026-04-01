# SPECs画面からのAgent起動（Worktree自動作成付き）

Parent: SPEC-1776

## Background

gwt-tui の SPECs 画面はローカル SPEC ディレクトリの一覧表示と詳細閲覧のみを提供している。SPEC を確認した後にAgent を起動するには、Management Layer を Branches タブに切り替え、対応ブランチを探してWizardを起動する必要がある。この操作は冗長であり、SPEC→実装開始の導線を直結させることで開発効率を向上させる。

### 現状の問題

1. SPECs 画面には閲覧以外のアクションが存在しない
2. SPEC に紐づくブランチの関連付けが metadata.json に保存されない
3. SPEC から Agent 起動するには Branches タブへの手動切り替えが必要

## User Stories

### US-1: SPECs一覧から直接Agent起動 (P0)

開発者として、SPECs一覧で選択中のSPECからShift+Enterで直接Agent起動したい。ブランチ検索・Worktree作成を自動化し、最小限の確認でAgent作業を開始したい。

**受け入れシナリオ:**

- AS-1-1: SPECs一覧でShift+Enter → 既存ブランチ検索 → Wizard起動
- AS-1-2: Wizardは最小ステップ（AgentSelect → ModelSelect → SkipPermissions）
- AS-1-3: Wizard完了 → Worktree自動作成 + Agent起動 → Agentペインに切替
- AS-1-4: 既存ブランチが複数ある場合、選択ダイアログ表示

### US-2: SPECs詳細画面からAgent起動 (P0)

開発者として、SPEC詳細を確認した後、同じ画面からShift+Enterで直接Agent起動したい。

**受け入れシナリオ:**

- AS-2-1: 詳細画面でShift+Enter → US-1と同じフロー
- AS-2-2: 詳細画面ヘッダーに `[Shift+Enter] Launch  [Esc] Back` ヒント表示
- AS-2-3: 一覧画面ヘッダーにも `[Shift+Enter] Launch` ヒント表示

### US-3: Phase警告での事故防止 (P1)

開発者として、draft/blocked phaseのSPECで誤ってAgent起動しないよう、確認ダイアログで警告してほしい。ただし調査目的での起動は許可したい。

**受け入れシナリオ:**

- AS-3-1: draft/blocked phaseでShift+Enter → 確認ダイアログ表示
- AS-3-2: 確認メッセージ: "SPEC-{N} is in '{phase}' phase. Launch agent anyway? [Y/n]"
- AS-3-3: Y/Enter → 起動続行、N/Esc → キャンセル
- AS-3-4: ready-for-dev/in-progress/planned/done phaseでは確認なしで起動

### US-4: ブランチ自動解決とmetadata記録 (P0)

開発者として、SPECに紐づくブランチをgwtが自動管理してほしい。初回はfeature/SPEC-{N}で新規作成し、2回目以降は前回のブランチを自動検出して再利用したい。

**受け入れシナリオ:**

- AS-4-1: metadata.jsonの`branches`配列から既存ブランチを優先検索
- AS-4-2: 既存ブランチが1件 → 自動選択、複数 → 選択ダイアログ、0件 → feature/SPEC-{N}新規作成
- AS-4-3: Agent起動成功後、metadata.jsonの`branches`配列にブランチ名を自動追記（重複回避）
- AS-4-4: git branchからもSPEC IDを含むブランチをフォールバック検索

### US-5: QuickStart連携 (P1)

開発者として、SPECに紐づくブランチにQuickStart履歴がある場合、前回設定でのワンクリック起動もできるようにしたい。

**受け入れシナリオ:**

- AS-5-1: ブランチ解決後、そのブランチのQuickStart履歴を検索
- AS-5-2: 履歴がある場合、WizardはQuickStartステップから開始
- AS-5-3: 履歴がない場合、AgentSelectステップから開始

## Functional Requirements

### キーバインド

- FR-001: SPECs一覧モードでShift+Enter → SpecsMessage::LaunchAgent を発行
- FR-002: SPECs詳細モードでShift+Enter → SpecsMessage::LaunchAgent を発行
- FR-003: 一覧・詳細の両ヘッダーにキーバインドヒントを表示

### Wizard拡張

- FR-010: WizardState に `open_for_spec(spec_id, branch_name, history)` メソッドを追加
- FR-011: `from_spec: bool` と `spec_id: Option<String>` フィールドをWizardStateに追加
- FR-012: `from_spec == true` の場合、BranchAction / BranchTypeSelect / IssueSelect / AIBranchSuggest / BranchNameInput をスキップ
- FR-013: `next_step()` / `prev_step()` の分岐に `from_spec` 条件を追加
- FR-014: QuickStart履歴があれば QuickStart → AgentSelect → ModelSelect → SkipPermissions
- FR-015: 履歴なければ AgentSelect → ModelSelect → SkipPermissions

### ブランチ解決

- FR-020: metadata.jsonの`branches`配列から既存ブランチを優先検索
- FR-021: フォールバック: git branchからSPEC IDを含むブランチを検索
- FR-022: 複数候補 → 選択ダイアログ（j/k選択、Enter確定、Esc取消 + 末尾に新規作成オプション）
- FR-023: 1件 → 自動選択
- FR-024: 0件 → `feature/SPEC-{N}` で新規作成

### Agent起動とWorktree

- FR-030: AgentLaunchBuilder に `auto_worktree = true` を設定
- FR-031: Agent起動成功後、metadata.jsonの`branches`配列にブランチ名を追記（重複回避）
- FR-032: 起動後はメインレイヤー（Agentペイン）に自動切替

### Phase警告

- FR-040: draft/blocked phaseでShift+Enter時に確認ダイアログ表示
- FR-041: Y/Enter で起動続行、N/Esc でキャンセル
- FR-042: ready-for-dev/in-progress/planned/done では確認なしで直接Wizard起動

### metadata.json拡張

- FR-050: SpecItem に `branches: Vec<String>` フィールド追加
- FR-051: load_specs() で metadata.json の `branches` を読み込み
- FR-052: Agent起動成功後に metadata.json へ `branches` を書き戻し

## Non-Functional Requirements

- NFR-001: ブランチ検索（metadata + git）は200ms以内
- NFR-002: metadata.json書き込みはAgent起動後のバックグラウンドで実行
- NFR-003: metadata.json破損時はブランチ検索をスキップし新規作成にフォールバック

## Success Criteria

- SC-001: SPECs一覧でShift+Enter → 最小Wizard → Agent起動 → Agentペイン切替
- SC-002: SPECs詳細でShift+Enter → 同上
- SC-003: draft phaseで確認ダイアログ → Y → 起動成功
- SC-004: 既存ブランチ自動検出 → Worktree再利用
- SC-005: 複数ブランチ候補 → 選択ダイアログ → 選択後起動
- SC-006: metadata.jsonにbranches配列が正しく書き込まれる
- SC-007: QuickStart履歴があるブランチ → QuickStartステップ表示
