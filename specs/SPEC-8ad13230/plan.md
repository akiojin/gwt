# 実装計画: Agent Mode Issue-first Spec Bundle CRUD

**仕様ID**: `SPEC-8ad13230` | **日付**: 2026-02-17 | **仕様書**: `specs/SPEC-8ad13230/spec.md`

## 目的

- Agent Mode の Issue-first 仕様成果物を Spec Kit 相当に拡張し、`tdd` と `contracts/checklists` の運用を実用レベルで完結させる。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-core/`, `crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **外部連携**: GitHub CLI (`gh issue`, `gh api graphql`), MCP stdio（`scripts/gwt_issue_spec_mcp.py`）
- **テスト**: `cargo test`, `python3 -m py_compile`, `vitest`, `svelte-check`
- **前提**: `gh` 認証済み、Project V2 は既存 `sync` 実装を流用

## 実装方針

### Phase 1: データモデルとCRUD基盤

- `SpecIssueSections` に `tdd/contracts/checklists` を追加する。
- artifact コメントモデル（`contract/checklist`）を定義し、`upsert/list/delete` API を `gwt-core` に追加する。
- marker 形式と legacy 形式の両方を読めるパーサーを実装する。

### Phase 2: Tauriコマンドと内蔵ツール

- `issue_spec` コマンドに artifact CRUD を追加する。
- `agent_tools` の built-in tool 定義を拡張し、内蔵ツールから同じCRUDを実行可能にする。
- `append_spec_contract_comment` は後方互換を保ちつつ upsert に統合する。

### Phase 3: Master Agent / MCP / UI

- Master Agent の Spec 準備処理で既存 sections を保持し、spec のみ追記更新する。
- MCP スクリプトに artifact CRUD を追加し、内蔵ツールと同等の操作を公開する。
- IssueSpecPanel に `TDD/Contracts/Checklists` の表示を追加する。

## 生成成果物

- `specs/SPEC-8ad13230/research.md`
- `specs/SPEC-8ad13230/data-model.md`
- `specs/SPEC-8ad13230/quickstart.md`
- `specs/SPEC-8ad13230/contracts/issue-spec-artifacts.md`
- `specs/SPEC-8ad13230/tdd.md`

## テスト

### バックエンド

- `cargo check`
- `cargo test -p gwt-core issue_spec -- --nocapture`
- `cargo test -p gwt-tauri agent_master::tests -- --nocapture`
- `cargo test -p gwt-tauri commands::issue_spec::tests -- --nocapture`
- `cargo test -p gwt-tauri agent_tools::tests -- --nocapture`

### フロントエンド

- `pnpm check`
- `pnpm vitest run src/lib/components/AgentModePanel.test.ts src/lib/components/MainArea.test.ts`
