# 実装計画: GUI Session Summary のスクロールバック要約（実行中対応）+ 永続キャッシュ/更新制御

**仕様ID**: `SPEC-3a1b7c2d` | **日付**: 2026-02-15 | **仕様書**: `specs/SPEC-3a1b7c2d/spec.md`

## 目的

- session_id 未保存でも Summary を表示できるようにする
- 同一Worktreeの複数paneから最新出力のpaneを選択する
- session_id 確定後は既存フローへ戻す
- ブランチ単位で「今/過去」の作業状況を即表示できるよう、要約キャッシュを永続化する
- Liveフォーカス/非フォーカス/タブ無しで更新頻度を分け、変更がない限り更新しない
- 何を要約しているか（入力ソース/識別子/入力更新時刻）が分かるようにする

## 技術コンテキスト

- **バックエンド**: Rust 2021 + Tauri v2（`crates/gwt-tauri/`）
- **フロントエンド**: Svelte 5 + TypeScript（`gwt-gui/`）
- **要約生成**: `gwt-core::ai::summarize_scrollback`
- **テスト**: `cargo test`（Rust）

## 実装方針

### Phase 1: バックエンド（sessions command 拡張）

- `get_branch_session_summary` に **スクロールバック fallback** を追加
  - session_id が無い場合、起動中paneの scrollback を要約入力にする
  - 取得対象は **最終出力が最新のpane**
- `ScrollbackSummaryJob` を追加して非同期生成
  - `session-summary-updated` イベントに `pane:` の擬似session_idを付与
- キャッシュは `SessionSummaryCache` を再利用

### Phase 2: 永続キャッシュ（バックエンド）

- 生成済み要約を repo+branch 単位で永続化し、アプリ再起動後も即表示できるようにする
  - 読み込み: `get_branch_session_summary` 初回呼び出し時に lazy load
  - 書き込み: 要約生成成功時に atomic write

### Phase 3: フロントエンド（表示ラベル/更新制御）

- `sessionId` が `pane:` の場合は `Live (pane summary)` 表示に切り替える
- 更新間隔を状態に応じて切り替える
  - タブ無し: 自動更新しない（キャッシュ表示のみ）
  - Liveフォーカス: 15秒
  - Live非フォーカス: 60秒
- Summary ヘッダに「入力ソース/識別子/入力更新時刻」を表示する（英語のみ）

### Phase 4: テスト

- Rust: 最新pane選定ロジックのユニットテストを追加
- Rust: scrollback fallback が job を返すことを検証
- Rust: 永続キャッシュが即表示に効くこと、タブ無しでは更新しないことを検証
- Frontend: 更新間隔の切り替え（15/60/無効）を検証

## テスト

### Rust（gwt-tauri）

- latest pane 選定のユニットテスト
- scrollback fallback の job 生成テスト
- 永続キャッシュのロード/セーブ、タブ無し挙動のユニットテスト

### フロントエンド

- 更新間隔の切り替えと、タブ無しでポーリングしないことのテスト
