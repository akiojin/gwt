> **⚠️ DEPRECATED (SPEC-1776)**: This SPEC describes GUI-only functionality (Tauri/Svelte/xterm.js) that has been superseded by the gwt-tui migration. The gwt-tui equivalent is defined in SPEC-1776.

# Agent Canvas メモタイル — spec.md

## Overview

Agent Canvas 上に自由配置可能な「メモタイル」を追加する。エージェント作業の計画・振り返り・メモをマークダウン形式で記録し、worktree タイルと空間的に関連づけて文脈を整理できるようにする。

## User Stories

### US-1: メモタイルの作成

**As** gwt ユーザー
**I want** Agent Canvas 上で新しいメモタイルを作成したい
**So that** 作業計画や思考メモを Canvas 上に残せる

**Acceptance Scenarios:**

1. Canvas のコンテキストメニューまたはツールバーから「Add Memo」を選択すると、新しいメモタイルが作成される
2. メモタイルはデフォルトサイズ（幅 320px × 高さ 240px）で作成され、ドラッグで移動可能
3. 作成直後は編集モード（テキストエリア）で開く

### US-2: マークダウン編集・プレビュー

**As** gwt ユーザー
**I want** メモタイル内でマークダウンを編集し、レンダリング結果をプレビューしたい
**So that** 構造化されたメモを素早く作成・確認できる

**Acceptance Scenarios:**

1. 編集モード: テキストエリアでマークダウンテキストを入力・編集できる
2. プレビューモード: 入力内容が MarkdownRenderer を通じてレンダリング表示される
3. 編集 ↔ プレビューをトグルボタンまたはキーボードショートカットで切り替え可能
4. プレビューモードでダブルクリックすると編集モードに戻る

### US-3: メモの永続化

**As** gwt ユーザー
**I want** メモ内容がアプリ再起動後も保持されてほしい
**So that** 記録した内容を失わない

**Acceptance Scenarios:**

1. メモ内容は `~/.gwt/memos/{project_hash}/` にマークダウンファイルとして保存される
2. アプリ再起動後、保存されたメモが Canvas 上の元の位置・サイズで復元される
3. メモの位置・サイズは既存の tileLayouts と同じ仕組みで永続化される

### US-4: メモタイルのリサイズ

**As** gwt ユーザー
**I want** メモタイルのサイズを自由に変更したい
**So that** 内容量に応じた適切な表示領域を確保できる

**Acceptance Scenarios:**

1. タイル端をドラッグしてリサイズ可能
2. リサイズ後のサイズは永続化される

### US-5: メモタイルの削除

**As** gwt ユーザー
**I want** 不要になったメモタイルを削除したい
**So that** Canvas を整理できる

**Acceptance Scenarios:**

1. メモタイルのヘッダーまたはコンテキストメニューから削除操作が可能
2. 削除前に確認ダイアログを表示（内容がある場合）
3. 削除後、永続化データからも除去される

### US-6: メモタイルと worktree タイルの手動紐付け

**As** gwt ユーザー
**I want** メモタイルを特定の worktree タイルに手動で紐付けたい
**So that** どのブランチの作業に関するメモかを視覚的に把握できる

**Acceptance Scenarios:**

1. メモタイルのコンテキストメニューから「Link to worktree」操作で worktree を選択し、relation edge が表示される
2. 紐付けられたメモタイルの edge を切断して独立メモに戻せる
3. worktree タイルが削除された場合、紐付けられたメモタイルは独立メモとして残る（メモ自体は削除されない）
4. 紐付けなしの独立メモタイルも自由に作成・配置できる

## Functional Requirements

- **FR-1**: 新しいカードタイプ `memo` を AgentCanvasCardType / AgentCanvasCard union に追加する
- **FR-2**: 編集モード（テキストエリア）とプレビューモード（MarkdownRenderer）を切り替え可能にする
- **FR-3**: メモ内容を `~/.gwt/memos/{project_hash}/` にマークダウンファイルとして保存する（プロジェクト単位で管理、ファイルシステムベースで既存のデータ永続化方針と整合）
- **FR-4**: タイルの位置・サイズは既存の tileLayouts の仕組みで管理する
- **FR-5**: メモタイルの作成・削除 UI を提供する
- **FR-6**: 既存の MarkdownRenderer コンポーネント（`gwt-gui/src/lib/components/MarkdownRenderer.svelte`）をプレビュー表示に再利用する
- **FR-7**: メモタイルと worktree タイルの手動 relation edge 接続・切断操作を提供する
- **FR-8**: worktree タイル削除時、紐付けられたメモタイルは独立メモとして保持する（メモ内容は失わない）
- **FR-9**: relation edge の接続状態をキャンバス永続化データに含める

## Non-Functional Requirements

- **NFR-1**: メモタイルはテキストのみのため軽量に保つ。viewport 外のメモは DOM 描画を省略可能
- **NFR-2**: メモ内容の自動保存（デバウンス付き、編集停止後 500ms 程度で保存）

## Technical Considerations

- 既存の MarkdownRenderer（marked + DOMPurify）を表示に再利用
- メモ内容の保存先: `~/.gwt/memos/{project_hash}/` 配下にマークダウンファイルとして保存。project_hash はプロジェクトパスのハッシュ値
- tileLayouts に `memo` タイプを追加し、既存のドラッグ・リサイズ・永続化機構をそのまま利用
- デフォルトサイズ: 幅 320px × 高さ 240px（既存カードの 280x164 より少し大きめ、テキスト記述に適したサイズ）
- メモタイルの最大数制限: なし（キャンバス上のタイルは自由配置が原則）
- Rust バックエンド（gwt-core）にメモファイルの CRUD コマンドを追加
- メモタイルと worktree タイルの手動 relation edge は v1 でサポート。既存の worktree→session edge と同じ描画・永続化の仕組みを再利用する

## Out of Scope (v1)

- メモのエクスポート機能
- メモの検索機能
- リアルタイムコラボレーション
- メモテンプレート

## Success Criteria

1. Agent Canvas 上でメモタイルを作成・編集・プレビュー・削除できる
2. メモ内容がアプリ再起動後も保持される
3. メモタイルのドラッグ移動・リサイズが既存タイルと同様に動作する
4. 既存の MarkdownRenderer を再利用してマークダウンが正しくレンダリングされる
5. メモタイルを worktree タイルに手動で紐付け・切断でき、relation edge が正しく表示される

## Clarification Resolution

- **保存先**: `~/.gwt/memos/{project_hash}/` にマークダウンファイルとして保存。理由: プロジェクト単位で管理でき、ファイルシステムベースで既存のデータ永続化方針（`~/.gwt/` 配下）と整合する
- **デフォルトサイズ**: 幅 320px × 高さ 240px。理由: 既存カード（280x164）より少し大きく、テキスト記述に適したサイズ
- **最大数制限**: 制限なし。理由: キャンバス上のタイルは自由配置が原則
- **worktree 関連付け**: v1 で手動紐付けをサポート。独立メモも自由に作成可能。worktree 削除時にメモは残る（ユーザー判断）
