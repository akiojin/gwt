# 機能仕様: Agent Mode Issue-first Spec Bundle CRUD

**仕様ID**: `SPEC-8ad13230`
**作成日**: 2026-02-17
**更新日**: 2026-02-17
**ステータス**: ドラフト
**カテゴリ**: GUI
**依存仕様**:

- `SPEC-ba3f610c`（エージェントモード）

**入力**: ユーザー説明: "Agent Mode issue-first Spec Kit artifacts (spec/plan/tasks/tdd/research/data-model/quickstart/contracts/checklists) with built-in + MCP CRUD"

## 背景

- Agent Mode の Issue-first 化は実装済みだが、Spec Kit 成果物の網羅性（`tdd.md`, `contracts/`, `checklists/`）と CRUD が不足している。
- `upsert_spec_issue` が既存セクションを空文字で上書きし、計画・タスク・TDDを消すリスクがある。
- MCP 連携は `contract append` のみで、更新・削除・一覧取得ができず、複数エージェント運用の再現性が低い。

## ユーザーシナリオとテスト *(必須)*

### ユーザーストーリー 1 - Master Agent が完全な成果物セットを維持する (優先度: P0)

Master Agent 利用者として、Issue-first Spec で `spec/plan/tasks/tdd/research/data-model/quickstart` を同一 Issue で扱いたい。再入力時に既存内容が消えないことが必要。

**独立したテスト**: `prepare_issue_spec` が既存 sections を保持しつつ `spec` を追記するテスト。

**受け入れシナリオ**:

1. **前提条件** 既存 Issue に `plan/tasks/tdd` が格納されている、**操作** 同じ SPEC-ID で再度メッセージ送信、**期待結果** `plan/tasks/tdd` が保持される。
2. **前提条件** 新規 SPEC-ID、**操作** 初回送信、**期待結果** Issue body に `TDD` セクションを含む成果物セットが作成される。

---

### ユーザーストーリー 2 - 成果物コメントを作成・更新・削除できる (優先度: P0)

Master Agent/サブエージェント利用者として、`contracts/*` と `checklists/*` を Issue コメント成果物として CRUD したい。

**独立したテスト**: artifact コメントのマーカー形式とレガシー形式をパースできるユニットテスト。

**受け入れシナリオ**:

1. **前提条件** 対象 Issue が存在、**操作** contract 成果物を upsert、**期待結果** 同名成果物は更新され、etag が変化する。
2. **前提条件** 対象 Issue に checklist 成果物が存在、**操作** delete 実行、**期待結果** delete 結果が `true` で返る。

---

### ユーザーストーリー 3 - MCP でも同じ操作を使える (優先度: P1)

任意エージェント利用者として、内蔵ツールと同等の CRUD を MCP から利用したい。

**独立したテスト**: MCP スクリプトの構文チェックと JSON スキーマ整合。

**受け入れシナリオ**:

1. **前提条件** MCP サーバー起動済み、**操作** `spec_issue_artifact_upsert` 実行、**期待結果** contract/checklist の両kindで成功する。
2. **前提条件** 既存成果物がある、**操作** `spec_issue_artifact_list` と `spec_issue_artifact_delete` 実行、**期待結果** 一覧取得と削除結果が一致する。

## エッジケース

- Legacy 形式（`contract:<name>` / `Checklist` 見出し）を既存データとして読み取れること。
- `expected_etag` 不一致時は更新・削除を拒否すること。
- コメント本文に空文字を渡した場合は upsert を拒否すること。
- 未存在成果物の delete は失敗ではなく `deleted: false` を返せること。

## 要件 *(必須)*

### 機能要件

- **FR-001**: Issue body は `Spec/Plan/Tasks/TDD/Research/Data Model/Quickstart/Contracts/Checklists` を保持しなければならない。
- **FR-002**: Master Agent の準備処理は既存 Issue 更新時に `plan/tasks/tdd/research/data-model/quickstart/contracts/checklists` を消してはならない。
- **FR-003**: システムは `contract` と `checklist` 成果物を Issue コメントとして `upsert/list/delete` できなければならない。
- **FR-004**: `append_spec_contract_comment` は後方互換を維持しつつ upsert 動作に統合しなければならない。
- **FR-005**: MCP サーバーは `spec_issue_artifact_upsert/list/delete` を提供しなければならない。
- **FR-006**: 成果物コメントは marker 形式（`<!-- GWT_SPEC_ARTIFACT:kind:name -->`）と legacy 形式を読み取らなければならない。

### 非機能要件

- **NFR-001**: 競合更新は `etag` により検出し、上書き前にエラーを返す。
- **NFR-002**: 既存の関連テスト（`gwt-core`, `gwt-tauri`, `gwt-gui` の対象範囲）が回帰しない。

## 制約と仮定

- GitHub 連携は `gh` CLI が利用可能で認証済みであることを前提とする。
- コメント成果物は 1 Issue あたり 100 件以内を実運用範囲とする（現行 GraphQL 取得制限）。
- UI 側は閲覧用途を優先し、契約編集はツール経由を前提とする。

## 成功基準 *(必須)*

- **SC-001**: `cargo test -p gwt-core issue_spec -- --nocapture` が成功する。
- **SC-002**: `cargo test -p gwt-tauri agent_master::tests -- --nocapture` と `commands::issue_spec::tests` が成功する。
- **SC-003**: `python3 -m py_compile scripts/gwt_issue_spec_mcp.py` が成功する。
- **SC-004**: `pnpm vitest run src/lib/components/AgentModePanel.test.ts src/lib/components/MainArea.test.ts` が成功する。
