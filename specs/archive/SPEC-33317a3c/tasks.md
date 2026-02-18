# タスク: 共通環境変数とローカル環境取り込み機能

**仕様ID**: `SPEC-33317a3c`
**前提**: SPEC-8adfd99e の Web UI 環境変数編集機能が存在し、Fastify/CLI 基盤が動作していること。

**ポリシー**: CLAUDE.md の TDD ルールに従い、各タスクは RED→GREEN→REFACTOR を明示。Lint/format/markdownlint の最低限チェックをタスク完了条件に含める。

## フェーズ1: データ層・設定スキーマ

- [ ] **T1001** [P] `tests/unit/config.tools.shared-env.test.ts` を追加し、`loadToolsConfig` / `saveToolsConfig` が `env` ルートと `updatedAt` を扱えることを RED
- [ ] **T1002** `src/types/tools.ts` / `src/types/api.ts` に `env`（共通）と `history` 型を追加し、`configApi` / `useConfig` への波及テストを更新して GREEN
- [ ] **T1003** `src/config/tools.ts` に `saveToolsConfig()`（テンポラリファイル＋600権限）と `mergeSharedEnv()` ヘルパーを実装し、T1001 を GREEN
- [ ] **T1004** `tests/unit/env.history.test.ts` を追加し、履歴ファイル `env-history.json` の追記ロジックを RED
- [ ] **T1005** `src/config/env-history.ts`（新規）を実装して T1004 を GREEN

## フェーズ2: サーバーサイド機能

- [ ] **T1010** `tests/web/server/env/importer.test.ts` でローカル取り込みホワイトリストのテストを RED
- [ ] **T1011** `src/web/server/env/importer.ts`（新規）を実装し、起動時に `process.env` → `tools.json` を同期、T1010 を GREEN
- [ ] **T1012** `tests/web/server/routes/config.routes.shared.test.ts` を追加し、`GET/PUT /api/config` が共通 env / history / imported フラグを返すことを RED
- [ ] **T1013** `src/web/server/routes/config.ts` を拡張して T1012 を GREEN（PUT で競合検出、履歴追記、エクスポートプレースホルダー）
- [ ] **T1014** `tests/web/server/routes/config.export.test.ts` を作成し、`.env` エクスポートの署名 URL/期限切れ挙動を RED
- [ ] **T1015** `src/web/server/routes/config-export.ts` を実装し、T1014 を GREEN

## フェーズ3: クライアント UI 拡張

- [ ] **T1020** `tests/web/client/pages/config-shared.view.test.tsx` を追加し、共通 env セクションの表示・取り込みバッジを RED
- [ ] **T1021** `src/web/client/src/pages/ConfigManagementPage.tsx` と `components/EnvEditor.tsx` を拡張し、共通 env / 個別 env を切替表示できるようにして T1020 を GREEN
- [ ] **T1022** `tests/web/client/components/env-conflict-modal.test.tsx` を追加し、競合解決ダイアログの動作（優先順位選択）を RED
- [ ] **T1023** 競合ダイアログコンポーネントを実装し、`useUpdateConfig` と連携させて T1022 を GREEN
- [ ] **T1024** `tests/web/client/hooks/useConfig.test.ts` を更新し、`env` / `history` / `importedFromOs` を扱うキャッシュ動作を検証

## フェーズ4: CLI 連携

- [ ] **T1030** `tests/unit/launcher.env-merge.test.ts` を追加し、共有→個別→process の優先順位を RED
- [ ] **T1031** `src/launcher.ts` へ `buildEnvironment(toolId)` ヘルパーを追加し、CLI 起動で共通 env が適用されるようにして T1030 を GREEN
- [ ] **T1032** CLI 側で `env-history` を参照する必要があればテスト & 実装

## フェーズ5: 監査/履歴/エクスポート

- [ ] **T1040** `tests/web/client/pages/config-history.test.tsx` を RED（履歴モーダルの表示/フィルタリング）
- [ ] **T1041** 履歴モーダル UI + `.env` エクスポートボタンを実装して T1040 を GREEN

## フェーズ6: 統合テスト・最終確認

- [ ] **T1050** Playwright シナリオ: OS env 取り込み → 共通値編集 → ツール起動 → CLI 起動 で一貫性を検証
- [ ] **T1051** 最終チェック: `bun run lint`, `bun run format:check`, `bunx --bun markdownlint-cli ...`, `bun run test`, `bun run build`
- [ ] **T1052** 手動検証レポート（README もしくは PR コメント）: 取り込み → 共通編集 → 優先順位切替 → エクスポート → CLI 起動のフローを記録
