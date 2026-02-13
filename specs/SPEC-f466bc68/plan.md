# 実装計画: プロジェクトを開いたときに前回のエージェントタブを復元する

**仕様ID**: `SPEC-f466bc68` | **日付**: 2026-02-13 | **仕様書**: `specs/SPEC-f466bc68/spec.md`

## 目的

- プロジェクト再オープン時に、前回表示していたエージェントタブ（順序/アクティブ）を復元する
- 復元直後のターミナルが空に見える時間を減らす（スクロールバック末尾の先出し）
- localStorage/Tauri API が利用できない環境でも壊れない（best-effort）

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
  - 既存の `list_terminals` / `capture_scrollback_tail` を利用する（本機能のための新規コマンド追加は不要）
- **フロントエンド**: Svelte 5 + TypeScript + Vite（`gwt-gui/`）
- **ストレージ**: localStorage（プロジェクト単位の UI 状態）
- **テスト**: vitest（jsdom）+ svelte-check
- **前提**: 本機能は「バックエンドに存在する pane の UI 復元」に限定し、存在しない pane の自動再起動は行わない

## 実装方針

### Phase 1: 永続化フォーマット（プロジェクト単位）

- `gwt-gui/src/lib/agentTabsPersistence.ts` を追加し、以下を提供する
  - localStorage key: `gwt.projectAgentTabs.v1`
  - `loadStoredProjectAgentTabs(projectPath)`
  - `persistStoredProjectAgentTabs(projectPath, state)`
  - `buildRestoredAgentTabs(stored, terminals)`（存在 pane のみ復元 + active 判定）
- 保存内容は `{ tabs: [{ paneId, label }], activePaneId }`（順序保持）
- 保存データが壊れている/容量不足などは握りつぶす（best-effort）

### Phase 2: App 統合（復元と保存のガード）

- `gwt-gui/src/App.svelte`
  - プロジェクトを開いたタイミング（`projectPath` がセットされた後）で復元を実行
  - `list_terminals` で存在 pane を取得し、保存状態から復元する
  - 復元完了前に「空状態」で上書きしないように hydration 完了フラグで保存をガードする

### Phase 3: TerminalView の初期表示改善

- `gwt-gui/src/lib/terminal/TerminalView.svelte`
  - `capture_scrollback_tail`（64KB）を best-effort で表示してから、`terminal-output` の購読を開始する
  - Tauri API が無い場合はスキップし、クラッシュさせない

## テスト

### フロントエンド（vitest）

- `gwt-gui/src/lib/agentTabsPersistence.test.ts`
  - 保存データの正規化（trim/dedup）と merge 保存
  - 存在 pane のみ復元され、存在しない active は復元されない
- `gwt-gui/src/lib/terminal/TerminalView.test.ts`
  - mount 時に `capture_scrollback_tail` が呼ばれ、スクロールバックが表示される
  - `terminal-output` 購読がセットアップされる

### 追加チェック

- `pnpm -C gwt-gui check`
- `pnpm -C gwt-gui test`
