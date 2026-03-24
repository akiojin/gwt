<!-- GWT_SPEC_ARTIFACT:doc:plan.md -->

# plan.md — Agent Canvas タイルシステム共通仕様

## Summary

Agent Canvas の既存タイル実装（assistant, worktree, agent, terminal）をリファクタリングし、全タイルタイプが共通インターフェースに準拠するタイルシステム基盤を構築する。新タイルタイプ（editor, memo, image）の追加が宣言的登録のみで完結する拡張性モデルを確立する。

## Technical Context

### 現在のアーキテクチャ

- **`gwt-gui/src/lib/agentCanvas.ts`**: Canvas のデータモデル。`AgentCanvasCardType`（4種）、`AgentCanvasCard`（union type）、`AgentCanvasGraph`（グラフ構造）、`buildAgentCanvasGraph`（グラフ構築）を定義
- **`gwt-gui/src/lib/components/AgentCanvasPanel.svelte`**: Canvas の UI レンダリング。カードサイズは固定値（`CARD_WIDTH=280`, `CARD_HEIGHT=164`）、ドラッグ・パン・ズーム操作、レイアウト永続化を実装
- **`gwt-gui/src/lib/agentCanvas.test.ts`**: グラフ構築ロジックのユニットテスト

### 現在の課題

- `AgentCanvasCardType` がハードコードされた union type で、新タイプ追加にはソースコード修正が必要
- タイルサイズが全タイプ共通の定数（`CARD_WIDTH`, `CARD_HEIGHT`）で固定
- viewport 外挙動の制御機構が存在しない
- コンテキストメニューが未実装
- 共通プロパティ（zIndex, visible, locked）が型定義に含まれていない

### 変更対象ファイル

| ファイル | 変更内容 |
|---------|---------|
| `gwt-gui/src/lib/agentCanvas.ts` | タイル共通型、タイプ登録レジストリ、サイズ制約、viewport 外挙動定義 |
| `gwt-gui/src/lib/agentCanvas.test.ts` | レジストリ・制約・edge モデルのテスト追加 |
| `gwt-gui/src/lib/components/AgentCanvasPanel.svelte` | レジストリ駆動レンダリング、リサイズ制約適用、viewport 外アンマウント、コンテキストメニュー |
| `gwt-gui/src/lib/components/AgentCanvasPanel.test.ts` | コンテキストメニュー・viewport 外挙動の UI テスト |
| `gwt-gui/src/lib/tileRegistry.ts` | 新規: タイルタイプ宣言的登録レジストリ（FR-6） |
| `gwt-gui/src/lib/tileRegistry.test.ts` | 新規: レジストリのユニットテスト |

## Constitution Check

- **シンプルさの追求**: 既存の union type + 固定定数パターンから、宣言的レジストリパターンへのリファクタリング。複雑な DI やプラグインシステムは導入しない
- **外科的変更**: 既存の `agentCanvas.ts` と `AgentCanvasPanel.svelte` を段階的にリファクタリングし、動作を壊さない
- **TDD**: 各フェーズでテストを先行して記述

## Project Structure

変更は `gwt-gui/src/lib/` 内に閉じる。Rust バックエンド（`gwt-core`, `gwt-tauri`）への変更は不要。

## Complexity Tracking

| 項目 | 見積 |
|------|------|
| 新規ファイル | 2（tileRegistry.ts, tileRegistry.test.ts） |
| 変更ファイル | 4（agentCanvas.ts, agentCanvas.test.ts, AgentCanvasPanel.svelte, AgentCanvasPanel.test.ts） |
| 推定コード行数 | 新規 ~300行、変更 ~200行 |
| リスク | 既存レイアウト永続化との互換性（Phase 2 で検証） |

## Phased Implementation

### Phase 1: タイル共通型 + タイプ登録レジストリ（FR-1, FR-2, FR-6）

**目的**: 全タイルが共通プロパティを持ち、タイプごとのサイズ制約を宣言的に登録できる基盤を構築する

- タイル共通プロパティ型を定義（id, type, geometry, title, zIndex, visible, locked）
- `TileTypeDefinition` 型（サイズ制約、viewport 外挙動、コンテキストメニュー拡張）を定義
- タイプ登録レジストリ（`registerTileType`, `getTileDefinition`）を実装
- 既存 4 タイプ（assistant, worktree, agent, terminal）を登録
- `AgentCanvasCard` 型を共通プロパティベースにリファクタリング
- `buildAgentCanvasGraph` が共通プロパティを出力するよう更新

### Phase 2: サイズ制約 + リサイズ（FR-2）

**目的**: タイルタイプごとの min/max/default サイズ制約を Canvas UI に適用する

- `AgentCanvasPanel.svelte` の固定サイズ定数をレジストリ参照に置換
- リサイズ UI（ドラッグハンドル）を実装
- リサイズ時に min/max 制約をクランプ
- `buildDefaultLayouts` がタイプ固有の default サイズを使用するよう更新
- 既存レイアウト永続化との後方互換性を維持

### Phase 3: viewport 外挙動 + relation edge（FR-3, FR-4）

**目的**: タイルタイプごとの viewport 外挙動と手動 edge 接続を実装する

- viewport 矩形判定ロジックを実装（IntersectionObserver またはスクロール座標計算）
- タイプ定義の `offscreenBehavior` に基づくアンマウント / keep 分岐
- アンマウント対象タイルの状態保存・復元フック
- 手動 edge 接続 UI（タイル間のドラッグ操作）
- edge 追加・削除のデータモデル更新

### Phase 4: コンテキストメニュー（FR-5）

**目的**: 全タイルに共通 + タイプ固有のコンテキストメニューを提供する

- コンテキストメニュー表示コンポーネント（右クリック / 長押し）
- 共通メニュー項目: 削除、edge 接続開始、サイズリセット
- タイプ固有メニュー項目をレジストリから取得して追加
- セパレータによる共通 / タイプ固有の視覚的分離
