# 実装計画: 共通環境変数とローカル環境取り込み機能

**仕様ID**: `SPEC-33317a3c` | **日付**: 2025-11-11 | **仕様書**: [spec.md](./spec.md)

## 1. ゴールと適用範囲

- `tools.json` に共通 `env` を追加し、Web UI / CLI / Web サーバーのすべてで利用する。
- serve 起動時に OS 環境からホワイトリスト（OPENAI_API_KEY など）を検査し、自動取り込みする。
- 競合や優先順位を UI/CLI 双方で統一し、履歴/エクスポートなど拡張機能の基盤を整える。
- 既存 SPEC (`SPEC-8adfd99e`) の UI に共通 env セクションを追加し、重複ロジックを統合する。

## 2. 設計・技術方針

### 2.1 スキーマ拡張

- `ToolsConfig` に `env: Record<string,string>` と `updatedAt` を正式追加（既に後方互換あり）。
- REST API (`GET /api/config`, `PUT /api/config`) では `ConfigPayload` に `env` と `history` を含むよう拡張。
- `CustomAITool.env` は従来通り。起動時は `sharedEnv` → `tool.env` → `process.env` の順で `Object.assign`。

### 2.2 ローカル環境取り込み

- `src/web/server/env/whitelist.ts`（新規）で対象キーの配列を定義。
- Fastify 起動時に `process.env` を走査し、`tools.json` に無いキーだけ `env` へ追記。追記後は `saveToolsConfig()` を通じてアトミック保存。
- 取り込み結果はフラグ（`importedFromOs: true`）を API 応答に含め、UI 表示時にバッジ化。

### 2.3 UI 変更

- `ConfigManagementPage` に「共通環境変数」カードを追加（既存 `EnvEditor` を拡張して共通/個別で再利用）。
- 行ごとに「ローカル値との差異」「取り込み済み」のステータスを表示。`useConfig` で `env` と `tools` を同じクエリで取得。
- 優先度説明や競合解決モーダルを `EnvEditor` に実装し、選択肢に応じて `env` と `tool.env` を更新。

### 2.4 CLI 連携

- `src/launcher.ts` の `launchCustomAITool` 呼び出し前に、`envManager.getMergedEnv(toolId)` のようなヘルパーを噛ませる。
- 既存 CLI コードが `CustomAITool.env` を直接読む箇所を `getSharedEnv()` + `tool.env` 合成に置き換え。
- ユニットテスト（`tests/unit/launcher.env.test.ts`）で共有→個別→プロセスの優先順位を検証。

### 2.5 履歴/エクスポート

- `~/.claude-worktree/env-history.json` に `{key, action, timestamp, source}` を追記するヘルパーを server 側に実装。
- Web UI では履歴をモーダルで閲覧、`.env` ダウンロードは Fastify にエンドポイント `/api/config/env/export` を追加（1分で失効する署名トークン）。

## 3. 実装ステップ（ハイレベル）

1. **データ層**: `Types` 更新、`loadToolsConfig`/`saveToolsConfig` の書き込み対応、`env-history` 追加。
2. **サーバー層**: Config ルート拡張、ローカル取り込みロジック、エクスポート用ルート、ログマスキング。
3. **クライアント層**: `useConfig` 型更新、共通 env UI、競合ダイアログ、履歴/エクスポート UI。
4. **CLI**: 共有 env を読み込むヘルパー、優先順位テスト、キャッシュ無効化。
5. **テスト/検証**: Vitest（unit/integration）、Playwright（Web UIフロー）、手動で CLI との整合確認。

## 4. テスト戦略

| レベル | 対象 | シナリオ |
| --- | --- | --- |
| Unit (server) | env import, config routes | ホワイトリスト取り込み、有無差での保存、マスクログ |
| Unit (client) | EnvEditor shared mode | 行追加/削除、優先順位切替、差分表示 |
| Unit (CLI) | Merged env helper | 共通→個別→process の優先順位、上書き確認 |
| Integration | configApi + hooks | GET/PUT/EXPORT の一貫性 |
| E2E | Playwright `/config` | 共有 env 追加→ツール起動→CLI 起動フロー |

## 5. リスクと軽減策

| リスク | 影響 | 軽減策 |
| --- | --- | --- |
| OS env を無制限で取り込む | シークレット漏洩 | ホワイトリスト制 & ログマスク |
| 優先順位の誤理解 | 不正な値が使われる | UI/CLI で同じ説明文とログを出す |
| 保存競合 | 共有 env が上書き | `updatedAt` + ETag 風比較、競合ダイアログ |
| 履歴に値が残る | 情報漏洩 | 履歴にはキー名と操作種別のみ保存 |

## 6. 次ステップ

- この plan 承認後、`/speckit.tasks` 相当のタスクを作成。
- 既存 SPEC-8adfd99e の plan/task で重複する分は統合 or リンクする。
