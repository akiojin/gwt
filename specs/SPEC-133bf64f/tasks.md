# タスクリスト: Project Version History（タグ単位のAI要約 + 簡易CHANGELOG）

## Phase 1: バックエンド（Tauri Commands + キャッシュ + イベント）

- [x] T1: `crates/gwt-tauri/src/commands/version_history.rs` 追加（versions 一覧 + per-version 生成）
- [x] T2: `git tag`/`git log`/`git rev-list`/`git rev-parse` によるタグ単位の履歴取得（CHANGELOG.md は参照しない）
- [x] T3: 簡易CHANGELOG生成（Conventional Commits 風グルーピング + `(+N more)` 省略）
- [x] T4: AI要約生成（英語Markdown固定 + `## Summary`/`## Highlights` バリデーション）
- [x] T5: AppState にキャッシュ + inflight 管理を追加（同一範囲の再生成抑制）
- [x] T6: `project-version-history-updated` イベントで結果を通知（UI非ブロッキング）

## Phase 2: メニュー統合（AI設定が有効な場合のみ表示）

- [x] T7: Native メニュー `Git > Version History...` 追加（トリガーのみ、一覧展開しない）
- [x] T8: 表示条件を実装（プロジェクトが開いている + AI設定が構成済み）
- [x] T9: Profiles 保存後にメニュー再構築し、表示のON/OFFが即時反映される

## Phase 3: フロントエンド（Version History タブ）

- [x] T10: `Tab` 型に `versionHistory` を追加し、メニューアクション `version-history` でタブを開く
- [x] T11: `VersionHistoryPanel.svelte` 追加（一覧取得 + 逐次生成 + generating 表示）
- [x] T12: `project-version-history-updated` を listen して該当カードを差分更新

## Phase 4: テストとローカル検証

- [x] T13: Rust テスト追加（versions 組み立て/簡易CHANGELOG整形）
- [x] T14: Svelte テスト追加（versions 取得と per-version 生成呼び出し）
- [x] T15: `cargo test -p gwt-tauri` が通る
- [x] T16: `cargo clippy -p gwt-tauri --all-targets --all-features -- -D warnings` が通る
- [x] T17: `pnpm -C gwt-gui test` が通る
- [x] T18: `pnpm -C gwt-gui check` が通る
