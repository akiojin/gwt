### 背景
MergeDialog、ReviewDialog、PRステータス表示、チェックスイート監視を包含するPR管理機能。操作フローはAssistantがAgentに指示する形式。Studio時代の #1544（GitHub連携）のPR関連機能を分離して再定義する。

Issue #1607 で、PR の `Blocked` 表示が「必須 CI 実行中」と「実際のブロック状態」を区別できず分かりにくいことが指摘された。既存の `mergeUiState` 契約は維持したまま、`checking` と `blocked` の意味を明確に分離する。

### 境界
- PR create/update/status/review lifecycle は本仕様の責務
- local branch/worktree inventory や local Git backend semantics は `#1644` が正本
- GitHub discovery / version history 全般は `#1643` と連携する

### ユーザーシナリオとテスト

**P0 / S1: PRステータス表示の意味分離**
- Given: PR に必須 checks が設定されている
- When: 必須 checks が `queued` または `in_progress` の状態で PR 管理画面を開く
- Then: 主状態は `Checking merge status...` と表示され、`Blocked` にはならない

**P0 / S2: 実ブロック状態の表示**
- Given: PR で必須 checks が失敗している、または review が `CHANGES_REQUESTED` である
- When: PR 管理画面を開く
- Then: 主状態は `Blocked` と表示される

**P1 / S3: 既存状態の維持**
- Given: PR が `MERGED` / `CLOSED` / `CONFLICTING` / `MERGEABLE` のいずれかである
- When: PR 管理画面または関連バッジを表示する
- Then: 既存の意味と表示は維持される

**P1 / S4: Retry 中の表示**
- Given: mergeability 再取得の retry 中である
- When: PR ステータスを表示する
- Then: 主状態は `Checking merge status...` を維持し、merge 実行はできない

### 機能要件

**FR-001: PR status 意味分離**
- `mergeUiState` の値集合は維持する
- `checking` は mergeability 未確定、retry 中、必須 checks 実行中を表す
- `blocked` は実際のマージ阻害状態のみを表す

**FR-002: BACKEND 合成ロジック**
- `mergeStateStatus=BLOCKED` 単体では `blocked` を確定しない
- 必須 checks が未完了で、失敗や `CHANGES_REQUESTED` が無い場合は `checking` を返す
- 必須 checks 失敗、`CHANGES_REQUESTED`、その他の明示的阻害は `blocked` を返す

**FR-003: UI 反映範囲**
- `PrStatusSection`、Sidebar/Worktree 系など `mergeUiState` を使う UI は同じ意味に揃える
- 文言が見えない箇所も badge class / 挙動が同じ判定に従う

**FR-004: 既存表示との互換**
- `merged` / `closed` / `conflicting` / `mergeable` の表示と操作条件は変更しない
- `nonRequiredChecksWarning` の挙動は変更しない

### 非機能要件

**NFR-001: 外科的変更**
- 新しい公開 enum 値を追加しない
- 既存 payload shape を壊さない

**NFR-002: 検証性**
- Rust unit tests と Svelte component tests で状態分離を再現できること

### 成功基準

**SC-001** 必須 checks 実行中の PR が `Blocked` ではなく `Checking merge status...` と表示される

**SC-002** 必須 checks 失敗または `CHANGES_REQUESTED` の PR は引き続き `Blocked` と表示される

**SC-003** `MERGED` / `CLOSED` / `CONFLICTING` / `MERGEABLE` の既存挙動に回帰がない

---
