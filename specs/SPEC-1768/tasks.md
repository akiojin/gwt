<!-- GWT_SPEC_ARTIFACT:doc:tasks.md -->

# tasks.md — Agent Canvas タイルシステム共通仕様

## Phase 1: タイル共通型 + タイプ登録レジストリ（FR-1, FR-2, FR-6）

### US-1: 新しいタイルタイプを追加する際に共通仕様に従える

- [ ] [TEST] タイル共通プロパティ型のテスト — `gwt-gui/src/lib/tileRegistry.test.ts`
  - 登録済みタイプの定義取得、未登録タイプでの undefined 返却、デフォルトサイズ・制約の正当性
- [ ] [TEST] 既存 4 タイプ（assistant, worktree, agent, terminal）の登録テスト — `gwt-gui/src/lib/tileRegistry.test.ts`
  - 各タイプが登録されていること、定義にサイズ制約が含まれること
- [ ] [P] `TileTypeDefinition` 型定義 — `gwt-gui/src/lib/tileRegistry.ts`
  - typeId, sizeConstraints（min/max/default width/height）, offscreenBehavior, contextMenuExtensions
- [ ] [P] タイプ登録レジストリ実装（`registerTileType`, `getTileDefinition`, `getAllTileTypes`）— `gwt-gui/src/lib/tileRegistry.ts`
- [ ] 既存 4 タイプの宣言的登録 — `gwt-gui/src/lib/tileRegistry.ts`
  - assistant, worktree, agent, terminal の各サイズ制約・viewport 外挙動を定義
- [ ] [TEST] `AgentCanvasCard` 共通プロパティ型のテスト — `gwt-gui/src/lib/agentCanvas.test.ts`
  - `buildAgentCanvasGraph` の出力が共通プロパティ（id, type, geometry, title, zIndex, visible, locked）を含むこと
- [ ] `AgentCanvasCard` 型を共通プロパティベースにリファクタリング — `gwt-gui/src/lib/agentCanvas.ts`
  - `AgentCanvasCardBase` に共通プロパティを集約、既存 union type を拡張型に変更
- [ ] `buildAgentCanvasGraph` の出力に共通プロパティを追加 — `gwt-gui/src/lib/agentCanvas.ts`
  - geometry（レジストリの default サイズ）、zIndex、visible、locked の初期値を設定
- [ ] 既存テストがパスすることを確認（リグレッション検証）

## Phase 2: サイズ制約 + リサイズ（FR-2）

### US-2: 全てのタイルが同じ操作体系で動作する（サイズ制約）

- [ ] [TEST] タイプ固有の default サイズでレイアウトが生成されるテスト — `gwt-gui/src/lib/components/AgentCanvasPanel.test.ts`
  - `buildDefaultLayouts` がレジストリの default サイズを参照すること
- [ ] [TEST] リサイズ操作時の min/max クランプテスト — `gwt-gui/src/lib/components/AgentCanvasPanel.test.ts`
  - リサイズ結果が min 以上 max 以下に制約されること
- [ ] `AgentCanvasPanel.svelte` の `CARD_WIDTH` / `CARD_HEIGHT` 定数をレジストリ参照に置換 — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] `buildDefaultLayouts` をレジストリの default サイズで駆動するよう変更 — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] [P] リサイズドラッグハンドル UI の実装 — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
  - 各タイルの右下にリサイズハンドルを配置、ドラッグでサイズ変更
- [ ] [P] リサイズ時の min/max クランプロジック — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
  - レジストリから制約を取得し、ドラッグ結果をクランプ
- [ ] 既存レイアウト永続化との後方互換性テスト — `gwt-gui/src/lib/components/AgentCanvasPanel.test.ts`
  - 旧形式の永続化データ（固定サイズ）が新形式でも正常にロードされること

## Phase 3: viewport 外挙動 + relation edge（FR-3, FR-4）

### US-3: viewport 外挙動による適切なリソース管理

- [ ] [TEST] viewport 外判定ロジックのテスト — `gwt-gui/src/lib/agentCanvas.test.ts`
  - タイルの geometry と viewport 矩形の交差判定が正しく動作すること
- [ ] [TEST] offscreenBehavior による表示/アンマウント分岐テスト — `gwt-gui/src/lib/components/AgentCanvasPanel.test.ts`
  - unmount 指定タイルが viewport 外で非表示、keep 指定タイルが viewport 外でも表示されること
- [ ] viewport 矩形との交差判定ユーティリティ — `gwt-gui/src/lib/agentCanvas.ts`
  - タイルの geometry と viewport の可視領域の矩形交差判定関数
- [ ] `AgentCanvasPanel.svelte` で offscreenBehavior に基づくレンダリング分岐 — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
  - viewport 外 + unmount 指定のタイルは Svelte の `{#if}` でコンポーネントをアンマウント
  - viewport 外 + keep 指定のタイルはそのまま DOM を維持
- [ ] [P] アンマウント対象タイルの状態保存・復元フック定義 — `gwt-gui/src/lib/tileRegistry.ts`
  - `TileTypeDefinition` に `saveState` / `restoreState` コールバック型を追加

### US-3b: 手動 edge 接続

- [ ] [TEST] 手動 edge の追加・削除テスト — `gwt-gui/src/lib/agentCanvas.test.ts`
  - 手動 edge が `AgentCanvasGraph.edges` に追加・削除されること
  - 自動 edge と手動 edge が共存すること
- [ ] edge データモデルに `auto` / `manual` 区分を追加 — `gwt-gui/src/lib/agentCanvas.ts`
  - `AgentCanvasEdge` 型に `edgeType: "auto" | "manual"` を追加
- [ ] [P] 手動 edge 接続 UI（タイル間ドラッグ操作）— `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
  - edge 接続モード: ソースタイルからターゲットタイルへのドラッグで edge を作成
- [ ] [P] edge 削除 UI — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
  - edge をクリック / 右クリックで削除可能

## Phase 4: コンテキストメニュー（FR-5）

### US-4: コンテキストメニューから共通操作とタイプ固有操作にアクセスする

- [ ] [TEST] コンテキストメニュー表示テスト — `gwt-gui/src/lib/components/AgentCanvasPanel.test.ts`
  - 右クリックでメニューが表示されること
  - 共通メニュー項目（削除、edge 接続、サイズリセット）が含まれること
  - タイプ固有メニュー項目がセパレータの後に表示されること
- [ ] [TEST] コンテキストメニュー操作テスト — `gwt-gui/src/lib/components/AgentCanvasPanel.test.ts`
  - 削除でタイルが除去されること
  - サイズリセットで default サイズに戻ること
- [ ] [P] コンテキストメニューコンポーネント — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
  - 右クリック / 長押しでメニュー表示、外クリックで閉じる
- [ ] [P] 共通メニュー項目の実装 — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
  - 削除: タイルを Canvas から除去（関連 edge も削除）
  - edge 接続: edge 接続モードに遷移（Phase 3 で実装済み）
  - サイズリセット: レジストリの default サイズに geometry を戻す
- [ ] タイプ固有メニュー項目のレジストリ統合 — `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
  - `TileTypeDefinition.contextMenuExtensions` からメニュー項目を取得・表示

## Verification

- [ ] `cd gwt-gui && pnpm test` — 全ユニットテスト通過
- [ ] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` — 型チェック通過
- [ ] 各 Phase 完了時に既存テストのリグレッションがないことを確認
