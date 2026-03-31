> **⚠️ DEPRECATED (SPEC-1776)**: This SPEC describes GUI-only functionality (Tauri/Svelte/xterm.js) that has been superseded by the gwt-tui migration. The gwt-tui equivalent is defined in SPEC-1776.

<!-- GWT_SPEC_ARTIFACT:doc:spec.md -->
doc:spec.md

# spec.md — Agent Canvas コードエディタタイル

## Overview

Agent Canvas 上に新しい「editor」タイルタイプを追加し、worktree 内のファイルをインラインで閲覧・編集できるようにする。

## User Stories

### US-1: worktree タイルからファイルを開く

ユーザーとして、worktree タイルのファイルツリーからファイルを選択し、コードエディタタイルとして開きたい。

**受け入れシナリオ:**

- worktree タイルのファイルツリーでファイルをダブルクリック（またはコンテキストメニュー）すると、editor タイルが Canvas 上に生成される
- editor タイルは worktree タイルと relation edge で接続される
- 対応する構文ハイライトが適用される

### US-2: エージェント変更の差分確認

ユーザーとして、エージェントが変更したファイルの差分をコードエディタタイルで確認したい。

**受け入れシナリオ:**

- agent タイルでファイル変更が発生した際、editor タイルで diff 表示モードに切り替えられる
- 変更前後のコードが並列（side-by-side）またはインライン（unified）で表示される
- diff 表示から通常の編集モードに戻れる

### US-3: ファイルの直接編集

ユーザーとして、コードエディタタイルでファイルを直接編集し、ディスクに保存したい。

**受け入れシナリオ:**

- editor タイルで読み取り専用モードと編集モードを切り替えられる
- 編集モードでファイルを変更し、Ctrl+S / Cmd+S で保存できる
- 外部エディタでの変更がリアルタイムに反映される（ファイル変更検出）
- 未保存の変更がある場合、タイル閉じる際に確認ダイアログが表示される

## Functional Requirements

### FR-1: editor タイルタイプ

- Canvas の `AgentCanvasCardType` に `editor` を追加
- タイルのヘッダーにファイル名、言語アイコン、読み取り/編集モード表示
- リサイズ・移動は既存タイルと同じ挙動

### FR-2: 構文ハイライト

- 対応言語: TypeScript, JavaScript, Rust, Svelte, HTML, CSS, Markdown, JSON, TOML, YAML
- 言語はファイル拡張子から自動判定
- テーマは Canvas のダーク/ライトモードに連動

### FR-3: ファイル変更検出

- 既存の Rust `notify` crate（gwt-tauri で使用済み）を活用してファイル変更を監視
- 外部変更時にエディタの内容を自動更新（未保存変更がある場合は確認）

### FR-4: diff 表示モード

- CodeMirror 6 の `@codemirror/merge` 拡張を使用して diff を表示
- unified diff と side-by-side diff の切り替え
- Git の working tree diff（ステージ前）を表示
- 変更行のハイライト（追加: 緑、削除: 赤）

### FR-5: worktree タイルとの連携

- worktree タイルのファイルツリーから editor タイルを生成
- relation edge で接続し、worktree 削除時に関連 editor タイルも閉じる

### FR-6: メモリ管理

- viewport 外の editor タイルはアンマウントしてメモリ節約（既存ターミナルタイルと同じ方針）
- アンマウント時にカーソル位置・スクロール位置を保持し、再マウント時に復元

## Technical Decisions

### エディタライブラリ: CodeMirror 6

**採用理由:** バンドルサイズが Monaco の 1/10 以下、Svelte との親和性が高く、拡張性が十分。

- コアパッケージ: `@codemirror/view`, `@codemirror/state`, `@codemirror/commands`
- 言語サポート: `@codemirror/lang-javascript`, `@codemirror/lang-rust`, `@codemirror/lang-html`, `@codemirror/lang-css`, `@codemirror/lang-json`, `@codemirror/lang-markdown`, `@codemirror/lang-yaml`
- diff 表示: `@codemirror/merge` 拡張で unified / side-by-side diff を実現
- テーマ: `@codemirror/theme-one-dark`（ダークモード）、デフォルトテーマ（ライトモード）

### Rust バックエンド

- ファイル読み書き: 既存の `gwt-core` のファイル操作 API を拡張
- ファイル監視: `notify` crate（gwt-tauri で使用済み、`notify = "8"` + `notify-debouncer-mini = "0.7"`）
- diff 生成: `similar` crate または Git の diff 機能を利用

### フロントエンド

- Svelte 5 コンポーネントとして `CodeEditorTile.svelte` を実装
- タイル状態管理は既存の Canvas store に統合

## Success Criteria

- [ ] editor タイルが Canvas 上に表示・リサイズ・移動できる
- [ ] worktree タイルからファイルを選択して editor タイルを開ける
- [ ] 主要言語（TS, Rust, Svelte, MD）で構文ハイライトが動作する
- [ ] ファイルの読み取り・編集・保存ができる
- [ ] diff 表示モードで変更前後を比較できる
- [ ] 外部変更が自動検出・反映される
- [ ] viewport 外のタイルがアンマウントされメモリが解放される
