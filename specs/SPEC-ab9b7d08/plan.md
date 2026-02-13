# 実装計画: Aboutダイアログにバージョン表示 + タイトルにプロジェクトパス表示（GUI）

## 目的

- ウィンドウタイトルをプロジェクトパス表示に固定する（`gwt` / `<projectPath>`）
- 実行中の gwt バージョンを About ダイアログで確認できるようにする

## 実装方針

### Phase 1: タイトル文字列生成の分離（TDD）

- タイトルのフォーマットをフロントエンドのユーティリティへ切り出す
- 文字列生成をユニットテストで固定し、回帰を防止する

### Phase 2: バージョン取得（best-effort）

- Tauri API からアプリバージョンを取得する
- 取得失敗時は `null` として扱い、About 表示は `Version unknown` とする

### Phase 3: タイトル更新 + About表示

- `document.title` を更新しつつ、Tauri 実行時は `setTitle()` でネイティブのタイトルバーも更新する
- About ダイアログへ `Version <...>` を表示する
- バージョン取得は UI 表示をブロックしない（取得完了後に About 表示へ反映）
- Tauri の `setTitle()` に必要な権限を有効化する

## テスト

- unit test（Vitest）でタイトルフォーマットと About 用バージョン表示を検証する
- `pnpm -C gwt-gui test` が成功すること
- `pnpm -C gwt-gui check`（svelte-check）でエラー・警告がないこと
