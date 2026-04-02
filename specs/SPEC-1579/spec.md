> **🔄 TUI MIGRATION (SPEC-1776)**: This SPEC requires adaptation for the gwt-tui migration. GUI-specific references (gwt-tauri, Svelte, xterm.js) should be read as gwt-tui equivalents. See SPEC-1776 for the migration plan.
> **Canonical Boundary**: `SPEC-1579` は gwt-spec workflow / storage / completion gate の正本である。ワークスペース初期化と SPEC 起点導線は `SPEC-1787` が担当する。

<!-- GWT_SPEC_ARTIFACT:doc:spec.md -->
doc:spec.md

# ワークフロー・ストレージ/API・完了ゲート統合仕様

## Background

- gwt's embedded skills define how agents register work, create specs, clarify scope, generate plans/tasks, analyze readiness, implement changes, and keep PR work moving.
- The old workflow stopped too often at handoff boundaries, especially between registration, clarify/plan/tasks/analyze, and implementation.
- PR maintenance also stopped too early on base-branch drift or high-confidence CI/review fixes even though gwt uses a remote-first, merge-based flow.
- This redesign makes workflow ownership explicit: `gwt-spec-ops` owns artifact stabilization, `gwt-spec-implement` owns execution, and PR skills auto-handle routine merge/fix loops.
- GitHub-backed skills currently lean too heavily on `gh pr list` / GraphQL paths for PR metadata and review inspection, which creates avoidable failures when REST token access still works or when GraphQL quotas are exhausted.
- The current runtime can contain helper skills that exist on disk but are not present in the user-visible skill catalog. User-facing workflow guidance must not point directly at hidden skills.
- The older storage design treated the GitHub Issue body as the canonical spec bundle with `Spec/Plan/Tasks/TDD/...` sections embedded directly in the body. The embedded skill workflow is now restructured around local file-based storage where `specs/SPEC-{N}/` directories hold the real content (`spec.md`, `plan.md`, `tasks.md`, and supporting artifacts) and the local SPEC directory is the canonical source.
- The current workflow only defined a pre-implementation `CLEAR` gate. #1654 exposed a missing completion gate: `tasks.md` and progress comments were marked complete while the implementation still diverged from `doc:spec.md`, `checklist:acceptance.md`, and `checklist:tdd.md`.
- This spec is the single canonical reference for the gwt-spec system: embedded workflow, storage/API, artifact CRUD, completion gate, registration contract, and GitHub transport policy. Issue-tab detail rendering lives in #1354.

## User Stories

### User Story 1 - Register new work into the right workflow without duplicate churn (Priority: P0)

As a developer, I want embedded skills to route new work through issue/spec registration without creating duplicates or stopping on clear existing owners.

**Acceptance Scenarios**

1. Given a new request, when `gwt-issue-register` finds a clear existing Issue or `gwt-spec`, then it continues with that owner instead of stopping.
2. Given a new request that needs a SPEC, when `gwt-issue-register` creates one, then it continues into `gwt-spec-ops` instead of ending at registration.
3. Given an existing Issue, when `gwt-issue-resolve` promotes it to the SPEC path, then it continues into the same artifact-first workflow.

### User Story 2 - Drive artifact-first spec work without normal handoff stops (Priority: P0)

As a developer, I want `spec.md`, `plan.md`, `tasks.md`, and analysis artifacts to be driven in order without stopping at every workflow stage.

**Acceptance Scenarios**

1. Given a new SPEC container, when registration completes, then `doc:spec.md` exists before planning starts and `gwt-spec-ops` remains the owner.
2. Given unresolved clarification that can be inferred safely, when clarify runs, then the artifact is repaired without asking the user.
3. Given analysis gaps that are mechanical rather than product decisions, when `gwt-spec-analyze` runs, then it reports `AUTO-FIXABLE` and returns control to `gwt-spec-ops`.
4. Given a true product or scope ambiguity, when the workflow cannot infer the answer safely, then it stops and asks the user.
5. Given a spec is planning-ready, when the next step is presented to the user or another agent, then the external routing target is `gwt-spec-ops` unless the intermediate skill is visible in the user-facing skill catalog.

### User Story 3 - Separate workflow ownership from implementation ownership (Priority: P0)

As a maintainer, I want execution ownership to be explicit so that spec work does not stall before code changes begin.

**Acceptance Scenarios**

1. Given a `CLEAR` analysis result, when execution begins, then `gwt-spec-implement` owns test-first implementation, progress updates, and PR flow.
2. Given execution reveals a spec bug, when the change requires artifact repair, then control returns to `gwt-spec-ops` instead of silently drifting from the spec.
3. Given workflow questions, when an implementer reads the specs, then #1579 is the canonical registration/workflow spec.

### User Story 4 - Keep PR maintenance autonomous for routine blockers (Priority: P1)

As a developer, I want PR skills to handle routine branch sync and high-confidence CI/review fixes automatically.

**Acceptance Scenarios**

1. Given the base branch has advanced, when `gwt-pr` or `gwt-pr-fix` runs, then it merges `origin/<base>` into the current branch and pushes without extra approval when the merge is clear.
2. Given CI or review blockers are high-confidence and local, when `gwt-pr-fix` runs, then it applies the fixes and continues.
3. Given a merge conflict or reviewer request is behaviorally ambiguous, when the correct resolution is not clear, then the skill asks the user.

### User Story 5 - Support migration as part of the workflow redesign (Priority: P1)

As a maintainer, I want migration to be part of the redesign rather than an afterthought.

**Acceptance Scenarios**

1. Given legacy Issue-based specs, when migration runs, then they can be converted into local SPEC directories (`specs/SPEC-{N}/`).
2. Given old monolithic `gwt-spec` Issues, when migration/repair runs, then their artifacts can be extracted into local SPEC files.
3. Given the user explicitly asks to migrate or convert, when the dry-run completes cleanly, then the migration skill proceeds without an extra confirmation loop.

### User Story 6 - Prefer REST for GitHub-backed embedded skills when practical (Priority: P1)

As a maintainer, I want GitHub-backed embedded skills to prefer REST for metadata and mutation paths where practical so that GraphQL-heavy flows stop being the default failure mode.

**Acceptance Scenarios**

1. Given `gwt-pr` needs PR discovery, create, update, or view metadata, when a GitHub token is available, then it uses REST-first transport rather than requiring `gh pr list` / GraphQL as the primary path.
2. Given `gwt-pr-check` needs PR existence, merged status, or post-merge metadata, when a GitHub token is available, then it uses REST-first transport for those reads.
3. Given `gwt-pr-fix` needs CI status, reviews, review comments, or PR issue comments, when those data are available through REST, then it uses REST-first transport for those reads.
4. Given unresolved review thread discovery or thread reply/resolve is required, when REST does not provide a practical equivalent, then GraphQL remains allowed for those thread-specific operations only.
5. Given direct token-based REST auth succeeds while `gh auth status` or GraphQL paths fail, when a REST-first skill runs, then it may proceed using REST auth without treating `gh auth status` as a hard blocker.

### User Story 7 - Persist spec artifacts without relying on a monolithic body (Priority: P0)

As a developer, I want the spec system to store `spec.md`, `plan.md`, `tasks.md`, and supporting docs as local files in `specs/SPEC-{N}/` rather than a giant Issue body.

**Acceptance Scenarios**

1. Given a new SPEC, when it is created or updated, then the canonical content lives in local files under `specs/SPEC-{N}/`.
2. Given a SPEC directory with local artifact files, when backend detail APIs are called, then they reconstruct `SpecIssueSections` from those local files.

### User Story 8 - Keep legacy issues readable during migration (Priority: P0)

As a developer, I want legacy Issue-based specs to remain readable while the system migrates to local file-based storage.

**Acceptance Scenarios**

1. Given a legacy `gwt-spec` Issue with body sections only, when detail APIs are called, then the same sections are returned.
2. Given a mixed state with both Issue-based artifacts and local SPEC files, when detail APIs are called, then local files take precedence and Issue body sections act as fallback.

### User Story 9 - Manage document artifacts through the same CRUD layer (Priority: P0)

As a developer or agent, I want `doc:*`, `contract:*`, and `checklist:*` artifacts to use the same local file CRUD model (`spec_artifact.py`).

**Acceptance Scenarios**

1. Given an artifact key `doc:plan.md`, when it is upserted, then the system stores it as a local file in `specs/SPEC-{N}/` with stable retrieval metadata.
2. Given an artifact key `contract:openapi.yaml` or `checklist:tdd.md`, when it is listed, then it is returned through the same local file API family.

### User Story 10 - Support migration tooling for old formats (Priority: P1)

As a maintainer, I want migration tooling to handle Issue-based specs and convert them to local SPEC directories.

**Acceptance Scenarios**

1. Given an existing `gwt-spec` Issue, when migration runs, then it can extract artifacts into local `specs/SPEC-{N}/` directories.
2. Given an existing body-canonical `gwt-spec` Issue, when migration or repair runs, then it can split the body into local `spec.md`, `plan.md`, `tasks.md`, and supporting files.

### User Story 11 - Implementation completion must be evidence-backed (Priority: P0)

As a maintainer, I want a SPEC to be marked complete only after tasks, acceptance, TDD, progress, and verification all agree.

**Acceptance Scenarios**

1. Given `doc:tasks.md` is all `[x]`, when completion is declared, then `checklist:acceptance.md` and `checklist:tdd.md` must also reflect the same completion state.
2. Given implementation still violates `doc:spec.md`, when someone tries to mark the SPEC done, then the workflow must route back to `gwt-spec-ops` instead of allowing completion.
3. Given a progress comment says `implementation is complete`, when the completion gate has not passed, then the workflow must treat that comment as invalid and require correction.

### User Story 12 - The workflow must distinguish preflight from exit gates (Priority: P0)

As an implementer, I want `gwt-spec-analyze` to stay a preflight check while a separate mandatory completion audit governs exit.

**Acceptance Scenarios**

1. Given a `CLEAR` analysis result, when implementation starts, then `gwt-spec-implement` may execute but cannot use that same `CLEAR` as the final completion proof.
2. Given implementation has finished, when the exit audit runs, then it must compare code and verification evidence against `doc:spec.md`, `doc:tasks.md`, and `checklist:*` artifacts.
3. Given the exit audit finds divergence, when completion is blocked, then the next step is `gwt-spec-ops` for artifact repair or task rollback.

### User Story 13 - Acceptance and TDD artifacts must stay machine-usable (Priority: P1)

As a workflow maintainer, I want checklist artifacts to be structured and current so they can participate in the completion gate.

**Acceptance Scenarios**

1. Given `checklist:tdd.md` exists, when the workflow reads it, then it must be in a clear, current, non-corrupted format.
2. Given acceptance scenarios exist in `doc:spec.md`, when `doc:tasks.md` is generated or updated, then verification tasks must map back to them.
3. Given a checklist artifact is stale or malformed, when the workflow reaches completion, then the SPEC cannot be marked done.

## Edge Cases

- New SPEC registration succeeds but no `doc:spec.md` is created.
- A migrated `gwt-spec` Issue has an artifact index body but stale legacy content in old comments.
- Clarification, planning, or analysis artifacts are partially present.
- Implementation reveals a spec bug after `CLEAR`; the workflow must repair artifacts before continuing.
- Base-branch merge conflicts touch logic that cannot be resolved without choosing behavior.
- REST token auth succeeds but GraphQL quota is exhausted.
- `GH_TOKEN` is invalid but `GITHUB_TOKEN` is valid, or vice versa.
- REST requests hit primary or secondary rate limits and require bounded retry/backoff.
- GraphQL is unavailable but review-thread-specific operations are requested.
- A helper skill exists on disk but is absent from the user-visible skill catalog.
- A spec issue has no `doc:*` comments and only body sections.
- A spec issue has partial `doc:*` coverage (for example `doc:spec.md` exists but `doc:plan.md` does not).
- Artifact comments use marker format or legacy prefix format.
- Consumers request only contract/checklist artifacts while `doc:*` artifacts also exist.
- `doc:tasks.md` is all complete but one or more acceptance items remain unchecked.
- Progress comments contain outdated `Done` statements after requirements changed.
- A spec issue inherited corrupted or partial `checklist:tdd.md` content from earlier migrations.
- An implementation is partially correct and needs task rollback rather than a brand-new spec.

## Functional Requirements

- **FR-001**: `gwt-issue-register` and `gwt-issue-resolve` must route spec work through a single artifact-first workflow and continue automatically when the correct owner is clear.
- **FR-002**: `gwt-spec-register` must create a SPEC container and seed `specs/SPEC-{N}/spec.md` before any planning artifacts are generated.
- **FR-003**: `gwt-spec-ops` must own clarify/plan/tasks/analyze sequencing and keep driving the workflow until a true decision blocker appears.
- **FR-004**: `gwt-spec-analyze` must classify readiness as `CLEAR`, `AUTO-FIXABLE`, or `NEEDS-DECISION`.
- **FR-005**: `gwt-spec-implement` must own execution after `CLEAR`, including test-first task execution, progress updates, and PR handoff.
- **FR-006**: `gwt-pr` and `gwt-pr-fix` must auto-merge `origin/<base>` using merge, not rebase, when the merge is behaviorally clear.
- **FR-007**: PR and SPEC skills must ask the user only for ambiguous conflicts, unresolved product decisions, missing auth, or risky destructive migration scope.
- **FR-008**: This spec is the single canonical reference for gwt-spec workflow, storage/API, and completion gate. #1354 remains the Issue detail/viewer canonical.
- **FR-009**: Migration is part of the redesign scope and must cover both local legacy specs and old body-canonical GitHub spec issues.
- **FR-010**: The repo-level constitution (`.gwt/memory/constitution.md`) must remain part of the planning gate.
- **FR-011**: GitHub-backed embedded skills must prefer REST for PR/Issue metadata, PR create/update, CI/check reads, reviews, review comments, and comment operations where GitHub REST provides practical coverage.
- **FR-012**: GraphQL may be used only for operations where GitHub REST does not provide practical coverage, specifically unresolved PR review thread discovery and thread reply/resolve mutations unless a future REST equivalent is adopted.
- **FR-013**: REST-first GitHub skills must support direct token-based auth probes using `GH_TOKEN` first and `GITHUB_TOKEN` second, and must not depend solely on `gh auth status` as their readiness gate.
- **FR-014**: REST-first GitHub skills must treat PR discovery, PR create/update, commit status, check-runs, reviews, review comments, and PR issue comments as REST-capable paths in both documentation and implementation planning.
- **FR-015**: When REST or GraphQL rate limits are hit, embedded GitHub skills must document bounded retry/backoff behavior and must not claim that REST is unlimited.
- **FR-016**: User-facing workflow guidance must reference only visible skills directly; if an internal helper skill such as `gwt-spec-plan` is not visible in the current user-facing catalog, the documented next step must route through its visible owner skill instead.
- **FR-017**: The local SPEC directory structure must support `doc`, `contract`, and `checklist` artifact families as files under `specs/SPEC-{N}/`.
- **FR-018**: `get_spec_issue_detail()` must reconstruct `SpecIssueSections` from local SPEC files first and Issue body sections second.
- **FR-019**: `SpecIssueDetail.sections` must remain the stable frontend-facing aggregate shape.
- **FR-020**: `spec_artifact.py` and related local CRUD operations must support `doc:*` artifacts alongside existing contract/checklist artifacts.
- **FR-021**: Legacy Issue-based `gwt-spec` specs must remain readable until migration to local SPEC directories is complete.
- **FR-022**: The canonical format for new SPECs must be a local `specs/SPEC-{N}/` directory with individual artifact files, not a monolithic Issue body.
- **FR-023**: Migration tooling must cover `Issue-based spec -> local specs/SPEC-{N}/` and `body-canonical issue -> local SPEC directory` flows.
- **FR-024**: `gwt-spec-analyze` must be documented as a pre-implementation readiness gate only.
- **FR-025**: `gwt-spec-implement` must include a mandatory post-implementation completion gate before tasks or progress can declare completion.
- **FR-026**: The completion gate must reconcile `doc:spec.md`, `doc:tasks.md`, `checklist:acceptance.md`, `checklist:tdd.md`, progress comments, and executed verification.
- **FR-027**: If reconciliation fails, the workflow must route back to `gwt-spec-ops` and must not leave the SPEC in a completed state.
- **FR-028**: TDD and acceptance checklists must remain structured, readable, and current enough to support the completion gate.
- **FR-029**: Completion comments such as `implementation is complete` must be treated as workflow outputs that require evidence, not as source-of-truth on their own.

## Non-Functional Requirements

- **NFR-001**: Workflow guidance must remain aligned across embedded skills, generated registration assets, and canonical specs.
- **NFR-002**: Registration/workflow changes must not leave viewer/search/storage ownership ambiguous.
- **NFR-003**: Autonomy improvements must preserve high-confidence conflict handling rather than mechanically taking one side.
- **NFR-004**: Migration scope must be documented before implementation proceeds further.
- **NFR-005**: Embedded GitHub skill guidance must accurately state that REST and GraphQL both have rate limits, and that REST-first is chosen for transport suitability rather than because REST is unlimited.
- **NFR-006**: Artifact-first migration must not require a breaking frontend payload change.
- **NFR-007**: Existing closed SPEC issues remain readable without manual repair.
- **NFR-008**: Rust/Tauri regression tests must cover artifact-first, legacy, and mixed-mode issues.
- **NFR-009**: Completion-gate rules must stay aligned across skill docs, command docs, and issue artifact conventions.
- **NFR-010**: The workflow must prefer rollback of false completion state over silently broadening the SPEC.
- **NFR-011**: The completion audit must be specific enough that a future implementer does not need to invent exit criteria.

## Success Criteria

- **SC-001**: Embedded skills describe one ordered artifact-first workflow from registration through analysis and implementation.
- **SC-002**: Ownership is explicit: #1579 workflow/storage/API/completion, #1354 viewer, #1643 search.
- **SC-003**: `gwt-spec-implement` exists as the implementation owner and `gwt-spec-ops` no longer stops at normal handoff boundaries.
- **SC-004**: `gwt-pr` and `gwt-pr-fix` auto-handle routine base merges and high-confidence PR fixes while escalating only ambiguous cases.
- **SC-005**: Migration is explicitly in scope for both local specs and body-canonical issues.
- **SC-006**: #1579 clearly defines a REST-first / GraphQL-only-where-needed transport policy for embedded GitHub skills.
- **SC-007**: The spec explicitly names the REST-first responsibility split across `gwt-pr`, `gwt-pr-check`, and `gwt-pr-fix` so implementation can proceed without transport ambiguity.
- **SC-008**: Planning-ready next-step guidance does not point users at hidden helper skills.
- **SC-009**: The backend can read index-only spec issues backed by `doc:*` comments.
- **SC-010**: The backend can still read legacy body-canonical spec issues.
- **SC-011**: Artifact CRUD supports `doc`, `contract`, and `checklist` consistently.
- **SC-012**: Migration scope explicitly covers old local specs and old body-canonical issues.
- **SC-013**: The workflow distinguishes `pre-implementation CLEAR` from `post-implementation completion`.
- **SC-014**: A SPEC cannot be marked complete while acceptance/TDD/task/progress artifacts disagree.
- **SC-015**: Corrupted or stale checklist artifacts are identified as blockers rather than ignored.
- **SC-016**: #1654 can be remediated under the new completion rules without inventing a second shell spec.

## Individual Skill Specifications

### gwt-issue-register

- **概要**: 新規作業（バグ報告、機能要求、ドキュメントタスク等）の登録エントリポイント。既存 Issue / `gwt-spec` を検索し、重複がなければ plain Issue または SPEC を作成する。
- **トリガー条件**: ユーザーが新規作業の登録・起票を依頼し、既存の GitHub Issue 番号や URL がまだ存在しない場合。
- **ワークフロー**:
  1. `gh auth status` で認証確認
  2. リクエストを正規化し、種別（BUG/FEATURE/ENHANCEMENT/DOCUMENTATION/CHORE/QUESTION）を分類
  3. `gwt-issue-search` で最低2件のセマンティッククエリを実行し、既存の重複を検索
  4. 重複が見つかった場合は既存オーナーに切り替え（`gwt-issue-resolve` または `gwt-spec-ops`）
  5. 重複なしの場合、plain Issue vs SPEC の判断基準に従い選択
  6. plain Issue の場合は `gh issue create` で作成、SPEC の場合は `gwt-spec-register` 経由で作成後 `gwt-spec-ops` へ継続
  7. 作成した Issue 番号・URL またはアクティブなオーナーを返却
- **入力/出力**: 入力: ユーザーからの作業依頼テキスト。出力: Registration Decision レポート（Request Type / Duplicate Check / Chosen Path / Action / Candidates）
- **依存スキル**: `gwt-issue-search`（必須プリフライト）、`gwt-issue-resolve`（既存 Issue 転送時）、`gwt-spec-register`（新規 SPEC 作成時）、`gwt-spec-ops`（SPEC ワークフロー継続時）
- **操作**: `gh issue create`、`gh api repos/<owner>/<repo>/issues`（レート制限時フォールバック）
- **停止条件**: gh 認証不可、重複検索が曖昧で既存オーナーを特定できない場合

### gwt-issue-resolve

- **概要**: 既存の GitHub Issue を分析し、直接修正・既存 SPEC 更新・新規 SPEC 作成のいずれかのパスで解決まで進める。
- **トリガー条件**: ユーザーが既存の Issue 番号または URL を指定して進捗を求めた場合。
- **ワークフロー**:
  1. `gh auth status` で認証確認
  2. Issue 番号または URL を解決し、アクセス可能か検証
  3. `inspect_issue.py` で Issue メタデータ・コメント・リンク済み PR・エラーメッセージ・ファイル参照を収集
  4. 既に `gwt-spec` ラベルや `GWT_SPEC_ID` マーカーがあれば `gwt-spec-ops` へ転送
  5. Direct-fix（バグ・小規模修正）vs Spec-needed（機能・大規模変更）の判断
  6. Direct-fix パス: コードベース検索、Issue Analysis Report 生成、高信頼度なら即時修正
  7. Spec-needed パス: `gwt-issue-search` で既存 SPEC 検索、見つからなければ `gwt-spec-register` で新規作成、`gwt-spec-ops` へ継続
  8. Issue Progress Comment を投稿
- **入力/出力**: 入力: `repo`（パス）、`issue`（番号/URL）、`focus`（オプション）。出力: Issue Analysis Report（Issue Type / Execution Path / Extracted Context / Actionable / Informational）
- **依存スキル**: `gwt-issue-search`（SPEC 必要時の事前検索）、`gwt-spec-register`（新規 SPEC 作成時）、`gwt-spec-ops`（SPEC ワークフロー継続時）
- **操作**: `inspect_issue.py` スクリプト、`gh issue comment`
- **停止条件**: gh 認証不可、Low confidence の actionable item で最小限の質問が必要な場合

### gwt-issue-search

- **概要**: ChromaDB ベクトル埋め込みを使用した GitHub `gwt-spec` Issue のセマンティック検索。既存仕様の重複確認やスコープオーナー特定に使用する。
- **トリガー条件**: `gwt-spec-register`、`gwt-spec-ops`、`gwt-issue-register`、`gwt-issue-resolve` の必須プリフライトとして、または既存仕様の検索要求時。
- **ワークフロー**:
  1. `gh auth status` の有効性を確認
  2. `index-issues` アクションで Issue インデックスを更新
  3. リクエストから導出した2-3件のセマンティッククエリで `search-issues` を実行
  4. 既存の canonical SPEC が見つかれば、それを推奨先として返却
  5. 見つからなければ新規 SPEC 作成を許可
- **入力/出力**: 入力: `--query`（検索クエリ）、`--db-path`（インデックスパス）、`--n-results`（結果数）。出力: JSON 形式の検索結果（number, title, url, state, labels, distance）
- **依存スキル**: なし（他スキルのプリフライトとして呼ばれる側）
- **操作**: `chroma_index_runner.py --action index-issues`、`chroma_index_runner.py --action search-issues`
- **停止条件**: Issue インデックスが利用不可の場合はその旨を報告してフォールバック

### gwt-spec-register

- **概要**: 既存の canonical SPEC が存在しない場合に、新規 `gwt-spec` Issue コンテナを作成し、`doc:spec.md` をシードする。作成後は `gwt-spec-ops` へ継続。
- **トリガー条件**: `gwt-issue-register` または `gwt-issue-resolve` が新規 SPEC 作成を判断した場合、またはユーザーが明示的に新規 SPEC 登録を要求した場合。
- **ワークフロー**:
  1. `gh auth status` で認証確認
  2. `gwt-issue-search` で最低2クエリの既存 SPEC 検索を実行
  3. canonical SPEC が存在すれば `gwt-spec-ops` へ転送
  4. 存在しなければ `gwt-spec:` プレフィックス付きタイトルで `gwt-spec` ラベルの Issue を作成
  5. `<!-- GWT_SPEC_ID:#{number} -->` マーカーとアーティファクトインデックスの Issue body を設定
  6. `spec_artifact.py` で初期 `doc:spec.md` コメントをシード（Background / User Stories / Acceptance Scenarios / Edge Cases / FR / NFR / SC 構造）
  7. register-only が明示指定されない限り `gwt-spec-ops` へ継続
- **入力/出力**: 入力: 作業リクエストのコンテキスト。出力: 作成された `gwt-spec` Issue 番号と URL
- **依存スキル**: `gwt-issue-search`（必須プリフライト）、`gwt-spec-ops`（継続先）
- **操作**: `gh issue create --label gwt-spec`、`gh issue edit`、`spec_artifact.py --upsert --artifact "doc:spec.md"`、レート制限時は `gh api` REST フォールバック
- **停止条件**: gh 認証不可、既存 canonical SPEC が明確にスコープをカバーしている場合（作成せず転送）

### gwt-spec-clarify

- **概要**: 既存 `gwt-spec` の `spec.md` から `[NEEDS CLARIFICATION]` マーカーを解決し、ユーザーストーリーを精緻化し、受け入れシナリオを確定させてプランニング可能状態にする。
- **トリガー条件**: `gwt-spec-ops` が `spec.md` に未解決の曖昧点を検出した場合、またはユーザーが直接 clarify を要求した場合。
- **ワークフロー**:
  1. 対象 SPEC の `spec.md` アーティファクトを読み込み
  2. `[NEEDS CLARIFICATION]` マーカー、弱い受け入れシナリオ、曖昧な要件、未記載のエッジケースを特定
  3. ソース Issue・既存コメント・現在の実装から推論可能なギャップを埋める
  4. 残りの高インパクト質問（最大5件）をユーザーに提示し、回答を待つ（エージェントが代わりに回答してはならない）
  5. ユーザー回答で `spec.md` を更新し、Clarification Log セクションに記録
  6. プランニング可能ならば `gwt-spec-ops` / `gwt-spec-plan` へ制御を返す
- **入力/出力**: 入力: `gwt-spec` Issue 番号。出力: Clarification Report（Resolved 数 / Remaining blockers / Next step）
- **依存スキル**: `gwt-spec-ops`（親ワークフロー）、`gwt-issue-search`（対象 SPEC 不明時）
- **操作**: `spec_artifact.py --get --artifact "doc:spec.md"`、`spec_artifact.py --upsert --artifact "doc:spec.md"`
- **停止条件**: ユーザーの製品・スコープ判断が必要な未解決ブロッカーが残っている場合、ユーザー回答待ち

### gwt-spec-ops

- **概要**: GitHub Issue ファーストの SPEC オーケストレーション。`spec.md` / `plan.md` / `tasks.md` / analysis ゲートを安定化させ、実装まで通しで駆動する中央ワークフローオーナー。
- **トリガー条件**: 対象の `gwt-spec` Issue が既に特定されている場合。`gwt-issue-register` / `gwt-issue-resolve` / `gwt-spec-register` からの継続先。
- **ワークフロー**:
  1. `gwt-issue-search` で既存 SPEC 検索（プリフライト）
  2. `spec.md` が未存在なら `gwt-spec-register` でシード
  3. 未解決 clarification → `gwt-spec-clarify` 実行 → ユーザー回答待ち → 回答反映後に継続
  4. プランニングアーティファクト未存在 → `gwt-spec-plan` 実行
  5. タスク未存在 → `gwt-spec-tasks` 実行
  6. 一貫性ゲート未実行 → `gwt-spec-analyze` 実行
  7. `CLEAR` → `gwt-spec-implement` で実装開始
  8. `AUTO-FIXABLE` → アーティファクト修復後に再分析
  9. `NEEDS-DECISION` → ユーザーに判断を求める
  10. 実装完了後の completion-gate reconciliation を要求
- **入力/出力**: 入力: `gwt-spec` Issue 番号。出力: Phase 遷移ステータス、アーティファクトセットの更新
- **依存スキル**: `gwt-issue-search`（プリフライト）、`gwt-spec-register`（シード時）、`gwt-spec-clarify`、`gwt-spec-plan`、`gwt-spec-tasks`、`gwt-spec-analyze`、`gwt-spec-implement`、`gwt-pr`、`gwt-pr-fix`
- **操作**: `gh issue view`、`gh issue edit`、`spec_artifact.py`（list / get / upsert）、`gh issue list --label gwt-spec`
- **停止条件**: gh 認証不可、既存オーナー検索が曖昧、製品・スコープ判断が推論不可、マージコンフリクトが高信頼度で解決不可、`gwt-spec-clarify` の未回答質問待ち

### gwt-spec-plan

- **概要**: clarify 済みの `spec.md` から実装準備可能なプランニングアーティファクト群（`plan.md` / `research.md` / `data-model.md` / `quickstart.md` / `contract:*`）を生成する。
- **トリガー条件**: `gwt-spec-ops` が clarify 完了後にプランニングフェーズへ遷移した場合、またはユーザーが直接プラン生成を要求した場合。
- **ワークフロー**:
  1. `spec.md` と `.gwt/memory/constitution.md` を読み込み
  2. 技術コンテキスト（影響ファイル・モジュール・外部制約）を特定
  3. Constitution Check を実行（違反があればリデザインまたは Complexity Tracking に記録）
  4. サポートアーティファクト生成: `research.md`（未知事項・トレードオフ）、`data-model.md`（エンティティ・ライフサイクル）、`quickstart.md`（最小検証フロー）、`contract:*`（インターフェース契約）
  5. `plan.md` を作成（Summary / Technical Context / Constitution Check / Project Structure / Complexity Tracking / Phased Implementation）
  6. `gwt-spec-ops` へ返却、またはフロー継続中なら `gwt-spec-tasks` へ直接進行
- **入力/出力**: 入力: `doc:spec.md`、`.gwt/memory/constitution.md`。出力: `doc:plan.md` 他サポートアーティファクト群
- **依存スキル**: `gwt-spec-clarify`（spec.md にギャップがある場合の事前処理）、`gwt-spec-tasks`（後続）
- **操作**: `spec_artifact.py --upsert` で各アーティファクトを Issue コメントとして作成/更新
- **停止条件**: `spec.md` が存在しない、またはユーザー判断がプランニングをブロックしている場合

### gwt-spec-tasks

- **概要**: 承認済みの `spec.md` と `plan.md` から実行可能なタスクリスト `tasks.md` を生成する。フェーズ別・ユーザーストーリー別にグループ化し、正確なファイルパス・`[P]` 並列マーカー・テストファースト順序を含む。
- **トリガー条件**: `gwt-spec-ops` がプラン完了後にタスク生成フェーズへ遷移した場合、またはユーザーが直接タスク生成を要求した場合。
- **ワークフロー**:
  1. `spec.md` と `plan.md` からユーザーストーリー・受け入れシナリオ・影響モジュール・契約を抽出
  2. フェーズ順序を設定（Setup → Foundational → US1/US2/... → Polish/Cross-Cutting）
  3. テストファーストタスクを生成（検証タスクを実装タスクの前または並列に配置）
  4. 具体的なファイルパス・モジュールで実装タスクを追加、`[P]` マーカーでスコープ非重複を示す
  5. トレーサビリティ検証（全ユーザーストーリー・受け入れシナリオ・契約変更がカバーされているか）
  6. `tasks.md` を書き込み、`gwt-spec-ops` へ返却または `gwt-spec-analyze` へ直接進行
- **入力/出力**: 入力: `doc:spec.md`、`doc:plan.md`、オプションで `doc:research.md` 等。出力: `doc:tasks.md`
- **依存スキル**: `gwt-spec-plan`（事前にプランが必要）、`gwt-spec-analyze`（後続）
- **操作**: `spec_artifact.py --upsert --artifact "doc:tasks.md"`
- **停止条件**: `plan.md` が存在しない、`spec.md` に clarification ブロッカーが残っている場合

### gwt-spec-analyze

- **概要**: 実装前の最終ゲートとして `spec.md` / `plan.md` / `tasks.md` / `.gwt/memory/constitution.md` のアーティファクトセットを完全性・一貫性の観点で分析する。自動修復可能なギャップと真の判断ブロッカーを区別する。
- **トリガー条件**: `gwt-spec-ops` がタスク生成完了後に分析ゲートへ遷移した場合、またはユーザーが直接分析を要求した場合。
- **ワークフロー**:
  1. 必須アーティファクトセット（`spec.md` / `plan.md` / `tasks.md` / `constitution.md`）を読み込み
  2. Clarification 完全性チェック（`[NEEDS CLARIFICATION]` 残存確認）
  3. Spec 完全性チェック（User Stories / Acceptance Scenarios / Edge Cases / Requirements / Success Criteria の存在確認）
  4. Plan 完全性チェック（Constitution Check / Technical Context / Phased Implementation の具体性確認）
  5. Task トレーサビリティチェック（全ユーザーストーリーにタスクがあるか、全受け入れシナリオに検証カバレッジがあるか）
  6. Constitution アラインメントチェック
  7. 判定結果を `CLEAR` / `AUTO-FIXABLE` / `NEEDS-DECISION` で出力
- **入力/出力**: 入力: アーティファクトセット。出力: Analysis Report（Status / Blocking items / Next step）
- **依存スキル**: `gwt-spec-ops`（親ワークフロー、`AUTO-FIXABLE` 時の修復先）
- **操作**: `spec_artifact.py --list`（アーティファクト一覧取得）
- **停止条件**: `NEEDS-DECISION` の場合はユーザーに判断を求める。`CLEAR` は実装可能を意味するが SPEC 完了は意味しない

### gwt-spec-implement

- **概要**: `CLEAR` 分析結果を持つ `gwt-spec` を `tasks.md` に基づいてエンドツーエンドで実装する。テストファースト実行、進捗アーティファクト更新、PR フローの維持を担当する実装オーナー。
- **トリガー条件**: `gwt-spec-ops` が `CLEAR` 分析結果を受けて実装フェーズへ遷移した場合。
- **ワークフロー**:
  1. 対象 Issue と3コアアーティファクト（`spec.md` / `plan.md` / `tasks.md`）、サポートアーティファクトを読み込み
  2. 依存順序でタスクを実行（Setup → Foundational → ユーザーストーリー別）
  3. テストファーストで作業（最小の失敗テストを追加してから実装）
  4. `tasks.md` の対象ファイルのみを編集（スコープ拡大は禁止）
  5. 最小有効な検証セットで検証、spec ギャップが判明した場合は `gwt-spec-ops` へ返却
  6. `tasks.md` の完了マーカー更新、Issue に `Progress / Done / Next` テンプレートで進捗コメント投稿
  7. PR フロー維持: 未 PR なら `gwt-pr`、CI/review ブロッカーは `gwt-pr-fix`
  8. 全タスク完了後に post-implementation completion gate 実行（`spec.md` / `tasks.md` / `acceptance.md` / `tdd.md` / 進捗コメント / 検証証拠の reconciliation）
- **入力/出力**: 入力: `doc:spec.md`、`doc:plan.md`、`doc:tasks.md`、最新 `CLEAR` 分析結果。出力: Implementation Report（Completed tasks / Updated files / Verification / Next）
- **依存スキル**: `gwt-spec-ops`（spec バグ発見時の返却先）、`gwt-pr`（PR 作成）、`gwt-pr-fix`（PR 修正）
- **操作**: テスト実行、コード編集、`spec_artifact.py` でタスク更新、`git commit` / `git push`
- **停止条件**: 製品・スコープ判断が推論不可、マージコンフリクトが曖昧、必要な認証・ツールが利用不可

### gwt-spec-to-issue-migration

- **概要**: レガシー仕様ソース（ローカル `specs/SPEC-*` ディレクトリ、body-canonical `gwt-spec` Issue）をアーティファクトファースト GitHub Issue 形式に移行する。
- **トリガー条件**: ユーザーがレガシー仕様の移行・変換を要求した場合。
- **ワークフロー**:
  1. ソース仕様ディレクトリの検査（自動検出または `--specs-dir` 指定）
  2. `--dry-run` で移行計画と削除対象を自動レビュー
  3. ユーザーが明示的に移行/変換を依頼した場合、dry-run 後に追加確認なく実行
  4. 移行された Issue の存在確認（`gwt-spec` ラベル）
  5. 成功時にレガシーローカル仕様ファイルの削除を確認
- **入力/出力**: 入力: `--specs-dir`（オプション）、`--convert-existing-issues`（既存 Issue 変換時）。出力: `migration-report.json`（成功時は自動削除）
- **依存スキル**: なし（独立実行）
- **操作**: `migrate-specs-to-issues.mjs --dry-run`、`migrate-specs-to-issues.mjs`、`migrate-specs-to-issues.mjs --convert-existing-issues`、`gh issue list --label gwt-spec`
- **停止条件**: 移行意図が不明な場合、または要求スコープに明らかでない破壊的変更が含まれる場合にユーザーに確認

### gwt-pr

- **概要**: GitHub PR の作成・更新を REST-first で行う。既存 PR のマージ状態に基づいて新規作成か push のみかを判断する。base=develop、head=現在ブランチがデフォルト。
- **トリガー条件**: ユーザーが PR の作成・編集を要求した場合、または `gwt-spec-implement` から PR ハンドオフされた場合。
- **ワークフロー**:
  1. リポジトリ・ブランチ確認（ブランチ作成・切り替え禁止）
  2. `main` をターゲットにできるのは `develop` のみ（それ以外は拒否）
  3. ローカル worktree 状態チェック（uncommitted changes があれば一時停止しユーザーに選択肢提示）
  4. `git fetch origin` で最新リモート取得
  5. base ブランチとの同期チェック、behind > 0 なら `git merge origin/<base>` で更新（rebase 禁止）
  6. REST API で既存 PR を検索（`GET /repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>`）
  7. OPEN unmerged PR 存在 → push のみ。全 merged → post-merge commit check で新規 PR 要否判断
  8. PR body テンプレートから Conventional Commits 準拠のタイトルと必須セクション（Summary/Changes/Testing/Closing Issues/Related Issues/Checklist）で PR を構築
  9. REST-first で PR 作成（`POST /repos/<owner>/<repo>/pulls`）、失敗時は `gh pr create` フォールバック
  10. PR 作成後、`gwt-pr-fix` ワークフローで CI/merge/review チェックを自動実行
- **入力/出力**: 入力: base ブランチ（デフォルト develop）。出力: PR URL
- **依存スキル**: `gwt-pr-fix`（PR 作成後の CI/review チェック）
- **操作**: `gh api repos/<owner>/<repo>/pulls` (POST/PATCH)、`gh pr create`（フォールバック）、`git merge origin/<base>`、`git push`
- **停止条件**: ローカル uncommitted changes でユーザー未選択、マージコンフリクトが高信頼度で解決不可、`main` への直接 PR で head が `develop` でない場合

### gwt-pr-check

- **概要**: 現在のブランチの PR 状態を REST-first で確認し、推奨アクション（CREATE_PR / PUSH_ONLY / NO_ACTION / MANUAL_CHECK）を報告する。チェック専用で状態変更は行わない。
- **トリガー条件**: PR 状態の確認が必要な場合。`gwt-pr` や `gwt-spec-implement` の事前チェックとして使用。
- **ワークフロー**:
  1. リポジトリ・head ブランチ・base ブランチを解決
  2. `git fetch origin` でリモート同期
  3. REST API で head ブランチの PR を検索（`GET /repos/<owner>/<repo>/pulls?state=all&head=<owner>:<head>`）
  4. 分類: PR なし → `NO_PR`、OPEN unmerged → `UNMERGED_PR_EXISTS`、CLOSED unmerged のみ → `CLOSED_UNMERGED_ONLY`
  5. 全 PR merged の場合: merge commit SHA の祖先チェック → post-merge commit count → base branch との diff 確認
  6. `ALL_MERGED_WITH_NEW_COMMITS`（新コミットあり + diff あり）or `ALL_MERGED_NO_PR_DIFF`（diff なし）を判定
  7. 人間可読な1-3行のサマリーをデフォルト出力
- **入力/出力**: 入力: `--repo`、`--base`（デフォルト develop）、`--json`（オプション）。出力: ステータスプレフィックス付きサマリー（`>>` CREATE PR / `>` PUSH ONLY / `--` NO ACTION / `!!` MANUAL CHECK）
- **依存スキル**: なし（他スキルから参照される側）
- **操作**: `check_pr_status.py`、`gh api repos/<owner>/<repo>/pulls`（REST-first）、`git rev-list`、`git diff --quiet`
- **停止条件**: なし（チェック専用、状態変更なし）

### gwt-pr-fix

- **概要**: GitHub PR の CI 失敗・マージコンフリクト・base ブランチ遅延・レビューコメント・Change Request・未解決 review thread を検査し、高信頼度のブロッカーを自律的に修正する。REST-first で CI/review/comment を取得し、GraphQL は review thread の解決にのみ使用。
- **トリガー条件**: PR の CI 失敗、マージコンフリクト、レビューブロッカーの修正が必要な場合。`gwt-pr` の事後チェック、または `gwt-spec-implement` からの呼び出し。
- **ワークフロー**:
  1. `gh auth status` で認証確認（`GH_TOKEN` / `GITHUB_TOKEN` の直接 REST auth も可）
  2. PR を解決（REST head-branch lookup 優先）
  3. モード別検査: `checks`（CI 失敗）、`conflicts`（マージ状態）、`reviews`（Change Request / review thread / reviewer comment）、`all`
  4. Diagnosis Report 生成（BLOCKED/CLEAR 判定、B1/B2... 形式のブロッキングアイテム、Auto-fix 可否判定）
  5. 全ブロッキングアイテムが Auto-fix: Yes → 即時修正実行
  6. Auto-fix: No があれば → ユーザーに曖昧な項目のみ確認
  7. 修正適用後、全未解決 review thread に返信してから resolve（`--reply-and-resolve` で全 thread カバー必須）
  8. `--add-comment` でレビュアーに修正サマリーを通知（REST-first、フォールバック `gh pr comment`）
  9. `inspect_pr_checks.py --mode all` で再検証、全解消まで繰り返し（同一 CI チェック3回連続失敗でループ安全ガード発動）
- **入力/出力**: 入力: `--repo`、`--pr`、`--mode`（checks/conflicts/reviews/all）、`--required-only`。出力: Diagnosis Report（Merge Verdict / Blocking items / Informational items / Summary）
- **依存スキル**: なし（`gwt-pr` や `gwt-spec-implement` から呼ばれる側）
- **操作**: `inspect_pr_checks.py`、`gh api` REST（CI/reviews/comments）、GraphQL（review thread reply/resolve のみ）、`git merge origin/<base>`（BRANCH-BEHIND 修正）
- **停止条件**: マージコンフリクトが高信頼度で解決不可、レビューリクエストが行動的に曖昧、同一 CI チェック3回連続失敗（ユーザー判断待ち）

### gwt-agent-dispatch

- **概要**: PTY ベースの Agent ペインへの命令ディスパッチ。Assistant から Agent ペインへのコマンド送信、出力キャプチャ、Agent ライフサイクル管理を行う。
- **トリガー条件**: Project Mode オーケストレーションで Agent ペインへの命令送信が必要な場合。
- **ワークフロー**:
  1. `list_terminals` でアクティブな Agent ペイン ID を取得
  2. `capture_scrollback_tail` でフォローアップ前に Agent の出力を読み取り
  3. `send_keys_to_pane` で特定ペインに命令を送信（確定的ディスパッチ優先）
  4. `send_keys_broadcast` で全 Agent ペインに同報送信（必要時のみ）
  5. `close_terminal` でエスカレーション時に Agent ペインを停止
- **入力/出力**: 入力: ペイン ID、命令テキスト。出力: Agent ペインの出力テキスト
- **依存スキル**: なし（gwt ターミナルコマンドに依存）
- **操作**: `send_keys_to_pane`、`send_keys_broadcast`、`capture_scrollback_tail`、`list_terminals`、`close_terminal`
- **停止条件**: なし（命令ディスパッチは即時完了）

### gwt-project-search

- **概要**: ChromaDB ベクトル埋め込みを使用したプロジェクトソースファイルのセマンティック検索。機能・バグ・概念に関連するファイルの発見に使用する。
- **トリガー条件**: タスク開始時のファイル調査、バグ調査時の関連ファイル特定、機能追加時の既存実装検索、アーキテクチャ理解。
- **ワークフロー**:
  1. `chroma_index_runner.py --action search` でセマンティッククエリを実行
  2. `--db-path "$GWT_PROJECT_ROOT/.gwt/index"` でプロジェクトのインデックスを指定
  3. `--query` で検索クエリ、`--n-results` で結果数を指定
  4. 距離値が低いほど関連性が高い結果を返却
- **入力/出力**: 入力: `--query`（検索クエリ）、`--db-path`（インデックスパス）、`--n-results`（結果数）。出力: JSON 形式の検索結果（path, description, distance）
- **依存スキル**: なし（独立実行）
- **操作**: `chroma_index_runner.py --action search`
- **停止条件**: なし（検索は即時完了）

### gwt-spec-search

- **概要**: ChromaDB ベクトル埋め込みを使用したローカル SPEC ファイル（`specs/SPEC-{N}/`）のセマンティック検索。既存スペックの発見・重複チェック・スコープ所有者の特定に使用する。
- **トリガー条件**: SPEC 新規作成前の重複チェック、関連仕様の検索、スコープ所有者の特定。`gwt-spec-register`、`gwt-spec-ops`、`gwt-issue-register`、`gwt-issue-resolve` の必須プリフライト。
- **ワークフロー**:
  1. `chroma_index_runner.py --action search-specs` でセマンティッククエリを実行
  2. `specs/` 配下の SPEC メタデータとコンテンツを検索対象とする
  3. 結果は spec_id, title, status, phase, relevance を含む
- **入力/出力**: 入力: 検索クエリ。出力: 関連 SPEC 一覧（ID・タイトル・ステータス・関連度）
- **依存スキル**: なし（独立実行）
- **操作**: `chroma_index_runner.py --action search-specs`
- **停止条件**: なし（検索は即時完了）

## Custom Slash Commands

カスタムスラッシュコマンド（`.claude/commands/gwt-*.md`）は、各スキルへのエントリポイントとして機能する。ユーザーが `/gwt:<command-name>` で呼び出すと、対応するコマンドファイルが読み込まれ、SKILL.md のワークフローに従って処理が実行される。

### コマンドファイルの構造

各コマンドファイルは YAML frontmatter + Markdown body で構成される:

```yaml
---
description: コマンドの説明（Claude のスキル一覧に表示される）
author: akiojin
allowed-tools: Read, Glob, Grep, Bash
---
```

- **description**: Claude がコマンドの用途を判断するために使用。スキルの description と補完的な内容。
- **allowed-tools**: コマンド実行時に許可されるツール。
- **body**: `## Steps` セクションで SKILL.md の読み込みとワークフローの手順を記述。

### コマンドとスキルの対応

| コマンド | 対応スキル | 概要 |
|---------|-----------|------|
| `/gwt:gwt-issue-register` | gwt-issue-register | 新規作業の登録 |
| `/gwt:gwt-issue-resolve` | gwt-issue-resolve | 既存 Issue の解決 |
| `/gwt:gwt-issue-search` | gwt-issue-search | Issue のセマンティック検索 |
| `/gwt:gwt-spec-register` | gwt-spec-register | 新規 SPEC の作成 |
| `/gwt:gwt-spec-clarify` | gwt-spec-clarify | SPEC の曖昧箇所の明確化 |
| `/gwt:gwt-spec-plan` | gwt-spec-plan | 計画アーティファクト生成 |
| `/gwt:gwt-spec-tasks` | gwt-spec-tasks | tasks.md 生成 |
| `/gwt:gwt-spec-analyze` | gwt-spec-analyze | SPEC 完全性チェック |
| `/gwt:gwt-spec-implement` | gwt-spec-implement | SPEC の実装 |
| `/gwt:gwt-spec-search` | gwt-spec-search | ローカル SPEC 検索 |
| `/gwt:gwt-pr` | gwt-pr | PR 作成・更新 |
| `/gwt:gwt-pr-check` | gwt-pr-check | PR ステータス確認 |
| `/gwt:gwt-pr-fix` | gwt-pr-fix | CI/レビュー修正 |
| `/gwt:gwt-project-search` | gwt-project-search | プロジェクトファイル検索 |
| `/gwt:gwt-agent-dispatch` | gwt-agent-dispatch | Agent ペインへの命令ディスパッチ |

### コマンドの配置

- **正本**: `.claude/commands/gwt-*.md`（git 追跡対象）
- **埋め込み**: `skill_registration.rs` の `CLAUDE_COMMAND_ASSETS` で `include_str!()` によりバイナリに埋め込み
- **展開先**: gwt がプロジェクトに登録する際に `.claude/commands/` に書き出し

### コマンドの設計原則

1. **スキルへの委譲**: コマンドは SKILL.md のワークフローを呼び出すエントリポイントであり、ビジネスロジックは SKILL.md に記述する。
2. **Step 1 は常に SKILL.md 読み込み**: `Load .claude/skills/<name>/SKILL.md and follow the workflow.`
3. **ツール制限**: `allowed-tools` で実行可能なツールを制限（通常は `Read, Glob, Grep, Bash`）。
4. **gwt-spec-ops にはコマンドなし**: オーケストレーションスキルは他のスキルから呼び出されるため、直接のスラッシュコマンドを持たない。
