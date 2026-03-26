# Agent Canvas メモタイル — plan.md

## Summary

Agent Canvas に新しい `memo` カードタイプを追加し、マークダウン形式のメモをキャンバス上に自由配置できるようにする。既存のタイルシステム（ドラッグ・リサイズ・永続化）と MarkdownRenderer を最大限再利用し、Rust バックエンドにメモファイルの CRUD を追加する。メモタイルは worktree タイルへの手動紐付けもサポートする。

## Technical Context

### 既存資産の活用

- **agentCanvas.ts**: `AgentCanvasCardType` union、`AgentCanvasCard` union、`AgentCanvasCardLayout`、`buildAgentCanvasGraph` — memo カードタイプを追加
- **AgentCanvasPanel.svelte**: カードのドラッグ移動・リサイズ・レンダリング — memo カードのレンダリングブランチを追加
- **MarkdownRenderer.svelte**: marked + DOMPurify によるマークダウンレンダリング — プレビューモードで再利用
- **agentTabsPersistence.ts**: localStorage ベースのレイアウト永続化 — tileLayouts に memo カードを含める
- **gwt-core**: ファイルシステム操作の基盤 — `~/.gwt/memos/` への CRUD を追加
- **relation edge 描画**: 既存の worktree→session edge の描画・永続化機構 — memo→worktree edge で再利用

### 保存アーキテクチャ

- メモ内容: `~/.gwt/memos/{project_hash}/{memo_id}.md` にマークダウンファイルとして保存（Rust バックエンド経由）
- メモレイアウト（位置・サイズ）: 既存の tileLayouts（localStorage）で管理
- メモメタデータ（ID リスト・タイトル）: `~/.gwt/memos/{project_hash}/index.json` で管理
- relation edge（メモ→worktree 紐付け）: tileLayouts 永続化データに含める

## Phased Implementation

### Phase 1: タイル型定義 + 基本レンダリング

- `AgentCanvasCardType` に `"memo"` を追加
- `AgentCanvasMemoCard` 型を定義（id, type, title, content）
- `AgentCanvasCard` union に追加
- `AgentCanvasPanel.svelte` に memo カードのレンダリングブランチ（テキストエリア + プレビュートグル）を追加
- `MemoCardContent.svelte` コンポーネントを新規作成（編集 / プレビュー切り替え）

### Phase 2: Rust バックエンド永続化

- gwt-core に memo モジュールを追加（CRUD: create / read / update / delete / list）
- gwt-tauri に Tauri コマンドを追加（`memo_create`, `memo_read`, `memo_update`, `memo_delete`, `memo_list`）
- project_hash の算出（プロジェクトパスの SHA-256 先頭 16 文字）
- フロントエンドから Tauri invoke 経由でメモを読み書き

### Phase 3: Canvas 統合 + UI

- Canvas ツールバーまたはコンテキストメニューに「Add Memo」を追加
- メモカードの作成フロー（デフォルト位置・サイズで生成、即編集モード）
- メモカードの削除（確認ダイアログ付き）
- tileLayouts への memo カード統合（位置・サイズの永続化）
- アプリ起動時に memo_list → カード復元

### Phase 3.5: Relation Edge（メモ-worktree 紐付け）

- メモタイルと worktree タイルの手動 edge 接続 UI
- edge 切断操作
- worktree 削除時のメモ保持ロジック
- edge 状態の永続化

### Phase 4: UX ポリッシュ

- 自動保存（デバウンス 500ms）
- リサイズハンドル（memo カード固有のデフォルトサイズ 320x240）
- プレビューモードでのダブルクリック → 編集モード切り替え
- キーボードショートカット（Escape で編集終了、等）

## Risks & Mitigations

- **tileLayouts との整合**: memo カードは worktree/session とは独立したライフサイクルを持つ。buildAgentCanvasGraph とは別に memo カード一覧を管理する必要がある
- **ファイル I/O エラー**: Rust 側で適切なエラーハンドリングとフロントエンドへのエラー通知を実装
- **relation edge の整合性**: worktree 削除時に orphan edge が残らないよう、削除イベントで edge をクリーンアップし、メモタイルを独立状態に戻す

## Decision Record

- 保存先に `~/.gwt/memos/` を選択: 既存の `~/.gwt/` 配下のデータ管理方針と整合し、localStorage の容量制限を回避
- デフォルトサイズ 320x240: 既存カード（280x164）より大きめで、テキスト記述に十分なスペースを確保
- 最大数制限なし: キャンバスの自由配置原則を尊重
- worktree 紐付けは v1 で手動サポート: ユーザー判断により v1 スコープに含める。独立メモと紐付けメモの両方をサポート
