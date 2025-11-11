# タスク: Web UI 環境変数編集機能

**入力**: `/specs/SPEC-8adfd99e/` の spec/plan
**前提条件**: plan 承認済み、feature/webui 系の土台が動作していること

**テスト方針**: CLAUDE.md の TDD ルールに従い、テスト→実装の順で進める。各フェーズ完了時に `bun run test`, `bun run build`, `bun run lint`, `bun run format:check`, `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore` を実行し、失敗時は修正する。

**フォーマット**: `[ID] [P?] [USx] 説明`
- `[P]` は並列実行可能タスク
- `[USx]` は対応するユーザーストーリー（spec.md参照）

## フェーズ1: サーバーAPI整備（US1/US2）

- [ ] **T901** [US1] `tests/web/server/routes/config.test.ts` を追加し、`GET /api/config` が `src/config/tools.ts#loadToolsConfig()` の実データを返し、env を key/value 配列へ変換するケースを RED で記述
- [ ] **T902** [US1] `src/web/server/routes/config.ts` を実装して `GET /api/config` をテストグリーン化（tools.json 取得、空ファイル時の空配列、読込エラー時の 500 応答）
- [ ] **T903** [US2] `tests/web/server/routes/config.test.ts` に `PUT /api/config` のバリデーション/競合/書き込み不可ケースを追加（RED）
- [ ] **T904** [US2] `src/config/tools.ts` に `saveToolsConfig()`・`validateToolsConfig()` 拡張を実装し、`src/web/server/routes/config.ts` で `PUT /api/config` を完成（アトミック保存、updatedAt 付与、マスク済ログ）、T903 をグリーン化
- [ ] **T905** [US2] `tests/unit/config.tools.test.ts` を追加して key/val 正規表現、最大数、競合チェックなど純粋関数ロジックをカバー

## フェーズ2: クライアント基盤（US1）

- [ ] **T910** [P] [US1] `src/web/client/src/router.tsx` に `/config` ルートを追加するテスト（`tests/web/client/router.test.tsx`）を先に作成し RED
- [ ] **T911** [US1] `src/web/client/src/router.tsx` と `src/web/client/src/main.tsx` を更新してテストをグリーン化、`BranchDetailPage` の設定リンクが実際に遷移することを確認
- [ ] **T912** [US1] `src/web/client/src/hooks/useConfig.ts` の `useConfig`/`useUpdateConfig` を、`version`/`updatedAt` を扱うよう単体テスト（React Query の mocking）で RED
- [ ] **T913** [US1] `useConfig` 実装を更新し、`configApi.get`/`update` のレスポンス型を追従させてテストをグリーン化

## フェーズ3: 環境変数可視化と編集（US1/US2）

- [ ] **T920** [US1] UI テスト（`tests/web/client/pages/config-management.view.test.tsx`）で、tools 配列があるときに env 行がマスク表示されるケースを RED で記述
- [ ] **T921** [US1] `src/web/client/src/pages/ConfigManagementPage.tsx` と `components/EnvEditor.tsx` を新規実装し、ツール一覧＋選択ツールの env テーブル（マスク表示・空状態）で T920 をグリーン化
- [ ] **T922** [US2] テストで「行追加→バリデーション→保存モーダル→成功トースト」フローを RED
- [ ] **T923** [US2] `EnvEditor` に行追加/削除/バリデーション（キー大文字化・正規表現、値長さ）、保存ボタン制御、`useUpdateConfig` 呼び出しと React Query キャッシュ更新を実装して T922 をグリーン化

## フェーズ4: セキュリティ操作（US3）

- [ ] **T930** [US3] UI テストで「👁 表示」クリック時に 3 秒後再マスク & 削除確認ダイアログの RED ケースを追加
- [ ] **T931** [US3] `EnvEditor` に一時表示タイマーと削除確認（`window.confirm` or カスタムダイアログ）を実装し T930 をグリーン化。削除後はローカル state と React Query キャッシュの両方を更新

## フェーズ5: CLI 一貫性と回帰（US4）

- [ ] **T940** [US4] `tests/unit/launcher.env.test.ts` を作成し、`tools.json` の env 変更が `launchCustomAITool` で利用されることを RED
- [ ] **T941** [US4] `src/launcher.ts`/周辺で必要ならキャッシュクリア or 最新ロードロジックを追加し、Web 保存後に CLI でも即反映されるようにして T940 をグリーン化
- [ ] **T942** [US4] 手動/自動テストドキュメント（`specs/SPEC-8adfd99e/quickstart.md` など）に Web→CLI 検証手順を追記

## フェーズ6: 最終検証

- [ ] **T950** [P] [US1-4] `bun run lint`, `bun run format:check`, `bunx --bun markdownlint-cli "**/*.md" --config .markdownlint.json --ignore-path .markdownlintignore`, `bun run test`, `bun run build` を実行し、結果を作業ログに記録
- [ ] **T951** [P] [US1-4] 仕様で求められたフロー（env 可視化→編集→一時表示→削除→CLI 反映）を手動で踏破してテストレポートを追加（docs or PR コメント）

> すべてのタスクは、仕様で定義された成功基準（SC-001〜SC-004）を満たすことを確認したうえでクローズする。
