> **🔄 TUI MIGRATION (SPEC-1776)**: This SPEC has been updated for the gwt-tui migration. The original TextMeshPro/Unity implementation has been replaced by ratatui + vt100 crate-based terminal emulation in gwt-tui.
> **Canonical Boundary**: `SPEC-1541` は vt100 / ANSI / resize / terminal rendering の正本である。グローバルな TUI 構成は `SPEC-1776`、入力操作ポリシーは `SPEC-1770` が担当する。

# TUI ターミナルエミュレーション

## Background

gwt-tui は ratatui + crossterm ベースの TUI アプリケーションであり、ターミナルエミュレーションには **vt100 crate** を使用する。PTY 出力は vt100 クレートの仮想ターミナルバッファに書き込まれ、ratatui でレンダリングされる。

> **実装方式（TUI 版）**: vt100 crate がANSI エスケープシーケンスの解析とバッファ管理を担当し、ratatui が描画を担当する。crossterm がホストターミナルとの入出力を仲介する。

現行の実装構成:

| コンポーネント | 役割 |
|---|---|
| `vt100::Parser` | ANSI エスケープシーケンス解析 + 仮想ターミナルバッファ管理 |
| `ratatui` レンダラー | vt100 バッファの内容を ratatui ウィジェットとして描画 |
| `crossterm` 入力 | キーイベントの受信と PTY への転送 |
| Shell タブ / Agent タブ | 管理画面でのタブ切り替えによる複数ターミナルペイン管理 |

## Architecture

**vt100 crate + ratatui レンダラーの 2 層構成**を採用する。

```text
PTY 出力 → vt100::Parser (ANSI パーサー + バッファ) → ratatui レンダラー → crossterm → ホストターミナル
キー入力 → crossterm イベント → PTY 入力（portable-pty WriteAsync）
```

- **エンジン層（vt100 crate）**: ANSI エスケープシーケンス解析、仮想ターミナルバッファ管理、カーソル制御。256色/TrueColor/代替画面バッファ対応。
- **レンダラー層（ratatui）**: vt100 バッファの状態を ratatui ウィジェットに変換し、crossterm バックエンドで描画。
- **入力層（crossterm）**: キーイベントを受信し、PTY に転送。
- **タブ管理層**: Shell タブ / Agent タブで複数ターミナルペインを切り替え。

## User Stories

- **US-1 [P0]**: エージェントタブを選択するとターミナル出力がリアルタイムで表示される
- **US-2 [P0]**: ANSI カラーコード付きのビルドログが正しく色付けされて表示される
- **US-3 [P0]**: Claude Code の Markdown 風出力（太字・色・罫線）が読みやすく表示される
- **US-4 [P0]**: スクロールバックバッファ内を上下スクロールして過去の出力を参照できる
- **US-5 [P1]**: ターミナル内のテキストを選択し、クリップボードにコピーできる
- **US-6 [P1]**: ターミナル内の URL をハイライト表示する
- **US-7 [P0]**: ターミナルリサイズ時に行数・列数が自動調整され PTY に通知される
- **US-8 [P1]**: vi, top 等の TUI アプリケーションが代替画面バッファで正常動作する（vt100 crate が対応）
- **US-9 [P2]**: 全ターミナルに同一キー入力を一斉送信（ブロードキャスト）できる

## Functional Requirements

| ID | 要件 |
|---|---|
| FR-001 | vt100 crate を使用し ANSI エスケープシーケンス（SGR 色、カーソル移動、画面クリア、スクロール、256色/TrueColor）を処理する |
| FR-002 | ratatui で等幅フォントによるターミナルグリッド表示を実現する |
| FR-003 | 10,000 行のスクロールバックバッファをサポートする |
| FR-004 | テキスト選択・クリップボードコピーをサポートする |
| FR-005 | ターミナルリサイズ時に行数・列数を再計算し PTY に SIGWINCH を通知する |
| FR-006 | URL を正規表現で検出しハイライト表示する |
| FR-007 | ダークテーマ（Catppuccin Mocha 相当の 16 色パレット）をデフォルトとする |
| FR-008 | 代替画面バッファ（vi, top 等で使用）をサポートする（vt100 crate の機能） |
| FR-009 | Shell タブ / Agent タブで複数ターミナルペインを切り替え可能にする |
| FR-010 | 全ターミナルへのキーブロードキャスト（同一キー入力の一斉送信）をサポートする |

## Non-Functional Requirements

| ID | 要件 |
|---|---|
| NFR-001 | 大量出力（秒間 1,000 行以上）でも描画が滞らない |
| NFR-002 | vt100 crate のバッファ操作は UI スレッドをブロックしない（tokio async） |
| NFR-003 | スクロールバックバッファのメモリ使用量が 10,000 行で 50MB を超えない |

## Success Criteria

| ID | 基準 |
|---|---|
| SC-001 | ANSI カラーコード（16色/256色/TrueColor）付きテキストが正しく表示される |
| SC-002 | Claude Code の出力が読みやすく表示される |
| SC-003 | スクロールバック・テキスト選択によるコピーが動作する |
| SC-004 | ターミナルリサイズ後もテキスト配置が崩れない |
| SC-005 | vi, top 等の TUI アプリケーションが代替画面バッファで正常に動作する |
| SC-006 | 全ターミナルへのキーブロードキャストが動作する |
