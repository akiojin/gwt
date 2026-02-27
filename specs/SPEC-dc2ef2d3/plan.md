# 実装計画: Worktree詳細ビューでCLAUDE.md/AGENTS.md/GEMINI.mdを確認・修正し編集起動

**仕様ID**: `SPEC-dc2ef2d3` | **日付**: 2026-02-27 | **仕様書**: `specs/SPEC-dc2ef2d3/spec.md`

## 目的

- Worktree詳細ビューから指示ファイルの整備をワンクリックで完了させる。
- Windows PowerShell/cmd でも編集起動を失敗させない。

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **対象UI**: `WorktreeSummaryPanel`（ヘッダーアクション）
- **ストレージ/ファイル**: 選択ブランチの worktree 上の Markdown ファイル3種
- **テスト**: Rust unit test / Vitest
- **前提**: worktree が存在しない branch は修正対象外としてエラー中断

## 実装方針

### Phase 1: Backend command 追加（検査・修正）

- `crates/gwt-tauri/src/commands/clause_docs.rs` を新設。
- `check_and_fix_agent_instruction_docs(projectPath, branch)` を実装。
- 処理:
1. `projectPath` から repo 解決。
2. `branch` から worktree 解決（remote prefix 正規化対応）。
3. `CLAUDE.md`（存在・非空）を保証。
4. `AGENTS.md` / `GEMINI.md` の `@CLAUDE.md` を保証。
5. `worktreePath`, `checkedFiles`, `updatedFiles` を返却。
- `CLAUDE.md` 初期内容は Qiita 指定記事の構成を反映したテンプレート定数を使用。

### Phase 2: Tauri command 配線

- `crates/gwt-tauri/src/commands/mod.rs` に module 登録。
- `crates/gwt-tauri/src/app.rs` の `invoke_handler` へ command 登録。

### Phase 3: Frontend UI + editor 起動連携

- `WorktreeSummaryPanel.svelte`
  - ヘッダーボタン追加（`Check/Fix Docs + Edit`）。
  - command 呼び出し、実行中 disabled、エラー表示を追加。
  - 成功時 `onOpenDocsEditor(worktreePath)` コールバックを実行。
- `Sidebar.svelte`
  - `onOpenDocsEditor` prop を中継。
- `App.svelte`
  - `handleOpenDocsEditor(worktreePath)` を追加。
  - `spawn_shell` で新規 terminal タブを開き、`write_terminal` で編集コマンド投入。
  - Windows:
    - shell が `wsl` -> `vi`
    - shell が `powershell`/`cmd` -> `code` 優先、失敗時 `notepad`
  - 非Windows -> `vi`

## テスト

### バックエンド

- worktree あり branch で3ファイルが未作成なら自動作成される。
- 既存 `CLAUDE.md` は保持し、`AGENTS.md` のみ不足補完される。
- worktree 未存在 branch はエラー。

### フロントエンド

- 新規ボタンが表示され、押下時に command が呼ばれる。
- command 成功時に `onOpenDocsEditor` が `worktreePath` で呼ばれる。
- command 失敗時にエラー表示し、コールバックは呼ばれない。

## リスクと緩和

- **リスク**: Windows shell の差異でコマンド互換性が崩れる。  
  **緩和**: `default_shell` を参照して shell 別コマンドを生成し、`cmd` は専用構文を使う。
- **リスク**: `CLAUDE.md` テンプレートの将来変更が埋め込み定数に反映されない。  
  **緩和**: テンプレートを単一定数に集約し、差し替え容易にする。
