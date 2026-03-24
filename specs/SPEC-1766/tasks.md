# tasks.md — Agent Canvas コードエディタタイル

## Phase 1: タイル型定義 + 基本レンダリング

### US-1: worktree タイルからファイルを開く（基盤）

- [ ] **T1-1** [TEST] `agentCanvas.test.ts` に `AgentCanvasEditorCard` 生成テストを追加
  - editor カードの型定義が正しいこと
  - `buildAgentCanvasGraph()` が editor カードを含む edge を生成すること
  - ファイル: `gwt-gui/src/lib/agentCanvas.test.ts`
- [ ] **T1-2** `AgentCanvasCardType` に `"editor"` を追加、`AgentCanvasEditorCard` 型を定義
  - ファイル: `gwt-gui/src/lib/agentCanvas.ts`
- [ ] **T1-3** [TEST] `read_file_content` Tauri コマンドのユニットテストを追加
  - ファイル: `crates/gwt-tauri/src/commands/` 配下
- [ ] **T1-4** Rust `read_file_content` Tauri コマンドを実装
  - ファイル: `crates/gwt-tauri/src/commands/` 配下
- [ ] **T1-5** `CodeEditorTile.svelte` の骨格を作成（ファイル名ヘッダー + プレーンテキスト表示）
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.svelte`
- [ ] **T1-6** `AgentCanvasPanel.svelte` に editor カードのレンダリングブランチを追加
  - ファイル: `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

## Phase 2: CodeMirror 6 統合 + 構文ハイライト

### US-1, US-3: エディタ統合

- [ ] **T2-1** [P] CodeMirror 6 パッケージをインストール（pnpm add）
  - `@codemirror/view`, `@codemirror/state`, `@codemirror/commands`
  - `@codemirror/lang-javascript`, `@codemirror/lang-rust`, `@codemirror/lang-html`, `@codemirror/lang-css`, `@codemirror/lang-json`, `@codemirror/lang-markdown`, `@codemirror/lang-yaml`
  - `@codemirror/theme-one-dark`
  - ファイル: `gwt-gui/package.json`
- [ ] **T2-2** [TEST] 言語判定ユーティリティのテストを追加
  - 拡張子 → 言語マッピングの正確性
  - ファイル: `gwt-gui/src/lib/editor/languageDetect.test.ts`
- [ ] **T2-3** 言語判定ユーティリティを実装
  - ファイル: `gwt-gui/src/lib/editor/languageDetect.ts`
- [ ] **T2-4** `CodeEditorTile.svelte` に CodeMirror 6 エディタを統合
  - EditorView の初期化・破棄のライフサイクル管理
  - ダーク/ライトモード連動テーマ
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.svelte`
- [ ] **T2-5** [TEST] モード切り替え（読み取り専用 / 編集）テストを追加
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.test.ts`
- [ ] **T2-6** 読み取り専用 / 編集モード切り替えを実装
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.svelte`
- [ ] **T2-7** [P] [TEST] `write_file_content` Tauri コマンドのユニットテストを追加
  - ファイル: `crates/gwt-tauri/src/commands/` 配下
- [ ] **T2-8** [P] Rust `write_file_content` Tauri コマンドを実装
  - ファイル: `crates/gwt-tauri/src/commands/` 配下
- [ ] **T2-9** Cmd+S / Ctrl+S キーバインドで `write_file_content` を呼び出し
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.svelte`

## Phase 3: ファイル変更検出 + diff 表示

### US-2: diff 表示、US-3: ファイル変更検出

- [ ] **T3-1** [TEST] `watch_file` / `unwatch_file` Tauri コマンドのユニットテストを追加
  - ファイル: `crates/gwt-tauri/src/commands/` 配下
- [ ] **T3-2** Rust `watch_file` / `unwatch_file` コマンドを実装（notify crate 活用）
  - ファイル: `crates/gwt-tauri/src/commands/` 配下
- [ ] **T3-3** Tauri イベントによるファイル変更通知をフロントエンドで受信
  - 未保存変更がある場合の確認ダイアログ
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.svelte`
- [ ] **T3-4** [P] `@codemirror/merge` パッケージをインストール
  - ファイル: `gwt-gui/package.json`
- [ ] **T3-5** [P] [TEST] `get_file_diff` Tauri コマンドのユニットテストを追加
  - ファイル: `crates/gwt-tauri/src/commands/` 配下
- [ ] **T3-6** [P] Rust `get_file_diff` コマンドを実装（Git working tree diff）
  - ファイル: `crates/gwt-tauri/src/commands/` 配下
- [ ] **T3-7** diff 表示モードを `CodeEditorTile.svelte` に実装
  - `@codemirror/merge` による unified / side-by-side 切り替え
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.svelte`

## Phase 4: 永続化 + viewport 外アンマウント

### US-1, US-3: 状態管理

- [ ] **T4-1** [TEST] editor カードの永続化・復元テストを追加
  - ファイル: `gwt-gui/src/lib/agentTabsPersistence.test.ts`
- [ ] **T4-2** editor カードの Canvas 状態永続化を実装
  - ファイル: `gwt-gui/src/lib/agentTabsPersistence.ts`, `gwt-gui/src/lib/agentCanvas.ts`
- [ ] **T4-3** viewport 外 editor タイルのアンマウント（カーソル・スクロール位置の保存/復元）
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.svelte`, `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] **T4-4** worktree 削除時の関連 editor カード自動削除
  - ファイル: `gwt-gui/src/lib/agentCanvas.ts`
- [ ] **T4-5** タイル閉じる際の未保存確認ダイアログ
  - ファイル: `gwt-gui/src/lib/components/CodeEditorTile.svelte`

## 備考

- `[P]` マーカーは並列実行可能なタスクを示す
- `[TEST]` マーカーはテストファーストで先に実装するタスクを示す
- 各 Phase 内は上から順に実行（ただし `[P]` タスクは前後のタスクと並列可）
