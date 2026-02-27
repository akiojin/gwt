# 実装計画: From Issue でブランチ名に prefix が二重表示される

**仕様ID**: `SPEC-1288` | **日付**: 2026-02-27 | **仕様書**: `specs/SPEC-1288/spec.md`

## 目的

- From Issue の Branch Name 表示を prefix/suffix 分離にして視認性を改善し、launch payload の既存仕様を維持する。

## 技術コンテキスト

- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/src/lib/components/AgentLaunchForm.svelte`）
- **バックエンド**: 変更なし
- **テスト**: `gwt-gui/src/lib/components/AgentLaunchForm.test.ts`
- **前提**: branch 作成の実体ロジックは変えず、UI 表示と branch 文字列の組み立て箇所のみを調整する

## 実装方針

### Phase 1: TDD（RED）

- From Issue の表示値が `issue-<number>` であることを検証するテストを追加する。
- Launch request の `branch` / `createBranch.name` が full branch name であることを検証する。

### Phase 2: UI / launch 組み立て修正

- `issueBranchName` を `issueBranchSuffix` に置換し、From Issue の readonly 入力欄は suffix のみ表示する。
- launch 時は `buildNewBranchName(newBranchPrefix, issueBranchSuffix)` で full branch name を構築する。
- Manual タブの既存ロジック (`newBranchFullName`) は維持する。

### Phase 3: 検証

- 変更対象テストを実行して pass を確認する。
- `svelte-check` を実行して型エラーがないことを確認する。

## テスト

### フロントエンド

- `keeps Launch disabled in fromIssue mode until a prefix is selected`
- `does not link or rollback issue branch before async launch job completion`（表示 + payload 検証を追加）

### バックエンド

- 変更なし
