# Tasks: Agent Canvas 画像ビューアタイル (#1769)

## Phase 1: 型定義・バックエンド基盤

### 1.1 型定義拡張 [TEST] [P]

- [ ] `agentCanvas.ts` に `AgentCanvasImageCard` 型を追加
- [ ] `AgentCanvasCardType` に `"image"` を追加
- [ ] `AgentCanvasCard` union に `AgentCanvasImageCard` を含める
- [ ] テスト: `agentCanvas.test.ts` に画像カードが `AgentCanvasState.cards` に含まれることを検証

ファイル: `gwt-gui/src/lib/agentCanvas.ts`, `gwt-gui/src/lib/agentCanvas.test.ts`

### 1.2 Rust 画像読み取りコマンド [TEST] [P]

- [ ] `crates/gwt-tauri/src/commands/image.rs` を作成
- [ ] `read_image_file(path: String) -> Result<ImageData>` を実装（ファイル読み取り + Base64 + MIME 判定）
- [ ] マジックバイトによるフォーマット検証（PNG/JPG/SVG/WebP/GIF/BMP）
- [ ] 50MB サイズ上限チェック
- [ ] 非対応フォーマット拒否
- [ ] テスト: `crates/gwt-tauri/tests/image.rs` にユニットテスト（正常読み取り、サイズ超過、非対応フォーマット）

ファイル: `crates/gwt-tauri/src/commands/image.rs`, `crates/gwt-tauri/src/commands/mod.rs`

### 1.3 Rust クリップボード画像コマンド [TEST]

- [ ] `paste_image_from_clipboard() -> Result<ImagePasteResult>` を実装
- [ ] `~/.gwt/images/` ディレクトリ自動作成
- [ ] UUID ファイル名で PNG 保存
- [ ] 保存後のパスと Base64 を返却
- [ ] テスト: ディレクトリ作成・保存パス生成のユニットテスト（クリップボード自体は結合テスト）

ファイル: `crates/gwt-tauri/src/commands/image.rs`

### 1.4 Rust URL 画像取得コマンド [TEST]

- [ ] `fetch_image_url(url: String) -> Result<ImageData>` を実装
- [ ] HTTP GET でバイナリ取得 → Base64 エンコード
- [ ] フォーマット検証（マジックバイト）
- [ ] タイムアウト設定（30秒）
- [ ] テスト: フォーマット検証のユニットテスト

ファイル: `crates/gwt-tauri/src/commands/image.rs`

### 1.5 Tauri コマンド登録・capability 更新 [P]

- [ ] `commands/mod.rs` に image モジュール追加
- [ ] `main.rs` にコマンドハンドラ登録
- [ ] `capabilities/default.json` に `fs:allow-read` 追加
- [ ] `tauri.conf.json` CSP に `img-src 'self' data:` 追加

ファイル: `crates/gwt-tauri/src/main.rs`, `crates/gwt-tauri/capabilities/default.json`, `crates/gwt-tauri/tauri.conf.json`

## Phase 2: フロントエンド画像表示

### 2.1 ImageViewerTile コンポーネント [TEST]

- [ ] `gwt-gui/src/lib/components/ImageViewerTile.svelte` を作成
- [ ] Base64 data URL からの画像表示
- [ ] ローディング状態表示
- [ ] エラー状態表示（読み込み失敗、非対応フォーマット）
- [ ] ファイル名・パス表示（タイルヘッダー）
- [ ] テスト: `ImageViewerTile.test.ts` にレンダリングテスト（正常表示、エラー表示、ローディング）

ファイル: `gwt-gui/src/lib/components/ImageViewerTile.svelte`, `gwt-gui/src/lib/components/ImageViewerTile.test.ts`

### 2.2 ズーム・パン操作 [TEST]

- [ ] ホイールイベントでズーム（MIN_ZOOM=0.1, MAX_ZOOM=10）
- [ ] ドラッグでパン（pointerdown/pointermove/pointerup）
- [ ] ダブルクリックでズームリセット（FR-5）
- [ ] イベント伝搬制御（タイル内操作がキャンバスドラッグに伝搬しない）
- [ ] テスト: ズーム値変更・リセットのユニットテスト

ファイル: `gwt-gui/src/lib/components/ImageViewerTile.svelte`, `gwt-gui/src/lib/components/ImageViewerTile.test.ts`

## Phase 3: 入力方式の実装

### 3.1 ドラッグ＆ドロップ [TEST]

- [ ] `AgentCanvasPanel.svelte` にドロップゾーン追加
- [ ] ドロップされたファイルパスから `read_image_file` 呼び出し
- [ ] ドロップ位置をタイルの初期座標に使用
- [ ] 視覚的ドロップフィードバック（ドラッグオーバー時のハイライト）
- [ ] テスト: ドロップイベントから画像カード追加フローのテスト

ファイル: `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

### 3.2 ファイル選択ダイアログ [TEST] [P]

- [ ] キャンバスのツールバーまたはコンテキストメニューに「Add Image」アクション追加
- [ ] Tauri `dialog:allow-open` でファイル選択（フィルタ: PNG/JPG/SVG/WebP/GIF/BMP）
- [ ] 選択ファイルから `read_image_file` 呼び出し → 画像タイル生成
- [ ] テスト: ファイル選択後の画像カード追加フローのテスト

ファイル: `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

### 3.3 クリップボード貼り付け [TEST] [P]

- [ ] キャンバス上での Ctrl/Cmd+V ハンドラ追加
- [ ] `paste_image_from_clipboard` 呼び出し → 画像タイル生成
- [ ] クリップボードに画像がない場合のハンドリング（無視またはトースト通知）
- [ ] テスト: ペーストイベントハンドラのテスト

ファイル: `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

### 3.4 URL 入力 [TEST]

- [ ] コンテキストメニューまたはダイアログで URL 入力
- [ ] `fetch_image_url` 呼び出し → 画像タイル生成
- [ ] URL 不正・取得失敗時のエラーハンドリング
- [ ] テスト: URL 入力からの画像カード追加フローのテスト

ファイル: `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`

## Phase 4: 永続化

### 4.1 画像カード永続化 [TEST]

- [ ] `AgentCanvasPersistedState` に `imageCards` フィールド追加
- [ ] `StoredImageCard` 型定義（id, filePath, layout, zoomState）
- [ ] `agentTabsPersistence.ts` の保存・復元ロジックに画像カード対応追加
- [ ] アプリ起動時に永続化された画像カードの復元
- [ ] 画像ファイルが存在しない場合のエラーハンドリング（復元時にファイル削除済み等）
- [ ] テスト: `agentTabsPersistence.test.ts` に画像カードの保存・復元テスト

ファイル: `gwt-gui/src/lib/agentCanvas.ts`, `gwt-gui/src/lib/agentTabsPersistence.ts`, `gwt-gui/src/lib/agentTabsPersistence.test.ts`

## Phase 5: Relation Edge

### 5.1 worktree との手動 relation edge [TEST]

- [ ] 画像タイルから worktree タイルへのエッジ作成 UI（ドラッグ接続またはコンテキストメニュー）
- [ ] 手動エッジの永続化（`AgentCanvasPersistedState` に `manualEdges` 追加）
- [ ] エッジの削除 UI
- [ ] テスト: 手動エッジの追加・削除・永続化テスト

ファイル: `gwt-gui/src/lib/agentCanvas.ts`, `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`, `gwt-gui/src/lib/agentTabsPersistence.ts`

## Phase 6: エージェント自動生成画像対応

### 6.1 エージェント生成画像の自動表示 [TEST]

- [ ] エージェントセッションが画像ファイルを生成した際の検出メカニズム（ファイル監視 or イベント）
- [ ] 検出した画像パスから自動的に画像タイル生成
- [ ] 生成元エージェントの worktree への自動エッジ設定
- [ ] テスト: 画像パスからの自動タイル生成テスト

ファイル: `gwt-gui/src/lib/components/AgentCanvasPanel.svelte`, `crates/gwt-tauri/src/commands/image.rs`

## Phase 7: 統合テスト・検証

### 7.1 統合検証

- [ ] 全フォーマット（PNG/JPG/SVG/WebP/GIF/BMP）の表示確認
- [ ] 大画像（10MB+）の読み込みパフォーマンス確認
- [ ] ズーム・パン操作の応答性確認
- [ ] 永続化→復元の往復確認
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` パス
- [ ] `cargo fmt --check` パス
- [ ] `cd gwt-gui && npx svelte-check --tsconfig ./tsconfig.json` パス
- [ ] `cd gwt-gui && pnpm test` パス
