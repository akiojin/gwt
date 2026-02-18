# 実装計画: Version History 即時表示（永続キャッシュ + プリフェッチ）

**仕様ID**: `SPEC-c9a2f731` | **日付**: 2026-02-18 | **仕様書**: `specs/SPEC-c9a2f731/spec.md`

## 目的

- Version History タブを開いた瞬間にキャッシュ済みコンテンツを 100ms 以内に表示する
- アプリ再起動後もキャッシュを維持する永続化機構を導入する
- プロジェクトオープン時にバックグラウンドでキャッシュを先行生成する

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **ストレージ**: `~/.gwt/cache/version-history/{repo-hash}.json`（JSON ファイル）
- **テスト**: cargo test / vitest
- **前提**: 既存の `get_project_version_history` / `list_project_versions` コマンドを拡張

## 実装方針

### Phase 1: 永続キャッシュ基盤（バックエンド）

- `crates/gwt-tauri/src/commands/version_history.rs` にキャッシュファイルの読み書き関数を追加
  - `cache_file_path(repo_path) -> PathBuf`: repo_path の SHA-256 先頭 16 文字から `~/.gwt/cache/version-history/{hash}.json` を算出
  - `load_disk_cache(repo_path) -> Option<HashMap<String, VersionHistoryCacheEntry>>`: JSON ファイルから読み込み
  - `save_disk_cache(repo_path, entries)`: JSON ファイルへ書き出し
- `get_cached_version_history` を拡張: インメモリ → ディスクの順でルックアップ
- `generate_and_cache_version_history` を拡張: 生成完了時にディスクにも書き出し
- changelog を先行返却: AI 生成開始前に `changelog_markdown` をセットした `"generating"` レスポンスを返す

### Phase 2: 並列生成 + プリフェッチ（バックエンド）

- 並列セマフォの導入: `AppState` に `Arc<Semaphore>` を追加（最大 3 permits）
- `generate_and_cache_version_history` をセマフォ配下で実行するように変更
- プリフェッチコマンド `prefetch_version_history` を新規追加
  - `list_project_versions` → キャッシュのないバージョンを特定 → セマフォ配下で並列生成
- プロジェクトオープンフック: `open_project` 完了後に `prefetch_version_history` をバックグラウンド実行

### Phase 3: フロントエンド対応

- `VersionHistoryPanel.svelte` の `loadVersions` を変更:
  - `list_project_versions` → 各バージョンの `get_project_version_history` を並列呼び出し
  - キャッシュヒット分は即座に描画、未キャッシュ分は changelog を先行表示
- 既存の `project-version-history-updated` イベントリスナーは維持（ステータスバッジ更新に使用）
- 逐次生成ロジック（`stepGenerate`）を並列対応に変更

## テスト

### バックエンド

- `cache_file_path` がリポジトリパスから一意のパスを生成すること
- `save_disk_cache` → `load_disk_cache` のラウンドトリップ
- 不正 JSON のキャッシュファイルを読むと None が返ること
- OID 不一致でキャッシュミスになること
- 言語不一致でキャッシュミスになること
- セマフォが 3 並列を超えないこと
- プリフェッチがキャッシュ済みバージョンをスキップすること

### フロントエンド

- キャッシュヒット時に即座に結果が表示されること
- AI 生成中に changelog が先行表示されること
- `project-version-history-updated` イベントでステータスバッジが更新されること
