> **ℹ️ TUI MIGRATION NOTE**: This SPEC was completed during the gwt-tauri era. The gwt-tauri frontend has been replaced by gwt-tui (SPEC-1776). GUI-specific references are historical.

### 背景

v8.3.1 で導入した API Key の peek/copy UI について、以下の表示回帰が報告された。

- macOS(v8.3.1): API Key 行の peek/copy ボタンが表示されない
- Windows(v8.3.0): API Key を表示( peek )した際に `_` が視認しづらい

既存の操作仕様（空キー時はボタン非表示、peek/copy挙動）は維持しつつ、クロスOSで表示互換性を担保する。

### ユーザーシナリオとテスト（受け入れシナリオ）

**US-1: macOSで API Key 操作ボタンが表示される** [P0]

- 前提: Profiles タブで API Key が非空
- 操作: Settings > Profiles > API Key 行を表示
- 期待: Peek ボタンと Copy ボタンが視認でき、押下操作可能

**US-2: Windowsで `_` を含む API Key が視認できる** [P0]

- 前提: API Key が `sk_test_ab_cd` のように `_` を含む
- 操作: Peek ボタンで表示状態にする
- 期待: `_` を含む値が欠落せず視認できる

**US-3: 既存操作仕様を維持する** [P0]

- 空キー時は peek/copy ボタン非表示
- mousedown/mouseup/mouseleave/keyboard による peek/copy 挙動は既存通り
- copy 成功フィードバック（Copied!）は既存通り

### 機能要件

| ID | 要件 |
|----|------|
| FR-001 | API Key 行の peek/copy ボタンは macOS/Windows で表示されなければならない |
| FR-002 | アイコン描画は WebKit 依存の疑似要素のみで実装してはならない |
| FR-003 | Peek/Copy ボタンはボタン内部に実体アイコン（SVG）を持たなければならない |
| FR-004 | API Key 入力欄は `_` を含む値の視認性を損なわないテキスト描画設定を持たなければならない |
| FR-005 | API Key が空の場合は peek/copy ボタンを表示してはならない |
| FR-006 | 既存の peek/copy 挙動（状態遷移・コピー動作・Copied! 表示）を維持しなければならない |

### 非機能要件

| ID | 要件 |
|----|------|
| NFR-001 | 変更範囲は `SettingsPanel.svelte` と関連テストに限定し、副作用を最小化する |
| NFR-002 | 既存テストを維持し、追加回帰テストを含めて `SettingsPanel.test.ts` が通過する |

### 成功基準

| ID | 基準 |
|----|------|
| SC-001 | API Key 非空時に peek/copy ボタンがDOM上で存在し、可視アイコンを持つ |
| SC-002 | `_` を含む API Key の表示値が保持される回帰テストがGREEN |
| SC-003 | 既存の API key peek/copy テスト群がすべてGREEN |
| SC-004 | `svelte-check` で型エラーなし |
