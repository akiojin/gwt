# SPEC-12 Tasks

> Test-first (TDD) ordering. `[P]` = 並列実行可（書き込み対象が重ならない）。

## Phase: Setup

- [ ] **T-001** `[P]` `crates/gwt-github/Cargo.toml` 作成、`workspace.members` に追加。依存: `reqwest{blocking,rustls-tls}`, `serde`, `serde_json`, `regex`, `fs2`, `thiserror`, `sha2`.
- [ ] **T-002** `[P]` `crates/gwt-github/src/lib.rs` スケルトン（モジュール宣言のみ）。
- [ ] **T-003** `[P]` `crates/gwt-github/tests/` ディレクトリと `common/` フィクスチャを配置。
- [ ] **T-004** `[P]` `.gitignore` に `specs/` を *未追加* のまま残すが、`~/.gwt/cache` は既にユーザーホームなので無視不要であることを README に追記する下書き。

## Phase: Foundational (sections / body / routing)

- [ ] **T-010** **[TEST]** `tests/sections_test.rs`: `<!-- artifact:spec BEGIN --> ... <!-- artifact:spec END -->` の境界抽出テスト（正常 / 入れ子 / 欠損 / 重複）。**Red**。
- [ ] **T-011** `crates/gwt-github/src/sections.rs`: `SectionName` enum、正規表現、`parse_sections(raw)` 実装。**Green**。
- [ ] **T-012** **[TEST]** `tests/body_parse_test.rs`: 本文 + コメント配列から `SpecBody` を組み立てるテスト（index マップ読み取りを含む）。**Red**。
- [ ] **T-013** `crates/gwt-github/src/body.rs`: `SpecMeta`, `SpecBody::parse`, `SpecBody::render`。**Green**。
- [ ] **T-014** **[TEST]** `tests/splice_test.rs`: `SpecBody::splice(Tasks, new, &mut routing)` が他セクションを変えないことの不変条件テスト。**Red**。
- [ ] **T-015** `body::splice` 実装（純関数、副作用なし）。**Green**。
- [ ] **T-016** **[TEST]** `tests/routing_test.rs`: 15 KiB は body、17 KiB は comment、60 KiB 超本文は他セクションも降格、のルーティング決定テスト。**Red**。
- [ ] **T-017** `crates/gwt-github/src/routing.rs`: `ROUTING_PROMOTE_THRESHOLD_BYTES = 16 * 1024`、`decide_routing(&SpecBody)`。**Green**。

## Phase: Client layer

- [ ] **T-020** **[TEST]** `tests/client_contract_test.rs`: `IssueClient` trait の契約テスト（fake impl で挙動を検証）。**Red**。
- [ ] **T-021** `crates/gwt-github/src/client.rs`: `IssueClient` trait、`FetchResult`, `IssueSnapshot`, `CommentSnapshot` 型。**Green**。
- [ ] **T-022** `crates/gwt-github/src/client/fake.rs`: テスト用 in-memory 実装。
- [ ] **T-023** **[TEST]** `tests/client_http_test.rs`: reqwest 実装に対する録画フィクスチャテスト（`wiremock` 採用）。**Red**。
- [ ] **T-024** `crates/gwt-github/src/client/http.rs`: GraphQL 1 呼び出し + REST PATCH 実装。**Green**。
- [ ] **T-025** **[TEST]** `tests/client_graphql_test.rs`: `list_spec_issues` が 1 ポイントで完了することを検証。**Red**。
- [ ] **T-026** `client::http::list_spec_issues` 実装。**Green**。

## Phase: Cache layer

- [ ] **T-030** **[TEST]** `tests/cache_write_test.rs`: atomic write（tmp → rename）と flock の取得/解放テスト。**Red**。
- [ ] **T-031** `crates/gwt-github/src/cache.rs`: `cache::write_all`, `cache::load`, `cache::lock`, `cache::history_rotate`。**Green**。
- [ ] **T-032** **[TEST]** `tests/cache_history_test.rs`: 直近 3 世代ローテーションの検証。**Red**。
- [ ] **T-033** history ローテーション実装。**Green**。

## Phase: spec_ops 統合

- [ ] **T-040** **[TEST]** `tests/spec_ops_read_test.rs`: `read_section` が 304 時に cache を返し、200 時に cache を書き換えることを fake client で検証。**Red**。
- [ ] **T-041** `crates/gwt-github/src/spec_ops.rs`: `read_section`, `read_all_sections`。**Green**。
- [ ] **T-042** **[TEST]** `tests/spec_ops_write_test.rs`: body→body 書き込み（1 call）、body→comment 昇格（2 call）のそれぞれをテスト。**Red**。
- [ ] **T-043** `spec_ops::write_section` 実装。**Green**。
- [ ] **T-044** **[TEST]** `tests/spec_ops_create_test.rs`: 新規 SPEC 作成時のコール順（create_issue → N × create_comment → patch_body）の検証。**Red**。
- [ ] **T-045** `spec_ops::create_spec` 実装。**Green**。

## Phase: CLI dispatch in gwt binary

- [ ] **T-050** **[TEST]** `crates/gwt-tui/tests/cli_dispatch_test.rs`: argv 判定で TUI vs CLI モードが分かれることの統合テスト。**Red**。
- [ ] **T-051** `crates/gwt-tui/src/main.rs`: `env::args` 判定 + `clap` 導入（既存 TUI には影響なし）。**Green**。
- [ ] **T-052** `crates/gwt-tui/src/cli/issue_spec.rs`: `spec <n>` / `--section` / `--edit` ハンドラ。
- [ ] **T-053** `crates/gwt-tui/src/cli/issue_spec_list.rs`: `list` / `pull` / `repair` ハンドラ。
- [ ] **T-054** `crates/gwt-tui/src/cli/issue_spec_create.rs`: `create` ハンドラ。
- [ ] **T-055** **[TEST]** `tests/cli_e2e_test.rs`: fake client + 一時 cache ディレクトリで `gwt issue spec <n> --section tasks` が期待通り stdout 出力することを検証。**Red** → **Green**。

## Phase: Migration

- [ ] **T-060** **[TEST]** `tests/migration_dry_run_test.rs`: テスト用 `specs/` fixture に対し dry-run が期待の Issue プレビューを生成することを検証。**Red**。
- [ ] **T-061** `crates/gwt-github/src/migration.rs`: `plan` (dry-run)。**Green**。
- [ ] **T-062** **[TEST]** `tests/migration_execute_test.rs`: fake client で 11 SPEC の Issue 化シーケンスを検証。冪等性テストを含む。**Red**。
- [ ] **T-063** `migration::execute` 実装。**Green**。
- [ ] **T-064** **[TEST]** `tests/migration_docs_rewrite_test.rs`: README/CLAUDE.md/AGENTS.md 内の `specs/SPEC-N` 参照が `#<issue>` に置換されることを検証。**Red**。
- [ ] **T-065** `migration::rewrite_docs` 実装。**Green**。
- [ ] **T-066** **[TEST]** `tests/migration_rollback_test.rs`: `--rollback` が作成済み Issue を ABANDONED で close し、`specs/` を復旧することを検証。**Red**。
- [ ] **T-067** `migration::rollback` 実装。**Green**。
- [ ] **T-068** `crates/gwt-tui/src/cli/migrate_specs.rs`: `gwt issue migrate-specs` ハンドラ。

## Phase: Skill updates

- [ ] **T-070** `[P]` `.claude/skills/gwt-spec-design/SKILL.md`: 新規 SPEC 作成を `gwt issue spec create` に書き換え。
- [ ] **T-071** `[P]` `.claude/skills/gwt-spec-plan/SKILL.md`: 読み書きを `gwt issue spec <n> --section/--edit` に書き換え。
- [ ] **T-072** `[P]` `.claude/skills/gwt-spec-build/SKILL.md`: tasks 更新を `--edit tasks` に書き換え。
- [ ] **T-073** `[P]` `.claude/skills/gwt-arch-review/SKILL.md`: 横断分析を `gwt issue spec list` 経由に。
- [ ] **T-074** `[P]` `.claude/skills/gwt-issue/SKILL.md`: SPEC 委譲セクションを追加。
- [ ] **T-075** `.claude/skills/gwt-search/SKILL.md`: インデックス対象を `~/.gwt/cache/issues/` に変更、type 分離を明記。
- [ ] **T-076** `.claude/skills/gwt-spec-to-issue-migration/SKILL.md`: 廃止通知 + 新マイグレーションへの案内のみに縮小。
- [ ] **T-077** `.claude/scripts/spec_artifact.py` を削除。
- [ ] **T-078** `CLAUDE.md` / `AGENTS.md` のワークフロー記述を新 CLI ベースに更新。
- [ ] **T-079** `README.md` / `README.ja.md` の SPEC 管理セクションを更新。

## Phase: Search integration

- [ ] **T-080** **[TEST]** `tests/search_index_target_test.rs`: ChromaDB watcher の対象ディレクトリ切り替えを検証。**Red**。
- [ ] **T-081** `gwt-search` の index 対象を `~/.gwt/cache/issues/` に切替、チャンクの `type` メタデータを `spec` / `issue` で分離。**Green**。
- [ ] **T-082** **[TEST]** `tests/startup_sync_test.rs`: `gwt` 起動時の 1 GraphQL 呼び出しによる差分同期を検証。**Red**。
- [ ] **T-083** `gwt-tui::startup` にバックグラウンド同期を実装。**Green**。

## Phase: Real API rehearsal

- [ ] **T-090** テスト用 GitHub リポジトリ（例: `akiojin/gwt-spec-sandbox`）で `migrate-specs --dry-run` 実行、件数/配分を確認。
- [ ] **T-091** 同リポジトリで `--execute` 実行、作成 Issue を確認、`--rollback` で戻す検証。
- [ ] **T-092** 本リポジトリで `--dry-run` 実行、プレビューをレビュー。

## Phase: Execute real migration

- [ ] **T-100** 本リポジトリで `gwt issue migrate-specs --execute` 実行（作業ブランチ: 現 `feature/specs`）。
- [ ] **T-101** `migration-report.json` を確認、11 SPEC が全成功していることを検証。
- [ ] **T-102** `git rm -rf specs/` + ドキュメント置換を含む単一 commit を作成（`refactor(specs)!: migrate local specs to GitHub Issues`）。
- [ ] **T-103** 旧 `.claude/scripts/spec_artifact.py` と `.claude/skills/gwt-spec-to-issue-migration/scripts/*.{mjs,py}` を削除。

## Phase: TUI integration

- [ ] **T-110** **[TEST]** `crates/gwt-tui/tests/spec_view_test.rs`: SPEC 一覧 / 詳細 / セクション切替の UI スナップショット。**Red**。
- [ ] **T-111** SPEC 一覧画面を cache 駆動で実装。**Green**。
- [ ] **T-112** SPEC 詳細画面・セクション切替 UI の実装。
- [ ] **T-113** フェーズ変更操作（ラベル更新）を REST PATCH 経由で実装。

## Phase: Polish / Quality gate

- [ ] **T-120** `cargo test -p gwt-core -p gwt-tui -p gwt-github` 全緑。
- [ ] **T-121** `cargo clippy --all-targets --all-features -- -D warnings` 全緑。
- [ ] **T-122** `cargo fmt --check` パス。
- [ ] **T-123** `bunx commitlint --from HEAD~<N> --to HEAD` パス。
- [ ] **T-124** `tasks/lessons.md` に「セクション境界マーカー手動編集禁止」など本 SPEC で得た学びを追記。
- [ ] **T-125** PR 作成: `feature/specs` → `develop`、タイトル `refactor(specs)!: migrate SPEC management to GitHub Issues`。

## Traceability

| User Story | 主要タスク |
|---|---|
| US-1: 1 API コール読取 | T-020〜T-026, T-040〜T-041 |
| US-2: セクション粒度読取 | T-010〜T-015, T-040〜T-041, T-052 |
| US-3: セクション単位書込 | T-014〜T-015, T-042〜T-043, T-052 |
| US-4: 64 KiB 自動回避 | T-016〜T-017, T-042〜T-043 |
| US-5: 一括マイグレーション | T-060〜T-068, T-090〜T-103 |
| US-6: スキルインプレース更新 | T-070〜T-079 |
| US-7: 起動時インデックス | T-080〜T-083 |
| US-8: TUI SPEC 操作 | T-110〜T-113 |
