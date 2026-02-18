# 実装計画: Project Version History（タグ単位のAI要約 + 簡易CHANGELOG）

**仕様ID**: `SPEC-133bf64f` | **日付**: 2026-02-10 | **仕様書**: `specs/SPEC-133bf64f/spec.md`

## 目的

- CHANGELOG.md を読まずに、タグ単位の更新履歴を把握できるようにする
- AI設定が有効な場合にのみ、バージョンごとの要約と簡易CHANGELOGをGUIから閲覧できるようにする
- 生成処理はバックグラウンドで行い、UIをブロックしない

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript + Vite（`gwt-gui/`）
- **AI**: OpenAI互換API（`gwt-core` の `AIClient`）
- **前提**: プロジェクトはgwt管理下のbareリポジトリでも良い（gitコマンドでタグ/ログが取得できること）

## 実装方針

### Phase 1: バックエンド（Tauri Commands + キャッシュ + イベント）

- `crates/gwt-tauri/src/commands/version_history.rs` を追加し、以下を提供する
- `list_project_versions(projectPath, limit)`:
  - `v*` タグを `--sort=-v:refname` で取得
  - 先頭に常に `Unreleased (HEAD)` を付与し、残りを最新順に最大9件まで返す
  - 各アイテムに commit count を付与する（`git rev-list --count`）
- `get_project_version_history(projectPath, versionId)`:
  - キャッシュがあれば即時 `ok` を返す
  - キャッシュがなければ `generating` を返し、バックグラウンドジョブを開始する
  - 完了したら `project-version-history-updated` を emit する
- AppState にバージョン履歴キャッシュとinflight管理を追加する
  - キャッシュキーはコミット範囲のOID（`rev-parse`）で無効化できること

### Phase 2: 簡易CHANGELOGの生成（非AI）

- 対象コミットの subject を取得して、Conventional Commits 風にグルーピングする
  - Features / Bug Fixes / Documentation / Performance / Refactor / Styling / Testing / Miscellaneous Tasks / Other
- 1グループあたり最大表示件数を持ち、超過は `(+N more)` として省略する

### Phase 3: AI要約の生成

- `ProfilesConfig::load()` と `resolve_active_ai_settings()` により有効な設定がある場合のみ実行する
- 入力は「簡易CHANGELOG + コミットsubjectのサンプル」を上限付きで整形する
- 出力は英語Markdownに固定し、最低限のバリデーションを行う
  - `## Summary` と `## Highlights` を含むこと
  - Highlightsに箇条書きが含まれること

### Phase 4: メニュー統合（AI設定が有効なときのみ表示）

- `crates/gwt-tauri/src/menu.rs` の `Git` メニューに `Version History...` を追加する
- 表示条件
  - フォーカス中ウィンドウでプロジェクトが開かれている
  - `ProfilesConfig` のAI設定が有効である
- `save_profiles` 後にメニューを再構築し、表示のON/OFFが即時反映されるようにする

### Phase 5: フロントエンド（Version History タブ）

- `Tab.type` に `versionHistory` を追加し、メニューアクション `version-history` でタブを開く
- `VersionHistoryPanel.svelte` を追加する
  - `list_project_versions` で一覧取得
  - `get_project_version_history` を順次呼び出し、生成中は generating 表示
  - `project-version-history-updated` を listen して該当カードを更新

## テスト

### Rust（gwt-tauri）

- タグ一覧の組み立て（Unreleased + tags）テスト
- 簡易CHANGELOGのグルーピング・整形テスト

### Svelte（vitest）

- `VersionHistoryPanel` が versions 取得と per-version 生成呼び出しを行うこと
