# tasks.md — Agent Canvas インタラクション詳細 (#1770)

## Phase 1: タイルリサイズ基盤 (FR-02)

- [ ] [TEST] agentCanvas.test.ts: タイルタイプ別リサイズ可否判定テスト（assistant=不可、worktree=不可、agent=不可、memo=可 等）
- [ ] [TEST] agentCanvas.test.ts: リサイズ後のサイズが min/max 制約内に収まることのテスト
- [ ] agentCanvas.ts: タイルタイプ別リサイズ制約定義ヘルパー（isResizable, getMinSize, getMaxSize）を追加
  - `gwt-gui/src/lib/agentCanvas.ts`
- [ ] AgentCanvasPanel.svelte: リサイズハンドル（右下コーナー）の描画ロジック追加（isResizable=true のカードのみ）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: リサイズポインターイベント処理（beginResize, handleResizeMove, endResize）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: リサイズ結果を cardLayouts に反映（既存永続化フローで保存）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

## Phase 2: スナップ・トゥ・グリッド (FR-01)

- [ ] [TEST] agentCanvas.test.ts: snapToGrid 関数のテスト（グリッドサイズ 20px 想定、座標の丸め検証）
- [ ] [TEST] agentCanvas.test.ts: スナップ無効時に座標がそのまま返ることのテスト
- [ ] [P] agentCanvas.ts: snapToGrid ユーティリティ関数を追加（座標とグリッドサイズを受け取り、最近接グリッド点を返す）
  - `gwt-gui/src/lib/agentCanvas.ts`
- [ ] [P] settingsPanelHelpers.ts: snapToGrid 設定の追加（デフォルト: false）
  - `gwt-gui/src/lib/components/settingsPanelHelpers.ts`
- [ ] [TEST] settingsPanelHelpers.test.ts: snapToGrid 設定の読み書きテスト
- [ ] SettingsPanel.svelte: スナップ・トゥ・グリッド トグル UI を追加
  - `gwt-gui/src/lib/components/SettingsPanel.svelte`
- [ ] AgentCanvasPanel.svelte: ドラッグ終了時（clearPointerState）にスナップ適用
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: リサイズ終了時にスナップ適用
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

## Phase 3: マルチ選択 (FR-05)

- [ ] [TEST] agentCanvas.test.ts: selectedCardIds の追加/解除ロジックテスト
- [ ] [TEST] agentCanvas.test.ts: 矩形選択の判定テスト（矩形内のカードIDリストを返す関数）
- [ ] [TEST] agentCanvas.test.ts: 一括ドラッグ移動のオフセット計算テスト
- [ ] [P] agentCanvas.ts: 矩形と cardLayout の交差判定ユーティリティを追加
  - `gwt-gui/src/lib/agentCanvas.ts`
- [ ] [P] agentCanvas.ts: AgentCanvasPersistedState に selectedCardIds を追加
  - `gwt-gui/src/lib/agentCanvas.ts`
- [ ] AgentCanvasPanel.svelte: Shift+クリックで selectedCardIds に追加/解除する処理
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: Shift+ドラッグで矩形選択（SelectionRect の描画 + 交差判定）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: 選択カードの一括ドラッグ移動（相対位置保持）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: Delete/Backspace キーで選択カードの一括削除
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: 空白領域クリックで全選択解除
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: 選択状態のカードにビジュアルフィードバック（ボーダーハイライト等）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

## Phase 4: 自動整列 (FR-04)

- [ ] [TEST] agentCanvas.test.ts: auto-arrange アルゴリズムのテスト（worktree 中心配置、children の周囲配置）
- [ ] [TEST] agentCanvas.test.ts: auto-arrange + スナップ有効時にグリッド吸着されることのテスト
- [ ] [TEST] agentCanvas.test.ts: assistant カードが固定位置に配置されることのテスト
- [ ] agentCanvas.ts: autoArrange 関数を追加（カード一覧・エッジ・スナップ設定を受け取り、新しい cardLayouts を返す）
  - `gwt-gui/src/lib/agentCanvas.ts`
- [ ] AgentCanvasPanel.svelte: auto-arrange ボタンをツールバーに追加
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: 実行前の確認ダイアログ表示（手動配置上書き警告）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: 確認後に autoArrange の結果を cardLayouts に適用
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

## Phase 5: ミニマップ (FR-03)

- [ ] [TEST] AgentCanvasMinimap.test.ts: ミニマップのビューポート矩形計算テスト
- [ ] [TEST] AgentCanvasMinimap.test.ts: ミニマップ上のドラッグ座標からキャンバス座標への変換テスト
- [ ] AgentCanvasMinimap.svelte: 新規コンポーネント作成（キャンバス全体の縮小表示 + ビューポート矩形オーバーレイ）
  - `gwt-gui/src/lib/components/AgentCanvasMinimap.svelte`
- [ ] AgentCanvasMinimap.svelte: ビューポート矩形のドラッグ操作でキャンバス位置を変更
  - `gwt-gui/src/lib/components/AgentCanvasMinimap.svelte`
- [ ] AgentCanvasPanel.svelte: ミニマップトグルボタンの追加（キャンバス右下）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: キーボードショートカットでミニマップ表示/非表示トグル
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`
- [ ] AgentCanvasPanel.svelte: ミニマップ表示状態の永続化（AgentCanvasPersistedState に追加）
  - `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

## Integration & Verification

- [ ] 全フェーズ統合後: 既存テスト（agentCanvas.test.ts）の回帰確認
- [ ] NFR-01: 100 タイルシナリオでの手動パフォーマンスチェック（60fps 維持）
- [ ] NFR-02: キーボードのみでマルチ選択・auto-arrange が操作可能であることを手動確認
- [ ] NFR-03: #1654 FR-009（自由配置・パン・ズーム）が破壊されていないことを既存テストで確認
- [ ] svelte-check 通過
- [ ] cargo clippy 通過（Rust 側変更がある場合）
