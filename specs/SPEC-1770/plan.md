# plan.md — Agent Canvas インタラクション詳細 (#1770)

## Overview

既存の AgentCanvasPanel.svelte（ドラッグ・パン・ズーム）を基盤に、5 つのインタラクション機能を段階的に追加する。各フェーズは独立してテスト・リリース可能な単位とし、既存の #1654 FR-009 を破壊しない。

## Architecture Decisions

- **状態管理**: agentCanvas.ts の型定義を拡張し、新規状態（選択セット、スナップ設定等）を AgentCanvasPersistedState に追加する
- **リサイズ制約**: #1768 の Tile Type Registry からリサイズ可否・最小/最大サイズを参照する。AgentCanvasPanel 側はレジストリの値を読み取るだけ
- **設定永続化**: 既存の SettingsPanel / settingsPanelHelpers を拡張してスナップ設定を追加する
- **ミニマップ**: 新規 Svelte コンポーネント（AgentCanvasMinimap.svelte）として分離し、AgentCanvasPanel から props で状態を受け取る

## Phased Implementation

### Phase 1: タイルリサイズ基盤 (FR-02)

**目的**: タイルタイプ別のリサイズ可否判定とリサイズハンドル表示

- agentCanvas.ts に `resizable` / `minSize` / `maxSize` をカードタイプ別に定義するヘルパーを追加
- AgentCanvasPanel.svelte にリサイズハンドル（右下コーナー）の描画とポインターイベント処理を追加
- リサイズ結果を cardLayouts に反映し、既存の永続化フローで保存

**依存**: #1768（Tile Type Registry）の型定義

### Phase 2: スナップ・トゥ・グリッド (FR-01)

**目的**: タイル配置時のグリッド吸着とユーザー設定

- agentCanvas.ts にスナップ計算ユーティリティ（座標をグリッドサイズで丸める関数）を追加
- AgentCanvasPanel.svelte のドラッグ終了時・リサイズ終了時にスナップ適用
- settingsPanelHelpers に snapToGrid 設定を追加し、SettingsPanel に UI トグルを追加
- 設定は既存の永続化機構で保存

**依存**: Phase 1（リサイズ終了時にもスナップ適用するため）

### Phase 3: マルチ選択 (FR-05)

**目的**: Shift+クリック個別選択とドラッグ矩形選択

- agentCanvas.ts の状態に `selectedCardIds: Set<string>` を追加（既存の `selectedCardId` と共存）
- Shift+クリックで選択セットに追加/解除する処理を AgentCanvasPanel.svelte に追加
- 空白領域でのドラッグ開始時にパンではなく矩形選択を開始する分岐（Shift+ドラッグ = 矩形選択、通常ドラッグ = パン）
- 選択セット内のカードを一括ドラッグ移動する処理
- 選択カードの一括削除（Delete/Backspace キー）
- 空白クリックで全選択解除

**依存**: なし（Phase 1/2 と並列可能だが、スナップとの統合テストは Phase 2 後）

### Phase 4: 自動整列 (FR-04)

**目的**: ワンクリックでツリー型自動配置

- agentCanvas.ts に auto-arrange アルゴリズムを追加: worktree カードを中心座標に配置し、紐づく session カードを周囲に等間隔で放射状に配置
- assistant カードは固定位置（左上）に配置
- 実行前に確認ダイアログを表示（手動配置の上書き警告）
- スナップがオンの場合、整列後の座標をグリッドに吸着

**依存**: Phase 2（スナップ統合）

### Phase 5: ミニマップ (FR-03)

**目的**: キャンバス全体のサムネイル表示とビューポート操作

- AgentCanvasMinimap.svelte を新規作成: キャンバス全体を縮小描画し、ビューポート矩形をオーバーレイ表示
- ミニマップ上のドラッグでビューポート位置を変更
- トグルボタン（キャンバス右下）とキーボードショートカットで表示/非表示を切り替え
- デフォルトは非表示、トグル状態は永続化

**依存**: なし（他フェーズと並列可能）

## Verification Strategy

- 各フェーズで agentCanvas.test.ts にユニットテストを追加（ロジック層）
- AgentCanvasPanel のインタラクションは E2E（Playwright）で検証
- NFR-01（60fps）は手動パフォーマンスチェック（100 タイルシナリオ）
- NFR-03（互換性）は既存テストの回帰確認で担保

## Risk & Mitigation

| リスク | 影響 | 対策 |
|--------|------|------|
| Phase 3 の矩形選択がパン操作と干渉 | UX 混乱 | Shift 修飾キーで明確に分離 |
| auto-arrange が大量タイルで重い | パフォーマンス劣化 | O(n) アルゴリズムで実装、100 タイル以下を想定 |
| ミニマップの再描画コスト | フレームレート低下 | requestAnimationFrame + debounce で制御 |
