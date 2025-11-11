# 実装計画: Web UI 環境変数編集機能

**仕様ID**: `SPEC-8adfd99e` | **日付**: 2025-11-11 | **仕様書**: [spec.md](./spec.md)
**概要**: Web UI から `~/.claude-worktree/tools.json` 内のカスタムAIツール環境変数を安全に閲覧・編集できるようにし、CLI との挙動を完全に一致させる。

## 1. 前提・対象範囲

- 既存の CustomAITool スキーマ（`src/types/api.ts` / `src/types/tools.ts`）をそのまま利用し、`env` を key/value 連想配列として扱う。
- Web UI では **Custom Tool 全体**ではなく「環境変数セクション」に機能範囲を限定する。defaultArgs や modeArgs の編集は別仕様とする。
- Fastify サーバー（`src/web/server`）はローカルホストのみで動作し、HTTPS やリモート公開を考慮しない。
- CLI 側の挙動は `tools.json` の値を直接読むため、Web UI で保存＝即 CLI でも有効となる。

## 2. 成功基準との対応

| 成功基準 (spec) | 計画での対策 |
| --- | --- |
| SC-001: 2分以内に env を追加 | UI を 3 ステップ（追加→入力→保存）に絞り、リアルタイムバリデーションで迷いを減らす |
| SC-002: CLI と値が一致 | 保存時に `tools.json` を上書きし、finish 後に React Query で再フェッチ → BranchDetail 側でも同データを参照 |
| SC-003: 無効値を 100% 防止 | API/フロントの二重バリデーション（正規表現 + 長さ）と、空フィールドが残ると保存できない UI |
| SC-004: 平文露出 0 件 | デフォルトはマスク表示、表示リクエストはクライアントメモリ内のみで保持し API では常にマスク済み文字列を返す |

## 3. アーキテクチャ方針

### 3.1 サーバー (Fastify)

1. `src/web/server/routes/config.ts`
   - `GET /api/config`: 既存の空配列レスポンスを置き換え、`src/config/tools.ts` の `loadToolsConfig()` を呼び `customTools` を返す。
   - `PUT /api/config`: 受信した `UpdateConfigRequest` を `validateToolsConfig()` で検証後、`~/.claude-worktree/tools.json` へ保存。保存前後で `updatedAt` を比較し、競合（If-Match 代替）として 409 を返す仕組みを追加。
   - 新規ヘルパー `saveToolsConfig(config: ToolsConfig)` を `src/config/tools.ts` に追加して reuse。
   - 例外ハンドリング時はログへ `***masked***` を出力し、HTTP レスポンスには一般化したエラー文言を返す。

2. 同期制御
   - Node.js `fs.promises.writeFile` を `mode: 0o600` で利用。
   - 書き込み時にテンポラリファイル (`tools.json.tmp`) に出力→`rename` でアトミック更新。

### 3.2 クライアント (React / Vite)

1. ルーティング
   - `src/web/client/src/router.tsx` に `/config` パスを追加。
   - BranchDetail の「カスタムツール設定を開く」リンクは既に存在するためそのまま利用。

2. 状態管理
   - 既存の `useConfig` フックを拡張し、`tools` のほかに `updatedAt` と `version` を保持。
   - 保存 API 用に `useUpdateConfig` を実装（既にフック作成済みなので `ConfigManagementPage` で使用）。

3. UI コンポーネント
   - `pages/ConfigManagementPage.tsx`（新規）: 左カラムでツール一覧、右カラムで選択ツールの env 編集フォームを表示。
   - `components/EnvEditor.tsx`（新規）: 
     - 行操作（追加/削除/並び替え）
     - キー入力は即時大文字化（A-Z,0-9,_）
     - 値はパスワード入力 type="password" + 「表示」トグル
     - 行ごとに `dirty` 状態を保持し、保存前に差分を抽出
   - 成功/失敗は `InlineBanner` + Toast で通知。

4. バリデーション UX
   - 保存ボタンは `invalidRows.length === 0` かつ 1 つ以上の変更がある場合のみ有効。
   - エラーメッセージは行にインライン表示（例: 「キーは半角大文字と数字、アンダースコアのみ」）。

### 3.3 CLI との連携

- `launchCustomAITool` は `tool.env` を `process.env` にマージする既存挙動を活用。Web での編集後に CLI で再読み込みされることを `tests/unit/launcher.test.ts` で検証（モック `loadToolsConfig` → env 反映）。

## 4. 実装ステップ (ハイレベル ToDo)

1. **サーバー層**
   - [ ] `src/config/tools.ts` に `saveToolsConfig` と `formatToolsForApi` を追加
   - [ ] `routes/config.ts` を実装（GET/PUT）し、JSON スキーマバリデーションとエラーハンドリングを追加
   - [ ] `tests/web/server/routes/config.test.ts` を新設し、正常系/バリデーション/権限エラー/競合をカバー

2. **クライアント層**
   - [ ] React Router に `/config` ルートを追加
   - [ ] `ConfigManagementPage` + `EnvEditor` コンポーネントを作成
   - [ ] `useConfig` / `useUpdateConfig` をフロントニーズに合わせて拡張
   - [ ] `tests/web/client/pages/config-management.test.tsx` で UI 操作を検証

3. **統合/回帰**
   - [ ] BranchDetail から `/config` 遷移後に React Query キャッシュが共有されるか確認
   - [ ] `launchCustomAITool` に関する既存ユニットテストを追加し、Web 保存→CLI 反映のケースを再現
   - [ ] `bun run build` + `bun test`（web/server, web/client）を実行

## 5. テスト戦略

| レベル | 対象 | シナリオ |
| --- | --- | --- |
| 単体 (server) | `config.routes` | 正常保存, JSON 構文エラー, 不正キー, 書き込み失敗, 競合 |
| 単体 (client) | `EnvEditor` | 行追加/削除, バリデーション, マスク表示, 保存ボタン制御 |
| 統合 | `configApi` + React Query | PUT 成功後にキャッシュが `tools` を更新 |
| E2E | Playwright (将来) | `/config` で env 追加→保存→BranchDetail でツール起動 |

## 6. リスクと軽減策

| リスク | 影響 | 対策 |
| --- | --- | --- |
| 同時編集による競合 | ファイルが上書きされる | `updatedAt` フィールドで競合検出、UI で再読み込みと上書きの明示的選択肢 |
| 秘匿情報の露出 | API レスポンスに平文が乗る | サーバーは値をそのまま返さず、クライアントでのみ平文を保持。必要であれば将来的に暗号化を検討 |
| 書き込み権限不足 | 保存できず体験劣化 | エラー詳細に `tools.json` のパス/権限ガイドを含め、ドキュメント README へリンク |
| 既存 CLI との乖離 | Web で保存しても CLI に反映されない | `tools.json` を単一ソースとして利用し、CLI 側のキャッシュ（もしあれば）をリセットするヘルパーを実装 |

## 7. オープン質問

1. 競合検知は単純な `updatedAt` で十分か、それともファイルハッシュ比較が必要か？（暫定: updatedAt）
2. 値の一時表示時間（仕様では 3 秒/10 秒のどちらか）を確定する必要あり（暫定 3 秒）。

---

次ステップ: この plan.md 承認後、/speckit.tasks 相当のタスクリストを作成し、TDD（テスト → 実装）の順で着手する。
