# 実装計画: GUI Worktree Summary 6タブ + Quick Launchヘッダー再編（Issue #1097）

**仕様ID**: `SPEC-7c0444a8` | **日付**: 2026-02-17 | **仕様書**: `specs/SPEC-7c0444a8/spec.md`

## 目的

- Worktree Summary を 6 タブ固定構成へ再編し、情報アクセスを役割別に明確化する。
- Quick Start の起動導線をヘッダーへ移し、`Continue/New` を `Launch Agent...` の左に常設する。
- Issue/PR/Workflow/Docker をブランチ文脈に沿って表示し、不要なフォールバック表示を排除する。
- 取得失敗をタブ単位に閉じ込め、パネル全体の継続利用性を担保する。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/src/commands/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/src/lib/components/WorktreeSummaryPanel.svelte`）
- **ストレージ/外部連携**: `gh`/Git metadata、session history、docker context detection
- **テスト**: `cargo test`（command周辺） / `pnpm test`（Vitest, Svelte Testing Library）
- **前提**:
  - ブランチ関連 Issue は `issue-<number>` 命名規約で判定する
  - PR/Workflow 取得は GitHub CLI の利用可否に依存するため、失敗時は空状態を返す

## 実装方針

### Phase 1: データ取得責務の整理（Backend/Type）

- Issue 表示対象を「ブランチ名から解釈した番号の Issue のみ」に制限する。
- PR 表示対象を「open 優先、なければ最新 closed/merged」で 1 件選定する。
- Workflow は選定 PR に紐づく check/workflow のみを扱い、PR 不在時は空状態にする。
- Docker は `detect_docker_context` の現在値と Quick Start 履歴由来値を併記できる型に整える。

### Phase 2: WorktreeSummaryPanel の 6 タブ + ヘッダー導線再編（Frontend）

- タブ列を固定順 (`Summary / Git / Issue / PR / Workflow / Docker`) で再構成し、既存カード混在レイアウトを分離する。
- `Quick Start` タブを廃止し、既存 Continue/New はヘッダー導線へ移設して挙動を維持する。
- `Git` / `Issue` / `PR` / `Workflow` / `Docker` をタブ単位で描画し、各タブで空状態/エラー状態を明示する。

### Phase 3: 回帰防止テストとエラーハンドリング整備

- `WorktreeSummaryPanel.test.ts` を 6 タブ + ヘッダー導線構成に合わせて更新し、固定順・責務分離・空状態・Continue/New の有効/無効を検証する。
- 必要な backend command テストを追加し、Issue/PR 選定ロジックの失敗ケースを検証する。
- 取得失敗が全体 UI を停止させないことを確認する。

## テスト

### バックエンド

- ブランチ名から `issue-<number>` を抽出し、該当 Issue のみ返すケース
- PR 選定（open 優先、fallback latest closed/merged）のケース
- PR なし時の workflow 空状態ケース

### フロントエンド

- 6 タブ固定順表示
- ヘッダー `Continue/New` の履歴あり/なし挙動
- Summary タブに Quick Start が混在しないこと
- Issue/PR/Workflow/Docker のデータあり/なし/取得失敗時の表示
- タブ単位失敗時でも他タブが利用可能であること
