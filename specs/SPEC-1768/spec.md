> **🔄 TUI MIGRATION (SPEC-1776)**: This SPEC was originally designed for the GUI Agent Canvas tile system. In the gwt-tui context, this is **P3 (low priority)** — TUI does not use a canvas-based tile system. The concepts below are retained as reference for potential future TUI split-pane or layout management.

# タイルシステム共通仕様

## Background

gwt-tui では GUI 時代の Agent Canvas タイルシステムは使用しない。代わりに、Shell タブ / Agent タブのタブベースレイアウトと、ratatui の split-pane レイアウトを採用する。

本 SPEC の概念（タイル共通プロパティ、サイズ制約、viewport 外挙動、relation edge）は、将来的に TUI でスプリットペインやフローティングウィンドウを導入する際の参考として保持する。

## 概要（元 GUI 仕様からの抜粋）

### タイル共通プロパティ

- **id**: 一意識別子
- **type**: タイプ識別子（shell, agent, terminal 等）
- **geometry**: 位置とサイズ
- **title**: ヘッダー表示名
- **visible**: 表示/非表示状態

### TUI での対応

| GUI 概念 | TUI 対応 |
|----------|----------|
| Agent Canvas タイル | Shell タブ / Agent タブ |
| タイルの自由配置 | タブ切り替え + ratatui レイアウト |
| relation edge | タブ名にブランチ/worktree 情報を表示 |
| コンテキストメニュー | キーバインドによる操作 |
| viewport 外アンマウント | 非アクティブタブの PTY 出力バッファリング |

## Success Criteria

- [ ] SC-1: TUI のタブベースレイアウトが安定して動作する（SPEC-1654 で実装）
- [ ] SC-2: 将来的な split-pane 導入時にこの SPEC の概念を参照できる状態で保持されている
