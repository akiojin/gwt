# plan.md — Agent Canvas コードエディタタイル

## Summary

Agent Canvas に `editor` カードタイプを追加し、CodeMirror 6 ベースのコードエディタをタイルとして表示する。worktree タイルからファイルを開き、構文ハイライト・編集・diff 表示・ファイル変更検出を提供する。

## Technical Context

### 既存資産の活用

- **agentCanvas.ts**: `AgentCanvasCardType` 型、`AgentCanvasCard` union、`buildAgentCanvasGraph()` によるグラフ構築パターン
- **AgentCanvasPanel.svelte**: カードレンダリング、viewport 管理、edge 描画の既存実装
- **notify crate**: `gwt-tauri` で `notify = "8"` + `notify-debouncer-mini = "0.7"` が既に依存済み
- **MarkdownRenderer パターン**: 既存の動的コンテンツレンダリングのパターンを参考にする

### 新規依存

- CodeMirror 6: `@codemirror/view`, `@codemirror/state`, `@codemirror/commands`, `@codemirror/merge`
- 言語パッケージ: `@codemirror/lang-javascript`, `@codemirror/lang-rust`, `@codemirror/lang-html`, `@codemirror/lang-css`, `@codemirror/lang-json`, `@codemirror/lang-markdown`, `@codemirror/lang-yaml`
- テーマ: `@codemirror/theme-one-dark`

## Architecture

### データフロー

1. ユーザーが worktree タイルでファイルを選択
2. フロントエンドが Tauri コマンド `read_file_content` を呼び出し
3. `AgentCanvasEditorCard` を生成し Canvas に追加
4. CodeMirror 6 エディタインスタンスを初期化
5. ファイル監視を Rust 側で開始し、変更を Tauri イベントでフロントエンドに通知

### カード型の拡張

- `AgentCanvasCardType` に `"editor"` を追加
- `AgentCanvasEditorCard` 型を新規定義（ファイルパス、言語、モード等を保持）
- `AgentCanvasCard` union に追加
- `buildAgentCanvasGraph()` で editor カードの生成・edge 接続ロジックを追加

### Tauri コマンド追加

- `read_file_content`: ファイル内容の読み取り
- `write_file_content`: ファイル内容の書き込み
- `watch_file`: ファイル監視の開始（notify crate 活用）
- `unwatch_file`: ファイル監視の停止
- `get_file_diff`: Git working tree diff の取得

## Phased Implementation

### Phase 1: タイル型定義 + 基本レンダリング

- `AgentCanvasCardType` に `"editor"` を追加
- `AgentCanvasEditorCard` 型を定義
- `CodeEditorTile.svelte` の骨格を作成（ファイル名表示、読み取り専用テキスト表示）
- `AgentCanvasPanel.svelte` にカード分岐を追加
- Tauri コマンド `read_file_content` を実装
- テスト: カード生成・グラフ構築のユニットテスト

### Phase 2: CodeMirror 6 統合 + 構文ハイライト

- CodeMirror 6 パッケージをインストール・統合
- ファイル拡張子から言語を自動判定するユーティリティ
- ダーク/ライトモード連動テーマ
- 読み取り専用 / 編集モードの切り替え
- Cmd+S / Ctrl+S での保存（`write_file_content` コマンド呼び出し）
- テスト: 言語判定、モード切り替えのテスト

### Phase 3: ファイル変更検出 + diff 表示

- `watch_file` / `unwatch_file` Tauri コマンド（notify crate 活用）
- Tauri イベントによるフロントエンドへの変更通知
- 未保存変更がある場合の確認ダイアログ
- `@codemirror/merge` による diff 表示モード
- `get_file_diff` Tauri コマンドで Git diff 取得
- unified / side-by-side 切り替え
- テスト: ファイル監視のユニットテスト、diff 表示の統合テスト

### Phase 4: 永続化 + viewport 外アンマウント

- editor カードの Canvas 状態永続化
- viewport 外のエディタアンマウント（カーソル・スクロール位置の保存/復元）
- worktree 削除時の関連 editor カード自動削除
- タイル閉じる際の未保存確認ダイアログ
- テスト: 永続化・復元のテスト

## Risks & Mitigations

| リスク | 影響 | 軽減策 |
|--------|------|--------|
| CodeMirror 6 バンドルサイズ増加 | 初回ロード時間の増加 | 動的インポートで遅延読み込み |
| 複数 editor タイルのメモリ消費 | パフォーマンス低下 | viewport 外アンマウント（Phase 4） |
| ファイル監視の競合 | 既存の notify 使用箇所との衝突 | 監視マネージャーで一元管理 |
